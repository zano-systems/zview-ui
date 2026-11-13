"""Broker system-native dialogs on the local machine; no Python-rendered UI."""

import asyncio
import contextlib
import json
import os
import shutil
import sys

from paths import input_path, local_path

KINDS = {"directory", "elf", "recording"}
TITLES = {
    "directory": "Choose build directory",
    "elf": "Choose matching ELF",
    "recording": "Choose recording",
}

# Arguments carry paths and prompts; none are interpolated into executable code.
MAC_SCRIPT = r"""
function run(argv) {
    const app = Application.currentApplication();
    app.includeStandardAdditions = true;
    const options = {withPrompt: argv[2], defaultLocation: Path(argv[1])};
    try {
        let selected;
        if (argv[0] === "directory") {
            selected = app.chooseFolder(options);
        } else {
            options.ofType = argv[0] === "elf" ? ["elf"] : ["gz"];
            selected = app.chooseFile(options);
        }
        return JSON.stringify(String(selected));
    } catch (error) {
        if (error.errorNumber === -128 || error.number === -128) return "null";
        throw error;
    }
}
"""

WINDOWS_SCRIPT = r"""
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
Add-Type -AssemblyName System.Windows.Forms
if ($env:ZVIEW_PICKER_KIND -eq 'directory') {
    $dialog = New-Object System.Windows.Forms.FolderBrowserDialog
    $dialog.SelectedPath = $env:ZVIEW_PICKER_INITIAL
    $dialog.Description = $env:ZVIEW_PICKER_TITLE
    $dialog.ShowNewFolderButton = $false
} else {
    $dialog = New-Object System.Windows.Forms.OpenFileDialog
    $dialog.InitialDirectory = $env:ZVIEW_PICKER_INITIAL
    $dialog.Title = $env:ZVIEW_PICKER_TITLE
    $dialog.Multiselect = $false
    if ($env:ZVIEW_PICKER_KIND -eq 'elf') {
        $dialog.Filter = 'ELF files (*.elf)|*.elf|All files (*.*)|*.*'
    } else {
        $dialog.Filter = 'ZView recordings (*.ndjson.gz)|*.ndjson.gz|All files (*.*)|*.*'
    }
}
try {
    if ($dialog.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) {
        if ($env:ZVIEW_PICKER_KIND -eq 'directory') {
            ConvertTo-Json -Compress -InputObject $dialog.SelectedPath
        } else {
            ConvertTo-Json -Compress -InputObject $dialog.FileName
        }
    } else { Write-Output 'null' }
} finally { $dialog.Dispose() }
"""


def initial_directory(root, current=None, project=None):
    """Existing field location first, otherwise current project/data root."""
    try:
        default = local_path(root, project or ".")
        if not default.is_dir():
            default = root
    except (ValueError, OSError, RuntimeError):
        default = root
    if current is None or current == "":
        return default
    if not isinstance(current, str):
        raise ValueError("Current selection must be a path string.")
    try:
        candidate = (root / current).resolve()
        if candidate.is_relative_to(root):
            if candidate.is_dir():
                return candidate
            if candidate.parent.is_dir():
                return candidate.parent
    except (ValueError, OSError, RuntimeError):
        pass
    return default


def validate_selection(root, selected, kind):
    if selected is None:
        return None
    if not isinstance(selected, str) or not selected:
        raise ValueError("The native dialog returned an invalid path.")
    if kind == "directory":
        path = local_path(root, selected)
        if not path.is_dir():
            raise ValueError("Choose a build directory.")
        return path.relative_to(root).as_posix()
    path, label = input_path(root, selected)
    suffix = ".elf" if kind == "elf" else ".ndjson.gz"
    if not path.name.lower().endswith(suffix):
        raise ValueError(f"Choose a {suffix} file.")
    return label


def native_command(kind, initial, *, platform=None, which=None):
    """Return an argv vector, environment and output format; never use a shell."""
    if kind not in KINDS:
        raise ValueError("Unknown native dialog kind.")
    platform = platform or sys.platform
    which = which or shutil.which
    title = TITLES[kind]
    if platform == "darwin":
        return (
            ["/usr/bin/osascript", "-l", "JavaScript", "-e", MAC_SCRIPT,
             "--", kind, str(initial), title],
            None, "json",
        )
    if platform == "win32":
        powershell = which("powershell.exe")
        if not powershell:
            raise ValueError("Windows PowerShell is required for the native dialog. Enter a path manually.")
        env = {**os.environ, "ZVIEW_PICKER_KIND": kind,
               "ZVIEW_PICKER_INITIAL": str(initial), "ZVIEW_PICKER_TITLE": title}
        return (
            [powershell, "-NoProfile", "-NonInteractive", "-STA", "-Command", WINDOWS_SCRIPT],
            env, "json",
        )
    if platform.startswith("linux"):
        zenity = which("zenity")
        if zenity:
            args = [zenity, "--file-selection", f"--title={title}",
                    f"--filename={str(initial).rstrip(os.sep)}{os.sep}"]
            if kind == "directory":
                args.append("--directory")
            else:
                pattern = "*.elf *.ELF" if kind == "elf" else "*.ndjson.gz *.NDJSON.GZ"
                args.append(f"--file-filter={title} | {pattern}")
            return args, None, "line"
        kdialog = which("kdialog")
        if kdialog:
            if kind == "directory":
                args = [kdialog, "--getexistingdirectory", str(initial)]
            else:
                pattern = "*.elf *.ELF" if kind == "elf" else "*.ndjson.gz *.NDJSON.GZ"
                args = [kdialog, "--getopenfilename", str(initial), f"{pattern}|{title}"]
            return [*args, "--title", title], None, "line"
        raise ValueError("Install zenity or kdialog for native dialogs on Linux, or enter a path manually.")
    raise ValueError("Native dialogs are unavailable on this platform. Enter a path manually.")


class NativeDialogBusy(Exception):
    pass


async def terminate(process):
    if process.returncode is None:
        with contextlib.suppress(ProcessLookupError):
            process.terminate()
        try:
            await asyncio.wait_for(process.wait(), timeout=2)
        except asyncio.TimeoutError:
            with contextlib.suppress(ProcessLookupError):
                process.kill()
            await process.wait()


class NativeDialogs:
    """One native dialog at a time, outside the web event loop, with shutdown cleanup."""

    def __init__(self, timeout=300):
        self.timeout = timeout
        self._lock = asyncio.Lock()
        self._process = None
        self._closed = False

    async def choose(self, kind, initial):
        if self._closed:
            raise ValueError("The adapter is shutting down.")
        if self._lock.locked():
            raise NativeDialogBusy("A system dialog is already open. Finish or cancel it first.")
        async with self._lock:
            args, env, output_format = native_command(kind, initial)
            process = await asyncio.create_subprocess_exec(
                *args, env=env, stdin=asyncio.subprocess.DEVNULL,
                stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE,
            )
            self._process = process
            try:
                if self._closed:
                    return None
                try:
                    stdout, stderr = await asyncio.wait_for(
                        process.communicate(), timeout=self.timeout
                    )
                except asyncio.TimeoutError:
                    raise ValueError("The system dialog timed out. Try again or enter a path manually.") from None
                if self._closed:
                    return None
                if process.returncode != 0:
                    # Linux choosers use exit 1 for Cancel; JXA may report -128.
                    if (output_format == "line" and process.returncode == 1 and not stderr.strip()) or b"(-128)" in stderr:
                        return None
                    detail = stderr.decode("utf-8", errors="replace")[:2048].strip()
                    raise ValueError(f"Cannot open the system dialog: {detail or process.returncode}")
                text = stdout.decode("utf-8")
                if output_format == "json":
                    return json.loads(text)
                # Remove only the command's final newline, not filename whitespace.
                return text.removesuffix("\n") or None
            finally:
                await terminate(process)
                if self._process is process:
                    self._process = None

    async def close(self):
        self._closed = True
        if self._process is not None:
            await terminate(self._process)

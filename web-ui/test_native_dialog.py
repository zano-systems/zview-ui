"""Native command construction and lifecycle, without opening unattended OS dialogs."""

import asyncio
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from aiohttp.test_utils import AioHTTPTestCase

from native_dialog import (
    NativeDialogBusy, NativeDialogs, initial_directory, native_command, validate_selection,
)
from server import CONTROLLER, create_app
from session import Session


class NativePathTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        (self.root / "project").mkdir()
        (self.root / "project/zephyr.elf").write_text("fixture")
        (self.root / "project/capture.ndjson.gz").write_text("fixture")

    def test_default_project_current_folder_and_file_parent(self):
        self.assertEqual(initial_directory(self.root), self.root)
        for current in (None, "", "missing/deep/file", "../outside"):
            self.assertEqual(initial_directory(self.root, current, "project"), self.root / "project")
        self.assertEqual(initial_directory(self.root, "project"), self.root / "project")
        self.assertEqual(initial_directory(self.root, "project/zephyr.elf"), self.root / "project")
        self.assertEqual(initial_directory(self.root, "project/missing.elf"), self.root / "project")
        self.assertEqual(initial_directory(self.root, project="missing"), self.root)
        with self.assertRaises(ValueError):
            initial_directory(self.root, current=12)

    def test_selection_validation_preserves_cancellation_and_relative_paths(self):
        self.assertIsNone(validate_selection(self.root, None, "elf"))
        for kind, selection in (("directory", "project"), ("elf", "project/zephyr.elf"),
                                ("recording", "project/capture.ndjson.gz")):
            self.assertEqual(validate_selection(self.root, str(self.root / selection), kind), selection)
        for selected, kind in (("../outside", "directory"), ("project", "elf"),
                               ("project/zephyr.elf", "recording"), ("", "elf")):
            with self.assertRaises((ValueError, OSError)):
                validate_selection(self.root, selected, kind)

    @unittest.skipIf(os.name == "nt", "Symlinks can require Windows privileges.")
    def test_external_symlink_selection_is_rejected(self):
        with tempfile.TemporaryDirectory() as external:
            (self.root / "outside").symlink_to(external, target_is_directory=True)
            with self.assertRaises(ValueError):
                validate_selection(self.root, str(self.root / "outside"), "directory")

    def test_native_commands_keep_paths_out_of_executable_script_text(self):
        initial = Path("/project with spaces/quoted';name")
        command, env, output = native_command("elf", initial, platform="darwin")
        self.assertEqual(command[0], "/usr/bin/osascript")
        self.assertEqual(command[-3:], ["elf", str(initial), "Choose matching ELF"])
        self.assertNotIn(str(initial), command[command.index("-e") + 1])
        self.assertEqual(output, "json")
        command, env, output = native_command("elf", initial, platform="win32", which=lambda _: "powershell.exe")
        self.assertNotIn(str(initial), command[-1])
        self.assertEqual(env["ZVIEW_PICKER_INITIAL"], str(initial))
        self.assertIn("-STA", command)
        command, _, output = native_command("directory", initial, platform="linux", which=lambda p: "/usr/bin/zenity" if p == "zenity" else None)
        self.assertIn("--directory", command)
        self.assertIn(f"--filename={str(initial).rstrip(os.sep)}{os.sep}", command)
        self.assertEqual(output, "line")
        command, _, _ = native_command("recording", initial, platform="linux", which=lambda p: "/usr/bin/kdialog" if p == "kdialog" else None)
        self.assertIn("--getopenfilename", command)
        self.assertTrue(any("*.ndjson.gz" in a for a in command))
        for platform in ("linux", "win32", "other"):
            with self.assertRaises(ValueError):
                native_command("elf", initial, platform=platform, which=lambda _: None)


class NativeProcessTests(unittest.IsolatedAsyncioTestCase):
    def command(self, code, output="json"):
        return patch("native_dialog.native_command", return_value=(
            [sys.executable, "-c", code], None, output
        ))

    async def active(self, service):
        for _ in range(100):
            if service._process is not None:
                return service._process
            await asyncio.sleep(0.01)
        self.fail("Native helper did not start.")

    async def test_result_cancel_and_error_protocols(self):
        service = NativeDialogs()
        for code, output, expected in (
            ("print('\"/project/a file.elf\"')", "json", "/project/a file.elf"),
            ("print('null')", "json", None),
            ("print('/project/a file.elf')", "line", "/project/a file.elf"),
            ("import sys; sys.exit(1)", "line", None),
            ("import sys; sys.stderr.write('User canceled. (-128)'); sys.exit(1)", "json", None),
        ):
            with self.command(code, output):
                self.assertEqual(await service.choose("elf", Path(".")), expected)
            self.assertIsNone(service._process)
        with self.command("import sys; sys.stderr.write('No display'); sys.exit(2)"):
            with self.assertRaisesRegex(ValueError, "No display"):
                await service.choose("elf", Path("."))
        with self.command("import sys; sys.stderr.write('No display'); sys.exit(1)", "line"):
            with self.assertRaisesRegex(ValueError, "No display"):
                await service.choose("elf", Path("."))
        with self.command("print('not JSON')"):
            with self.assertRaises(ValueError):
                await service.choose("elf", Path("."))
        await service.close()

    async def test_busy_and_shutdown_close_the_helper(self):
        service = NativeDialogs()
        with self.command("import time; time.sleep(30)"):
            task = asyncio.create_task(service.choose("elf", Path(".")))
            process = await self.active(service)
            with self.assertRaises(NativeDialogBusy):
                await service.choose("elf", Path("."))
            await service.close()
            self.assertIsNone(await task)
            self.assertIsNotNone(process.returncode)
            self.assertIsNone(service._process)
            await service.close()
            with self.assertRaises(ValueError):
                await service.choose("elf", Path("."))

    async def test_timeout_and_task_cancellation_release_the_process(self):
        service = NativeDialogs(timeout=0.03)
        with self.command("import time; time.sleep(30)"):
            with self.assertRaisesRegex(ValueError, "timed out"):
                await service.choose("elf", Path("."))
        self.assertIsNone(service._process)
        service = NativeDialogs()
        with self.command("import time; time.sleep(30)"):
            task = asyncio.create_task(service.choose("elf", Path(".")))
            process = await self.active(service)
            task.cancel()
            with self.assertRaises(asyncio.CancelledError):
                await task
            self.assertIsNotNone(process.returncode)
            self.assertIsNone(service._process)


class ScriptedDialogs:
    """The native OS boundary is replaced, not the HTTP or Rust command path."""
    def __init__(self):
        self.result = None
        self.requests = []
        self.closed = False
        self.wait = None
        self.entered = asyncio.Event()

    async def choose(self, kind, initial):
        self.requests.append((kind, initial))
        self.entered.set()
        if self.wait is not None:
            await self.wait.wait()
        return self.result

    async def close(self):
        self.closed = True


class NativeDialogApiTests(AioHTTPTestCase):
    async def get_application(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        (self.root / "project").mkdir()
        (self.root / "project/zephyr.elf").write_text("fixture")
        (self.root / "project/capture.ndjson.gz").write_text("fixture")
        self.dialogs = ScriptedDialogs()
        return create_app(self.root, dialogs=self.dialogs)

    async def test_default_project_selection_and_cancel_do_not_change_session(self):
        self.app[CONTROLLER].session = Session(
            "existing", {"mode": "replay", "recording": "recording"}, "elf",
            status="ended", context={"source_dir": "project"}
        )
        for kind, expected in (("directory", "project"), ("elf", "project/zephyr.elf"),
                               ("recording", "project/capture.ndjson.gz")):
            self.dialogs.result = str(self.root / expected)
            response = await self.client.post("/api/files/choose", json={"kind": kind})
            self.assertEqual(response.status, 200)
            self.assertEqual((await response.json())["path"], expected)
            self.assertEqual(self.dialogs.requests[-1], (kind, self.root / "project"))
        self.dialogs.result = None
        response = await self.client.post("/api/files/choose", json={"kind": "elf"})
        self.assertIsNone((await response.json())["path"])
        snapshot = await (await self.client.get("/api/session")).json()
        self.assertEqual(snapshot["session_id"], "existing")
        self.assertEqual(snapshot["data"]["status"], "ended")

    async def test_validation_root_boundary_and_http_security(self):
        for body in ([], {}, {"kind": []}, {"kind": "unknown"},
                     {"kind": "elf", "current": 12}, {"kind": "elf", "unexpected": True}):
            response = await self.client.post("/api/files/choose", json=body)
            self.assertEqual(response.status, 400, body)
        self.assertFalse(self.dialogs.requests)
        for headers in ({"Origin": "https://untrusted.example"},
                        {"Host": "untrusted.example"}, {"Sec-Fetch-Site": "cross-site"}):
            response = await self.client.post("/api/files/choose", json={"kind": "elf"}, headers=headers)
            self.assertEqual(response.status, 403)
        self.assertEqual((await self.client.post("/api/files/choose", data="")).status, 415)
        self.dialogs.result = str(self.root.parent)
        response = await self.client.post("/api/files/choose", json={"kind": "directory"})
        self.assertEqual(response.status, 400)
        self.assertIsNone((await (await self.client.get("/api/session")).json())["data"])
        # The custom filesystem browser no longer exists.
        self.assertEqual((await self.client.post("/api/files/list", json={})).status, 405)


if __name__ == "__main__":
    unittest.main()

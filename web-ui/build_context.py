"""Resolve build context without opening a probe or changing an active session."""

import subprocess
from pathlib import Path

import yaml
from zview.runner_config import find_elf, image_dir
from zview.target import AVAILABLE_RUNNERS, Target

from paths import input_path, local_path


def artifact(root, path):
    """Validate generated artifacts too: a build can contain external symlinks."""
    resolved = path.resolve()
    if not resolved.is_relative_to(root):
        raise ValueError("Build artifacts must stay inside the configured data root.")
    if not resolved.exists():
        return None
    return input_path(root, str(resolved))[0]


def cache_values(path):
    values = {}
    if path:
        for line in path.read_text().splitlines():
            if "=" in line and not line.startswith(("#", "//")):
                key, value = line.split("=", 1)
                values[key.split(":", 1)[0]] = value
    return values


def board_name(config, cache):
    values = {}
    if config:
        for line in config.read_text().splitlines():
            if line.startswith(("CONFIG_BOARD_TARGET=", "CONFIG_BOARD=")):
                key, value = line.split("=", 1)
                values[key] = value.strip('"')
    return (
        values.get("CONFIG_BOARD_TARGET")
        or cache.get("BOARD")
        or values.get("CONFIG_BOARD")
        or None
    )


def checkout_context(root, source):
    """Current checkout only, never evidence of which commit produced an ELF."""
    if not source:
        return None, None
    source_path = Path(source).resolve()
    if not source_path.is_relative_to(root) or not source_path.is_dir():
        return None, None

    def git(*args):
        return subprocess.check_output(
            ["git", "--no-optional-locks", "-C", str(source_path), *args],
            text=True, stderr=subprocess.DEVNULL, timeout=2,
        ).strip()

    try:
        commit = git("rev-parse", "--verify", "HEAD")
        branch = git("branch", "--show-current") or None
        dirty = bool(git("status", "--porcelain", "--untracked-files=no", "--", "."))
        return source_path.relative_to(root).as_posix(), {
            "commit": commit, "branch": branch, "dirty": dirty,
            "provenance": "current_checkout",
        }
    except (OSError, subprocess.SubprocessError):
        return source_path.relative_to(root).as_posix(), None


def live_runner(path, warnings):
    """Prefer the build's attach/debug runner, not its flashing command."""
    if path is None:
        return None
    config = yaml.safe_load(path.read_text())
    if not isinstance(config, dict):
        raise ValueError("runners.yaml must contain a runner configuration.")
    listed = config.get("runners", [])
    if not isinstance(listed, list) or not all(isinstance(r, str) for r in listed):
        raise ValueError("runners.yaml has an invalid runner list.")
    debug = config.get("debug-runner")
    if debug in AVAILABLE_RUNNERS:
        return debug
    if debug:
        warnings.append(f"Build debug runner '{debug}' is not supported by ZView.")
    flash = config.get("flash-runner")
    if flash in {"jlink", "pyocd", "gdb"}:
        return flash
    # A persistent probe backend avoids starting a flashing-tool subprocess for
    # every memory read. Keep the build's ordering among its supported choices.
    detected = next((runner for runner in listed if runner in {"jlink", "pyocd"}), None)
    if detected is not None:
        warnings.append(f"Using build-configured '{detected}' for Live reads instead of flash runner '{flash}'.")
    return detected


def resolve_build(root, data):
    allowed = {"mode", "build_dir", "domain", "elf", "runner", "target", "frequency"}
    if not isinstance(data, dict) or set(data) - allowed:
        raise ValueError("Unsupported build configuration fields.")
    mode = data.get("mode")
    if mode not in {"live", "replay"}:
        raise ValueError("Mode must be live or replay.")
    for key in ("domain", "elf", "runner", "target"):
        value = data.get(key)
        if value is not None and (not isinstance(value, str) or not value.strip()):
            raise ValueError(f"{key} must be a non-empty string when provided.")
    frequency = data.get("frequency")
    if frequency is not None and (type(frequency) is not int or frequency <= 0):
        raise ValueError("Probe frequency must be a positive integer in Hz.")
    if data.get("runner") is not None and data["runner"] not in AVAILABLE_RUNNERS:
        raise ValueError(f"Supported runners: {', '.join(AVAILABLE_RUNNERS)}.")
    if mode == "replay" and any(data.get(k) is not None for k in ("runner", "target", "frequency")):
        raise ValueError("Replay does not use live connection overrides.")
    build = local_path(root, data.get("build_dir"))
    if not build.is_dir():
        raise ValueError("Build directory must be a directory.")

    domains_file = artifact(root, build / "domains.yaml")
    domains = []
    selected = data.get("domain")
    if domains_file:
        config = yaml.safe_load(domains_file.read_text())
        if not isinstance(config, dict) or not isinstance(config.get("domains"), list):
            raise ValueError("Malformed domains.yaml.")
        domains = [d["name"] for d in config["domains"]]
        if not all(isinstance(d, str) for d in domains):
            raise ValueError("Firmware image names must be strings.")
        selected = selected or config.get("default")
    else:
        domains = ["app"]
        selected = selected or "app"

    # The package remains authoritative for sysbuild path/default resolution.
    image = local_path(root, str(image_dir(build, selected)))
    if not image.is_dir():
        raise ValueError("Firmware image build directory must be a directory.")
    runners = artifact(root, image / "zephyr" / "runners.yaml")
    elf_override = data.get("elf")
    elf_override = input_path(root, elf_override)[0] if elf_override else None
    warnings = []
    if mode == "live":
        runner = data.get("runner") or live_runner(runners, warnings)
        target = Target.from_build_dir(
            build, selected, elf=str(elf_override) if elf_override else None,
            runner=runner, target=data.get("target"),
            frequency=frequency, warn=warnings.append,
        )
        elf = input_path(root, str(target.elf))[0]
        runner, mcu, clock = target.runner, target.target, target.frequency
    else:
        elf = input_path(root, str(elf_override or find_elf(image)))[0]
        runner, mcu, clock = None, None, None

    cache = cache_values(artifact(root, image / "CMakeCache.txt"))
    board = board_name(artifact(root, image / "zephyr" / ".config"), cache)
    source = cache.get("APPLICATION_SOURCE_DIR")
    source_dir, git = checkout_context(root, source)
    if elf_override:
        warnings.append("ELF override selected. Ensure it matches this build and recording.")
    if not board:
        warnings.append("Board is unknown: no board identity was found in the build.")
    return {
        "build_dir": build.relative_to(root).as_posix(),
        "image_dir": image.relative_to(root).as_posix(),
        "domain": selected, "domains": domains,
        "elf": elf.relative_to(root).as_posix(),
        "elf_override": elf_override.relative_to(root).as_posix() if elf_override else None,
        "board": board,
        "project": Path(source).name if source else build.name,
        "source_dir": source_dir, "git": git,
        "runner": runner, "target": mcu, "frequency": clock,
        "warnings": warnings,
    }

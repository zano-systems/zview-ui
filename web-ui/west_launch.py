"""Pure helpers for ``west zview-ui``: no west, aiohttp or ZView imports."""

import os
import re
from pathlib import Path

HERE = Path(__file__).resolve().parent
REQUIREMENTS = HERE / "requirements.txt"
LIVE_OVERRIDES = ("domain", "elf", "runner", "target", "frequency")


def zview_pin(requirements=REQUIREMENTS):
    """The ZView commit the adapter is pinned to, from its requirements file."""
    match = re.search(r"^zview\s*@\s*git\+\S+@([0-9a-f]{7,40})\s*$",
                      Path(requirements).read_text(), re.MULTILINE)
    if match is None:
        raise ValueError(f"No pinned zview commit in {requirements}.")
    return match.group(1)


def application_source(build_dir):
    """The application source directory recorded in ``CMakeCache.txt``, if any."""
    cache = Path(build_dir) / "CMakeCache.txt"
    if not cache.is_file():
        return None
    for line in cache.read_text(errors="replace").splitlines():
        if line.startswith("APPLICATION_SOURCE_DIR:"):
            path = Path(line.split("=", 1)[1].strip())
            return path.resolve() if path.is_dir() else None
    return None


def data_root_for(build_dir, topdir=None, extra=()):
    """
    The narrowest trusted root holding the build, its sources and ``extra`` paths.
    The west workspace is preferred when it holds them all. When their only common
    ancestor is the filesystem root, only the build directory (and ``extra``) is used.
    """
    build = Path(build_dir).resolve()
    required = [build, *(Path(p).resolve() for p in extra)]
    source = application_source(build)
    wanted = [*required, *([source] if source else [])]
    if topdir is not None:
        top = Path(topdir).resolve()
        if all(p.is_relative_to(top) for p in wanted):
            return top
    for paths in (wanted, required):
        common = Path(os.path.commonpath([str(p) for p in paths]))
        if common != Path(common.anchor):
            return common if common.is_dir() else common.parent
    raise ValueError("The build and its inputs share no directory but the filesystem root.")


def live_request(root, build_dir, **overrides):
    """The adapter's Live configuration: root-relative paths, only the overrides set."""
    root = Path(root).resolve()
    request = {"build_dir": Path(build_dir).resolve().relative_to(root).as_posix()}
    for key in LIVE_OVERRIDES:
        value = overrides.get(key)
        if value is None:
            continue
        if key == "elf":
            value = Path(value).resolve().relative_to(root).as_posix()
        request[key] = value
    return request

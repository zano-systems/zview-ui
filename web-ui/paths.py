"""File access shared by build resolution and replay; all paths are server-local."""

from pathlib import Path

INPUT_LIMIT = 32 * 1024 * 1024


def local_path(root, value):
    if not isinstance(value, str) or not value.strip():
        raise ValueError("Provide a non-empty server-local path.")
    path = (root / value).resolve(strict=True)
    if not path.is_relative_to(root):
        raise ValueError("Paths must stay inside the configured data root.")
    return path


def input_path(root, value):
    path = local_path(root, value)
    if not path.is_file():
        raise ValueError("Inputs must be regular files.")
    if path.stat().st_size > INPUT_LIMIT:
        raise ValueError("Each input is limited to 32 MiB.")
    return path, path.relative_to(root).as_posix()

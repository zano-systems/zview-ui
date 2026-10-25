#!/usr/bin/env python3
"""Build the shared Rust UI and serve only its static output and selected JSON."""
import argparse
import functools
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlsplit
from urllib.request import urlopen
import webbrowser

ROOT = Path(__file__).resolve().parents[1]
SITE = ROOT / "target" / "web"
LIMIT = 16 * 1024 * 1024
BINDGEN_VERSION = "0.2.129"  # Must match Cargo.toml and Cargo.lock.
FONT_BASE = (
    "https://raw.githubusercontent.com/longbridge/gpui-kit/"
    "67265a36e260ffe03c5aaebdcd8d654dbf892eab/crates/story-web/fonts/"
)
FONTS = {
    "IBMPlexSans-Regular.source.ttf": "975dcda37d80f038dcd143c22e33ca2d97a0cc5a929aace1c749153b0fe1afa5",
    "JetBrainsMono-Regular.source.ttf": "a0bf60ef0f83c5ed4d7a75d45838548b1f6873372dfac88f71804491898d138f",
    "OFL.txt": "69219391c699364b1d7901dc91d8c8d1fc03a30e58ad2d30fbba095f67d70d3d",
}


def build(release=False):
    """No npm dependencies; wasm-bindgen's web output is directly browser-loadable."""
    bindgen = ROOT / "target" / "tools" / "bin" / (
        "wasm-bindgen.exe" if os.name == "nt" else "wasm-bindgen"
    )
    if not bindgen.exists():
        found = shutil.which("wasm-bindgen")
        if not found:
            raise ValueError("Browser tools are missing. Run `just web-setup` first.")
        bindgen = Path(found)
    version = subprocess.check_output([str(bindgen), "--version"], text=True).strip()
    if version != f"wasm-bindgen {BINDGEN_VERSION}":
        raise ValueError(f"Need wasm-bindgen {BINDGEN_VERSION}; run `just web-setup`.")
    toolchain = os.environ.get("ZVIEW_WEB_TOOLCHAIN", "nightly")
    profile = "release" if release else "web"
    # GPUI's wasm_thread/parking_lot dependencies require nightly even though
    # this application uses the single-threaded WebPlatform.
    subprocess.run([
        "cargo", f"+{toolchain}", "rustc", "--locked", "--lib",
        "--target", "wasm32-unknown-unknown", "--profile", profile,
        "--", "-C", "link-arg=-zstack-size=8388608",
    ], cwd=ROOT, check=True)
    metadata = json.loads(subprocess.check_output(
        ["cargo", f"+{toolchain}", "metadata", "--no-deps", "--format-version", "1"],
        cwd=ROOT, text=True,
    ))
    wasm = Path(metadata["target_directory"]) / "wasm32-unknown-unknown" / profile / "zview_desktop.wasm"
    SITE.mkdir(parents=True, exist_ok=True)
    subprocess.run([
        str(bindgen), str(wasm), "--target", "web", "--no-typescript",
        "--out-dir", str(SITE / "pkg"),
    ], check=True)
    for name in ("index.html", "main.js", "documents.mjs"):
        shutil.copyfile(ROOT / "web" / name, SITE / name)
    fonts = SITE / "fonts"
    fonts.mkdir(exist_ok=True)
    for name, digest in FONTS.items():
        path = fonts / name
        if path.exists() and hashlib.sha256(path.read_bytes()).hexdigest() == digest:
            continue
        print(f"Downloading pinned browser font asset: {name}", flush=True)
        with urlopen(FONT_BASE + name, timeout=60) as response:
            data = response.read()
        if hashlib.sha256(data).hexdigest() != digest:
            raise ValueError(f"Checksum mismatch for {name}")
        path.write_bytes(data)
    print(f"Static site: {SITE}", flush=True)


def read_document(path):
    """Read once, with the same byte bound as the application's native loader."""
    with Path(path).open("rb") as source:
        data = source.read(LIMIT + 1)
    if len(data) > LIMIT:
        raise ValueError("Dump exceeds the 16 MiB limit.")
    json.loads(data.decode("utf-8"))
    return data


class Handler(SimpleHTTPRequestHandler):
    """Never serve the repository root, user file directory, or directory listings."""

    def __init__(self, *args, document=None, name=None, **kwargs):
        self.document = document
        self.name = name
        super().__init__(*args, directory=str(SITE), **kwargs)

    def do_GET(self):
        # Reject foreign Host headers (DNS rebinding) and cross-site fetches.
        port = self.server.server_port
        if self.headers.get("Host") not in (f"127.0.0.1:{port}", f"localhost:{port}"):
            self.send_error(403)
            return
        if self.headers.get("Sec-Fetch-Site") == "cross-site":
            self.send_error(403)
            return
        path = urlsplit(self.path).path
        if path == "/session.json":
            self.send_bytes(json.dumps({"name": self.name}).encode(), "application/json")
        elif path == "/initial.json" and self.document is not None:
            self.send_bytes(self.document, "application/json")
        else:
            super().do_GET()

    def send_bytes(self, data, content_type):
        self.send_response(200)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def end_headers(self):
        self.send_header("Cache-Control", "no-store")
        self.send_header("X-Content-Type-Options", "nosniff")
        super().end_headers()

    def list_directory(self, path):
        self.send_error(404)
        return None


def serve(file="", port=0, open_browser=True, skip_build=False):
    # Validate before spending time building. The document is never copied to SITE.
    document = read_document(file) if file else None
    if not skip_build:
        build()
    if not (SITE / "pkg" / "zview_desktop_bg.wasm").is_file():
        raise ValueError("No web build found. Run `just web-build` first.")
    handler = functools.partial(Handler, document=document, name=Path(file).name if file else None)
    with ThreadingHTTPServer(("127.0.0.1", port), handler) as server:
        url = f"http://127.0.0.1:{server.server_port}/"
        print(f"ZView: {url}\nCtrl+C stops the server. JSON stays on this machine.", flush=True)
        if open_browser:
            webbrowser.open(url)
        try:
            server.serve_forever()
        except KeyboardInterrupt:
            pass


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    build_parser = commands.add_parser("build")
    build_parser.add_argument("--release", action="store_true")
    serve_parser = commands.add_parser("serve")
    serve_parser.add_argument("file", nargs="?", default="")
    serve_parser.add_argument("--port", type=int, default=0)
    serve_parser.add_argument("--no-open", action="store_true")
    serve_parser.add_argument("--skip-build", action="store_true")
    args = parser.parse_args()
    try:
        if args.command == "build":
            build(args.release)
        else:
            serve(args.file, args.port, not args.no_open, args.skip_build)
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"ZView: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())

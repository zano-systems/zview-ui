"""Opt-in production-adapter check using an existing mps2/an385 firmware build."""

import argparse
import asyncio
import shutil
import socket
import subprocess
import tempfile
import time
import uuid
from pathlib import Path

from aiohttp import ClientSession, web

from build_context import resolve_build
from server import create_app


async def run(args):
    if not args.fixture:
        await check(args.build_dir, args.qemu)
        return
    root = Path(__file__).resolve().parent.parent
    # The checked-in benchmark's README records its mps2/an385 provenance.
    # Wrap that real ELF in only the metadata the build resolver needs.
    with tempfile.TemporaryDirectory(dir=root / "target", prefix="qemu-heap-fixture-") as temp:
        build = Path(temp) / "build"
        zephyr = build / "zephyr"
        zephyr.mkdir(parents=True)
        shutil.copyfile(root / "test-data/zview-bench-60s/zephyr.elf", zephyr / "zephyr.elf")
        (zephyr / ".config").write_text('CONFIG_BOARD_TARGET="mps2/an385"\n')
        (zephyr / "runners.yaml").write_text(
            "runners: [gdb]\nconfig:\n  elf_file: zephyr.elf\nargs:\n  gdb: []\n"
        )
        await check(build, args.qemu)


async def check(build_dir, qemu):
    build_dir = Path(build_dir).resolve(strict=True)
    root = build_dir.parent
    with socket.socket() as reservation:
        reservation.bind(("127.0.0.1", 0))
        port = reservation.getsockname()[1]
    configuration = {
        "build_dir": str(build_dir), "runner": "gdb", "target": f"127.0.0.1:{port}",
    }
    context = resolve_build(root, {"mode": "live", **configuration})
    if context["board"] != "mps2/an385":
        raise ValueError("This check requires an existing mps2/an385 build; it does not rebuild firmware.")
    elf = root / context["elf"]
    with tempfile.TemporaryFile(dir=Path(__file__).resolve().parent.parent / "target") as log:
        process = subprocess.Popen([
            qemu, "-cpu", "cortex-m3", "-machine", "mps2-an385",
            "-vga", "none", "-net", "none", "-display", "none",
            "-monitor", "none", "-serial", "null",
            "-gdb", f"tcp:127.0.0.1:{port}", "-kernel", str(elf),
        ], stdout=log, stderr=log)
        runner = web.AppRunner(create_app(root))
        try:
            await asyncio.sleep(.5)
            if process.poll() is not None:
                log.seek(0)
                raise RuntimeError(log.read().decode(errors="replace"))
            await runner.setup()
            site = web.TCPSite(runner, "127.0.0.1", 0)
            await site.start()
            url = f"http://127.0.0.1:{runner.addresses[0][1]}"
            async with ClientSession() as client:
                async def command(path, body):
                    async with client.post(url + path, json=body) as response:
                        data = await response.json()
                        if response.status >= 300:
                            raise RuntimeError(data)
                        return data

                client_id = uuid.uuid4().hex
                async with client.ws_connect(f"{url}/api/events?client_id={client_id}") as stream:
                    await stream.receive_json(timeout=10)
                    opened = await command("/api/session/live", configuration)
                    session_id = opened["session_id"]

                    async def collect(count):
                        frames, maps = [], []
                        deadline = time.monotonic() + 20
                        while len(frames) < count:
                            message = await stream.receive_json(timeout=max(.1, deadline - time.monotonic()))
                            kind, data = message["type"], message["data"]
                            if kind == "frame":
                                frames.append(data)
                            elif kind == "heap_detail" and data["last_map"] is not None:
                                maps.append(data["last_map"])
                            elif kind == "status" and data["status"] in {"failed", "lost"}:
                                raise RuntimeError(data)
                            if time.monotonic() >= deadline:
                                raise TimeoutError("Live frames did not arrive in time.")
                        return frames, maps

                    baseline, _ = await collect(40)
                    heap = max(baseline[0]["groups"]["heaps"],
                               key=lambda h: h["allocated_bytes"] + h["free_bytes"])
                    choice = {k: heap[k] for k in ("address", "name")}
                    body = {"session_id": session_id, "client_id": client_id, "heap": choice}
                    await command("/api/session/heap-detail", body)
                    sampled, maps = await collect(40)
                    assert len(maps) >= 2, "No repeated chunk-map acquisition."
                    assert all(m["heap"] == choice for m in maps), "Wrong heap identity."
                    for captured in maps:
                        chunks = captured["chunks"]
                        assert chunks and all(c["size"] > 0 for c in chunks), "Invalid map."
                    await command("/api/session/heap-detail", {**body, "heap": None})
                    async with client.get(url + "/api/session") as response:
                        released = (await response.json())["data"]["heap_detail"]
                    checkpoint = released["last_map"]["frame_number"]
                    _, released_maps = await collect(12)
                    # Releasing publishes retained map metadata, not a new map.
                    assert all(m["frame_number"] <= checkpoint
                               for m in released_maps), "Capture continued after release."
                    await command("/api/session/stop", {"session_id": session_id})
                    for label, frames in (("statistics only", baseline), ("maps enabled", sampled)):
                        elapsed = frames[-1]["live_elapsed"] - frames[0]["live_elapsed"]
                        print(f"{label}: {len(frames)} frames, {(len(frames)-1)/elapsed:.2f} frames/s")
                    print(f"Heap {choice}: {len(maps)} map updates, "
                          f"{[len(m['chunks']) for m in maps]} chunks; release and stop passed.")
        finally:
            await runner.cleanup()
            process.terminate()
            try:
                await asyncio.to_thread(process.wait, timeout=3)
            except subprocess.TimeoutExpired:
                process.kill()
                await asyncio.to_thread(process.wait)
            print("Test adapter and QEMU stopped; firmware build was not modified.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--build-dir", help="Existing mps2/an385 build directory; read-only.")
    source.add_argument("--fixture", action="store_true",
                        help="Use the checked-in QEMU benchmark ELF with temporary build metadata.")
    parser.add_argument("--qemu", default="qemu-system-arm")
    args = parser.parse_args()
    asyncio.run(run(args))

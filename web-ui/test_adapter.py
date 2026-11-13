"""Exercise the installed Python package and the real HTTP/WebSocket boundary."""

import asyncio
import json
import os
import tempfile
import threading
import unittest
from pathlib import Path

from aiohttp import WSMsgType
from aiohttp.test_utils import AioHTTPTestCase

from server import CONTROLLER, INPUT_LIMIT, create_app, input_path
from session import CLIENT_QUEUE_SIZE, SessionBusy, SessionController

ROOT = Path(__file__).resolve().parent.parent
RECORDING = "test-data/tests/fixtures/recordings/sys_heap_v4.3.ndjson.gz"
ELF = "test-data/tests/fixtures/zephyr.elf"


async def terminal_events(socket):
    messages = []
    while True:
        message = await asyncio.wait_for(socket.receive(), timeout=15)
        if message.type != WSMsgType.TEXT:
            raise AssertionError(f"Unexpected WebSocket message: {message}")
        event = json.loads(message.data)
        messages.append(event)
        if event["type"] == "status" and event["data"]["status"] in {
            "ended", "stopped", "failed"
        }:
            return messages


class AdapterTests(AioHTTPTestCase):
    async def get_application(self):
        return create_app(ROOT)

    async def open(self, **overrides):
        return await self.client.post(
            "/api/session/replay",
            json={"recording": RECORDING, "elf": ELF, **overrides},
        )

    async def stop(self):
        current = await (await self.client.get("/api/session")).json()
        return await self.client.post("/api/session/stop", json={"session_id": current["session_id"]})

    async def test_real_replay_stream_and_multiple_clients(self):
        first = await self.client.ws_connect("/api/events")
        second = await self.client.ws_connect("/api/events")
        self.assertIsNone((await first.receive_json())["data"])
        self.assertIsNone((await second.receive_json())["data"])
        response = await self.open()
        self.assertEqual(response.status, 202)
        opened = await response.json()
        self.assertEqual(opened["version"], 4)
        left, right = await asyncio.gather(
            terminal_events(first), terminal_events(second)
        )
        self.assertEqual(left, right)
        frames = [event for event in left if event["type"] == "frame"]
        self.assertEqual(len(frames), 20)
        self.assertTrue(all(f["session_id"] == opened["session_id"] for f in frames))
        self.assertEqual(left[-1]["data"]["status"], "ended")
        self.assertEqual(
            [event["sequence"] for event in left],
            sorted({event["sequence"] for event in left}),
        )
        # Match the independently retained frame-2 desktop regression sample.
        expected = json.loads((ROOT / "sample.json").read_text())
        threads = frames[1]["data"]["groups"]["threads"]
        for thread in expected["threads"]:
            actual = next(t for t in threads if t["address"] == thread["address"])
            self.assertEqual(actual["runtime"]["cpu_normalized"],
                             thread["runtime"]["cpu_normalized"])
        self.assertIn("heaps", frames[1]["data"]["groups"])
        self.assertIn("decoded_at", frames[1]["data"])
        self.assertNotIn("recording_time", frames[1]["data"])
        current = await (await self.client.get("/api/session")).json()
        self.assertEqual(current["data"]["frames_received"], 20)
        self.assertEqual(current["data"]["status"], "ended")
        # A late client gets a complete current snapshot, without starting a poller.
        late = await self.client.ws_connect("/api/events")
        self.assertEqual(await late.receive_json(), current)
        await first.close()
        await second.close()
        await late.close()

    async def test_invalid_requests_leave_existing_session_unchanged(self):
        opened = await (await self.open()).json()
        for body in (
            {},
            [],
            {"recording": RECORDING, "elf": 12},
            {"recording": "../outside.ndjson.gz", "elf": ELF},
            {"recording": "missing.ndjson.gz", "elf": ELF},
            {"recording": RECORDING, "elf": "test-data"},
            {"recording": RECORDING, "elf": ELF, "mode": "live"},
        ):
            response = await self.client.post("/api/session/replay", json=body)
            self.assertEqual(response.status, 400, body)
        malformed = await self.client.post(
            "/api/session/replay",
            data="{",
            headers={"Content-Type": "application/json"},
        )
        self.assertEqual(malformed.status, 400)
        form = await self.client.post(
            "/api/session/replay", data={"recording": RECORDING, "elf": ELF}
        )
        self.assertEqual(form.status, 415)
        current = await (await self.client.get("/api/session")).json()
        self.assertEqual(current["session_id"], opened["session_id"])
        self.assertEqual(current["data"]["source"]["recording"], RECORDING)
        self.assertNotEqual(current["data"]["status"], "failed")
        await self.stop()

    async def test_bad_recording_and_bad_elf_report_failure_and_allow_retry(self):
        for overrides in ({"recording": "sample.json"}, {"elf": "sample.json"}):
            socket = await self.client.ws_connect("/api/events")
            await socket.receive_json()
            response = await self.open(**overrides)
            self.assertEqual(response.status, 202)
            messages = await terminal_events(socket)
            self.assertEqual(messages[-1]["data"]["status"], "failed")
            self.assertTrue(messages[-1]["data"]["error"])
            await socket.close()
        socket = await self.client.ws_connect("/api/events")
        await socket.receive_json()
        self.assertEqual((await self.open()).status, 202)
        messages = await terminal_events(socket)
        self.assertEqual(messages[-1]["data"]["status"], "ended")
        await socket.close()

    async def test_stop_busy_and_restart(self):
        socket = await self.client.ws_connect("/api/events")
        await socket.receive_json()
        self.assertEqual((await self.open()).status, 202)
        self.assertEqual((await self.open()).status, 409)
        response = await self.stop()
        state = (await response.json())["data"]
        self.assertEqual(state["status"], "stopped")
        count = state["frames_received"]
        await asyncio.sleep(0.2)
        state = (await (await self.client.get("/api/session")).json())["data"]
        self.assertEqual(state["frames_received"], count)
        await socket.close()
        self.assertEqual((await self.open()).status, 202)
        await self.stop()

    async def test_security_and_static_frontend(self):
        for route in ("/", "/main.js", "/documents.mjs", "/pkg/zview_desktop.js"):
            response = await self.client.get(route)
            self.assertEqual(response.status, 200)
            self.assertEqual(response.headers["X-Content-Type-Options"], "nosniff")
            self.assertIn("frame-ancestors 'none'",
                          response.headers["Content-Security-Policy"])
            self.assertIn("'wasm-unsafe-eval'",
                          response.headers["Content-Security-Policy"])
            self.assertNotIn("'unsafe-eval'",
                             response.headers["Content-Security-Policy"])
        wasm = await self.client.get("/pkg/zview_desktop_bg.wasm")
        self.assertEqual(wasm.status, 200)
        self.assertEqual(wasm.content_type, "application/wasm")
        for headers in (
            {"Origin": "https://untrusted.example"},
            {"Host": "untrusted.example"},
            {"Sec-Fetch-Site": "cross-site"},
        ):
            response = await self.client.post("/api/session/stop", headers=headers)
            self.assertEqual(response.status, 403)
        with self.assertRaises(Exception) as rejected:
            await self.client.ws_connect(
                "/api/events", headers={"Origin": "https://untrusted.example"}
            )
        self.assertEqual(rejected.exception.status, 403)
        self.assertEqual((await self.client.get("/session.py")).status, 404)

    @unittest.skipIf(os.name == "nt", "Symlinks can require Windows privileges.")
    async def test_symlink_escape_is_rejected(self):
        with tempfile.TemporaryDirectory(dir=ROOT / "web-ui") as directory:
            link = Path(directory) / "outside"
            link.symlink_to(ROOT.parent, target_is_directory=True)
            escaped = link / ROOT.name / ELF
            # This symlink resolves back inside ROOT and should still be valid.
            valid = escaped.relative_to(ROOT).as_posix()
            self.assertEqual((await self.open(elf=valid)).status, 202)
            await self.stop()
            # The directory itself resolves outside, so it cannot be an input.
            invalid = link.relative_to(ROOT).as_posix()
            self.assertEqual((await self.open(elf=invalid)).status, 400)

    async def test_oversized_file_and_http_body_are_rejected(self):
        with tempfile.TemporaryDirectory(dir=ROOT / "web-ui") as directory:
            oversized = Path(directory) / "large.elf"
            with oversized.open("wb") as file:
                file.truncate(INPUT_LIMIT + 1)
            response = await self.open(elf=oversized.relative_to(ROOT).as_posix())
            self.assertEqual(response.status, 400)
        response = await self.client.post(
            "/api/session/replay",
            data=" " * 8193,
            headers={"Content-Type": "application/json"},
        )
        self.assertEqual(response.status, 413)

    async def test_regular_file_outside_data_root_is_rejected(self):
        # Both files are in the worktree; only the selected root is narrower.
        root = ROOT / "test-data/tests/fixtures/recordings"
        with self.assertRaises(ValueError):
            input_path(root, str(ROOT / ELF))


class ControllerTests(unittest.IsolatedAsyncioTestCase):
    async def test_stop_and_shutdown_wait_for_resource_release(self):
        entered = threading.Event()
        released = threading.Event()

        def reader(recording, elf, stop, emit):
            try:
                entered.set()
                emit("status", {"status": "reading"})
                stop.wait()
                return "stopped", None
            finally:
                released.set()

        controller = SessionController(reader)
        await controller.start(Path("recording"), Path("elf"), "recording", "elf")
        self.assertTrue(await asyncio.to_thread(entered.wait, 2))
        with self.assertRaises(SessionBusy):
            await controller.start(Path("other"), Path("elf"), "other", "elf")
        result = json.loads(await controller.stop())
        self.assertTrue(released.is_set())
        self.assertEqual(result["data"]["status"], "stopped")
        # Idempotent stop and a fresh session after resources are released.
        self.assertEqual(await controller.stop(), controller.snapshot())
        await controller.start(Path("other"), Path("elf"), "other", "elf")
        await controller.stop()

    async def test_slow_client_is_disconnected_without_blocking_other_clients(self):
        controller = SessionController()
        controller.session = None
        slow = controller.subscribe()
        fast = controller.subscribe()
        await fast.get()
        for _ in range(CLIENT_QUEUE_SIZE):
            controller._broadcast('{"test":true}')
            self.assertEqual(await fast.get(), '{"test":true}')
        self.assertIsNone(await slow.get())
        controller.unsubscribe(slow)
        controller.unsubscribe(fast)

    async def test_shutdown_stops_active_worker(self):
        entered = threading.Event()
        released = threading.Event()

        def reader(recording, elf, stop, emit):
            entered.set()
            stop.wait()
            released.set()
            return "stopped", None

        controller = SessionController(reader)
        app = create_app(ROOT, controller)
        await controller.start(Path("r"), Path("e"), "r", "e")
        self.assertTrue(await asyncio.to_thread(entered.wait, 2))
        app.freeze()
        await app.shutdown()
        self.assertTrue(released.is_set())
        self.assertEqual(app[CONTROLLER].session.status, "stopped")


if __name__ == "__main__":
    unittest.main()

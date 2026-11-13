"""Live lifecycle and HTTP contract without opening a physical probe."""

import asyncio
import contextlib
import json
import queue
import tempfile
import threading
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from aiohttp.test_utils import AioHTTPTestCase
from zview.frames import Fatal, Frame, Notice
from zview.core import Core

from server import CONTROLLER, create_app
from session import SampleRate, SessionController, read_live
from test_build_context import make_build


class LiveReaderTests(unittest.TestCase):
    def test_real_core_polling_stops_before_the_backend_context_closes(self):
        order = []
        scraper = SimpleNamespace(
            selection=None, is_live=True, available={"threads": True},
            absent_features=lambda: (), waitq_flavor="simple",
            connect=lambda: None, read_frame=lambda: {"threads": []},
        )
        core = Core(scraper)
        stop = threading.Event()

        @contextlib.contextmanager
        def opened():
            try:
                yield core
            finally:
                order.append(core.is_polling)

        def emit(kind, data):
            if kind == "frame":
                stop.set()

        self.assertEqual(read_live(SimpleNamespace(open=opened), stop, emit), ("stopped", None))
        self.assertEqual(order, [False])
        self.assertFalse(core.is_polling)

    def target(self, events, order):
        messages = queue.Queue()
        for event in events:
            messages.put(event)
        core = SimpleNamespace(
            groups=("threads", "heaps"),
            build=SimpleNamespace(missing=("Stack watermark unavailable.",), is_waitq_walkable=False),
            events=messages, is_polling=False,
            select=lambda selection: None,
            start=lambda period, **_: order.append(("start", period)),
            stop=lambda: order.append("stop"),
        )

        @contextlib.contextmanager
        def opened():
            order.append("open")
            try:
                yield core
            finally:
                order.append("disconnect")
        return SimpleNamespace(open=opened)

    def test_frames_faults_and_loss_are_distinct_from_opening_and_always_release(self):
        order, emitted = [], []
        target = self.target([
            Notice("retry"), Frame({"threads": []}, at=100),
            Frame({"threads": []}, at=101), Fatal("cable removed"),
        ], order)
        result = read_live(target, threading.Event(), lambda *event: emitted.append(event))
        self.assertEqual(result, ("lost", "cable removed"))
        self.assertEqual(order, [
            "open", ("start", 0.1), "stop", ("start", 0.1), "stop",
            ("start", 0.1), "stop", "stop", "disconnect",
        ])
        self.assertEqual([kind for kind, _ in emitted],
                         ["capabilities", "notice", "status", "frame", "frame"])
        self.assertEqual(emitted[0][1]["missing"], ["Stack watermark unavailable."])
        frames = [data for kind, data in emitted if kind == "frame"]
        self.assertEqual(frames[0]["live_elapsed"], 0)
        self.assertGreater(frames[1]["live_elapsed"], 0)
        self.assertEqual(frames[0]["decoded_at"], 100)
        self.assertNotIn("recording_time", frames[0])

    def test_pre_open_stop_and_stop_during_open_never_start_polling(self):
        stop = threading.Event()
        stop.set()
        target = SimpleNamespace(open=lambda: self.fail("must not open"))
        self.assertEqual(read_live(target, stop, lambda *_: None), ("stopped", None))
        order = []
        original = self.target([], order)

        @contextlib.contextmanager
        def opened():
            with original.open() as core:
                stop.set()
                yield core
        stop.clear()
        self.assertEqual(read_live(SimpleNamespace(open=opened), stop, lambda *_: None),
                         ("stopped", None))
        self.assertEqual(order, ["open", "stop", "disconnect"])

    def test_consumer_error_releases_resources(self):
        order = []
        def emit(*_):
            raise RuntimeError("consumer failed")
        with self.assertRaisesRegex(RuntimeError, "consumer failed"):
            read_live(self.target([], order), threading.Event(), emit)
        self.assertEqual(order[-2:], ["stop", "disconnect"])

    def test_sample_rate_uses_rolling_monotonic_time_and_expires_old_frames(self):
        now = [0.]
        rate = SampleRate(lambda: now[0])
        self.assertIsNone(rate.value())
        for i in range(10):
            now[0] = i / 10
            rate.add()
        now[0] = 1.
        self.assertEqual(rate.value(), 10.)
        now[0] = 6.
        self.assertEqual(rate.value(), 0.)
        now[0] = 7.
        rate.add()
        self.assertEqual(rate.value(), 0.2)


class LiveApiTests(AioHTTPTestCase):
    async def get_application(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        make_build(self.root)
        self.targets = []
        self.released = []
        self.fail_open = False

        def reader(target, stop, emit, *, capture=None, period=0.1, view=None):
            self.targets.append(target)
            try:
                if self.fail_open:
                    raise RuntimeError("Wrong firmware image")
                emit("capabilities", {"groups": ["threads"], "missing": [], "is_waitq_walkable": True})
                emit("status", {"status": "reading"})
                emit("frame", {"decoded_at": 1., "live_elapsed": 0., "groups": {"threads": []}})
                emit("health", {"samples_per_second": 9.8})
                stop.wait()
                return "stopped", None
            finally:
                self.released.append(target)
        return create_app(self.root, controller=SessionController(live_reader=reader))

    async def snapshot(self):
        return await (await self.client.get("/api/session")).json()

    async def wait_for(self, predicate):
        for _ in range(100):
            current = await self.snapshot()
            if predicate(current["data"]):
                return current
            await asyncio.sleep(0.01)
        self.fail(f"Session did not reach expected state: {current}")

    async def open(self, **overrides):
        return await self.client.post("/api/session/live", json={"build_dir": "build", **overrides})

    async def stop(self, session_id):
        return await self.client.post("/api/session/stop", json={"session_id": session_id})

    async def test_live_resolves_actual_settings_and_shares_one_worker_between_tabs(self):
        first = await self.client.ws_connect("/api/events")
        second = await self.client.ws_connect("/api/events")
        await first.receive_json()
        await second.receive_json()
        with patch("zview.target.Target.open", side_effect=AssertionError("probe access during resolution")):
            response = await self.open()
            self.assertEqual(response.status, 202)
            opened = await response.json()
            self.assertEqual(opened["data"]["source"]["mode"], "live")
            self.assertNotIn("recording", opened["data"]["source"])
            self.assertEqual(opened["data"]["source"]["runner"], "jlink")
        current = await self.wait_for(lambda s: s["samples_per_second"] == 9.8)
        self.assertEqual(current["data"]["frames_received"], 1)
        self.assertEqual(len(self.targets), 1)
        self.assertEqual(self.targets[0].frequency, 4_000_000)
        self.assertEqual(self.targets[0].target, "nRF5340_xxAA")
        self.assertEqual((await self.open()).status, 409)
        late = await self.client.ws_connect("/api/events")
        self.assertEqual(await late.receive_json(), current)
        await first.close()
        self.assertEqual((await self.snapshot())["data"]["status"], "reading")
        stopped = await (await self.stop(opened["session_id"])).json()
        self.assertEqual(stopped["data"]["status"], "stopped")
        self.assertIsNone(stopped["data"]["samples_per_second"])
        self.assertEqual(len(self.released), 1)
        await second.close()
        await late.close()

    async def test_reconnect_is_explicit_fresh_and_session_scoped(self):
        old = await (await self.open(runner="gdb", target="localhost:3333")).json()
        await self.wait_for(lambda s: s["frames_received"] == 1)
        command = {"session_id": old["session_id"]}
        self.assertEqual((await self.client.post("/api/session/reconnect", json=command)).status, 409)
        await self.stop(old["session_id"])
        response = await self.client.post("/api/session/reconnect", json=command)
        self.assertEqual(response.status, 202)
        new = await response.json()
        self.assertNotEqual(new["session_id"], old["session_id"])
        await self.wait_for(lambda s: s["frames_received"] == 1)
        self.assertEqual(len(self.targets), 2)
        self.assertIsNot(self.targets[0], self.targets[1])
        self.assertEqual(self.targets[1].runner, "gdb")
        self.assertEqual((await self.stop(old["session_id"])).status, 409)
        self.assertEqual((await self.client.post("/api/session/reconnect", json=command)).status, 409)
        self.assertEqual((await self.snapshot())["session_id"], new["session_id"])
        await self.stop(new["session_id"])

    async def test_invalid_requests_and_preview_never_mutate_an_active_session(self):
        old = await (await self.open()).json()
        for data in ({}, [], {"build_dir": ".."}, {"build_dir": "build", "frequency": 0},
                     {"build_dir": "build", "recording": "capture.ndjson.gz"}):
            response = await self.client.post("/api/session/live", json=data)
            self.assertEqual(response.status, 400)
        self.assertEqual((await self.client.post("/api/session/stop", json={})).status, 400)
        self.assertEqual((await self.client.post("/api/session/live", data="")).status, 415)
        await self.client.post("/api/build/resolve", json={"mode": "live", "build_dir": "build"})
        self.assertEqual((await self.snapshot())["session_id"], old["session_id"])
        await self.stop(old["session_id"])

    async def test_open_failure_can_be_retried_without_automatic_reconnect(self):
        self.fail_open = True
        opened = await (await self.open()).json()
        failed = await self.wait_for(lambda s: s["status"] == "failed")
        self.assertEqual(failed["data"]["error"], "Wrong firmware image")
        self.assertEqual(failed["data"]["frames_received"], 0)
        await asyncio.sleep(0.05)
        self.assertEqual(len(self.targets), 1)
        self.fail_open = False
        response = await self.client.post("/api/session/reconnect", json={"session_id": opened["session_id"]})
        self.assertEqual(response.status, 202)
        new = await self.wait_for(lambda s: s["frames_received"] == 1)
        await self.stop(new["session_id"])


class LiveControllerTests(unittest.IsolatedAsyncioTestCase):
    async def test_warning_state_survives_snapshot_until_a_successful_frame(self):
        from session import Session
        controller = SessionController()
        controller.session = Session("live", {"mode": "live"}, "elf", status="reading")
        await controller._event("live", "notice", {"message": "retry"})
        warning = json.loads(controller.snapshot())["data"]
        self.assertTrue(warning["read_warning"])
        await controller._event("live", "health", {"samples_per_second": 10})
        self.assertTrue(controller.session.read_warning)
        await controller._event("live", "frame", {
            "decoded_at": 1., "live_elapsed": 0., "groups": {"threads": []},
        })
        healthy = json.loads(controller.snapshot())["data"]
        self.assertFalse(healthy["read_warning"])
        self.assertEqual(healthy["last_notice"], "retry")

    async def test_stop_during_open_waits_for_cleanup_and_does_not_report_a_late_open_error(self):
        entered = threading.Event()
        finish = threading.Event()
        closed = threading.Event()

        def opening(target, stop, emit, *, capture=None, period=0.1, view=None):
            entered.set()
            try:
                finish.wait(2)
                raise RuntimeError("driver error after cancellation")
            finally:
                closed.set()

        controller = SessionController(live_reader=opening)
        opened = json.loads(await controller.start_live(
            object(), {"mode": "live", "configuration": {"build_dir": "build"}},
            "elf", None,
        ))
        self.assertTrue(await asyncio.to_thread(entered.wait, 2))
        stopping = asyncio.create_task(controller.stop(opened["session_id"]))
        try:
            await asyncio.sleep(0.02)
            self.assertFalse(stopping.done())
            self.assertFalse(closed.is_set())
            self.assertEqual(controller.session.status, "stopping")
        finally:
            finish.set()
            result = json.loads(await stopping)
        self.assertTrue(closed.is_set())
        self.assertEqual(result["data"]["status"], "stopped")
        self.assertIsNone(result["data"]["error"])

    async def test_old_worker_events_and_stop_cannot_change_a_replacement_session(self):
        def reader(target, stop, emit, *, capture=None, period=0.1, view=None):
            return "lost", "lost"

        controller = SessionController(live_reader=reader)
        source = {"mode": "live", "configuration": {"build_dir": "build"}}
        first = json.loads(await controller.start_live(object(), source, "elf", None))
        await controller._task
        second = json.loads(await controller.start_live(object(), source, "elf", None))
        await controller._task
        await controller._event(first["session_id"], "status", {"status": "reading"})
        await controller._event(first["session_id"], "health", {"samples_per_second": 100})
        self.assertEqual(controller.session.id, second["session_id"])
        self.assertEqual(controller.session.status, "lost")
        self.assertIsNone(controller.session.samples_per_second)

"""Single-heap ownership, bounded public-Core polls and browser lifetime."""

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
from zview.backend.base import HeapInfo
from zview.core import Core
from zview.frames import Frame, Notice

from heap_capture import HeapCapture, HeapCaptureBusy
from server import create_app
from session import SessionController, read_live
from test_build_context import make_build


HEAP = {"address": 4096, "name": "packet_heap"}
OTHER = {"address": 8192, "name": "other_heap"}


class CaptureTests(unittest.TestCase):
    def test_fast_mode_uses_start_to_start_period_and_no_forced_cooldown(self):
        now = [0.]
        capture = HeapCapture(lambda: now[0], period=.1, cooldown=0)
        capture.choose("tab", (HEAP["address"], HEAP["name"]))
        first = capture.due()
        now[0] = .02
        capture.completed(first)
        now[0] = .099
        self.assertIsNone(capture.due())
        now[0] = .1
        slow = capture.due()
        self.assertIsNotNone(slow)
        now[0] = 2.
        capture.completed(slow)
        self.assertIsNotNone(capture.due())

    def test_timing_validation_rejects_nonfinite_negative_and_zero_heap_periods(self):
        for value in (-1, float("nan"), float("inf"), True, "0"):
            with self.subTest(value=value):
                with self.assertRaises(ValueError):
                    SessionController(period=value)
                with self.assertRaises(ValueError):
                    HeapCapture(period=value)
                with self.assertRaises(ValueError):
                    HeapCapture(cooldown=value)
        with self.assertRaises(ValueError):
            HeapCapture(period=0)
        controller = SessionController(period=0, heap_period=.1, heap_cooldown=0)
        self.assertEqual(controller.timing, {"period": 0., "heap_period": .1, "heap_cooldown": 0.})

    def test_real_core_captures_a_controlled_allocate_free_sequence_at_poll_boundaries(self):
        now = [0.]
        layouts = [
            [{"size": 32, "used": True}, {"size": 96, "used": False}],
            [{"size": 32, "used": True}, {"size": 32, "used": False},
             {"size": 32, "used": True}, {"size": 32, "used": False}],
            [{"size": 128, "used": False}],
        ]
        group = SimpleNamespace(detailed=None)
        captured = []
        stop = threading.Event()
        interest = HeapCapture(lambda: now[0])
        interest.choose("tab", (HEAP["address"], HEAP["name"]))
        reads = []

        def read_frame():
            detailed = group.detailed
            reads.append(detailed)
            chunks = layouts[len(captured)] if detailed is not None else None
            return {"threads": [], "heaps": [
                HeapInfo(HEAP["name"], HEAP["address"], 64, 32, 64, 33., chunks),
            ]}

        core = Core(SimpleNamespace(
            groups={"heaps": group, "threads": SimpleNamespace(usage_window=1)},
            parts={"usage_snapshot"}, selection=None, is_live=True, available={"heaps": True},
            absent_features=lambda: (), waitq_flavor="simple", connect=lambda: None,
            read_frame=read_frame,
        ))

        @contextlib.contextmanager
        def opened():
            try:
                yield core
            finally:
                self.assertFalse(core.is_polling)
                self.assertIsNone(group.detailed)

        def emit(kind, data):
            if kind == "frame":
                now[0] += .5
            elif kind == "heap_map":
                captured.append(data["chunks"])
                if len(captured) == len(layouts):
                    stop.set()

        self.assertEqual(
            read_live(SimpleNamespace(open=opened), stop, emit, capture=interest),
            ("stopped", None),
        )
        self.assertEqual(captured, layouts)
        self.assertEqual(reads.count(HEAP["address"]), 3)
        self.assertIn(None, reads)
        fragmentation = []
        for chunks in captured:
            free = sum(c["size"] for c in chunks if not c["used"])
            largest = max((c["size"] for c in chunks if not c["used"]), default=0)
            fragmentation.append(100 * (1 - largest / free) if free else 0)
        self.assertEqual(fragmentation, [0, 50, 0])

    def test_slow_maps_leave_a_normal_polling_interval_after_completion(self):
        now = [0.]
        capture = HeapCapture(lambda: now[0])
        capture.choose("tab", (HEAP["address"], HEAP["name"]))
        request = capture.due()
        now[0] = 2.
        capture.completed(request)
        self.assertIsNone(capture.due())
        now[0] = 2.99
        self.assertIsNone(capture.due())
        now[0] = 3.
        self.assertIsNotNone(capture.due())

    def test_failed_switch_releases_the_old_view_without_stealing_another_tab(self):
        capture = HeapCapture()
        first = HEAP["address"], HEAP["name"]
        second = OTHER["address"], OTHER["name"]
        capture.choose("a", first)
        capture.choose("b", first)
        with self.assertRaises(HeapCaptureBusy):
            capture.choose("b", second)
        capture.choose("a", None)
        self.assertIsNone(capture.identity())
        capture.choose("b", second)
        self.assertEqual(capture.identity(), second)

    def test_one_heap_shared_until_last_owner_leaves_and_old_results_are_rejected(self):
        now = [0.]
        capture = HeapCapture(lambda: now[0])
        selected = (HEAP["address"], HEAP["name"])
        capture.choose("first", selected)
        request = capture.due()
        self.assertTrue(capture.accepts(request))
        self.assertIsNone(capture.due())
        capture.choose("second", selected)
        with self.assertRaises(HeapCaptureBusy):
            capture.choose("second", (OTHER["address"], OTHER["name"]))
        capture.choose("second", selected)
        capture.choose("first", None)
        self.assertEqual(capture.identity(), selected)
        now[0] = 1.
        self.assertIsNotNone(capture.due())
        capture.choose("second", None)
        self.assertIsNone(capture.identity())
        self.assertFalse(capture.accepts(request))
        capture.choose("first", selected)
        capture.fail(capture.due())
        now[0] = 100.
        self.assertIsNone(capture.due())
        capture.clear()
        capture.choose("first", selected)
        self.assertIsNotNone(capture.due())

    def run_worker(self, *, fault=False, period=0.1):
        poll_period = period
        now, details, maps, frames = [0.], [], [], []
        capture = HeapCapture(lambda: now[0])
        capture.choose("tab", (HEAP["address"], HEAP["name"]))
        stop, events = threading.Event(), queue.Queue()
        selected, calls = [None], [0]

        def detail(group, address):
            self.assertEqual(group, "heaps")
            selected[0] = address
            details.append(address)

        def start(period, frames):
            self.assertEqual((period, frames), (poll_period, 1))
            calls[0] += 1
            now[0] += 0.1
            if fault and calls[0] == 2:
                events.put(Notice("Unreadable heap"))
            chunks = [{"size": 32, "used": True}, {"size": 64, "used": False}]
            events.put(Frame({"threads": [], "heaps": [{
                **HEAP, "allocated_bytes": 24, "free_bytes": 56,
                "chunks": chunks if selected[0] and not (fault and calls[0] == 2) else None,
            }]}, at=100 + now[0]))

        @contextlib.contextmanager
        def opened():
            yield SimpleNamespace(
                groups=("heaps",), build=SimpleNamespace(missing=(), is_waitq_walkable=True),
                select=lambda selection: None,
                detail=detail, start=start, stop=lambda: None, events=events, is_polling=False,
            )

        def emit(kind, data):
            if kind == "frame":
                frames.append(data)
                if len(frames) == 25:
                    stop.set()
            elif kind == "heap_map":
                maps.append(data)

        with patch("session.serialize_frame", side_effect=lambda groups: groups):
            result = read_live(SimpleNamespace(open=opened), stop, emit, capture=capture, period=poll_period)
        self.assertEqual(result, ("stopped", None))
        self.assertIsNone(selected[0])
        return details, maps, frames

    def test_only_due_frames_contain_maps_and_normal_measurements_continue(self):
        details, maps, frames = self.run_worker()
        self.assertEqual(len(frames), 25)
        self.assertEqual(len(maps), 3)
        self.assertEqual(details.count(HEAP["address"]), 3)
        self.assertEqual(sum(f["groups"]["heaps"][0]["chunks"] is not None for f in frames), 3)
        self.assertTrue(all(m["heap"] == HEAP for m in maps))
        self.assertTrue(all(m["error"] is None for m in maps))

    def test_zero_polling_period_continues_until_stopped_not_just_one_frame(self):
        _, maps, frames = self.run_worker(period=0)
        self.assertEqual(len(frames), 25)
        self.assertGreater(len(maps), 1)

    def test_a_map_fault_disables_optional_reads_without_hiding_the_notice(self):
        details, maps, frames = self.run_worker(fault=True)
        self.assertEqual(details.count(HEAP["address"]), 1)
        self.assertEqual(len(frames), 25)
        self.assertEqual(len(maps), 1)
        self.assertIsNone(maps[0]["chunks"])
        self.assertIn("read fault", maps[0]["error"])
        self.assertTrue(all(f["groups"]["heaps"][0]["chunks"] is None for f in frames))


class CaptureApiTests(AioHTTPTestCase):
    async def get_application(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name).resolve()
        make_build(root)

        def reader(target, stop, emit, *, capture=None, period=0.1, view=None):
            emit("status", {"status": "reading"})
            emit("frame", {"decoded_at": 1., "live_elapsed": 0., "groups": {
                "threads": [], "heaps": [HEAP, OTHER],
            }})
            stop.wait()
            return "stopped", None

        self.controller = SessionController(live_reader=reader)
        return create_app(root, controller=self.controller)

    async def open(self):
        result = await self.client.post("/api/session/live", json={"build_dir": "build"})
        session_id = (await result.json())["session_id"]
        for _ in range(100):
            if self.controller.session.frames_received:
                break
            await asyncio.sleep(0.01)
        return session_id

    async def choose(self, session_id, client_id, heap):
        return await self.client.post("/api/session/heap-detail", json={
            "session_id": session_id, "client_id": client_id, "heap": heap,
        })

    async def test_one_controlling_tab_and_closed_tab_cleanup(self):
        first = await self.client.ws_connect("/api/events?client_id=first")
        second = await self.client.ws_connect("/api/events?client_id=second")
        await first.receive_json()
        await second.receive_json()
        session_id = await self.open()
        self.assertEqual((await self.choose(session_id, "first", HEAP)).status, 200)
        self.assertEqual((await self.choose(session_id, "second", HEAP)).status, 409)
        conflict = await self.choose(session_id, "second", OTHER)
        self.assertEqual(conflict.status, 409)
        self.assertEqual((await conflict.json())["code"], "view_busy")
        self.assertEqual(self.controller.session.heap_detail["heap"], HEAP)
        self.assertEqual((await self.choose("old-session", "first", None)).status, 409)
        self.assertEqual((await self.choose(session_id, "unknown", HEAP)).status, 409)
        self.assertEqual((await self.choose(session_id, "first", None)).status, 200)
        self.assertIsNone(self.controller.session.heap_detail["heap"])
        await first.close()
        for _ in range(100):
            if self.controller.session.live_view["owner"] is None:
                break
            await asyncio.sleep(0.01)
        self.assertIsNone(self.controller.session.live_view["owner"])
        self.assertEqual((await self.choose(session_id, "second", OTHER)).status, 200)
        await self.controller.stop(session_id)
        self.assertIsNone(self.controller.session.heap_detail["heap"])
        await second.close()

    async def test_validation_and_old_capture_cannot_change_new_interest(self):
        socket = await self.client.ws_connect("/api/events?client_id=tab")
        await socket.receive_json()
        session_id = await self.open()
        for heap in ({"address": True, "name": "bad"}, {"address": 0, "name": "bad"},
                     {"address": 4096, "name": "wrong"}, [], {}):
            self.assertEqual((await self.choose(session_id, "tab", heap)).status, 400)
        self.assertEqual((await self.client.post("/api/session/heap-detail", data="")).status, 415)
        self.assertEqual((await self.client.post("/api/session/heap-detail", json={
            "session_id": session_id, "client_id": "tab", "heap": HEAP,
        }, headers={"Origin": "https://foreign.invalid"})).status, 403)
        await self.choose(session_id, "tab", HEAP)
        request = self.controller._capture.due()
        await self.choose(session_id, "tab", OTHER)
        await self.controller._event(session_id, "heap_map", {
            "generation": request[0], "heap": HEAP, "chunks": [],
            "captured_at": 1., "live_elapsed": 0., "error": None,
        })
        self.assertIsNone(self.controller.session.heap_detail["last_map"])
        self.assertEqual(self.controller.session.heap_detail["heap"], OTHER)
        self.assertEqual(json.loads(self.controller.snapshot())["data"]["heap_detail"]["heap"], OTHER)
        await socket.close()

    async def test_snapshot_retains_a_map_between_frames_but_not_between_sessions(self):
        socket = await self.client.ws_connect("/api/events?client_id=tab")
        await socket.receive_json()
        session_id = await self.open()
        await self.choose(session_id, "tab", HEAP)
        generation, _ = self.controller._capture.due()
        result = {
            "generation": generation, "heap": HEAP,
            "chunks": [{"size": 64, "used": False}],
            "captured_at": 1., "live_elapsed": .1, "error": None,
        }
        await self.controller._event(session_id, "heap_map", result)
        captured = self.controller.session.heap_detail["last_map"].copy()
        await self.controller._event(session_id, "frame", {
            "decoded_at": 2., "live_elapsed": .2, "groups": {"threads": [], "heaps": [
                {**HEAP, "chunks": None}, OTHER,
            ]},
        })
        snapshot = json.loads(self.controller.snapshot())["data"]
        self.assertEqual(snapshot["heap_detail"]["last_map"], captured)
        self.assertIsNone(snapshot["last_frame"]["groups"]["heaps"][0]["chunks"])
        await self.controller.stop(session_id)
        new_id = await self.open()
        await self.controller._event(session_id, "heap_map", result)
        self.assertNotEqual(new_id, session_id)
        self.assertIsNone(self.controller.session.heap_detail["last_map"])
        await socket.close()


class TimingControllerTests(unittest.IsolatedAsyncioTestCase):
    async def test_configured_timing_reaches_live_worker_and_snapshot_but_not_replay(self):
        received = []

        def live_reader(target, stop, emit, *, capture, period, view):
            received.append((period, capture.period, capture.cooldown))
            return "stopped", None

        def replay_reader(recording, elf, stop, emit):
            return "ended", None

        controller = SessionController(
            replay_reader, live_reader, period=0, heap_period=.1, heap_cooldown=0,
        )
        await controller.start_live(object(), {"mode": "live"}, "elf", None)
        await controller._task
        self.assertEqual(received, [(0., .1, 0.)])
        self.assertEqual(json.loads(controller.snapshot())["data"]["timing"],
                         {"period": 0., "heap_period": .1, "heap_cooldown": 0.})
        await controller.start("recording", "elf", "recording", "elf")
        await controller._task
        self.assertIsNone(json.loads(controller.snapshot())["data"]["timing"])

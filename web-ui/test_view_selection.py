"""Screen-driven acquisition, exact object identity and single-tab ownership."""

import asyncio
import contextlib
import json
import tempfile
import threading
import unittest
from pathlib import Path
from types import SimpleNamespace

from aiohttp.test_utils import AioHTTPTestCase
from zview.backend.base import HeapInfo
from zview.core import Core
from zview.selection import Selection

from heap_capture import HeapCapture
from server import create_app
from session import SessionController, read_live
from test_build_context import make_build
from view_selection import KERNEL_GROUPS, LiveView, catalog, compile_view


A = {"address": 4096, "name": "heap[0]*"}
B = {"address": 8192, "name": "heap0other"}
THREAD = {"address": 12288, "name": "worker"}
OBJECTS = {
    "threads": [THREAD], "heaps": [A, B],
    **{group: [{"address": 16384, "name": "item"}] for group in KERNEL_GROUPS if group != "heaps"},
}


class SelectionTests(unittest.TestCase):
    def test_each_screen_selects_only_its_groups_and_exact_rows(self):
        overview = compile_view({"screen": "overview", "heap": A}, OBJECTS)
        self.assertTrue(overview.reads("threads", "worker"))
        self.assertTrue(overview.reads("heaps", A["name"]))
        self.assertFalse(overview.reads("heaps", B["name"]))
        self.assertFalse(overview.reads_group("semaphores"))
        thread = compile_view({"screen": "thread", "object": THREAD}, OBJECTS)
        self.assertTrue(thread.reads("threads", "worker"))
        self.assertFalse(thread.reads("threads", "other"))
        self.assertFalse(thread.reads_group("heaps"))
        for group in KERNEL_GROUPS:
            row = OBJECTS[group][0]
            detail = compile_view({"screen": "detail", "group": group, "object": row}, OBJECTS)
            self.assertEqual(detail.groups, frozenset({group}))
            self.assertTrue(detail.reads(group, row["name"]))
            self.assertFalse(detail.reads_group("threads"))
        literal = compile_view({"screen": "detail", "group": "heaps", "object": A}, OBJECTS)
        self.assertFalse(literal.reads("heaps", "heap0suffix"))

    def test_object_type_and_query_filters_are_compiled_without_target_reads(self):
        all_objects = compile_view({"screen": "objects", "kind": None, "query": ""}, OBJECTS)
        self.assertEqual(all_objects.groups, frozenset(KERNEL_GROUPS))
        self.assertFalse(all_objects.reads_group("threads"))
        matched = compile_view({"screen": "objects", "kind": "heaps", "query": "0x00001000"}, OBJECTS)
        self.assertTrue(matched.reads("heaps", A["name"]))
        self.assertFalse(matched.reads("heaps", B["name"]))
        empty = compile_view({"screen": "objects", "kind": "heaps", "query": "absent"}, OBJECTS)
        self.assertFalse(any(empty.reads_group(group) for group in KERNEL_GROUPS))
        for view in (
            {"screen": "detail", "group": "heaps", "object": {**A, "address": 99}},
            {"screen": "detail", "group": "unknown", "object": A},
            {"screen": "overview", "heap": A, "extra": True},
            {"screen": "objects", "kind": "threads", "query": ""},
        ):
            with self.assertRaises(ValueError):
                compile_view(view, OBJECTS)

    def test_public_core_selection_skips_unrelated_reads_after_initial_inventory(self):
        reads, frames = [], []
        group = SimpleNamespace(detailed=None)
        stop = threading.Event()
        view = LiveView()
        view.objects = OBJECTS
        view.choose("tab", {"screen": "detail", "group": "heaps", "object": A})
        capture = HeapCapture()
        capture.choose("tab", (A["address"], A["name"]))
        scraper = SimpleNamespace(
            selection=Selection(), is_live=True, groups={"heaps": group},
            available={"threads": True, "heaps": True}, absent_features=lambda: (),
            waitq_flavor="simple", connect=lambda: None,
        )

        def read_frame():
            groups = {}
            if scraper.selection.reads_group("threads"):
                reads.append(("threads", THREAD["name"]))
                groups["threads"] = []
            if scraper.selection.reads_group("heaps"):
                groups["heaps"] = []
                for row in (A, B):
                    if scraper.selection.reads("heaps", row["name"]):
                        reads.append(("heaps", row["name"]))
                        chunks = [{"size": 64, "used": False}] if group.detailed == row["address"] else None
                        groups["heaps"].append(HeapInfo(
                            row["name"], row["address"], 56, 0, 0, 0., chunks,
                        ))
            return groups

        scraper.read_frame = read_frame
        core = Core(scraper)

        @contextlib.contextmanager
        def opened():
            yield core
            self.assertFalse(core.is_polling)

        def emit(kind, data):
            if kind == "frame":
                if data["view"] is None:
                    self.assertIn("threads", data["groups"])
                    self.assertEqual(len(data["groups"]["heaps"]), 2)
                    reads.clear()
                else:
                    frames.append(data)
                    if len(frames) == 3:
                        stop.set()

        self.assertEqual(read_live(SimpleNamespace(open=opened), stop, emit,
                                   capture=capture, view=view), ("stopped", None))
        self.assertEqual(reads, [("heaps", A["name"])] * 3)
        self.assertTrue(all(set(frame["groups"]) == {"heaps"} for frame in frames))


class ViewApiTests(AioHTTPTestCase):
    async def get_application(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        root = Path(temp.name).resolve()
        make_build(root)

        def reader(target, stop, emit, *, capture, period, view):
            emit("status", {"status": "reading"})
            emit("frame", {"decoded_at": 1., "live_elapsed": 0., "groups": OBJECTS})
            stop.wait()
            return "stopped", None

        self.controller = SessionController(live_reader=reader)
        return create_app(root, controller=self.controller)

    async def start(self):
        socket = await self.client.ws_connect("/api/events?client_id=owner")
        await socket.receive_json()
        response = await self.client.post("/api/session/live", json={"build_dir": "build"})
        session_id = (await response.json())["session_id"]
        for _ in range(100):
            if self.controller.session.inventory is not None:
                return socket, session_id
            await asyncio.sleep(.01)
        self.fail("No inventory received.")

    async def choose(self, sid, view, owner="owner"):
        return await self.client.post("/api/session/view", json={
            "session_id": sid, "client_id": owner, "view": view,
        })

    async def test_atomic_selection_detail_and_obsolete_frames_are_not_presented(self):
        socket, sid = await self.start()
        heap_view = {"screen": "detail", "group": "heaps", "object": A}
        self.assertEqual((await self.choose(sid, heap_view)).status, 200)
        old = self.controller.session.live_view["revision"]
        self.assertEqual(self.controller._capture.identity(), (A["address"], A["name"]))
        thread_view = {"screen": "thread", "object": THREAD}
        self.assertEqual((await self.choose(sid, thread_view)).status, 200)
        self.assertIsNone(self.controller._capture.identity())
        count = self.controller.session.frames_received
        await self.controller._event(sid, "frame", {
            "decoded_at": 2., "live_elapsed": .1, "groups": {"heaps": [A]},
            "view": heap_view, "view_revision": old,
        })
        self.assertEqual(self.controller.session.frames_received, count)
        await self.controller._event(sid, "frame", {
            "decoded_at": 3., "live_elapsed": .2, "groups": OBJECTS,
            "view": thread_view, "view_revision": self.controller.session.live_view["revision"],
        })
        snapshot = json.loads(self.controller.snapshot())["data"]
        self.assertEqual(set(snapshot["last_frame"]["groups"]), {"threads"})
        self.assertEqual(snapshot["inventory"], catalog(OBJECTS))
        self.assertEqual((await self.choose(sid, {"screen": "detail", "group": "heaps", "object": B})).status, 200)
        await socket.close()

    async def test_only_one_tab_controls_and_disconnect_releases_ownership(self):
        socket, sid = await self.start()
        observer = await self.client.ws_connect("/api/events?client_id=observer")
        await observer.receive_json()
        overview = {"screen": "overview", "heap": A}
        self.assertEqual((await self.choose(sid, overview)).status, 200)
        self.assertEqual((await self.choose(sid, overview, "observer")).status, 409)
        self.assertEqual((await self.choose("old-session", overview)).status, 409)
        self.assertEqual((await self.choose(sid, overview, "unknown")).status, 409)
        await socket.close()
        for _ in range(100):
            if self.controller.session.live_view["owner"] is None:
                break
            await asyncio.sleep(.01)
        self.assertEqual((await self.choose(sid, overview, "observer")).status, 200)
        await observer.close()

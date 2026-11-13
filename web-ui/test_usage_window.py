"""Use the installed TUI's counter aggregation, not frontend percentage means."""

import contextlib
import threading
import unittest
from types import SimpleNamespace

from zview.backend.base import ThreadInfo
from zview.core import Core
from zview.kernel.threads import Threads
from zview.selection import Selection

from session import read_live
from view_selection import LiveView


class UsageWindowTests(unittest.TestCase):
    def run_counter_stream(self, deltas, *, change_after=None, snapshot=True):
        group = Threads()
        group.idle_addresses = (2,)
        view = LiveView()
        stop = threading.Event()
        events, applied, captured = [], [], []
        samples = iter(deltas)
        firmware = SimpleNamespace(layout=SimpleNamespace(thread_usage=1, cpu_usage=1))
        infos = [ThreadInfo(i, None, None, name, None)
                 for i, name in ((1, "worker"), (2, "idle"), (3, "other"))]

        def read_frame():
            # Only raw counter acquisition is substituted. The installed Threads
            # aggregation and finalization produce the serialized percentages.
            raw = next(samples)
            captured.append(raw)
            summed = group._windowed(raw)
            rows = [{"info": info, "usage_delta": summed.get(info.address, 0),
                     "is_active": raw.get(info.address, 0) > 0,
                     "watermark": None, "stack_usage_pct": None} for info in infos]
            return {"threads": group._finalize(firmware, rows, summed, True)}

        scraper = SimpleNamespace(
            groups={"threads": group}, parts={"usage_snapshot"} if snapshot else set(),
            selection=Selection(), is_live=True, available={"threads": True},
            absent_features=lambda: (), waitq_flavor="simple", read_frame=read_frame,
            connect=lambda: None,
        )
        core = Core(scraper)

        @contextlib.contextmanager
        def opened():
            yield core
            self.assertFalse(core.is_polling)

        def emit(kind, data):
            if kind == "usage_window":
                self.assertFalse(core.is_polling)
                applied.append(data["polls"])
            if kind == "frame":
                events.append(data)
                if change_after and len(events) == change_after[0]:
                    with view.lock:
                        view.usage_window = change_after[1]
                if len(events) == len(deltas):
                    stop.set()

        result = read_live(SimpleNamespace(open=opened), stop, emit, period=0, view=view)
        self.assertEqual(result, ("stopped", None))
        self.assertEqual(captured, deltas)
        return events, applied

    def test_sixteen_poll_cpu_window_rolls_without_batching_and_load_excludes_idle(self):
        frames, applied = self.run_counter_stream(
            [{1: 100, 2: 0}] * 8 + [{1: 0, 2: 100}] * 16,
        )
        self.assertEqual(applied, [16])
        worker = [frame["groups"]["threads"][0]["runtime"] for frame in frames]
        self.assertEqual(worker[7]["cpu_normalized"], 100.)
        self.assertEqual(worker[15]["cpu_normalized"], 50.)
        self.assertEqual(worker[15]["cpu"], 100.)
        self.assertEqual(worker[-1]["cpu_normalized"], 0.)
        self.assertEqual(worker[-1]["cpu"], 0.)
        self.assertEqual(len(worker), 24)

    def test_counter_weighting_load_mean_and_runtime_resize_match_the_tui(self):
        frames, applied = self.run_counter_stream(
            [{1: 10, 2: 0, 3: 0}, {1: 0, 2: 0, 3: 90},
             {1: 20, 2: 0, 3: 0}],
            change_after=(2, 1),
        )
        self.assertEqual(applied, [16, 1])
        runtime = frames[1]["groups"]["threads"][0]["runtime"]
        self.assertEqual(runtime["cpu_normalized"], 10.)  # Not mean(100%, 0%).
        self.assertEqual(runtime["cpu"], 10.)
        self.assertEqual(frames[-1]["groups"]["threads"][0]["runtime"]["cpu"], 100.)

    def test_load_uses_the_same_window_when_two_threads_share_busy_cycles_equally(self):
        frames, _ = self.run_counter_stream(
            [{1: 100, 3: 0}] * 8 + [{1: 0, 3: 100}] * 8,
        )
        runtime = frames[-1]["groups"]["threads"][0]["runtime"]
        self.assertEqual(runtime["cpu_normalized"], 50.)
        self.assertEqual(runtime["cpu"], 50.)

    def test_effective_window_is_reported_when_the_backend_cannot_sum_snapshots(self):
        _, applied = self.run_counter_stream([{1: 10, 2: 0}], snapshot=False)
        self.assertEqual(applied, [1])

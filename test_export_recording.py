import io
import json
import unittest
from pathlib import Path
from unittest.mock import MagicMock, patch

from export_recording import TimedReplay, TimestampQueue, desktop_import, export
from zview.backend.base import (
    HeapInfo,
    MemSlabInfo,
    MsgqInfo,
    MutexInfo,
    SemaphoreInfo,
    WorkqInfo,
)


class ExportTests(unittest.TestCase):
    def test_desktop_bridge_keeps_diagnostics_out_of_json(self):
        def decode(*args):
            print("decoder diagnostic")
            return [{"threads": [], "elapsed_seconds": 0}]

        output, errors = io.StringIO(), io.StringIO()
        with patch("export_recording.export", side_effect=decode), patch(
            "sys.argv", ["bridge", "--desktop-import", "recording", "firmware"]
        ), patch("sys.stdout", output), patch("sys.stderr", errors):
            self.assertEqual(desktop_import(), 0)
        self.assertEqual(json.loads(output.getvalue()), [{"threads": [], "elapsed_seconds": 0}])
        self.assertIn("decoder diagnostic", errors.getvalue())

    def test_desktop_bridge_reports_errors_without_a_document(self):
        output, errors = io.StringIO(), io.StringIO()
        with patch("export_recording.export", side_effect=ValueError("bad recording")), patch(
            "sys.argv", ["bridge", "--desktop-import", "recording", "firmware"]
        ), patch("sys.stdout", output), patch("sys.stderr", errors):
            self.assertEqual(desktop_import(), 1)
        self.assertEqual(output.getvalue(), "")
        self.assertIn("bad recording", errors.getvalue())

    def test_timestamp_is_attached_before_backend_advances(self):
        backend = TimedReplay("unused")
        backend._pace_to(2.5)
        messages = TimestampQueue(backend)
        messages.put({"threads": []})
        backend._pace_to(3.5)
        self.assertEqual(messages.get()["recording_time"], 2.5)

    def test_exports_real_fixture_in_one_pass(self):
        root = Path(__file__).resolve().parent / "test-data"
        frames = export(
            root / "tests/fixtures/recordings/sys_heap_v4.3.ndjson.gz",
            root / "tests/fixtures/zephyr.elf",
            15,
        )
        times = [f["elapsed_seconds"] for f in frames]
        self.assertGreater(len(frames), 2)
        self.assertEqual(times[0], 0)
        self.assertTrue(all(a < b for a, b in zip(times, times[1:], strict=False)))
        self.assertLessEqual(times[-1], 15)
        self.assertEqual(frames[1]["heaps"][0]["allocated_bytes"], 1536)
        # This older acquisition did not capture fragmentation reads.
        self.assertTrue(all(h["chunks"] is None for f in frames for h in f["heaps"]))

    def test_export_preserves_real_captured_maps(self):
        root = Path(__file__).resolve().parent / "test-data/zview-bench-60s"
        frames = export(root / "capture.ndjson.gz", root / "zephyr.elf", 1)
        maps = [h["chunks"] for f in frames for h in f["heaps"]]
        self.assertTrue(any(maps))
        self.assertTrue(all(c["size"] > 0 for chunks in maps if chunks for c in chunks))

    def test_export_preserves_every_kernel_object_group_and_unknown_waiters(self):
        # Synthetic dataclasses use the same serializer as real polling frames.
        message = {
            "threads": [],
            "heaps": [HeapInfo("heap", 1, 0, 64, 64, 100.0,
                               [{"size": 64, "used": True}], None)],
            "semaphores": [SemaphoreInfo("sem", 2, 0, 3, ("worker",))],
            "mutexes": [MutexInfo("mutex", 3, 1, 100, "owner", ())],
            "msgqs": [MsgqInfo("msgq", 4, 2, 4, 16, ())],
            "mem_slabs": [MemSlabInfo("slab", 5, 8, 32, 2, 6, None)],
            "workqs": [WorkqInfo("work", 6, 3, ("task",), True, "worker", ())],
        }

        def poll(messages, stop, period):
            messages.put(message)
            messages.put({"replay_complete": True})

        with patch("export_recording.TimedReplay") as replay, patch(
            "export_recording.ZScraper"
        ) as scraper:
            replay.return_value.__enter__.return_value = MagicMock(recording_time=10.0)
            scraper.return_value.start_polling_thread.side_effect = poll
            frame = export(Path("unused"), Path("unused"), 15)[0]

        self.assertEqual(set(frame), set(message) | {"elapsed_seconds"})
        self.assertEqual(frame["elapsed_seconds"], 0)
        self.assertEqual(frame["heaps"][0]["chunks"], [{"size": 64, "used": True}])
        self.assertIsNone(frame["heaps"][0]["waiters"])
        self.assertEqual(frame["semaphores"][0]["waiters"], ("worker",))
        self.assertEqual(frame["mutexes"][0]["owner_address"], 100)
        self.assertEqual(frame["msgqs"][0]["used_msgs"], 2)
        self.assertEqual(frame["mem_slabs"][0]["max_used"], 6)
        self.assertIsNone(frame["mem_slabs"][0]["waiters"])
        self.assertTrue(frame["workqs"][0]["pending_truncated"])
        scraper.return_value.finish_polling_thread.assert_called_once()


if __name__ == "__main__":
    unittest.main()

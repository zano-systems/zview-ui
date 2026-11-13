"""Deterministic terminal-event races and the installed package's context cleanup."""

import contextlib
import importlib.metadata
import json
import queue
import threading
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from zview.backend.replay import ReplayScraper
from zview.frames import Fatal, Frame, Notice, ReplayEnd
from zview.target import Target

from session import read_replay

ROOT = Path(__file__).resolve().parent.parent
RECORDING = ROOT / "test-data/tests/fixtures/recordings/sys_heap_v4.3.ndjson.gz"
ELF = ROOT / "test-data/tests/fixtures/zephyr.elf"


class LateEvents(queue.Queue):
    """The poller publishes its final events just after a get times out."""
    def __init__(self, pending):
        super().__init__()
        self.pending = pending

    def get(self, *args, **kwargs):
        if self.pending is not None:
            pending, self.pending = self.pending, None
            for item in pending:
                self.put_nowait(item)
            raise queue.Empty
        return super().get(*args, **kwargs)


class ReplayLifecycleTests(unittest.TestCase):
    def test_installed_backend_matches_the_exact_web_requirement(self):
        requirement = next(
            line for line in (ROOT / "web-ui/requirements.txt").read_text().splitlines()
            if line.startswith("zview @ ")
        )
        expected = requirement.rsplit("@", 1)[1].strip()
        source = importlib.metadata.distribution("zview").read_text("direct_url.json")
        actual = json.loads(source or "{}").get("vcs_info", {}).get("commit_id")
        self.assertEqual(
            actual, expected,
            "Reinstall the web backend with --force-reinstall --no-deps -r web-ui/requirements.txt",
        )

    def run_reader(self, pending, emit):
        order = []
        core = SimpleNamespace(
            events=LateEvents(pending),
            is_polling=False,
            start=lambda period: order.append("start"),
            stop=lambda: order.append("stop"),
        )

        @contextlib.contextmanager
        def opened():
            try:
                yield core
            finally:
                order.append("disconnect")

        with patch("session.Target", return_value=SimpleNamespace(open=opened)):
            try:
                return read_replay(RECORDING, ELF, threading.Event(), emit)
            finally:
                self.assertEqual(order, ["start", "stop", "disconnect"])

    def test_late_end_and_failure_events_are_drained_after_the_poller_stops(self):
        for terminal, expected in (
            (ReplayEnd("done"), ("ended", None)),
            (Fatal("last read failed"), ("failed", "last read failed")),
        ):
            with self.subTest(terminal=terminal):
                emitted = []
                result = self.run_reader(
                    [Frame({"threads": []}, at=123.0), Notice("last notice"), terminal],
                    lambda kind, data: emitted.append((kind, data)),
                )
                self.assertEqual(result, expected)
                self.assertEqual([kind for kind, _ in emitted], ["status", "frame", "notice"])
                self.assertEqual(emitted[1][1]["decoded_at"], 123.0)
                self.assertEqual(emitted[2][1]["message"], "last notice")

    def test_missing_terminal_is_still_an_error_and_emit_failures_release_resources(self):
        with self.assertRaisesRegex(RuntimeError, "without an end event"):
            self.run_reader([], lambda *_: None)

        def failing_emit(*_):
            raise ValueError("consumer failed")

        with self.assertRaisesRegex(ValueError, "consumer failed"):
            self.run_reader([ReplayEnd("done")], failing_emit)

    def test_stop_before_open_does_not_acquire_a_target(self):
        stop = threading.Event()
        stop.set()
        with patch("session.Target") as target:
            self.assertEqual(read_replay(RECORDING, ELF, stop, lambda *_: None), ("stopped", None))
            target.assert_not_called()

    def test_installed_target_context_stops_polling_before_disconnect_on_all_exits(self):
        disconnect_states = []
        original = ReplayScraper.disconnect
        current = {}

        def disconnect(backend):
            disconnect_states.append(current["core"].is_polling)
            return original(backend)

        for fail in (False, True):
            expected = self.assertRaisesRegex(ValueError, "frontend exit") if fail else contextlib.nullcontext()
            with expected, patch.object(ReplayScraper, "disconnect", disconnect):
                # Deliberately no explicit stop: this exercises the merged public contract.
                with Target(elf=str(ELF), recording=str(RECORDING), is_paced=False).open() as core:
                    current["core"] = core
                    core.start(0.1)
                    while True:
                        item = core.events.get(timeout=5)
                        if isinstance(item, Frame):
                            break
                        self.assertNotIsInstance(item, (Fatal, ReplayEnd))
                    if fail:
                        raise ValueError("frontend exit")
            self.assertFalse(core.is_polling)
        self.assertEqual(disconnect_states, [False, False])

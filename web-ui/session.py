"""One live target or replay, independent of the HTTP transport."""

import asyncio
import json
import queue
import threading
import time
import uuid
from collections import deque
from dataclasses import asdict, dataclass, field
from functools import partial
from pathlib import Path

from zview.frames import Fatal, Frame, Notice, ReplayEnd
from zview.kernel.threads import USAGE_WINDOW_MAX
from zview.selection import Selection
from zview.snapshot import serialize_frame
from zview.target import Target

from heap_capture import HeapCapture, identity, seconds
from view_selection import GROUPS, LiveView, ViewBusy, catalog

PROTOCOL_VERSION = 5
CLIENT_QUEUE_SIZE = 16
POLL_PERIOD = 0.1


class SampleRate:
    """Successful frames received by the adapter, before browser fan-out."""

    def __init__(self, clock=time.monotonic):
        self.clock = clock
        self.started = clock()
        self.samples = deque(maxlen=4096)

    def add(self):
        self.samples.append(self.clock())

    def value(self):
        now = self.clock()
        while self.samples and self.samples[0] <= now - 5:
            self.samples.popleft()
        elapsed = now - self.started
        return len(self.samples) / min(5, elapsed) if elapsed >= 1 else None


def read_live(target: Target, stop: threading.Event, emit, *, capture=None,
              period=POLL_PERIOD, view=None):
    """Open only on explicit Connect. Never perform probe work on the event loop."""
    if stop.is_set():
        return "stopped", None
    with target.open() as core:
        try:
            if stop.is_set():
                return "stopped", None
            emit("capabilities", {
                "groups": list(core.groups),
                "missing": list(core.build.missing),
                "is_waitq_walkable": core.build.is_waitq_walkable,
            })
            capture = capture or HeapCapture()
            view = view or LiveView()
            bootstrap = True
            rate = SampleRate()
            first = None
            usage_window = None
            health_at = time.monotonic()
            while not stop.is_set():
                with view.lock:
                    revision, selected_view, selection = view.snapshot()
                    requested_usage_window = view.usage_window
                    if bootstrap:
                        selection, selected_view = Selection(), None
                    request = None if bootstrap else capture.due()
                try:
                    # The same Core retains counter deltas across bounded polls.
                    # Set only between polls, never concurrently with acquisition.
                    core.usage_window = requested_usage_window
                    if usage_window != core.usage_window:
                        usage_window = core.usage_window
                        emit("usage_window", {"polls": usage_window})
                    core.select(selection)
                    if request is not None:
                        core.detail("heaps", request[1][0])
                    # Bound each public Core poll to one successful frame. This
                    # lets the owner change optional detail between reads, never
                    # racing an in-flight poll or reading maps on every frame.
                    # Empty filters must not generate an unbounded empty-frame loop.
                    poll_period = period if any(selection.reads_group(g) for g in GROUPS) else max(period, .1)
                    core.start(poll_period, frames=1)
                    while not stop.is_set():
                        try:
                            item = core.events.get(timeout=0.1)
                        except queue.Empty:
                            if not core.is_polling and core.events.empty():
                                return "lost", "Live polling stopped without a terminal event."
                            if time.monotonic() - health_at >= 1:
                                emit("health", {"samples_per_second": rate.value()})
                                health_at = time.monotonic()
                            continue
                        if isinstance(item, Frame):
                            now = time.monotonic()
                            if first is None:
                                first = now
                                emit("status", {"status": "reading"})
                            rate.add()
                            groups = serialize_frame(item.groups)
                            emit("frame", {
                                "decoded_at": item.at,
                                "live_elapsed": now - first,
                                "groups": groups,
                                "view_revision": revision,
                                "view": selected_view,
                            })
                            bootstrap = False
                            if request is not None and capture.accepts(request):
                                heap = next((h for h in groups.get("heaps", ())
                                             if (h["address"], h["name"]) == request[1]), None)
                                chunks = heap.get("chunks") if heap else None
                                emit("heap_map", {
                                    "generation": request[0], "heap": identity(request[1]),
                                    "chunks": chunks, "captured_at": item.at,
                                    "live_elapsed": now - first,
                                    "error": None if chunks is not None else
                                    "The selected heap has no captured map in this frame.",
                                })
                                if chunks is None:
                                    capture.fail(request)
                            break
                        if isinstance(item, Notice):
                            emit("notice", {"message": item.text})
                            if request is not None:
                                # The package reports optional map faults as
                                # frame faults. Disable detail before its next
                                # retry; do not parse exception strings.
                                core.detail("heaps", None)
                                capture.fail(request)
                                emit("heap_map", {
                                    "generation": request[0], "heap": identity(request[1]),
                                    "chunks": None, "captured_at": None,
                                    "live_elapsed": None,
                                    "error": "Heap map capture stopped after a read fault. Close all views of this heap and reopen to retry.",
                                })
                                request = None
                        elif isinstance(item, Fatal):
                            return "lost", item.text
                        elif isinstance(item, ReplayEnd):
                            return "lost", "A live backend unexpectedly reported replay completion."
                finally:
                    core.stop()
                    if request is not None:
                        core.detail("heaps", None)
                        capture.completed(request)
                if time.monotonic() - health_at >= 1:
                    emit("health", {"samples_per_second": rate.value()})
                    health_at = time.monotonic()
            return "stopped", None
        finally:
            core.stop()


def read_replay(recording: Path, elf: Path, stop: threading.Event, emit):
    """Own the backend context and its poller on a worker, never on the event loop."""
    if stop.is_set():
        return "stopped", None
    # This milestone streams decoded frames, not original-time playback. Avoid
    # ReplayScraper's private timing hooks and long, non-interruptible pacing sleeps.
    target = Target(elf=str(elf), recording=str(recording), is_paced=False)
    with target.open() as core:
        try:
            if stop.is_set():
                return "stopped", None
            core.start(POLL_PERIOD)
            emit("status", {"status": "reading"})
            while not stop.is_set():
                try:
                    item = core.events.get(timeout=0.1)
                except queue.Empty:
                    if not core.is_polling and core.events.empty():
                        raise RuntimeError("Replay polling stopped without an end event.")
                    continue
                if isinstance(item, Frame):
                    emit(
                        "frame",
                        {
                            # Acquisition time, deliberately not a recording timestamp.
                            "decoded_at": item.at,
                            "groups": serialize_frame(item.groups),
                        },
                    )
                elif isinstance(item, Notice):
                    emit("notice", {"message": item.text})
                elif isinstance(item, Fatal):
                    return "failed", item.text
                elif isinstance(item, ReplayEnd):
                    return "ended", None
            return "stopped", None
        finally:
            # Target.open() closes the backend; stop must finish first.
            core.stop()


@dataclass
class Session:
    id: str
    source: dict
    elf: str
    status: str = "opening"
    frames_received: int = 0
    last_frame: dict | None = None
    last_notice: str | None = None
    read_warning: bool = False
    error: str | None = None
    context: dict | None = None
    capabilities: dict | None = None
    samples_per_second: float | None = None
    heap_detail: dict = field(default_factory=lambda: {
        "heap": None, "last_map": None, "error": None,
    })
    timing: dict | None = None
    inventory: dict | None = None
    live_view: dict | None = None
    usage_window: int | None = None

    @property
    def mode(self):
        return self.source["mode"]


class SessionBusy(Exception):
    pass


class SessionController:
    """One acquisition owner, with bounded fan-out to independent web clients."""

    def __init__(self, reader=read_replay, live_reader=read_live, *,
                 period=POLL_PERIOD, heap_period=1., heap_cooldown=1.):
        self.session: Session | None = None
        self.sequence = 0
        self._reader = reader
        self._live_reader = live_reader
        self._task: asyncio.Task | None = None
        self._stop = threading.Event()
        self._lock = asyncio.Lock()
        self._clients: set[asyncio.Queue] = set()
        self._client_ids = {}
        self._capture = None
        self._view = None
        self.timing = {
            "period": seconds(period),
            "heap_period": seconds(heap_period, allow_zero=False),
            "heap_cooldown": seconds(heap_cooldown),
        }

    def _message(self, kind, data):
        return json.dumps(
            {
                "version": PROTOCOL_VERSION,
                "type": kind,
                "session_id": self.session.id if self.session else None,
                "sequence": self.sequence,
                "data": data,
            },
            allow_nan=False,
        )

    def snapshot(self):
        return self._message(
            "snapshot", asdict(self.session) if self.session else None
        )

    def subscribe(self, client_id=None):
        if client_id in self._client_ids:
            raise SessionBusy("This browser connection is already registered.")
        events = asyncio.Queue(maxsize=CLIENT_QUEUE_SIZE)
        events.put_nowait(self.snapshot())
        self._clients.add(events)
        if client_id is not None:
            self._client_ids[client_id] = events
        return events

    def unsubscribe(self, events, *, notify=True):
        self._clients.discard(events)
        for owner, queue_ in tuple(self._client_ids.items()):
            if queue_ is events:
                del self._client_ids[owner]
                if self._capture is not None:
                    with self._view.lock:
                        self._capture.choose(owner, None)
                        self._view.release(owner)
                    if notify:
                        self._sync_detail()
                        self._sync_view()

    def _sync_detail(self):
        if self.session is None:
            return
        selected = identity(self._capture.identity()) if self._capture else None
        detail = self.session.heap_detail
        if selected != detail["heap"]:
            detail["heap"] = selected
            detail["error"] = None
            if selected is not None:
                detail["last_map"] = None
            self.sequence += 1
            self._broadcast(self._message("heap_detail", detail))

    def _sync_view(self):
        if self.session is not None and self._view is not None:
            state = self._view.describe()
            if self.session.live_view != state:
                self.session.live_view = state
                self.sequence += 1
                self._broadcast(self._message("view", state))

    def choose_view(self, session_id, client_id, selected):
        self.require_session(session_id)
        if client_id not in self._client_ids:
            raise SessionBusy("Reconnect this browser before selecting a view.")
        if self.session.mode != "live" or self.session.status != "reading" or self._view is None:
            raise SessionBusy("View selection requires a polling Live session.")
        with self._view.lock:
            # Validate and claim ownership before changing optional map demand.
            self._view.choose(client_id, selected)
            heap = selected["object"] if selected["screen"] == "detail" and selected["group"] == "heaps" else None
            self._capture.choose(client_id, (heap["address"], heap["name"]) if heap else None)
        self._sync_detail()
        self._sync_view()

    def choose_heap(self, session_id, client_id, heap):
        """Compatibility operation; the browser uses the combined view endpoint."""
        self.require_session(session_id)
        if client_id not in self._client_ids:
            raise SessionBusy("Reconnect this browser before requesting heap details.")
        if self.session.mode != "live" or self.session.status != "reading" or self._capture is None:
            raise SessionBusy("Heap capture requires a polling Live session.")
        self.choose_view(session_id, client_id, {
            "screen": "detail", "group": "heaps", "object": identity(heap),
        } if heap is not None else {"screen": "overview", "heap": None})

    def choose_usage_window(self, session_id, client_id, polls):
        self.require_session(session_id)
        if type(polls) is not int or not 1 <= polls <= USAGE_WINDOW_MAX:
            raise ValueError(f"CPU averaging must be from 1 to {USAGE_WINDOW_MAX} polls.")
        if (
            client_id not in self._client_ids or self._view is None
            or self.session.mode != "live" or self.session.status != "reading"
        ):
            raise SessionBusy("CPU averaging requires a connected, polling Live session.")
        with self._view.lock:
            if self._view.owner != client_id:
                raise ViewBusy("Only the tab controlling Live acquisition can change CPU averaging.")
            self._view.usage_window = polls
        self._sync_view()

    def _broadcast(self, message):
        for events in tuple(self._clients):
            if events.full():
                # Do not silently lose warnings or terminal events. Disconnect
                # lagging clients; reconnect gives them a complete latest snapshot.
                self.unsubscribe(events, notify=False)
                # Finish delivering the current sequence before announcing an
                # interest change caused by this slow client's removal.
                asyncio.get_running_loop().call_soon(self._sync_detail)
                asyncio.get_running_loop().call_soon(self._sync_view)
                while not events.empty():
                    events.get_nowait()
                events.put_nowait(None)
            else:
                events.put_nowait(message)

    async def _event(self, session_id, kind, data):
        if self.session is None or self.session.id != session_id:
            return
        # Reject non-JSON values before mutating the publicly observable state.
        json.dumps(data, allow_nan=False)
        if kind == "frame":
            if self._view is not None:
                revision, selected, selection = self._view.snapshot()
                if data.get("view_revision", revision) != revision:
                    return
                if self.session.inventory is None:
                    self.session.inventory = catalog(data["groups"])
                    with self._view.lock:
                        self._view.objects = self.session.inventory
                    selected = None
                elif data.get("view", selected) is not None:
                    # Keep the wire scope truthful even for injected test readers.
                    data = {**data, "groups": {
                        group: [row for row in rows if selection.reads(group, row["name"])]
                        for group, rows in data["groups"].items() if selection.reads_group(group)
                    }}
                data = {
                    **data, "view_revision": revision, "view": data.get("view", selected),
                    "sampled_groups": list(data["groups"]),
                }
            self.session.frames_received += 1
            self.session.last_frame = data
            self.session.read_warning = False
        elif kind == "notice":
            self.session.last_notice = data["message"]
            self.session.read_warning = True
        elif kind == "status":
            # A concurrent stop must not briefly return the UI to Polling.
            if self.session.status == "stopping" and data["status"] == "reading":
                return
            self.session.status = data["status"]
            self.session.error = data.get("error")
            if data["status"] in {"stopping", "stopped", "lost", "failed", "ended"}:
                self.session.samples_per_second = None
                if self._capture is not None:
                    with self._view.lock:
                        self._capture.clear()
                        if self._view.owner is not None:
                            self._view.release(self._view.owner)
                    self._sync_detail()
                    self._sync_view()
        elif kind == "heap_map":
            request = (data["generation"], (data["heap"]["address"], data["heap"]["name"]))
            if self._capture is None or not self._capture.accepts(request):
                return
            detail = self.session.heap_detail
            detail["error"] = data["error"]
            if data["chunks"] is not None:
                detail["last_map"] = {k: v for k, v in data.items()
                                      if k not in {"generation", "error"}}
                detail["last_map"]["frame_number"] = self.session.frames_received
            kind, data = "heap_detail", detail
        elif kind == "capabilities":
            self.session.capabilities = data
        elif kind == "usage_window":
            self.session.usage_window = data["polls"]
        elif kind == "health":
            if self.session.status == "stopping":
                return
            self.session.samples_per_second = data["samples_per_second"]
        self.sequence += 1
        self._broadcast(self._message(kind, data))

    async def start(self, recording, elf, recording_label, elf_label, *, context=None):
        return await self._start(
            {"mode": "replay", "recording": recording_label}, elf_label, context,
            self._reader, (recording, elf),
        )

    async def start_live(self, target, source, elf_label, context, *, expected_id=None):
        capture = HeapCapture(period=self.timing["heap_period"], cooldown=self.timing["heap_cooldown"])
        view = LiveView()
        if expected_id is not None:
            self.require_session(expected_id)
            view.usage_window = self.session.live_view["usage_window"]
        return await self._start(
            source, elf_label, context,
            partial(self._live_reader, capture=capture, period=self.timing["period"], view=view),
            (target,), expected_id=expected_id, capture=capture, timing=self.timing.copy(), view=view,
        )

    def require_session(self, expected_id):
        if self.session is None or self.session.id != expected_id:
            raise SessionBusy("The session changed. Refresh its state before trying again.")

    async def _start(self, source, elf_label, context, reader, args, *, expected_id=None,
                     capture=None, timing=None, view=None):
        async with self._lock:
            if expected_id is not None:
                self.require_session(expected_id)
            if self._task is not None and not self._task.done():
                raise SessionBusy("Disconnect or stop the active session before opening another.")
            self.session = Session(
                uuid.uuid4().hex, source, elf_label, context=context, timing=timing,
                live_view=view.describe() if view else None,
            )
            self.sequence = 0
            self._stop = threading.Event()
            self._capture = capture
            self._view = view
            self._broadcast(self.snapshot())
            self._task = asyncio.create_task(
                self._run(self.session.id, reader, args, self._stop)
            )
            return self.snapshot()

    async def _run(self, session_id, reader, args, stop):
        loop = asyncio.get_running_loop()

        def emit(kind, data):
            # At most one pending delivery per worker. A busy event loop cannot
            # accumulate an unbounded chain of thread-safe callbacks.
            asyncio.run_coroutine_threadsafe(
                self._event(session_id, kind, data), loop
            ).result()

        try:
            status, error = await asyncio.to_thread(
                reader, *args, stop, emit
            )
        except Exception as exc:
            status, error = ("stopped", None) if stop.is_set() else ("failed", str(exc))
        await self._event(session_id, "status", {"status": status, "error": error})

    async def stop(self, expected_id=None):
        async with self._lock:
            if expected_id is not None:
                self.require_session(expected_id)
            if self._task is not None and not self._task.done():
                self._stop.set()
                await self._event(
                    self.session.id, "status", {"status": "stopping"}
                )
                # Don't cancel the asyncio task: its worker must release resources.
                await self._task
            return self.snapshot()

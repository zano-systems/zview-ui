"""One session's optional heap-map request, shared by interested browser clients."""

import math
import threading
import time


def seconds(value, *, allow_zero=True):
    if (
        type(value) not in (int, float) or not math.isfinite(value)
        or value < 0 or (not allow_zero and value == 0)
    ):
        raise ValueError("Timing values must be finite seconds, "
                         + ("zero or greater." if allow_zero else "greater than zero."))
    return float(value)


class HeapCaptureBusy(Exception):
    pass


class HeapCapture:
    """Thread-safe interest and cadence; probe operations stay with the live worker."""

    def __init__(self, clock=time.monotonic, *, period=1., cooldown=1.):
        self._lock = threading.Lock()
        self._clock = clock
        self.period = seconds(period, allow_zero=False)
        self.cooldown = seconds(cooldown)
        self._heap = None
        self._owners = set()
        self._generation = 0
        self._next = 0.
        self._failed = False

    def choose(self, owner, heap):
        with self._lock:
            others = self._owners - {owner}
            if heap is not None and others and heap != self._heap:
                # A tab that navigated away no longer owns its previous view,
                # even when its new selection conflicts with another tab.
                self._owners.discard(owner)
                raise HeapCaptureBusy("Another tab is inspecting a different heap. Close that heap detail first.")
            self._owners.discard(owner)
            if heap is not None:
                self._owners.add(owner)
            selected = heap if heap is not None else self._heap if self._owners else None
            if selected != self._heap:
                self._heap = selected
                self._generation += 1
                self._next = 0.
                self._failed = False
            return self._heap

    def clear(self):
        with self._lock:
            self._owners.clear()
            self._heap = None
            self._generation += 1

    def due(self):
        with self._lock:
            if self._heap is None or self._failed or self._clock() < self._next:
                return None
            self._next = self._clock() + self.period
            return self._generation, self._heap

    def accepts(self, request):
        with self._lock:
            return request == (self._generation, self._heap)

    def completed(self, request):
        with self._lock:
            if request == (self._generation, self._heap):
                # Keep the requested start-to-start period, plus any minimum
                # cooldown after completion. Zero cooldown explicitly opts out
                # of the default protection for slow map reads.
                self._next = max(self._next, self._clock() + self.cooldown)

    def fail(self, request):
        with self._lock:
            if request == (self._generation, self._heap):
                self._failed = True

    def identity(self):
        with self._lock:
            return self._heap


def identity(heap):
    return None if heap is None else {"address": heap[0], "name": heap[1]}

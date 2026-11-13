"""Compile one active screen into ZView's public, immutable Selection."""

import threading
from glob import escape

from zview.kinds import KINDS
from zview.selection import Selection

GROUPS = tuple(kind.key for kind in KINDS)
KERNEL_GROUPS = tuple(group for group in GROUPS if group != "threads")


class ViewBusy(Exception):
    pass


def catalog(groups):
    return {
        group: [{"address": row["address"], "name": row["name"]} for row in rows]
        for group, rows in groups.items()
    }


def compile_view(view, objects):
    if not isinstance(view, dict):
        raise ValueError("Provide a live view.")
    screen = view.get("screen")

    def exact(group, row):
        if (
            not isinstance(row, dict) or set(row) != {"address", "name"}
            or type(row["address"]) is not int or row["address"] <= 0
            or not isinstance(row["name"], str)
            or row not in objects.get(group, ())
        ):
            raise ValueError("The selected object is not in this session's inventory.")
        return group, escape(row["name"])

    if screen == "overview" and set(view) == {"screen", "heap"}:
        groups = {"threads"}
        include = [("threads", "*")]
        if view["heap"] is not None:
            include.append(exact("heaps", view["heap"]))
            groups.add("heaps")
    elif screen == "thread" and set(view) == {"screen", "object"}:
        groups = {"threads"}
        include = [exact("threads", view["object"])]
    elif screen == "objects" and set(view) == {"screen", "kind", "query"}:
        if view["kind"] is not None and view["kind"] not in KERNEL_GROUPS:
            raise ValueError("Unknown kernel-object group.")
        if not isinstance(view["query"], str) or len(view["query"]) > 256:
            raise ValueError("Object query must be at most 256 characters.")
        groups = set(KERNEL_GROUPS) if view["kind"] is None else {view["kind"]}
        query = view["query"].strip().lower()
        include = [(None, "*")] if not query else [
            (group, escape(row["name"]))
            for group in groups for row in objects.get(group, ())
            if query in row["name"].lower() or query in f"0x{row['address']:x}"
            or query in f"0x{row['address']:08x}"
        ]
    elif screen == "detail" and set(view) == {"screen", "group", "object"}:
        if view["group"] not in KERNEL_GROUPS:
            raise ValueError("Unknown kernel-object group.")
        groups = {view["group"]}
        include = [exact(view["group"], view["object"])]
    else:
        raise ValueError("Unsupported live view fields.")
    return Selection(groups=frozenset(groups), include=tuple(include))


class LiveView:
    """One controlling connection; no union or acquisition worker per tab."""

    def __init__(self):
        self.lock = threading.RLock()
        self.objects = None
        self.owner = None
        self.revision = 0
        self.view = {"screen": "overview", "heap": None}
        self.selection = Selection(groups=frozenset({"threads"}))

    def describe(self):
        with self.lock:
            return {"revision": self.revision, "owner": self.owner, "view": self.view}

    def choose(self, owner, view):
        with self.lock:
            if self.owner is not None and self.owner != owner:
                raise ViewBusy("Only one tab controls Live acquisition. Close the controlling tab first.")
            if self.objects is None:
                raise ValueError("Wait for the initial object inventory before selecting a view.")
            selection = compile_view(view, self.objects)
            if self.view != view:
                self.revision += 1
            self.owner, self.view, self.selection = owner, view, selection

    def release(self, owner):
        with self.lock:
            if self.owner == owner:
                self.owner = None

    def snapshot(self):
        with self.lock:
            return self.revision, self.view, self.selection

# ZView UI — experimental desktop

Version: **0.2.0** (experimental). Contributions follow Semantic Versioning and
Conventional Commits; see [CONTRIBUTING.md](CONTRIBUTING.md).

Standalone experimental desktop frontend for
[ZView](https://github.com/rodrigopex/zview), extracted from its `desktop/`
directory. The Rust application can open dump JSON without Python. Recording
import uses the existing ZView Python backend; live API/MCP integration remains
paused pending agreement on that interface.

## Optional Zephyr west module

Add this project to your application's west manifest alongside ZView:

```yaml
manifest:
  projects:
    - name: zview-ui
      url: https://github.com/zano-systems/zview-ui
      revision: main
      path: tools/zview-ui
```

Use `west update zview-ui`, then `cd tools/zview-ui` and `cargo run`.
Pin `revision` to a published commit for reproducible workspaces. Until this
extraction is published, use the local checkout directly.
`zephyr/module.yml` identifies a host-tool module only: it adds no firmware
sources, Kconfig options, snippets, or automatic Cargo build steps.

## Python backend setup

Only recording import, export, and Python tests require a Python environment.
Install the matching ZView checkout (including its dependencies), for example:

```sh
python3 -m venv .venv
.venv/bin/python -m pip install -e /absolute/path/to/zview
. .venv/bin/activate
export ZVIEW_PYTHON="$PWD/.venv/bin/python"
```

Do not point Python at this repository's Rust `src/` directory. The repositories
may live anywhere; there is no assumed sibling layout. On Windows, use the
environment's `Scripts/python.exe` instead of `bin/python`.

Native Threads overview, thread-detail, Kernel objects overview, semaphore,
message queue, memory slab, mutex, work queue and heap details built with
[GPUI Kit](https://gpui-kit.com/), using its nested Sidebar, Input, Select,
plot/tooltip primitives, Progress, Button, and dialog components.
The Python CLI and TUI are unchanged. This app consumes their dump JSON rather
than implementing another ELF reader or probe backend.

## Run

Install Rust and the [GPUI Kit platform prerequisites](https://gpui-kit.com/docs/installation)
(macOS 15+, Windows with the MSVC toolchain, or graphical Linux with the listed
libraries and a working Vulkan driver). Run these commands from the repository root:

```sh
cargo run
```

The initial screen uses a real sample extracted from ZView's
`sys_heap_v4.3.ndjson.gz`, frame 2, with its matching `zephyr.elf`.
Those regression inputs are retained under `test-data/tests/fixtures/`.
It has two non-idle threads and one heap, so the
table has fewer rows than the design mockup. Extra dump fields are deliberately
omitted from `sample.json`; the loader accepts the full CLI output.

To load your own data, export a snapshot using an installed ZView CLI:

```sh
zview dump -e test-data/tests/fixtures/zephyr.elf \
  -i test-data/tests/fixtures/recordings/sys_heap_v4.3.ndjson.gz \
  --frame 2 --json > snapshot.json
cargo run -- snapshot.json
```

Alternatively, open the **Offline ▾** session menu at the top right and choose
**Open dump…**. **Reload current file** rereads that file; it does not poll a
probe or advance a recording. Invalid files leave the current data visible and
display an error. Input is limited to 16 MiB.

The session menu replaces the separate Open dump, Open recording, and Reload
toolbar buttons on every screen. It identifies the current offline source
(and the matching ELF for imported recordings), with full paths available on
hover. Opening and reloading are disabled during import; **Cancel import** is
available in the menu as well as in the loading banner. The disabled
**Connect to target…** entry reserves the future connection workflow: there is
no MCP connection, target polling, or subprocess server launch yet. Offline
data is never labelled Live, and Disconnect is not offered without a connection.

## Included

- Dark-purple app shell with a collapsible GPUI Kit sidebar and Help dialog.
- CPU, combined thread-stack watermark, and selected-heap summary charts.
- Searchable heap selector, keyed by name and address so duplicate names can
  be distinguished. Changing it updates both the measurement and its history.
- CPU-sorted thread table, selectable rows, activity indicators, and per-thread
  stack bars. CPU and Load retain the TUI's field mapping:
  `runtime.cpu_normalized` and `runtime.cpu`, respectively.
- Green Activity icons mean CPU cycles accumulated since the previous sample;
  they do not mean "running now." Gray CircleDashed icons mean no observed activity or unavailable
  runtime data, distinguished by the icon's tooltip. Scheduler states such as
  waiting or suspended cannot be inferred from this activity flag.
- Buttons, inputs, focus rings, floating search and dropdowns use the shared
  purple theme palette, including hover/pressed and selected-list colors.
- Independently scrollable Threads and Kernel objects rows, with pinned column
  headers (and the kernel aggregate row). Wide tables scroll horizontally without
  moving the page. Up/Down keeps the selected thread in view.
- Height-bounded screens keep navigation, session controls, summary cards and
  charts visible at the supported minimum window size of **1000 × 680**.
  Charts resize with the window instead of enforcing fixed minimum heights.
  Overview charts and the bordered thread table split available height equally,
  excluding the table heading and activity legend; detail charts fill the
  remaining space. Long wait queues and pending-work lists scroll inside panels.
- Overview and Thread detail chart cards use compact, aligned headings without
  repeated time footers; time remains on the chart axes and session controls.
- Supporting descriptions and long values are truncated rather than expanding
  the page; hover to read their complete text. Summary cards can still wrap.
- Missing measurements display `—`, not zero. Idle is excluded from both the
  table and thread aggregates. Stack watermark is **not total device RAM**.

## Breadcrumb navigation

Breadcrumbs open the destination they name, independently of the previous list:

- **Threads** opens the Threads overview.
- **Kernel objects** opens the all-types overview, clearing type and search filters.
- An object type such as **Mutex** opens that type's list with search cleared.
- The current object name is a label, not a navigation link.

Breadcrumb navigation retains session time, playback, and the overview heap
selection. **Back** and **Escape** instead return with the previous list's search
and type filter intact.

## Phase 2 — Thread detail

Click a row to open its details. With the table
focused, use **Up/Down** to select a row and **Enter** to open it. The table is also
reachable with **Tab**.

The detail screen shows:

- A compact summary matching the TUI: thread name, CPU %, load %, stack-watermark
  bar and percentage, and watermark bytes / stack capacity.
- Priority, Options (raw hexadecimal flags), and a wider Entry card combining
  the function symbol, source file/line, and entry address where available.
- Two large CPU and load history charts below the summary and metadata, growing
  to fill available height. CPU is the share of total cycles; load is the share
  of non-idle cycles. The stack watermark is shown as a high-water mark in the
  summary, not as an additional history chart or instantaneous stack consumption.
- An explicit absent-thread message when navigating to a frame that does not
  contain that thread. Missing runtime data does not hide available metadata.

**Escape**, **Back to Threads**, the **Threads** breadcrumb, and the sidebar
**Threads** entry return to the overview. Opening and closing details does not
change the selected frame or heap; **Previous/Next** work in either screen.
Returning restores keyboard focus to the table and retains the highlighted row.
A successfully opened or reloaded document starts at its normal initial frame
in the overview. Invalid input keeps the existing screen and data.

History is matched using address, name, stack start, and entry address, not
just the display name or sorted row position. A known identity change or missing
measurement breaks the history rather than joining unrelated threads.
Older dumps without the additional metadata fields still load; their unavailable
values appear as `—`.

The entry source uses the same ELF/DWARF declaration-site resolver as the TUI.
The displayed path uses its last three components, like the TUI—for example,
`zephyr/kernel/init.c:283`—while the full source path stays in the dump.
New dumps include `entry_file` and `entry_line`. Re-export older recordings
with their matching ELF to populate those fields; a pre-existing JSON file
cannot recover a source location from the entry address alone.

## Phase 3 — Kernel objects overview

Choose **Kernel objects** in the sidebar. The table includes heaps, memory slabs,
message queues, mutexes, semaphores, and work queues from the selected snapshot.
It is sorted by type, name, then address; duplicate names remain separate rows.

- Expand **Kernel objects** in the sidebar to choose **All types** or one of
  the six object types. Each entry shows its object count at the selected frame,
  before applying text search. Empty types remain available. Clicking the parent
  opens the overview without resetting the selected type; its chevron toggles
  the submenu. Collapsing the sidebar hides the children but preserves selection.
- Search by name or hexadecimal address (case-insensitive), combined with the
  sidebar type selection. The search field and **Clear filters** sit in the table
  toolbar, alongside the object count and sort information. **Clear filters**
  clears the search and returns to **All types**.
- Rows show type, name, address, state/usage, and waiting threads. Up to three
  waiter names are listed alongside the full count; `—` means the queue could
  not be read, while `0` means a known empty queue.
- The first table row retains the TUI's **All Objects** summary (or **All
  Semaphores**, etc. for a type filter). It summarizes the displayed rows after
  search/type filtering: object count, contended count, objects with waiters,
  and total queued threads. Heap/slab filters say **exhausted**; work queues
  say **refusing**. A partial waiter total is shown as `N+?` when any wait queue
  is unavailable. The summary bar is the proportion of objects with waiters,
  in-use mutexes/work queues, or contention/exhaustion/refusal, counted once
  per object—not an average of the per-object usage bars.
- State labels follow the TUI: semaphore count/limit (including uninitialized
  and unlimited), queue occupancy and message size, slab occupancy/block size
  and optional peak, heap allocated/capacity bytes, mutex owner/recursive count,
  and work-queue busy/idle/start state and pending depth (`+` means truncated).
- Bars represent each type's capacity or binary state, **not CPU usage**.
  Amber marks waiting/locked objects; red marks full queues, exhausted memory,
  contended locks, or draining/plugged work queues. Missing measurements are
  not converted to zero. Unlimited/uninitialized semaphores have no ratio bar.
- **Previous/Next** uses the same frame as Threads. Navigation between screens
  preserves time, heap selection, and object filters. Filters also remain when
  opening or reloading a document; those actions still return to Threads.
- Missing groups in older dumps are accepted. Empty snapshots and empty search
  results have separate messages. A wide table scrolls horizontally on smaller
  windows. All six object types open detail screens by click or keyboard.

The recording exporter retains every kernel-object group, including captured heap
fragmentation maps. No additional backend is needed.
The bundled single-snapshot sample only contains a heap, so it cannot demonstrate
every type. For interaction checks with all six types, use this **synthetic test
fixture**, not a captured target session:

```sh
cargo run -- test-data/kernel-objects.json
```

## Shared wait queue presentation

All wait queues use purple rounded thread tags with a **Spool** icon, distinct
from the yellow briefcase tags for pending work. Detail panels show one thread per
row in an independently scrollable area; overview cells wrap tags inside the
scrollable table. Both preserve the complete captured queue order.

- Known empty: the queue area is blank, with no placeholder text or zero prefix.
- Unknown: **Wait queue unavailable**, never treated as an empty queue.
- Summary counters and chart readings still show numeric counts, including zero.

## Phase 4 — Semaphore detail

Click a semaphore row, or focus it with **Tab** and press **Enter**. The detail
screen shows its name, address, current count, limit, full waiting-thread list,
and count history. Counts are not CPU percentages; the summary bar represents
available count relative to a finite limit.

- History is a diverging step-area chart: available tokens above zero in purple,
  waiting-thread counts below zero in red. Negative positions encode the number
  of waiters, not negative semaphore counts. Both measurements are independent.
  The available-count step outline is green when count equals the finite limit,
  yellow when tokens remain below that limit, and red at zero, including periods
  with waiting threads. The purple area fill is unchanged.
  Unlimited semaphores with available tokens use yellow. Unknown/zero limits
  do not imply a full or partial count. Nonzero waiters keep the negative area
  red. There is no separate colored zero-line overlay. Historical colors use each sample's
  own limit rather than today's limit.
  The shared integer axis is symmetric and includes the largest observed count,
  waiter count and finite current limit (minimum extent `−1…1`).
- A finite positive limit has a dashed purple reference line at 20% opacity. Unlimited
  (`0xffffffff`), unknown and zero limits have no limit line and use observed
  counts only. A zero limit still says **Not initialized**.
- Unknown count/limit measurements display `—`, not zero. A known empty wait
  queue is blank. Unlimited/uninitialized semaphores
  have no count/limit ratio bar.
- Areas hold the left source reading until the next captured timestamp, not the
  exact operation time. A missing reading starts a gap at its timestamp rather
  than dropping the preceding valid interval. Count and wait-queue gaps remain
  independent. There are no persistent dots, matching the mutex chart.
  Nothing extends past the selected frame. Hover shows
  the held source timestamp and both original nonnegative readings, or `—`.
  Matching uses address and name. Future readings do not set the scale.
  A single snapshot remains available in the summary and on hover, without
  an invented area duration.
- The full **Wait queue** matches the mutex layout: unnumbered, purple-themed
  tags in queue order, one thread per row in a scrollable panel beside the chart.
  Summary fields and the
  finite-limit capacity bar remain above the chart.
- These are sampled counts, not individual give/take events. Operations between
  snapshots can cancel out and leave no trace, as in the TUI.
- **Previous/Next** changes the same session frame. An absent semaphore gets a
  GPUI Kit warning notification and unavailable current values rather than stale
  readings. The in-app notification remains until dismissed, the object returns,
  or you leave its detail screen. Redraws do not create duplicate notifications.
- **Escape** and **Back to Kernel objects** return to the overview with its
  search, type filter, time, and heap selection intact. Breadcrumbs use the
  destinations described above. The sidebar keeps Kernel objects active while inspecting a semaphore.
  Returning focuses the search input.

The JSON currently supplies the object address, not the TUI's ELF-resolved
declaration site. This screen does not claim a source location it cannot resolve.
The synthetic demo above includes a semaphore with counts varying from `0` to `3`
over seven frames, with waiter `worker` when its count is zero.

For a dedicated synthetic 20-second step-area demo:

```sh
cargo run -- test-data/semaphores.json
```

Open `buffer_tokens` at `0x00002000` in Kernel objects → Semaphore. The demo
starts at 0 seconds; advance to 10 seconds to see four waiting threads. It includes irregular capture
intervals, token exhaustion/recovery, independent missing measurements, an
absent object, duplicate names, and unlimited/uninitialized/unknown semaphores.
All source intervals are drawn without averaging; zoom and pixel-bucket
aggregation are not implemented. This fixture is illustrative, not a target capture.

## Phase 5 — Message queue detail

Click a message queue row, or focus it with **Tab** and press **Enter**.
The screen shows name, address, current depth, capacity, message size in bytes,
the complete waiter list, and a depth history chart.

- The count axis includes the largest reported capacity and depth through the
  selected frame (minimum 1), never future readings. A smaller current capacity
  does not clip earlier counts; the scale label distinguishes the history range
  from current capacity. Missing or zero capacity still has no ratio bar.
- Full queues are red; otherwise nonempty wait queues are amber. Zephyr uses one
  shared wait queue: full queues imply blocked senders, empty queues imply blocked
  receivers. Intermediate or unavailable state does not imply a direction.
- Missing measurements display `—`; a known empty wait queue is blank.
  An absent queue has an explicit message, not stale current values.
- Histories match address and name, use real timestamps, preserve missing-value
  gaps, and omit future samples. Tooltips report actual depth readings. These are
  snapshots, not individual send/receive events.
- **Previous/Next**, **Escape**, Back, breadcrumb, and sidebar behavior match
  semaphore details. Returning preserves the overview search/type filters and time.
  No ELF declaration site is available in the current queue dump.

With the synthetic demo above, open **Kernel objects → Message queue**. It opens
paused at **6 seconds**, with three queues and visible history:

- `messages`: capacity 8, 32-byte messages, fills from 0 to 8, drains, and refills.
  At 6 seconds it is full with two blocked producers; at 0 and 10 seconds it has
  two waiting consumers.
- `telemetry`: capacity 16, 64-byte messages, a full queue with a waiting writer
  at 6 seconds, and an unavailable reading at 8 seconds to demonstrate a gap.
- `commands`: capacity 4, 16-byte messages, blocked senders at 4 seconds and
  receivers at 0/8 seconds. It is absent at 10 seconds and returns at 12 seconds.

Use **Previous/Next** to explore all seven frames from 0 to 12 seconds.
These values are hand-authored demonstration data, not a recorded target session.
Other kernel types are included in the first frame; later frames focus on queues
and the semaphore. The original two-frame regression fixture is retained as
`test-data/kernel-objects-minimal.json`.

Headless tests additionally cover partial fields, keyboard/click entry,
return controls, retained filters, and the minimum window size.

## Phase 7 — Mutex detail

Click a mutex row, or focus it with **Tab** and press **Enter**. Details show
name, address, lock state, recursive lock depth, owner name/address, and the
complete **Wait queue** as unnumbered tags in a vertical list, one thread per row,
retaining queue order. The queue sits to the right of the history chart and
scrolls independently without moving the chart. A zero owner address means **Free**; unknown
ownership is not treated as free. A nonzero owner with an unavailable wait
queue says **Locked · Contention unknown**.

The numeric-time **Lock depth & contention** chart uses two independent step
areas: recursive depth above zero in purple, waiting-thread count below zero
in red. Negative positions encode waiters, not negative lock depth. There is
no capacity-limit line. When the wait queue is known empty, the zero line
is green when free and red when held with positive recursive depth.
No colored ownership overlay is drawn during contention or for unknown data;
the negative area already shows the waiting-thread count.
Zero-waiter hover markers use the same ownership colors. Missing queue
readings never imply zero waiters, and future time has no ownership coloring.
The summary retains the GPUI Kit state icons.
Hover for both original nonnegative counts, source timestamp (or frame number),
owner and state. Unknown counts/objects create independent gaps; known counts
remain visible even if ownership is unknown. A reading is held until the next
captured timestamp, with gaps beginning at missing readings. The mutex chart
has no persistent dots; hover markers identify source readings. Future space
stays blank, and a terminal value is not given an invented duration.
Both halves use a common integer scale based on the highest depth/waiter count
through the selected frame (minimum extent `−1…1`).

When adjacent samples report different nonzero owner addresses, the upper
outline is interrupted at that timestamp and a dashed vertical separator
marks the handoff. The waiter series remains independent. Unknown owners are
not guessed, and an observed release to zero already separates lock sequences.

A held mutex with waiting threads is contended regardless of
recursive depth; mutexes have no configured count limit. Lock depth is not the
number of takes over time. These are sampled counts, not a lock/unlock event log
or hold-time measurement. Matching uses address and name to keep duplicate
names distinct.

**Previous/Next** retains the inspected identity. An absent mutex shows a
GPUI Kit warning notification, not an inline alert, and no stale current values.
It remains until dismissed, recovery, or navigation away, without repeating
on redraw. **Escape** and Back preserve search, type filter, selected frame and
heap on return. Breadcrumbs open their named lists, as described above.
Missing measurements display `—`; an empty known queue is blank.
Dumps do not provide an ELF declaration site for mutexes.

For a synthetic thirteen-frame demo covering recursive locking, contention,
unknown values, duplicate names, disappearance, owner handoff and release:

```sh
cargo run -- test-data/mutexes.json
```

The timed demo opens at 0 seconds, with five mutexes in the overview. Advance
to 10 seconds and open `resource_lock` at `0x00003000` to see `storage_worker` holding it at depth 3
with six waiting threads. The other mutexes demonstrate free, busy and unknown
states, including another `resource_lock` at a different address.
Move backward for missing data and disappearance; move forward to see the
queue drain, ownership transfer to `sensor_reader` and `telemetry_writer`,
and eventual release.
This fixture is illustrative, not a hardware capture.

## Phase 6 — Memory slab detail

Click a memory slab row, or focus it with **Tab** and press **Enter**. The
screen shows name, address, used and total blocks, block size, total capacity
in bytes, target-reported peak usage, the full wait queue, and block-count history.

- The count axis includes the largest reported block capacity and usage through
  the selected frame (minimum 1), never future readings. The scale label shows
  a smaller current capacity separately, so past counts remain accurate.
  The capacity bar always remains visible: zero usage
  shows an empty track and `0.0%`; a missing ratio, zero capacity or absent slab
  shows an empty track labelled **Usage ratio unavailable**, not a zero reading.
- Exhausted slabs are red; otherwise slabs with waiting threads are amber.
- Unknown measurements display `—`, not zero. An empty known
  queue is blank. Missing peak data says **Not tracked /
  unavailable**; it is not reconstructed from the visible history.
- Histories match address and name, preserve real timestamps and gaps, and do
  not draw future readings. Tooltips report block counts. These are sampled
  states, not individual allocation/free events.
- **Previous/Next** updates the shared session frame. An absent slab shows an
  explicit message instead of carrying current values forward.
- **Escape** and Back preserve search, type filter, time, and heap selection on
  return. Breadcrumbs open their named lists. The sidebar remains in Kernel
  objects. The current dump has no ELF declaration-site metadata for slabs.

The two-frame `test-data/kernel-objects-minimal.json` fixture includes a slab
with 2 of 8 blocks used, 32-byte blocks and a peak of 6 in its first frame;
the slab is absent in its second frame. Use it to check the absent-object state.

For a dedicated multi-frame visual demo, run from the repository root:

```sh
cargo run -- test-data/memory-slabs.json
```

Open **Kernel objects → Memory slab**, then `packet_pool`. This synthetic
fixture has 13 frames over 24 seconds and opens paused at 0 seconds, with five
slabs available:

- `packet_pool`: allocations and releases, exhaustion with two waiters at
  2 seconds and four at 18 seconds, unknown usage at 7 seconds, absence at
  10 seconds, and zero usage at 22 seconds. Peak stays 8 after blocks are released.
- `untracked_pool`: a separate usage history with no peak tracking, including
  exhaustion and a waiting writer at 9 seconds.
- `empty_pool`: zero usage with a known capacity of 16 blocks.
- `unknown_pool`: unavailable usage/capacity, including partially known fields
  in earlier frames.
- `zero_capacity_pool`: zero blocks configured, with an unavailable ratio rather
  than a misleading percentage.

The last three are diagnostic examples available at 0, 2, 5 and 12 seconds;
other frames demonstrate their absence. These are hand-authored examples, not
target recordings. Use Previous/Next to inspect gaps, changing wait queues and
the always-visible capacity track.

## Charts and multiple snapshots

All charts share mouse-wheel time zoom: scroll up to zoom in and down to zoom
out (1×–128×). Each chart has independent zoom and an expand-to-fit icon button
with the **Fit history** tooltip.
After zooming, the selected timestamp stays at the right edge and history
moves left as playback advances. At the first timestamp the original
single-point domain remains until there is a nonzero history interval.
Only the viewport changes: source readings, missing-data gaps and object
metadata remain intact. Clipping retains the segment entering the left edge,
including a held reading whose original timestamp precedes the visible window.
Tooltips continue to report that original source timestamp.
**Fit history** restores the full-document axis, including future blank space.

A single dump contains one measurement and no time history. The charts show a
single point rather than fabricated activity. For history, open a JSON array
of complete dump objects in capture order:

```json
[
  {"threads": [], "heaps": []},
  {"threads": [], "heaps": []}
]
```

Use **Previous** and **Next** to navigate. Charts include measurements through
the selected frame and use a fixed 0–100% scale. Without timestamps, their horizontal
scale is capture order, not elapsed seconds. Heap history
matches by address, never by array position. Unknown measurements split the
line into separate segments: earlier history stays visible, but no line crosses
a missing-data gap.

### Recorded-speed playback

Timed JSON sequences have **Play/Pause** and **Reset** controls beside
Previous/Next, using icon-only buttons with tooltips and accessible labels.
Play advances from the selected reading at **1× acquisition
speed**, using the original timestamp intervals rather than a fixed frame rate.
All summaries, tables, charts and detail screens share the same playback frame.
Switching screens does not stop playback.

During playback, the viewport uses a continuous monotonic clock and requests
display-frame redraws, independently of acquisition updates. Zoomed charts
therefore move between recorded samples rather than jumping on each new sample.
The current value is visually held only as far as that playback cursor; this
does not create a new measurement. Tooltips keep the original source timestamp.
Rendering skips history outside the visible interval while retaining its entering
step. Actual display frame rate still depends on build mode and rendering cost.

Pause freezes the reading and fractional viewport time; Play resumes from
that exact position. Reset
pauses at the first frame without changing the current screen or filters.
Previous/Next pauses before stepping. Playback stops at the final frame;
Play at the end restarts from the beginning. Opening or reloading a valid
document cancels playback and uses the usual paused initial frame. Invalid
input leaves the current session intact.

If rendering falls behind, playback catches up to the latest due frame instead
of accumulating timing drift. Charts still retain all captured history through
that frame. No values are interpolated. Untimed dumps and single snapshots
cannot play because they do not provide a recorded acquisition cadence.

Try the preserved real acquisition:

```sh
cargo run -- test-data/zview-bench-60s/session.json
```

Choose **Reset**, then **Play** to watch the full recording from its start.

### A paused view within a 15-second recording

Export a timed sequence with the existing Python decoder. From the repository
root, with ZView's Python dependencies installed:

```sh
mkdir -p local
python export_recording.py \
  --recording /path/to/zview_qemu.ndjson.gz \
  --elf /path/to/build/zephyr/zephyr.elf \
  --duration 15 \
  --output local/qemu-session.json
cargo run -- local/qemu-session.json
```

The exporter preserves the recording clock as `elapsed_seconds`, relative to
the first usable polling frame. It decodes sequentially so CPU deltas retain
their shared baseline; it does not repeatedly invoke single-frame dumps.
It retains captured heap fragmentation maps for the Heap detail screen.
Recordings that did not capture the required reads cannot recover a map.
Maps increase document size; the desktop loader's 16 MiB input limit still applies.

Documents open and reload paused at the first frame (0 seconds for timed
recordings). All cards and the table use that same selected frame.
The charts reserve space for the full window but draw only the past; the future
remains blank. The shared percentage plot uses each original timestamp and value
on a numeric time axis, using sample-and-hold steps for every chart.
Each reading stays horizontal until the next capture; changes are vertical at
that capture's timestamp, never diagonal ramps. Tooltips use the held source
reading. This shows sampled states, not the exact time of changes between polls.
There is no uniform-grid resampling, hold-last-value projection, cubic smoothing,
or curve overshoot. Stack watermark remains a sequence of recorded high-water
marks, not instantaneous memory use. Tooltips snap to an original reading and
show its source timestamp/value, never a synthetic intermediate measurement.
Gaps and future space have no readings. Isolated measurements are shown as dots.
Percentage plots keep the fixed 0–100% domain; out-of-range values are visually clamped,
but their source values are retained in tooltips. No values are randomly generated.
**Previous** and **Next** move one recorded frame at a time. Heap switching keeps
the same selected time.

The `local/` directory is Git-ignored for machine-specific data.

Live probe capture is not implemented yet. Direct recording import is described below.

## Phase 10a — Open recordings directly

Use **Open recording**, choose a ZView `.ndjson.gz` memory-read recording, then
choose the **exact matching Zephyr ELF**. Cancelling either file picker leaves
the session unchanged. No intermediate JSON export is required.

The app runs the existing Python replay decoder off the UI thread. It decodes
sequentially, retaining CPU baselines, original timestamps, kernel-object groups,
and any captured heap maps. This is offline replay; it does not connect to a probe.
An incompatible ELF normally causes a replay/read mismatch; the app cannot
guarantee firmware identity, so always supply the ELF used during acquisition.

Python 3 with the matching ZView package installed is required (see setup above).
By default the app executes `python3`. To use an existing environment:

```sh
ZVIEW_PYTHON=/absolute/path/to/venv/bin/python \
  cargo run
```

`ZVIEW_PYTHON` is an executable path, not a shell command or a list of arguments.
The bridge uses the ZView package installed in that interpreter, without
assuming a source checkout location. The decoder script itself is embedded in
the desktop binary.

- A loading banner and **Cancel** button appear during import. The old session
  remains usable, including playback. Only a successfully decoded and validated
  document replaces it; it opens paused at the first frame.
- Invalid recordings, missing/incompatible ELFs, missing Python dependencies,
  timeout, and oversized output leave the previous data, screen, frame, filters,
  and playback intact. Errors appear in the app; another import can be attempted.
- **Reload** decodes the same recording with its remembered ELF again.
  The session footer identifies both files. Opening a JSON dump clears that ELF.
- Cancel stops the decoder subprocess and discards any completed result.
  Closing the app also requests cancellation. Imports time out after 120 seconds;
  decoded JSON retains the existing 16 MiB document limit. This is a bounded
  whole-document import, not streaming replay; the Python backend buffers the
  source recording in memory. Very large recordings should be exported to a
  shorter sequence with `export_recording.py --duration`.
- Only gzip recordings supported by the existing replay backend are accepted.
  Startup's positional argument remains a dump JSON path; recording import uses
  the two file pickers. Live capture, probe reconnection, and saved recent
  sessions remain follow-up work.

Try **Open recording** with
`test-data/zview-bench-60s/capture.ndjson.gz`, then its adjacent
`zephyr.elf`. Full import includes 578 frames and captured heap maps (one frame
at about 60.006 seconds beyond the old JSON export's 60-second cutoff). Check
Play/Pause, frame navigation, details, and Reload. Try a wrong file, then retry
with the correct inputs; the previous session should survive the failed import.

The optional integration tests exercise this same Python subprocess bridge and
the two native file-picker responses in headless GPUI:

```sh
ZVIEW_PYTHON=/absolute/path/to/venv/bin/python \
  cargo test recording::tests::real_recording -- --ignored
ZVIEW_PYTHON=/absolute/path/to/venv/bin/python \
  cargo test direct_recording_opens -- --ignored
```

## Phase 8 — Work queue detail

For a real 60-second benchmark session, see
[`test-data/zview-bench-60s/`](test-data/zview-bench-60s/README.md).
It includes the original QEMU recording, matching ELF, and desktop JSON.

Click a work queue row, or focus its row with **Tab** and press **Enter**.
The detail shows identity, worker thread, decoded flags and raw hexadecimal
flags, current sampled state, observed busy percentage, captured pending items
in queue order, and the complete drain-waiter list. Unknown measurements show `—`.
Known-empty pending lists contain no item tags;
drain waiters use the shared wait queue presentation above.
Busy does not imply a nonempty backlog: the running handler has already left
the pending list.

The lower row keeps the page's outer margins and the gaps between panels.
Of the remaining panel width, **60%** goes to Pending backlog, **20%** to Items
queue, and **20%** to Drain waiters.
All three panels have the same height, including at the minimum window size.
Each list scrolls independently; long names truncate with their full text on hover.
The **Items queue** shows a generic **Work item** while the queue's Busy flag
is set. That item is not counted in the pending backlog, and no handler name is
guessed from pending items. Its color follows queue pressure: yellow when the
pending list is complete and empty, red when work is waiting, and neutral when
backlog data is unknown. Pending work uses red briefcase tags; waiting threads
use purple spool tags, retaining queue order. Idle has no colored state fill.
Delayed and cancelled item states are not supplied by this dump format.

The **Activity history** strip shares the heap map's icon-only Zoom in, Zoom out
and Fit controls, pointer-anchored wheel zoom, drag/horizontal/Shift+scroll pan,
and Left/Right/Home/End keyboard navigation. Its coordinates are actual recorded
timestamps (or frame order for untimed documents), not converted heap offsets.
Each work queue retains its own view independently of the backlog chart.
Opening a new document resets it. The fit view uses the recording's full time
axis; unobserved/future space stays blank.

While zoomed in, a compact **minimap** beside the Activity history heading shows
the full timeline with the visible time window outlined and surrounding context
dimmed. Click outside the outline to recenter or drag it to pan without changing the zoom,
selected frame, or backlog chart. The minimap also supports Left/Right/Home/End
when focused, and disappears on Fit. It uses the same sampled pressure colors
and leaves future space blank; it does not add inferred activity.

- Timeline blocks have no inline labels; colors indicate queue pressure.
  The current-state summary and hover/click inspection retain the actual worker state.
  Red means **work waiting**, whether the worker is processing or not; yellow
  means **processing without backlog**. Idle/inactive queues without backlog
  are unfilled. Unknown pressure uses neutral stripes.
  A positive truncated pending list confirms waiting work and is red; an empty
  truncated list does not prove an empty queue and remains neutral.
  These are fixed levels, not a gradient normalized to the recording's maximum.
  **Not started** and **Not processing** remain explicit.
  **Idle** requires a started queue, a clear Busy flag and a complete, empty
  pending list. **Not processing** has a clear Busy flag but unknown/incomplete
  backlog data; pending items with a clear Busy flag instead mean **Ready**.
- Adjacent equal states and pressure levels may share a visual block; hover and click inspection
  retain the original source timestamp. A selected past reading stays available
  during forward playback, but is cleared when stepping before that reading.
  An isolated or terminal sample is a marker, not an invented state duration.
- **Observed busy · last 10s** is the time-weighted share of known started
  intervals whose left sample reports Busy. The window ends at the selected
  captured timestamp and is shorter near the beginning of a recording.
  Unknown/missing and not-started intervals are excluded from the denominator.
  Coverage and median capture interval are shown alongside the estimate.
  One snapshot or untimed data has no busy percentage. This is a sampled
  estimate, not CPU utilization or exact execution time; short jobs between
  polls can be missed. Zooming the strip does not change this explicit window.
  Its yellow meter measures activity, not backlog severity.

The neutral purple pending-backlog chart uses actual timestamps and an observed count scale
(minimum 1), not percentages or capacity. Matching uses address and name.
It uses the shared mouse-wheel zoom and **Fit history** controls.
Missing objects, missing lists, and truncated lists break exact-count history;
their observed intervals are shown as light-grey **Unknown count** background
bands spanning the numeric count plot. They are not plotted as zero
or interpolated counts, and future/unobserved space remains blank.
truncated current lists show `N+` and a partial-list notice instead of claiming
an exact total. The backlog is a sample-and-hold step line: each captured count
holds until the next timestamp, including up to the start of a missing reading.
Tooltips use that held source reading, not a nearer future sample. Neither
persistent nor hover dots are drawn. No duration is invented after the selected frame. These are
captured counts, not execution events.
Future readings are neither drawn nor used for scaling.

Previous/Next uses the shared frame. An absent queue produces the same warning
notification as mutex/semaphore details and clears current values. Escape and
Back retain search, type, time, and heap selection. Breadcrumbs open their named
lists. Declaration-site metadata is not available in the dump.

Synthetic demo (not a target capture):

```sh
cargo run -- test-data/workqs.json
```

Open `system_workq` under Kernel objects → Work queue. The fixture includes
duplicate names at different addresses, busy with an empty backlog, draining,
plugged, truncated/unknown lists and address reuse with a changed name.

For a synthetic activity demonstration with Processing, Idle, Ready, Unknown,
Not started, and Not processing states:

```sh
cargo run -- test-data/workq-activity.json
```

Open `activity_demo` and press Play. These are illustrative snapshots, not a
target recording. In the preserved real benchmark, `bench_workq` reports Busy at
every captured sample, including samples with no pending work; `k_sys_work_q`
reports Idle throughout. Neither observation is a CPU-utilization measurement.

## Phase 9 — Heap detail and fragmentation map

Click a heap row, or focus it with **Tab** and press **Enter**. The screen shows
heap identity, allocated/free bytes, payload capacity, target-reported peak
allocated bytes, usage bar, complete wait queue, and allocated-byte history.
Peak is not reconstructed from visible history. Older dumps without peak or
chunk fields still open with those fields explicitly unavailable.

- History uses actual timestamps, shared zoom/playback, and sample-and-hold
  steps. Its byte axis includes the largest reported capacity through the
  selected frame, not future capacity. Missing heaps break the line. Identity
  is matched by address and name, never by row position or name alone.
- A separate **red Fragmentation ratio** plot beside allocated bytes uses a
  fixed 0–100% scale, exact percentage tooltips, and independent zoom. It uses
  the same map accounting as the current fragmentation value:
  `100 × (1 − largest_free / free_map_bytes)`, or zero when no map bytes are
  free. Missing, empty, invalid, or uncaptured maps create gaps rather than zero
  readings. Both histories follow the shared session time without page scrolling.
- The full-width fragmentation map is a **single proportional strip**, ordered
  by captured byte offset from left to right. Purple means allocated; free
  memory is unfilled. It never wraps blocks into additional rows.
- Icon-only **Zoom in**, **Zoom out**, and **Fit entire heap** controls change the
  byte viewport, not the measurements. Vertical scrolling zooms around the
  pointer. Drag, horizontal scroll, or Shift+scroll pans. Focus the strip and
  use Left/Right or Home/End to pan with the keyboard. Zoom is bounded by the
  complete map and a one-byte visible range; panning cannot leave the map.
- While zoomed in, a compact **minimap** beside the Fragmentation map heading
  shows the entire captured heap with the current byte viewport outlined.
  Click outside the outline to recenter, or drag it to pan at the same zoom level. Focus it
  for Left/Right/Home/End navigation. Fit hides the minimap; missing/invalid
  maps do not show one. This shares its navigation and viewport overlay with
  the work queue activity minimap, while retaining byte-accurate heap data.
- Hover any visible block for its number, full size, state and relative offset
  range. Click to retain that inspection beneath the map. Selection clears
  when the captured frame or map changes, so it does not imply allocation
  identity across snapshots. Each heap has independent spatial zoom/pan state,
  retained on navigation and clamped to the current map span. Opening a new
  document resets these views.
- Labels such as `16B` and `4KiB` appear only when their entire text fits inside
  the **visible portion** of a block. Even a partly visible block reports its
  full original size, including allocator overhead. Labels do not widen blocks.
  At overview scale, subpixel blocks are coverage-weighted into pixel bins;
  zooming resolves their exact boundaries, and hit testing uses original chunks.
- Both history charts fill the area below the map, with the independently
  scrollable wait queue on the right. All remain visible without page scrolling.
- The current dump contains ordered chunk sizes, **not absolute chunk
  addresses**. Offsets begin at the first captured chunk; they are not offsets
  from the `k_heap` object. The UI does not invent memory addresses.
- Largest free chunk, allocated/total chunk count, and fragmentation use the
  TUI's map accounting. Fragmentation is `100 × (1 − largest_free / free_bytes)`
  or zero when no map bytes are free. Chunk sizes include allocator overhead,
  so map totals can differ from payload statistics.
- Uncaptured maps, empty maps, and invalid maps (zero-size or overflowing sizes)
  have explicit messages, not an all-free visualization. An absent heap clears
  current values and shows the shared warning notification.
- Escape and Back preserve frame, filters, search and overview heap selection.
  Breadcrumbs open their named lists. Narrow layouts resize the chart and map;
  scrolling stays inside the wait queue.

Small synthetic edge-case fixture (not the visual benchmark or a target acquisition):

```sh
cargo run -- test-data/heaps.json
```

Choose **Kernel objects → Heap → packet_heap** at `0x1000`. The initial paused
frame shows a fragmented heap. Previous/Next covers exhaustion, waiting threads,
address reuse/absence, recovery, and a later frame without a captured map.
The second identically named heap in the first frame checks identity handling.


For visual validation, use the full real benchmark below rather than the small
edge-case fixture. The preserved benchmark JSON predates map retention. To inspect its real maps
without changing the preserved fixture, re-export the original acquisition:

```sh
mkdir -p local
python export_recording.py \
  --recording test-data/zview-bench-60s/capture.ndjson.gz \
  --elf test-data/zview-bench-60s/zephyr.elf --duration 60 \
  --output local/bench-with-maps.json
cargo run -- local/bench-with-maps.json
```

Choose **Kernel objects → Heap → bench_heap**. This real QEMU session contains
577 snapshots over 59.905 seconds, with 421 distinct captured maps. Chunk counts
range from 2 to 130, allocated bytes range from 12 to 1,580, and map fragmentation
ranges from 0% to about 97.7%. Waiting threads are also captured.

The session opens paused at 0 seconds. Advance near 29.93 seconds to see 720 allocated bytes, 852 free bytes,
119 chunks, and 87.5% map fragmentation. Useful validation positions include
6.79 seconds (two chunks), 18.67 seconds (130 chunks), and 45.39 seconds
(about 97.7% fragmentation). These values describe this preserved acquisition,
not a synthetic workload or a performance guarantee.

The export lives in Git-ignored `local/`; regenerate it on each machine.
After exporting, run the opt-in headless integration check:

```sh
cargo test \
  recorded_heap_maps_render_and_play_from_the_full_benchmark -- --ignored
```

It checks the actual file, the full-width strip across eight representative
frames, waiting-thread visibility, chunk tooltips, chart zoom,
recorded-speed playback and narrow layout. It is ignored in the default suite
because it requires the local export; missing or incorrect data fails explicitly.
Manual native visual review remains a separate check.

## Validate

```sh
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
```

With ZView's Python dependencies installed, validate the recording exporter:

```sh
python -m unittest discover -s . -p test_export_recording.py -v
```

For a manual interaction check, open a dump, collapse and expand the sidebar,
open Help. Open thread details by clicking a row or pressing Enter after selecting
with Up/Down; return with Escape, the Back button, and breadcrumb.
With a multi-frame, multi-heap document, check frame navigation in both screens
and that returning to the overview retains the selected time and heap.
Try malformed JSON and confirm the previous document and screen remain visible
with an error. `cargo test` includes headless GPUI tests for these navigation
paths in addition to metadata and history model tests; native visual review is
a separate check.

For Kernel objects, check the sidebar submenu, name/address search, type entries,
Clear filters, and Previous/Next using the synthetic fixture above. At the
midpoint frame all three message queues are available. Switch to Threads and back to confirm
the frame and filters are retained. Resize to the minimum window width and check
horizontal scrolling. Compare state/waiter labels against a real target dump
when one with those object groups is available.

For semaphore details, click the semaphore row and check the count, finite-limit
bar, history, and waiter list at both frames. Return using Back and Escape;
confirm search/type filters and frame are retained. Check that **Kernel objects**
in the breadcrumb clears both filters, while the type breadcrumb selects that
type and clears search; both retain the frame. Also focus the row
using Tab and open it with Enter. Resize to the minimum width and check the
summary, chart, and wait queue do not overlap. Headless tests cover these paths,
missing objects/fields, duplicate names, and unlimited/uninitialized limits.

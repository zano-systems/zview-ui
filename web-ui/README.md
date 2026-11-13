# Python session adapter for the shared Rust web UI

Phase 1 connects the **existing GPUI Rust/WASM frontend** to the ZView Python
package. There is no second UI framework or Rust server. Rust owns HTTP commands,
WebSocket consumption, typed messages, client state and rendering. The browser's
existing JavaScript host still initializes GPUI and handles local JSON files.

The adapter uses `Target.open()`, `Core`, and `serialize_frame()` directly. It
does not drive the TUI or implement another ELF/memory-read decoder.

This branch is based on remote `main` at
`803bff9cea26a5739f39c8d424c1d8a36baf90ef`, which already included the shared
browser frontend. The Python package is pinned independently to upstream PR #110's
merged commit `cfc6d6d383ff4fbe04ec35f7a8d84ed99d5b71ac` (ZView 0.28.1),
not a moving branch or an earlier PR revision.

## Run

Use the repository's existing WASM setup/build, plus a separate Python 3.12
environment for the adapter. From the repository root:

```sh
just web-setup
python3 scripts/web_ui.py build
python3.12 -m venv .venv-web
.venv-web/bin/python -m pip install -r web-ui/requirements.txt
.venv-web/bin/python web-ui/server.py
```

The first setup installs nightly Rust, the WASM target and wasm-bindgen under
`target/tools`. Builds download pinned fonts. No Node/npm or probe is needed for Replay.
On Windows use `.venv-web\Scripts\python.exe` instead of `.venv-web/bin/python`.
Do not reuse the desktop exporter's environment: it targets an older ZView API.

When updating an existing web environment between commits with the same package
version, pip may keep the previously installed commit. After installing the
requirements normally, explicitly reinstall the direct pins and check dependencies:

```sh
.venv-web/bin/python -m pip install --force-reinstall --no-deps -r web-ui/requirements.txt
.venv-web/bin/python -m pip check
```

The replay lifecycle tests verify the installed package's `direct_url.json`
commit against `requirements.txt`, not just its unchanged `0.28.1` version.
CI reinstalls the exact direct pins too. Keep the desktop exporter environment
separate: its older backend pin is intentional.

Open **http://127.0.0.1:8765** in a WebGPU/WebGL2-capable browser with a viewport
of at least **1000 × 680**. Select **Configure…** in the **top-right session header**
to open **Session configuration**. On the **Replay** tab, use:

- Recording: `test-data/tests/fixtures/recordings/sys_heap_v4.3.ndjson.gz`
- Matching ELF: `test-data/tests/fixtures/zephyr.elf`

Choose **Open replay**: 20 frames arrive, the Threads/Heap views update, then
status becomes `ended`. Use **Stop decoding** in the header before opening
another active session.

For other trusted local captures:

```sh
.venv-web/bin/python web-ui/server.py --data-root /path/to/captures --port 8765
```

Paths refer to files on the adapter machine, not browser-uploaded files. Relative
paths resolve under `--data-root`; absolute paths must stay inside it too. The
input limit is 32 MiB per file, and symlink escapes are rejected.

The existing `just web-ui` JSON launcher and static hosting remain supported.
The Rust client probes `/api/session`; a 404 means static/JSON-only hosting, so
it does not show adapter controls or start a reconnect loop.

## Phase 2 — Build-aware session configuration

Session context and actions are in the top-right header on every inspection screen,
not in the sidebar. They remain visible when the sidebar is collapsed. The header
shows mode/status, observed progress/rate, the full board target and runner or
recording filename, and compact project/build context. Long context is truncated;
hover for source/ELF paths and board/build/Git details. At the minimum width the
header wraps below navigation without shrinking or hiding its buttons.

**Configure…** (settings icon) opens the draft dialog. One directly accessible
acquisition action is shown, with a labelled icon: **Connect** (plug) opens a Live
draft when no source exists; **Disconnect** (unplug) or **Stop decoding** stops an
active session; **Reconnect** (circular arrow) reopens a stopped/lost Live source;
**Open replay** (folder) configures an inactive Replay source. Actions are disabled
while pending, stopping, or disconnected from the adapter. The configuration
dialog retains its submit action, but does not duplicate Disconnect/Reconnect.

The status indicator uses both shape and text:
- **Green filled circle:** receiving fresh frames, not merely an open WebSocket.
- **Spinner:** connecting, changing configuration, or stopping.
- **Amber alert circle:** a read warning, or no fresh frame within five seconds.
- **Red alert circle:** connection/acquisition failed or lost.
- **Gray dashed circle:** inactive, finished, or adapter disconnected.

A valid new frame clears the transient warning indicator; snapshots retain the
active warning separately from the historical last-notice text. Freshness expires even
if no new events arrive; a restored old snapshot cannot turn the light green.
Stale live rate text is replaced by `— samples/s`. Reduced-motion preferences are
handled by the shared spinner component. There is no displayed sample-age counter.

The dialog has only **Live** and **Replay** tabs:

- Use the segmented tab bar to switch modes. When focused, **Left/Right** and
  **Home/End** select Live/Replay without applying the draft.
- **Replay:** select a recording and either its matching ELF or a Zephyr build
  directory. With a build directory, leave the ELF field empty to use the
  discovered ELF; a supplied ELF is an explicit override. It must match the
  recording. The file format still requires an external ELF.
- **Live:** only the build directory is required. Choosing or typing it
  automatically previews the board, ELF, debug runner, target and probe clock.
  **Advanced overrides** optionally enables image/ELF/runner/MCU/GDB-address/clock
  fields; with it off, only the build directory is submitted.
  Choosing a different build clears the previous build's overrides.
  Detection never opens a probe; **Connect** does, after server-side revalidation.
- **Resolve build context** is read-only. It delegates firmware-image and ELF
  resolution to ZView's Python package. `runners.yaml` supplies Live defaults:
  the supported `debug-runner` takes precedence over `flash-runner`, so a Nordic
  build flashed with `nrfutil` but debugged with `jlink` uses J-Link for reads.
  Without a supported debug runner, the adapter prefers a supported persistent
  flash runner or listed J-Link/pyOCD backend, then the package's own fallback.
  Replay does not require a supported probe runner. Warnings are displayed.
- A sysbuild's default image is selected unless **Firmware image** names another.
  After resolution, multi-image builds also show image-selection buttons.
  This selects one image, not simultaneous acquisition from several cores.
- Paths are server-local, relative to `--data-root`. Selected files, image
  directories, and generated artifacts must remain inside that root, including
  symlink resolution. Choose a common workspace root if source/build directories
  are separate and you want source metadata.
- Build directory, ELF and Recording fields have browse icons that open the
  **system's native file/folder dialog**. Rust owns the controls and draft state;
  the local Python adapter invokes the OS dialog and validates its returned path.
  There is no custom in-app filesystem browser and no Python-rendered GUI.
  An empty field starts in the current project's source folder when known and
  accessible, otherwise the configured data root (the app project by default).
  A populated field reopens at the selected folder or file's parent.
  Cancelling the native dialog leaves the draft unchanged. Paths must still
  stay inside `--data-root`, even though the OS dialog can navigate elsewhere;
  an out-of-root/wrong-type selection is rejected without replacing the field.
  Manual entry remains available. Only one system dialog may be open at a time.
  Late replies cannot overwrite intervening edits or a dismissed draft.
- Board identity comes from `.config` (`CONFIG_BOARD_TARGET`, with older-board
  fallbacks) and CMake cache information, never from a guessed filename.
  Missing identity is **Board unknown**.
- Git information is best-effort, captured from the available source checkout
  during resolution. It is labelled **Checkout at resolution**, not proof of
  the ELF's build revision. No remote is contacted. Context supplied at session
  open is not metadata embedded in the original recording.
- Inputs are drafts: resolving, changing modes, or cancelling does not change
  acquisition. **Escape/Cancel** discards the draft; **Enter/Connect/Open replay**
  submits the configuration. The dialog closes on HTTP acceptance;
  later decode failures remain visible with the previous displayed data.
- Outdated resolver responses are ignored after edits or dismissal. An active
  session must be stopped/disconnected explicitly before opening another.

Successful replay opens retain their resolved context in the shared snapshot,
so reconnect/reload/other tabs show the same board/build details without
resolving the filesystem again. Changing a build later does not silently relabel
an existing session; resolve and open a new one.

For Replay the header shows **decoded frames**, not a recording's sample rate or
Playing/Paused time. For Live it shows status, board/runner, observed samples/s,
and project/build context. There is no sample-age line.
There is no Offline mode in the adapter dialog; legacy desktop
and static-JSON workflows remain separate and unchanged.

## Phase 3 — Live monitoring

1. Start the local adapter with a `--data-root` containing the build and its ELF.
2. Select **Connect** or **Configure…** in the header, choose **Live**, and select
   the build directory. No runner, target or ELF entry is needed for a complete build.
3. Review the automatically detected settings. **Advanced overrides** is only
   needed for an explicit alternative, such as an external GDB server address
   that is not recorded in the build. The probe clock is not the frame polling rate.
4. Select **Connect**. The server resolves the
   request again; browser previews are not trusted connection configurations.
5. **Connecting** stays visible until the first valid frame, then **Polling**.
   Firmware/driver/open failures show **Connection failed**; fatal read errors
   show **Connection lost**. Transient faults appear as notices.
6. **Disconnect** waits for acquisition cleanup and releases the connection.
   Last received measurements remain labelled as retained data. **Reconnect**
   explicitly creates a new Target/Core, re-resolves the saved requested build
   configuration, and starts fresh baselines/history with a new session ID.
   It returns to the Threads overview; an open detail view and its chart history
   are not resumed. CPU averaging is preserved separately.
   If the build changed, the newly resolved board/ELF/settings are shown.

By default polling requests one frame every 100 ms (10/s), but reads may take longer.
The observed **samples/s** counts successful frames received by the adapter
before browser fan-out, over up to five monotonic seconds. It stays `—` until
one second of observations exists, falls to zero when frames stop arriving,
and is cleared on termination. It is not a low-level memory-read rate or a
guarantee of lossless target sampling: Core's bounded queue can drop frames.
The browser's redraw rate does not control acquisition.

The active screen selects the live groups and rows read through `Core.select()`.
Firmware capability notices and wait-queue limitations are shown in the dialog.
Live heap statistics are available when supported. Fragmentation maps are requested
only while a browser is inspecting a heap, as described in Phase 4 below.
Unavailable measurements remain unknown, not zero.

Live CPU and Load use the **same counter window as ZView's TUI** through
`Core.usage_window`, defaulting to **16 polls**. The **CPU / Load** controls on
Threads and Thread details halve/double this to 1–64 polls; with this control
focused, `+`/`=` doubles and `-` halves, as in the TUI. Each poll still
produces a reading: this is a rolling window, not batches of 16 or a slower
acquisition rate. The backend sums cycle deltas before division: CPU is thread
cycles / all cycles; Load is thread cycles / non-idle cycles. It does not average
rounded percentages or compressed chart changes. Early readings use the available
polls. Only the tab controlling Live acquisition can change the window; the applied
value is shared with other tabs and survives reload and explicit reconnect.
Changing it takes effect between polls and marks a boundary in chart history;
existing points are not recalculated. Unsupported snapshot aggregation reports
the backend's effective one-poll window. Replay is unchanged.

The newest received sample is always followed. Tables/details keep only the latest
full Live frame; charts retain **five minutes by elapsed time for the current
screen visit**, not a fixed number of frames. Entering a screen starts fresh: zero
is its first valid reading, not the session's start. Returning to that screen
starts a new visit rather than restoring its earlier curve.

The default visible window is **20 seconds**. It initially grows from zero with
the collected observations, then rolls forward. The footer's **Window** slider
selects **1–300 seconds** for all Live charts on the screen, including the work-queue
activity strip. Short windows have finer pointer control using a logarithmic track.
The duration readout appears once, after the slider; clicking it restores 20 seconds.
With the control focused, arrow keys change
one second, Page Up/Down change ten, and Home/End select the bounds.

Changing the slider resets individual chart zoom/pan to follow the selected window.
Wheel zoom can still inspect an individual chart more closely, **Fit history** shows
retained history, and **Last Ns** restores the selected slider duration. Neither the
slider nor zoom changes acquisition or deletes retained observations. The selected
duration remains while navigating screens; page reload restores the 20-second default.

Chart storage is separate from full snapshots. Per-object correlated measurements
(including ownership/state needed by tooltips), overview metrics and fragmentation
measurements use change logs: repeated values advance the last confirmed observation
time without allocating another historical sample. Both the first and last actual
observation in an unchanged run remain available for drawing, so compressing a
flat run does not shift a later rise or fall earlier in time. Large heap chunk maps are not
copied into statistical history. The latest map remains separately retained.

Leaving a screen discards its visit's chart history. A read warning or a transport
interruption within the same visit ends confirmed coverage; charts do not bridge
those gaps or substitute zero. A constant value means unchanged at the observations,
not proof that no operations occurred between reads.

Live percentage charts (including fragmentation) connect actual readings with
straight lines, not stairs or smoothed/generated samples. Counts and categorical
states keep their sample-and-hold steps. Acquisition cadence remains independent
of storage and zoom: a slow map period/cooldown cannot capture fast transitions.
Dense numeric charts draw per-pixel minimum/maximum envelopes so spikes do not
disappear into averages. Zooming in reveals exact transitions; hover always queries
the retained observations, not an invented averaged sample. Mutex/semaphore
companion state stays timestamp-aligned. The work-queue busy estimate remains
time-weighted; change intervals are not mislabelled as polling cadence.

A conservative **16 MiB accounting budget for chart history** is a safety bound,
not a limit on total browser memory. Rapidly changing data may reach it before five
minutes: oldest coverage is then removed without averaging, and the footer/captions
explicitly report the shortened history. Full-resolution data already discarded
cannot be recovered by zooming. History is local to each browser tab and resets
on page reload or a new session; it is not recording-to-disk.

Live chart axes use monotonic elapsed **adapter receipt time relative to the
current visit**, not the board's clock or original recording time. They restart
on entering a screen, not when the rolling time window expires.
Live Play/Pause/Previous/Next/Restart controls are unavailable.
Previous data is never relabelled as a newly selected target while it connects
or fails; a notice identifies the retained display until a valid new frame arrives.

Only one tab controls Live selection at a time; selections are not combined.
A closed tab or a disconnected WebSocket releases control and map demand but
does not stop the probe. Reconnecting the browser restores a snapshot,
not missed history, without opening another probe. Target reconnection is never
automatic. Source changes require an explicit stop. Server shutdown requests
stop and waits for the current operation to finish; a blocking native driver
cannot be safely force-cancelled by cancelling its Python thread.

No flashing, rebuilding, recording-to-disk, or multi-target acquisition is added.
Actual board/probe attachment,
unplug/reconnect, running-target behavior and release must be validated on
hardware. Automated Live tests use a controlled backend, not an attached board.

### Native dialog availability

The app always runs locally: the browser and adapter run on the same user's
machine. The system dialog opens on that machine, with no file uploads.

- **macOS:** built-in `osascript`/Standard Additions file and folder dialogs.
- **Windows:** Windows PowerShell in STA mode, using system file/folder dialogs.
- **Linux desktop:** `zenity`, falling back to `kdialog`, must be installed.
- Without an interactive desktop/native helper, enter paths manually. Missing
  helpers or display errors appear in the draft; they do not stop acquisition.

Native helpers run asynchronously, time out after five minutes, and are
terminated on adapter shutdown. Finish or cancel an existing system dialog
before requesting another. Restart the adapter and refresh the browser after
updating the dialog endpoint/frontend.

## Screen-driven Live selection

The Rust screen declares its requirements and posts them to
`POST /api/session/view`. The Python adapter validates against the session's
discovered inventory and compiles the request into ZView's public `Selection`.
Selection and optional heap detail are changed together between bounded polls:

| Screen | Acquired data |
|---|---|
| Threads overview | Threads, plus the one heap selected for its cards/charts |
| Thread detail | Selected thread only; ZView retains its own CPU-accounting dependencies |
| Kernel objects | Selected type, or all kernel-object groups for All types; query filters narrow rows |
| Heap detail | Selected heap statistics and its map when due |
| Semaphore/message queue/slab/mutex/work queue detail | Selected row of that group |

One initial full sample supplies object names/addresses for navigation and heap
choices. It is not repeated on every screen change. Sidebar counts describe
that inventory, not freshly polled counts of every object type.
Inventory is not refreshed during an active session: threads or kernel objects
created after connection are not newly discovered or selectable. Disconnect and
Reconnect to discover them; periodic runtime discovery is a follow-up.

Frames include `view`, `view_revision`, and `sampled_groups`. Old in-flight frames
are rejected after a view change. Until a matching sample arrives, the new screen
shows a loading state instead of treating omitted groups as empty or showing
retained values as current. Object search stays editable during that transition.
An empty query result is throttled rather than creating a busy empty-frame loop.

Only one connected browser is the controller. Another tab may receive the
stream, but cannot change acquisition until the controlling tab closes. There
is no union of tab selections and no extra poller. Closing a tab removes map
demand; ordinary acquisition retains its last selection until another controller
chooses a view or the session is stopped.
After an ownership conflict, the waiting browser retries when the owner releases
control, even if the retained selection belongs to a different screen. The last
selection is not itself evidence that a controller is still connected.

Replay is unchanged: navigation never changes its recorded read sequence.

## Phase 4 — Live heap fragmentation

Open **Kernel objects**, then a heap row. Live heap details automatically request
the selected heap's chunk map through the package's public `Core.detail("heaps",
address)` API. The address is the heap identity in the received measurements.
No second heap decoder, target writes, or extra probe connection is introduced.

- Only **one detailed heap per session** is selected by the controlling tab.
  There is no rotation, multi-tab union, or background acquisition of other heaps.
- By default map capture is requested at most once per second. Normal statistics
  keep the requested 100 ms period between captures. Large maps can take longer
  and lower the observed sampling rate. A full second of cooldown follows each
  completed map read, so a slow probe cannot immediately start another map and
  starve ordinary statistics. Startup options below can explicitly change this.
- The worker uses public one-frame bounded Core polls, stops/joins each poll and
  changes detail only between those polls. This avoids an in-flight detail race
  and accidental full-map acquisition on every ordinary frame.
- Leaving heap details releases that tab's interest. Closing/reloading a tab,
  losing its WebSocket, or removing a slow client releases its interest too.
  The controlling tab leaving disables map reads. Reconnecting the browser
  resubmits only its currently displayed heap.
- The last successfully captured map is retained between captures, with its
  actual live elapsed time. It is not copied into later measurement frames.
  Fragmentation history has samples only where maps were captured. Reload
  restores the last map, not all missed history.
- An empty map, absent map, invalid map and stopped acquisition keep distinct
  presentations. Unavailable fragmentation is `—`, not zero. Layout zoom/pan
  continues to use the existing heap map.
- A read notice during a map attempt disables optional map capture conservatively
  and shows an error; the Core may retry normal reads. It does not assume the
  notice was definitely caused by the map. Close all views of that heap and reopen
  to retry. Genuine connection failures retain the normal lost-session behavior.
- Disconnect, target loss and a new session clear interests. Old-session and
  superseded-heap results cannot populate the new map.

This is sampled allocator layout, not an allocation/free event trace. Map chunk
bytes include allocator layout overhead and must not be equated directly with
payload byte counters.

### High-speed Live acquisition

These adapter-level startup options apply to Live sessions, not Replay:

- `--period`: minimum frame polling period in seconds; `0` polls back-to-back.
- `--heap-period`: positive minimum start-to-start map period in seconds.
- `--heap-cooldown`: minimum wait after completing a map; `0` disables the
  protective cooldown. It is independent of the start-to-start period.

For back-to-back polling with a requested maximum of 10 map updates/s:

```sh
.venv-web/bin/python web-ui/server.py --port 8766 --data-root /path/to/project \
  --period 0 --heap-period 0.1 --heap-cooldown 0
```

Defaults remain `--period 0.1 --heap-period 1 --heap-cooldown 1`. Values must be
finite; frame period/cooldown may be zero, but heap period must be positive.
Restarting the adapter applies new options; then reconnect the target. Snapshots
report the effective `timing`, and the header tooltip/map caption show those
requested values instead of assuming the default rates.

The configured rate is not guaranteed. Reads, serialization, and browser delivery
take time. A map read longer than 100 ms cannot sustain 10 updates/s. With zero
cooldown on a slow probe, every frame may include a costly map; monitor observed
samples/s and return to the defaults if the additional probe/CPU load is unwanted.

An opt-in integration check starts QEMU and the production adapter on private
loopback ports, measures 40 ordinary frames and 40 frames with map capture,
checks repeated maps and release, then stops both processes:

```sh
.venv-web/bin/python web-ui/qemu_heap_smoke.py --build-dir /absolute/path/to/existing/build
```

For a check that does not depend on a separately built application, use the
checked-in `mps2/an385` benchmark ELF:

```sh
.venv-web/bin/python web-ui/qemu_heap_smoke.py --fixture
```

This creates temporary build-resolution metadata under `target/` around the
existing ELF. It does not compile firmware or change the fixture.

It requires `qemu-system-arm` and an already-built `mps2/an385` image with ZView's
heap configuration. It never rebuilds, flashes, or edits that build. QEMU/GDB
does not validate physical probe drivers or USB behavior.

## Replay behavior and limits

- This section describes Replay, not a live board connection.
- Frames decode at up to 10/s, not original recorded timing. `decoded_at` is
  acquisition time, not a recorded timestamp or board sample-rate measurement.
- Frames feed the shared Rust model and existing screens. The last 256 received
  frames are retained for inspection; original-time Play/Pause is unavailable.
- Two tabs share one backend poller. Each tab has an independent stream consumer.
  Closing a tab does not stop acquisition.
- Reconnect/reload restores the current status and latest frame, not all missed
  history. The last displayed document survives an unsuccessful open.
- Typed Rust envelopes cover snapshots, frames, notices and statuses. Unsupported
  versions, unknown variants and malformed data are rejected. Duplicate, stale
  and wrong-session events are ignored; sequence gaps cause snapshot recovery.
- HTTP replies acknowledge commands; they cannot overwrite newer stream state.
- Runtime `is_active` from the new package is accepted alongside legacy `active`
  by the shared model; CPU/Load retain their existing meanings.
- Explicit `is_idle` controls thread rows and CPU/stack aggregates. Only older
  dumps without that field fall back to the exact legacy name `idle`; explicit
  false is honored even for a thread named `idle`. The wire round-trip preserves
  absent flags rather than introducing false.
- Work queues accept both `is_pending_truncated` and legacy `pending_truncated`.
  A truncated empty list does not imply an idle worker or no backlog.
- After a queue timeout, replay rechecks queued events once polling stops before
  inferring a missing terminal event. Late `ReplayEnd`/failure events are drained.
  Explicit worker cleanup remains in place alongside `Target.open()`'s merged
  stop-before-disconnect guarantee.

Self-contained recordings and faithful replay timing remain
later work. The application still supports its pre-existing offline JSON
workflows; this phase does not remove those supported interfaces.

## HTTP and WebSocket contract

The adapter serves `target/web/` and API from the same origin. Build the Rust
application before running static-asset or browser tests. An unbuilt frontend
returns 503 with setup instructions; the API can still run.

| Route | Behavior |
|---|---|
| `POST /api/files/choose` | Open a system dialog: `{"kind": "elf", "current": "..."}`; kinds: `directory`, `elf`, `recording`. Returns `{"path": "..."}` or `{"path": null}` on cancellation |
| `POST /api/build/resolve` | Read-only context preview: `mode`, `build_dir`, optional `domain`/`elf`; Live also accepts `runner`, `target`, `frequency` (Hz) |
| `GET /api/session` | Latest session snapshot |
| `POST /api/session/replay` | JSON `{"recording": "...", "elf": "..."}` or `{"recording": "...", "build_dir": "...", "domain": "..."}`; ELF override optional with a build |
| `POST /api/session/live` | `build_dir` plus optional `domain`, `elf`, `runner`, `target`, `frequency`; opens one target |
| `POST /api/session/stop` | `{"session_id":"..."}`; wait for cleanup; idempotent for the same current session |
| `POST /api/session/heap-detail` | Compatibility endpoint: select heap details, or Overview with `heap:null`; the browser uses `/api/session/view` instead |
| `POST /api/session/view` | `{"session_id":"...","client_id":"...","view":{...}}`; select one screen's groups/rows and optional map together; a second controller receives 409 |
| `POST /api/session/usage-window` | `{"session_id":"...","client_id":"...","polls":16}`; controlling Live tab requests 1–64 polls; `usage_window` events acknowledge the backend's applied value |
| `POST /api/session/reconnect` | `{"session_id":"..."}`; reopen an inactive Live session with fresh resolution/identity/history |
| `GET /api/events` | WebSocket; initial snapshot, then events |

Open returns 202 after path validation, not after successful decoding. Decoder
errors arrive as `failed` status. Invalid requests return 400, an active session
returns 409, and non-JSON open requests return 415.

```json
{
  "version": 5,
  "type": "status",
  "session_id": "opaque-session-id",
  "sequence": 1,
  "data": {"status": "reading"}
}
```

Types: `snapshot`, `status`, `frame`, `notice`, `capabilities`, `health`, `heap_detail`, `view`, `usage_window`.
Statuses: `opening`, `reading`, `stopping`, `stopped`, `ended`, `failed`, `lost`.
Sequences increase within a session. `source` is a tagged union:
`{"mode":"replay","recording":"..."}` or
`{"mode":"live","configuration":{"build_dir":"...", "...":"..."},
"runner":"...", "target":"...", "frequency":4000000}`.
The Live configuration stores requested inputs; effective settings and build
context are separate. Snapshots include common `elf`, `context`, frame count,
last frame/notice/error, `read_warning`, optional capabilities, and `samples_per_second`.
Live snapshots additionally include `timing` (`period`, `heap_period`,
`heap_cooldown` in seconds); Replay leaves it null. Timing is informational here,
not a browser-supplied connection override.
They also include the initial `inventory` and `live_view` (view descriptor,
revision, controlling connection ID). A `view` event updates that state.
Live frames identify the selection revision and actual sampled groups.
Frames carry `decoded_at` (Core acquisition epoch time) and serialized `groups`;
Live additionally carries `live_elapsed` (monotonic elapsed adapter receipt time).
`health` carries `samples_per_second`; `capabilities` carries discovered `groups`,
`missing` firmware notices and `is_waitq_walkable`.

`heap_detail` contains the selected `heap`, optional `last_map`, and optional
`error`. A map carries heap identity, chunks, `captured_at`, `live_elapsed` and
`frame_number`; it is separate from current-frame freshness. Browser WebSockets
register a fresh per-connection UUID through `/api/events?client_id=...`. Only a
connected client can acquire interest. The ID is a lifetime association, not
remote authentication.

**Migration from protocol 4:** rebuild WASM, restart the adapter and refresh tabs
together. Protocol 5 adds `usage_window: {"polls":16}` events and the snapshot's
effective `usage_window` (null before worker acknowledgement); `live_view.usage_window`
is the requested value. CPU/Load smoothing happens in the backend, before serialization.

**Migration from protocol 3:** rebuild WASM, restart the adapter and refresh tabs
together. Use one controlling Live tab. View changes now reduce actual reads,
and partial frames must not be interpreted as complete snapshots.

**Migration from protocol 2:** rebuild WASM, restart the adapter and refresh tabs
together. Protocol 3 introduced heap-detail events; protocol 4 also
coordinates screen-driven selection. Always update client and adapter together.

**Migration from protocol 1:** rebuild the WASM client and restart the Python
server together, then refresh all tabs. Top-level `mode`/`recording` move into
`source`; stop commands now require the expected session ID. Stale stop/reconnect
commands return 409 and cannot affect a replacement session. The shared
native/static JSON document format is unchanged.

One Python worker consumes the core queue and forwards to the asyncio loop with
bounded outstanding delivery. Each browser has a 16-message queue. Slow clients
are disconnected with code 1013 and recover from a snapshot, rather than silently
losing terminal events. This is a monitoring stream, not lossless recording.
Stop/shutdown wait for the in-flight read and `Core.stop()` before closing the
target context; they do not forcibly terminate threads.

## Local security boundary

The service binds only `127.0.0.1`, rejects foreign Hosts and cross-origin
requests/upgrades, and serves only generated assets and explicit API routes.
The native-dialog endpoint returns only the user's selected, validated relative
path. There is no directory-listing or arbitrary file-serving API. Selections
outside `--data-root`, including symlink escapes, are rejected. OS helpers are
invoked with argument arrays (no shell); paths are never interpolated into script
code. There is no remote-service mode or authentication. Do not expose this
service through a public proxy/tunnel.

Use trusted files only. ZView buffers decompressed entries: the compressed-file
size limit does not cap decompressed memory or parsing time. This is not an
untrusted-file sandbox. Direct Python dependencies and the backend commit are
pinned; transitive pip dependencies are not fully locked.

The content policy permits same-origin scripts/WASM and the existing GPUI host's
inline styles. It does not allow inline scripts or JavaScript `unsafe-eval`.

## Validate

From the repository root:

```sh
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo +nightly clippy --locked --lib --target wasm32-unknown-unknown -- -D warnings
python3 scripts/web_ui.py build
node --test web/documents.test.mjs
python3 -m unittest discover -s scripts -p test_web_ui.py -v
.venv-web/bin/python -m pip check
.venv-web/bin/python -m unittest discover -s web-ui -p 'test_*.py' -v
```

For real-browser validation:

```sh
.venv-web/bin/python -m pip install -r web-ui/requirements-browser.txt
.venv-web/bin/python -m playwright install chromium
.venv-web/bin/python web-ui/browser_smoke.py
```

Pass `--executable-path /path/to/chrome` to use an existing Chromium browser,
or `--screenshot /path/to/image.png` to save the final rendered view.

The smoke test loads the actual GPUI WASM bundle. It calls the Rust replay command
functions and inspects a read-only Rust state export (`adapter_state`), since
canvas-drawn rows have no DOM table. It also uses real canvas mouse/keyboard
events at fixed viewports for header/dialog controls. It verifies 20 decoded
frames, thread count/CPU, two-tab snapshots, reconnect/reload, failure/retry,
stop/restart, build resolution, keyboard apply/cancel, a late cancelled resolver
response, controls with the sidebar collapsed, keyboard mode tabs, and the minimum viewport.
Live controls use a controlled acquisition worker (not hardware) and real header
clicks for Disconnect/Reconnect. Checks include spinner/green/amber/red/gray
transitions, stale-frame expiry even with a connected socket, old snapshot recovery,
bounded history, and controls on both Threads and Kernel objects.
Native-picker bridge tests replace only the OS-dialog boundary with scripted
responses: the actual Rust buttons, HTTP route, validation and draft updates run.
They cover all three selections, cancellation, out-of-root rejection and late
results after edits/dismissal. They do **not** automate the OS's native window.
Unit tests cover platform commands and process cleanup/timeout/cancellation;
manual native-dialog checks are still needed on each OS.
Screenshot output also includes `phase2-session-dialog.png`, `phase3-live-dialog.png`,
`session-header.png` and `session-header-stopped.png` for layout/status inspection.
Browser-wide assistive-technology behavior and other canvas interactions still
need manual validation.

The existing browser CI job now runs adapter and compiled-WASM smoke checks
after its normal web build. A local pass is not a hosted Linux CI result.

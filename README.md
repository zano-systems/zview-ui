# ZView UI

An experimental native and browser application for exploring Zephyr RTOS thread
activity, kernel objects, and memory usage, built in Rust with
[GPUI Kit](https://gpui-kit.com/).

ZView UI is a companion frontend to [ZView](https://github.com/rodrigopex/zview),
which provides data acquisition and decoding. Viewing exported JSON does not
require ZView's Python backend; native recording import and the web session adapter do.

## Status

**Under construction.** The application and its documentation are experimental
and may change. The project is not open for contributions at this stage.
The local Python web adapter supports Live connections and Replay. Live hardware
support requires the appropriate probe tools/drivers and hardware validation;
MCP integration is not implemented.

## Current capabilities

- Offline snapshot inspection and recorded-session playback.
- Threads overview and per-thread runtime and stack details.
- Kernel objects overview with search and type filters.
- Detail views for semaphores, message queues, mutexes, memory slabs,
  work queues, and heaps.
- Time-history charts, work queue activity, and captured heap fragmentation maps.
- One shared Rust UI for native and WebAssembly builds. The browser accepts
  already-decoded JSON without a Python backend. An experimental
  [Python session adapter](web-ui/README.md) streams live target measurements or
  decoded recordings into that browser UI through HTTP commands and WebSocket events.
- The adapter UI has persistent top-right session controls, with editable
  Live/Replay settings, explicit Connect/Disconnect/Reconnect, and observed live
  samples/s (successful frames received by the adapter). Build directories
  supply board/image/ELF context; Git details are labelled as checkout metadata,
  not proof of the firmware's build revision.
- Connection actions have labelled icons. A green circle means fresh frames are
  arriving; busy, waiting/warning, failed and disconnected states have distinct
  indicators and text, not color alone.
- Opening Live heap details requests one heap's fragmentation map on demand.
  One controlling tab selects only the groups and rows shown by its screen.
  Leaving heap details releases map demand. Acquisition periods are configurable;
  default map capture includes a one-second cooldown.
- Live charts retain up to five minutes of change-aware history, independent of
  polling rate, starting fresh at zero on each screen visit. The visible range
  grows up to a 20-second rolling window, with per-chart wheel zoom, accurate
  visible-duration labels, and explicit reset/fit actions. Percentage
  charts connect real readings; count/state charts retain steps. Observation
  gaps remain gaps; dense views preserve extrema using pixel envelopes.
  A chart-memory safeguard explicitly reports any shortened retention.

## Quick start

Install Rust, [just](https://just.systems/), and the
[GPUI Kit native prerequisites](https://gpui-kit.com/docs/installation).
From the repository root:

```sh
just ui
```

For the browser, also install Python 3.10+ (standard library only) and set up
the WASM build tools once:

```sh
just web-setup
just web-ui
```

Both launch commands default to the official
[`session-with-heap-maps.json`](test-data/zview-bench-60s/session-with-heap-maps.json):
577 snapshots over approximately 60 seconds, all object groups, and 421 distinct
heap maps. Open **Kernel objects → Heap → bench_heap** to inspect fragmentation.
See the [benchmark guide](test-data/zview-bench-60s/README.md) for provenance,
regeneration, and checksums.

Pass another decoded JSON file explicitly, or `""` for the bundled small sample:

```sh
just ui file.json
just web-ui "path with spaces/file.json"
just web-ui ""
```

Bare `cargo run` also uses the small sample. Native typography is unchanged;
the browser bundles IBM Plex Sans and JetBrains Mono.

## West integration (`west zview-ui`)

In a Zephyr workspace, ZView UI is a west module whose `west zview-ui` command
builds the browser UI, starts the [Python session adapter](web-ui/README.md),
connects Live to the target described by the build directory, and opens the
browser. Add both projects to your manifest. ZView must be at the commit the
adapter is pinned to (see `web-ui/requirements.txt`):

```yaml
manifest:
  remotes:
    - name: wkhadgar
      url-base: https://github.com/wkhadgar
    - name: zano-systems
      url-base: https://github.com/zano-systems
  projects:
    - name: zview
      remote: wkhadgar
      revision: cfc6d6d383ff4fbe04ec35f7a8d84ed99d5b71ac
      path: modules/tools/zview
      west-commands: scripts/west-commands.yml
    - name: zview-ui
      remote: zano-systems
      revision: main
      path: modules/tools/zview-ui
      west-commands: scripts/west-commands.yml
```

One-time setup: install the Python dependencies of both modules (ZView's and
`aiohttp`), and then the WASM tools inside the module checkout:

```sh
west update zview zview-ui
west packages pip --install
(cd modules/tools/zview-ui && just web-setup)
```

Build with the ZView snippet and launch:

```sh
west build -b <board> app -S zview
west zview-ui                  # build dir from build.dir-fmt, cwd, or ./build
west zview-ui -d other-build --no-open --port 0
west zview-ui -r gdb -t 127.0.0.1:1234   # e.g. QEMU's gdbstub (no runners.yaml)
```

The command uses the manifest's `zview` checkout, falling back to an installed
`zview` package. It warns if the checkout differs from the adapter's pin.
Runner, target and probe clock come from the build's `runners.yaml`
(`debug-runner` before `flash-runner`). Override them with `-r/-t/--frequency`,
the ELF with `-e`, or the sysbuild image with `--domain`. The adapter may only
read paths under its data root. That root is the west workspace when it holds
the build and the application sources, otherwise their nearest common directory;
`--data-root` overrides it. If auto-connect fails, the error is printed and the
server keeps running so you can fix the settings in **Configure…**. Each run
rebuilds the WASM site incrementally; `--no-build` serves the existing one.
It listens on `127.0.0.1:8765`, or on a free port (printed) when 8765 is taken;
an explicit `--port` that is taken is an error.
**Ctrl+C** disconnects the probe and stops the server.

### Browser behavior and static hosting

The launcher prints a local URL, opens the browser, and serves only generated
assets and the selected JSON on `127.0.0.1`. Keep the terminal running;
**Ctrl+C** stops the server. **Offline ▾ → Open dump…** reads another file
locally without uploading. Select a changed file again to reload it. Failed
loads retain the current document. Inputs are limited to 16 MiB.

Use a desktop-sized viewport of at least **1000 × 680**. GPUI prefers WebGPU
and falls back to WebGL2; browser/driver support and background-tab playback
timing can differ from native. This is not a mobile layout.

GPUI Kit 0.7.3's web dependencies require nightly Rust. `web-setup` installs
the WASM target and matching wasm-bindgen CLI under `target/tools`; native
builds remain on stable Rust. Set `ZVIEW_WEB_TOOLCHAIN` to override the web
toolchain. The first build downloads pinned, checksum-verified fonts and Rust
dependencies; Node/npm and a ZView Python installation are not needed to launch.

```sh
just web-build
```

Serve `target/web/`, including `pkg/` and `fonts/`, over HTTP(S), not `file://`.
Keep `fonts/OFL.txt` with the distribution. Static output contains no selected
user JSON and starts with the small sample; choose **Open dump…** to load the
official test session or another file.

## Validation

```sh
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
node --test web/documents.test.mjs
python3 -m unittest discover -s scripts -p test_web_ui.py -v
cargo +nightly clippy --locked --lib --target wasm32-unknown-unknown -- -D warnings
just web-build
```

The full-data heap-map test runs in the normal Rust suite without Python.
For recording-import integration tests, install the pinned ZView backend and
dependencies from [CI](.github/workflows/ci.yml), set `ZVIEW_PYTHON` to that
interpreter, and run `cargo test --locked -- --include-ignored`. CI also
regenerates the official fixture and compares it with the committed JSON.

Builds and headless tests do not replace browser/native visual checks. Inspect
file-error recovery, navigation, playback, heap maps, zoom, and resizing.
Firefox/Safari and hosted CI still require validation; the browser CI job uses
a rolling nightly toolchain. See [CONTRIBUTING.md](CONTRIBUTING.md) for maintainer
conventions.

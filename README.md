# ZView UI

An experimental native and browser application for exploring Zephyr RTOS thread
activity, kernel objects, and memory usage, built in Rust with
[GPUI Kit](https://gpui-kit.com/).

ZView UI is a companion frontend to [ZView](https://github.com/rodrigopex/zview),
which provides data acquisition and decoding. Viewing exported JSON does not
require ZView's Python backend; native recording import does.

## Status

**Under construction.** The application and its documentation are experimental
and may change. The project is not open for contributions at this stage.
Live target connectivity and MCP integration are not implemented.

## Current capabilities

- Offline snapshot inspection and recorded-session playback.
- Threads overview and per-thread runtime and stack details.
- Kernel objects overview with search and type filters.
- Detail views for semaphores, message queues, mutexes, memory slabs,
  work queues, and heaps.
- Time-history charts, work queue activity, and captured heap fragmentation maps.
- One shared Rust UI for native and WebAssembly builds. The browser accepts
  already-decoded JSON only; recording/ELF decoding remains native-only.

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

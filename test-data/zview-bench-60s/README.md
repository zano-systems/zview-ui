# ZView benchmark — 60-second QEMU capture

Real execution of [zview-bench](https://github.com/wkhadgar/zview-bench) in
dynamic mode, not synthetic readings. Captured on 2026-10-19.

## Official UI test dataset

`session-with-heap-maps.json` is the official full-data fixture for native and
browser UI testing. It is already decoded, includes all object groups and
captured heap chunk maps, and needs no Python backend to open.
From the zview-ui repository root, both recipes default to this file:

```sh
just ui
just web-ui
```

Without `just`, pass the file explicitly:

```sh
cargo run -- test-data/zview-bench-60s/session-with-heap-maps.json
```

Open **Kernel objects → Heap → bench_heap** for the fragmentation map, heap
statistics, waiting threads, and time history.

Open **Kernel objects → Work queue → bench_workq**. Pending work includes
`tick_work (work_handler)` and `trail_work (work_handler)`; observed backlog
varies from 0 to 2. `bench_workq` reports Started/Busy throughout, including
131 snapshots with no pending items. The system work queue reports Started
without Busy (Idle) throughout. These are sampled handler states, not CPU
utilization. Drain waiters are empty in this capture;
use the synthetic work queue fixture to exercise nonempty drain queues.

## Contents

- `capture.ndjson.gz`: original memory-read recording; 578 polling frames.
- `zephyr.elf`: exact matching firmware, required to decode/replay the recording.
- `session-with-heap-maps.json`: official UI test session; 577 usable frames,
  spanning 0–59.905303 seconds, with a heap chunk map in every frame. Size:
  5,734,942 bytes, below the UI's 16 MiB document limit.
- `session.json`: preserved legacy export of the same capture, without heap
  chunk maps. Its original bytes and checksum remain unchanged.

The recording command was bounded at 60 seconds with a requested 100 ms
polling period. Actual capture intervals vary; the final sample need not land
exactly at 60 seconds. This is emulator execution, not hardware performance
evidence. GDB briefly halts the target for memory reads.

Each decoded frame includes 18 threads (including idle), one heap, two
semaphores, one mutex, one message queue, one memory slab, and two work queues.
The official fixture was regenerated from the preserved recording and matching
ELF using ZView backend revision `40c85a79ab88491e4050e84e03d94b6790c1dca6`
(version 0.24.1), matching the backend pin in CI. Its snapshot timestamps match
the legacy export exactly.

## Heap screen validation

Open the official fixture using either command above, then choose
**Kernel objects → Heap → bench_heap**. The full-benchmark heap rendering and
playback test runs with `cargo test`; it needs no local export or Python setup.

This acquisition contains 421 distinct maps across 577 snapshots, with 2–130
chunks and up to about 97.7% map fragmentation. The midpoint shows 119 chunks
with 720 allocated bytes and 852 free bytes. Payload statistics and map totals
use different accounting because chunk sizes include allocator overhead.

The existing synthetic fixtures remain useful for edge cases not exercised by
this real acquisition.

## Provenance

- Benchmark commit: `090444b432978a046b929989d2e09dad5c2cdbdc`
- Zephyr commit: `25c8f4a23988dd3b2cfb463613622738298c2d6c` (4.5.0-rc1)
- Board: `mps2/an385`, Cortex-M3
- QEMU: 11.1.1
- Compiler: `arm-none-eabi-gcc` 16.2.0
- Benchmark configuration: unmodified upstream `app/prj.conf`, dynamic mode.
- Benchmark source is Apache-2.0; see its upstream copyright/license.

## Capture and export

Build the pinned benchmark with the matching Zephyr environment using
`west build -b mps2/an385 zview-bench/app`, then run:

```sh
qemu-system-arm -machine mps2-an385 -cpu cortex-m3 \
  -kernel test-data/zview-bench-60s/zephyr.elf \
  -nographic -monitor none -serial stdio -gdb tcp:127.0.0.1:12349
```

In another terminal, with ZView and its Python dependencies installed:

```sh
python -m zview.cli record \
  -e test-data/zview-bench-60s/zephyr.elf \
  -r gdb -t 127.0.0.1:12349 --period 0.1 --duration 60 \
  -o test-data/zview-bench-60s/capture.ndjson.gz
```

To reproduce the official JSON from the retained inputs, install the pinned
backend revision and export to a scratch path rather than overwriting fixtures:

```sh
mkdir -p local
python export_recording.py \
  --recording test-data/zview-bench-60s/capture.ndjson.gz \
  --elf test-data/zview-bench-60s/zephyr.elf --duration 60 \
  --output local/bench-with-maps.json
```

CI regenerates this export and compares its decoded JSON with
`session-with-heap-maps.json`. Deliberately review fixture updates when changing
the capture, decoder revision, or exporter.

Do not overwrite the preserved fixture when experimenting; use another output
directory. Source/build downloads stay under Git-ignored `local/`.

## SHA-256

```text
4d1f3e96ea7a526ff4492bef9c18cb826e5d4d6705c5a7150689496ef42c7b7e  capture.ndjson.gz
31ef25d29dd1676d8311842fec0e9ce685afbda3970d9af846b652d8e4bed50a  session.json
025420510e673a01a48b31330c27712ec25ac5b41e89c62f9d3a4841b17f27a2  session-with-heap-maps.json
85932fc875cfa391635f8fa92b706ef82a08ec7824b473264ee73923aa9ed610  zephyr.elf
```

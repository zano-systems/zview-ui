# ZView benchmark — 60-second QEMU capture

Real execution of [zview-bench](https://github.com/wkhadgar/zview-bench) in
dynamic mode, not synthetic readings. Captured on 2026-10-19.

## Open in the desktop app

From the zview-ui repository root:

```sh
cargo run -- test-data/zview-bench-60s/session.json
```

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
- `session.json`: desktop-ready dump sequence; 577 usable frames, spanning
  0–59.905303 seconds after the runtime baseline frame.

The recording command was bounded at 60 seconds with a requested 100 ms
polling period. Actual capture intervals vary; the final sample need not land
exactly at 60 seconds. This is emulator execution, not hardware performance
evidence. GDB briefly halts the target for memory reads.

Each decoded frame includes 18 threads (including idle), one heap, two
semaphores, one mutex, one message queue, one memory slab, and two work queues.
This preserved JSON predates heap-map retention and omits chunk maps. They
remain in the raw recording. The current exporter retains them; export to a
different path (for example `local/bench-with-maps.json`) to inspect
Heap detail without overwriting this fixture.

## Heap screen validation

From the repository root, with ZView and its Python dependencies installed,
generate a map-preserving export:

```sh
mkdir -p local
python export_recording.py \
  --recording test-data/zview-bench-60s/capture.ndjson.gz \
  --elf test-data/zview-bench-60s/zephyr.elf --duration 60 \
  --output local/bench-with-maps.json
cargo run -- local/bench-with-maps.json
```

Then open **Kernel objects → Heap → bench_heap**.

This acquisition contains 421 distinct maps across 577 snapshots, with 2–130
chunks and up to about 97.7% map fragmentation. The midpoint shows 119 chunks
with 720 allocated bytes and 852 free bytes. Payload statistics and map totals
use different accounting because chunk sizes include allocator overhead.

The preserved `session.json` above remains unchanged, including its checksum.

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
python export_recording.py \
  --recording test-data/zview-bench-60s/capture.ndjson.gz \
  --elf test-data/zview-bench-60s/zephyr.elf --duration 60 \
  --output test-data/zview-bench-60s/session.json
```

Do not overwrite the preserved fixture when experimenting; use another output
directory. Source/build downloads stay under Git-ignored `local/`.

## SHA-256

```text
4d1f3e96ea7a526ff4492bef9c18cb826e5d4d6705c5a7150689496ef42c7b7e  capture.ndjson.gz
31ef25d29dd1676d8311842fec0e9ce685afbda3970d9af846b652d8e4bed50a  session.json
85932fc875cfa391635f8fa92b706ef82a08ec7824b473264ee73923aa9ed610  zephyr.elf
```

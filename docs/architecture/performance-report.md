# Latest Local Performance Report

Date: 2026-05-17

Command:

```bash
cargo run -p meshmend -- perf rose/raw.stl --output outputs/perf-rose.json
```

Local `rose/raw.stl` result:

- triangles: 1,949,244
- chunks: 20
- parse total: 188.337 ms
- GPU upload total: 110.559 ms
- screenshot/readback: 657.024 ms
- render coverage: 0.0616

The first obvious remaining measurement gap is interaction FPS. The next
optimization loop should add scripted orbit/pan/zoom sampling before changing
renderer hot paths.

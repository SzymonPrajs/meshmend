# Latest Local Performance Report

Date: 2026-05-17

Command:

```bash
cargo run -p meshmend -- perf rose/raw.stl --output outputs/perf-rose.json
```

Local `rose/raw.stl` result:

- triangles: 1,949,244
- chunks: 20
- parse total: 175.680 ms
- GPU upload total: 111.844 ms
- time to interactive: 288.178 ms
- screenshot/readback: 648.313 ms
- idle FPS sample: 98.6
- orbit FPS sample: 78.0
- pan FPS sample: 84.2
- zoom FPS sample: 85.2
- p95 frame: 25.590 ms
- p99 frame: 26.181 ms
- CPU RSS: 255.563 MB
- GPU buffer allocation: 118.973 MB
- render coverage: 0.0616

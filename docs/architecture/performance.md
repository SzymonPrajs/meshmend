# Performance Methodology

Use committed fixtures for repeatable CI-safe smoke metrics and `rose/raw.stl`
for local large-model measurements.

Commands:

```bash
cargo run -p meshmend -- perf fixtures/stl/cube_binary.stl --output outputs/perf-cube.json
cargo run -p meshmend -- perf rose/raw.stl --output outputs/perf-rose.json
```

The JSON report records:

- file map time
- validation time
- parse time
- GPU upload time
- first-frame/screenshot readback time
- time to interactive
- scripted idle/orbit/pan/zoom FPS samples
- p95/p99 frame time across the scripted samples
- CPU RSS on Unix platforms where `ps` is available
- GPU buffer allocation totals from MeshMend-owned buffers
- render nonblank coverage

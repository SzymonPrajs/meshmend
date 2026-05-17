# Verification

Core viewer checks:

```bash
just lint
just test
just release
```

Parser checks:

```bash
cargo test -p meshmend-stl
cargo run -p meshmend -- inspect fixtures/stl/cube_binary.stl
cargo run -p meshmend -- inspect rose/raw.stl --parallel
```

CLI analysis checks:

```bash
cargo test -p meshmend-analysis
cargo run -p meshmend -- analyze fixtures/stl/cube_binary.stl --output outputs/analysis-cube.json
```

CLI project checks:

```bash
cargo test -p meshmend-project
cargo run -p meshmend -- project validate path/to/project.meshmend
```

CLI worker checks:

```bash
just worker-build
cargo run -p meshmend -- worker-smoke cgal fixtures/stl/cube_binary.stl
cargo run -p meshmend -- worker-smoke openvdb fixtures/stl/cube_binary.stl
cargo run -p meshmend -- hole-fill fixtures/stl/cube_missing_top.stl --output outputs/cube-missing-top-filled.stl
cargo run -p meshmend -- analyze outputs/cube-missing-top-filled.stl --output outputs/cube-missing-top-filled-analysis.json
```

Renderer checks:

```bash
just verify
cargo run -p meshmend -- --cross-section-screenshot fixtures/stl/cube_binary.stl outputs/cube-cross-section.png
cargo run -p meshmend -- --screenshot fixtures/stl/cube_binary.stl outputs/cube.png
just verify-rose
```

`--verify-render` captures pixels from the native WGPU surface and fails if the
image is effectively blank. `--verify-view-modes` repeats the same blank-frame
check across every first-class viewport mode, including normals, surface wire,
x-ray wire, and transparent. Outputs under `outputs/` are
ignored.

`--verify-hit-stack` exercises the CPU selection BVH through the renderer and
fails unless a center ray returns a multi-hit stack. That path is a CLI
diagnostic only; the active viewer UI no longer exposes hit-stack selection or
starts selection-geometry preparation on load.

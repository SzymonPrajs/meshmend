# Verification

Core checks:

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

Analysis checks:

```bash
cargo test -p meshmend-analysis
cargo run -p meshmend -- analyze fixtures/stl/cube_binary.stl --output outputs/analysis-cube.json
```

Project checks:

```bash
cargo test -p meshmend-project
cargo run -p meshmend -- project validate path/to/project.meshmend
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
x-ray wire, transparent, and cross-section. Outputs under `outputs/` are
ignored.

`--verify-hit-stack` exercises the CPU selection BVH through the renderer and
fails unless a center ray returns a multi-hit stack, proving x-ray selection can
see beyond the front-most GPU pick.

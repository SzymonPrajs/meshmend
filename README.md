# MeshMend

This workspace is focused on MeshMend, a native Rust STL inspection app for
AI-generated 3D model meshes.

The previous Python mesh-repair toolchain has been moved out of the active
source tree:

```text
archive/python-resinmesh/
```

Keep it only as a reference for scripts, rendering ideas, and diagnostics. Do
not treat it as the active product direction.

## Current Assets

Only current model asset:

```text
rose/raw.stl
```

`rose/raw.stl` is intentionally ignored by Git because this repository should
track code and planning files, not raw model data.

## Current Implementation

The active implementation is now the native viewer:

- native `winit` desktop window
- native `wgpu` renderer
- Rust binary STL parsing and validation
- orbit, pan, zoom, fit, and reset camera controls
- mesh stats, notes, selection, screenshots, and performance metrics

## Current Plan

The next active product step is cross-section inspection:

```text
docs/cross-section-inspection-plan.md
```

## App Location

The native app crate is under:

```text
apps/meshmend/
```

The intended stack is:

- Rust workspace
- `winit` for the native event loop
- `wgpu` for native GPU rendering
- `egui` for overlay UI
- `rayon` and `memmap2` for large STL loading

Current implementation commands:

```bash
npm run dev
cargo run -p meshmend
cargo run -p meshmend -- inspect fixtures/stl/cube_binary.stl
cargo run -p meshmend -- fixtures/stl/cube_binary.stl
cargo run -p meshmend -- --verify-render fixtures/stl/cube_binary.stl
cargo run -p meshmend -- --screenshot fixtures/stl/cube_binary.stl outputs/cube.png
cargo run -p meshmend -- perf fixtures/stl/cube_binary.stl --output outputs/perf-cube.json
```

The Codex app run action can use the root `npm run dev` script. It starts the
native viewer with `rose/raw.stl` when that ignored local asset is present, and
otherwise opens the viewer without an initial STL.

Verification:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace --release
```

Local large-model checks use the ignored file:

```bash
cargo run -p meshmend -- inspect rose/raw.stl --parallel
cargo run -p meshmend -- rose/raw.stl
cargo run -p meshmend -- --verify-render rose/raw.stl
cargo run -p meshmend -- perf rose/raw.stl --output outputs/perf-rose.json
```

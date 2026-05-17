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
- mesh stats, cross-section inspection, brush labels, issue marking, selection,
  screenshots, and performance metrics

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
just run
just run-file fixtures/stl/cube_binary.stl
just build
just release
just test
just lint
just verify
just smoke
just verify-rose
just perf fixtures/stl/cube_binary.stl
```

The Codex app run action uses the root `just run` recipe. It starts the
native viewer with `rose/raw.stl` when that ignored local asset is present, and
otherwise opens the viewer without an initial STL.

Verification:

```bash
just lint
just test
just release
just verify
```

Local large-model checks use the ignored file:

```bash
just verify-rose
just perf rose/raw.stl
```

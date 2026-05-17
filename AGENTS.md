# Agent Notes

This workspace has pivoted away from the failed global mesh-repair experiments.
The active direction is MeshMend, a native Rust STL inspection app for viewing,
annotating, and later repairing AI-generated mesh files.

## Current Boundary

Build toward:

- a local native desktop app
- STL input only
- orbit, pan, and zoom viewing
- a clean foundation for selection, notes, validation, and later repair tools

Do not revive the old Python repair pipeline as active product code. It is
archived at:

```text
archive/python-resinmesh/
```

You may reuse archived code as reference material only, especially for mesh
statistics, rendering experiments, and diagnostic report ideas.

## Assets

The only current model asset is:

```text
rose/raw.stl
```

`rose/raw.stl` is ignored by Git because this repository should track code and
planning files, not raw model data.

## Implementation Rules

- Put the app under `apps/meshmend/`.
- Use a Rust workspace with `winit`, native `wgpu`, and `egui`.
- Keep the hot path in Rust: STL parsing, validation, chunking, GPU upload,
  camera math, picking, notes, screenshots, and performance metrics.
- Do not reintroduce the Three.js/Vite/Tauri webview viewer as active product
  code.
- Keep the native viewer focused on inspection. No repair, mesh
  simplification, slicing, ROI tools, or defect classification until the native
  viewer core is stable.
- Keep source assets and generated app artifacts separate.
- Keep generated build outputs, Rust `target`, large STL outputs, and local
  raw model files ignored.

## First Milestone Definition

The native viewer milestone is complete when:

- a native Rust app launches locally
- the app accepts an STL file selected from disk
- `rose/raw.stl` renders correctly as the local test model
- orbit, pan, and zoom work smoothly enough to inspect the loaded mesh
- the camera fits the model to view after loading
- the app reports basic mesh stats such as triangle count and bounds
- basic selection, notes, screenshots, and performance metrics work

Anything beyond that belongs to a later milestone.

Use these checks after native viewer changes:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p meshmend -- --verify-render fixtures/stl/cube_binary.stl
```

# Agent Notes

This workspace has pivoted away from the failed global mesh-repair experiments.
The active direction is MeshMend, a native Rust STL viewer foundation. Keep the
app small and reliable before reintroducing any repair workflow.

## Current Boundary

Build toward:

- a local native desktop app
- STL input only
- orbit, pan, and zoom viewing
- clear view modes from a top toolbar
- normal file-menu style open/save/export behavior
- a compact status bar

The single source of truth is:

```text
plan.md
```

Follow that reset plan phase by phase. Do not create separate plan files.

The old Python repair pipeline has been deleted after its useful diagnostics,
ROI, voxel, and CLI ideas were ported into the native Rust/C++ implementation.
Do not recreate it as active product code.

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
  camera math, indexed mesh storage, topology, BVH picking, project state,
  screenshots, and performance metrics.
- Do not reintroduce the Three.js/Vite/Tauri webview viewer as active product
  code.
- Keep C++ geometry work isolated in process workers. Do not expose those
  workers in the viewer UI during the reset.
- Do not use or recreate the old Python repair pipeline as active product code.
- Keep source assets and generated app artifacts separate.
- Keep generated build outputs, Rust `target`, large STL outputs, and local
  raw model files ignored.

## Current Milestone Definition

The current reset milestone is complete when:

- a native Rust app launches locally
- the app accepts an STL file selected from disk
- `rose/raw.stl` renders correctly as the local test model
- orbit, pan, and zoom work smoothly enough to inspect the loaded mesh
- the camera fits the model to view after loading
- the app can save/export the current STL
- view modes are controlled from a clear top toolbar
- the right panel and repair/analysis/remesh UI are absent
- the bottom status bar reports basic viewer state

Use these checks after native viewer changes:

```bash
just lint
just test
just verify
```

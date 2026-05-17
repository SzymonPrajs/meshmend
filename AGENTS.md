# Agent Notes

This workspace has pivoted away from the failed global mesh-repair experiments.
The active direction is MeshMend, a native Rust STL repair workstation for
viewing, inspecting, repairing, remeshing, validating, and exporting
AI-generated mesh files for resin printing.

## Current Boundary

Build toward:

- a local native desktop app
- STL input only
- orbit, pan, and zoom viewing
- cross-section and x-ray inspection
- repair-first tools for holes, local cavities, cuts, physical scale, remeshing,
  validation, and export

The single source of truth is:

```text
docs/meshmend-master-plan.md
```

Follow that plan phase by phase. Do not create separate plan files.

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
  camera math, indexed mesh storage, topology, BVH picking, project state,
  screenshots, and performance metrics.
- Do not reintroduce the Three.js/Vite/Tauri webview viewer as active product
  code.
- Keep C++ geometry work isolated in process workers where the master plan calls
  for CGAL or OpenVDB.
- Do not use the old Python repair pipeline as active product code.
- Keep source assets and generated app artifacts separate.
- Keep generated build outputs, Rust `target`, large STL outputs, and local
  raw model files ignored.

## Current Milestone Definition

The current inspection milestone is complete when:

- a native Rust app launches locally
- the app accepts an STL file selected from disk
- `rose/raw.stl` renders correctly as the local test model
- orbit, pan, and zoom work smoothly enough to inspect the loaded mesh
- the camera fits the model to view after loading
- the app reports basic mesh stats such as triangle count and bounds
- basic selection, issue marking, screenshots, and performance metrics work
- a cross-section plane can inspect hidden internal geometry along X, Y, or Z
- brush labels can mark healthy boundary and repair target regions
- the app follows the current master plan phase being implemented

The master plan supersedes this historical inspection milestone when the two
conflict.

Use these checks after native viewer changes:

```bash
just lint
just test
just verify
```

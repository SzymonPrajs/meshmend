# Agent Notes

This workspace has pivoted away from the failed global mesh-repair experiments.
The active direction is MeshMend, a Tauri STL viewer prototype for inspecting
and later repairing AI-generated mesh files.

## Current Boundary

Build toward:

- a local desktop app
- STL input only
- orbit, pan, and zoom viewing
- a clean foundation for later defect marking and cross-section tools

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
- Use Tauri 2 with a Rust shell and a TypeScript frontend.
- Use Three.js first. It already provides the practical viewer pieces needed
  for STL loading, camera control, lighting, and ray picking later.
- Treat WebGPU as a later comparison point, not as the first milestone.
- Keep milestone 1 viewer-only. No repair, no mesh simplification, no slicing,
  no ROI tools, and no defect classification.
- Keep source assets and generated app artifacts separate.
- Keep generated build outputs, `node_modules`, Rust `target`, and large STL
  outputs ignored.

## First Milestone Definition

The first milestone is complete when:

- a Tauri app launches locally
- the app accepts an STL file selected from disk
- `rose/raw.stl` renders correctly as the local test model
- orbit, pan, and zoom work smoothly enough to inspect the loaded mesh
- the camera fits the model to view after loading
- the app reports basic mesh stats such as triangle count and bounds

Anything beyond that belongs to a later milestone.

Use these checks after viewer changes:

```bash
cd apps/meshmend
npm run build
npm run verify:viewer
npm run tauri build -- --bundles app
```

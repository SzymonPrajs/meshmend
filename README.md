# 3D Modelling

This workspace is now focused on a clean Tauri STL viewer prototype.

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

## Active Plan

The viewer plan is here:

```text
docs/tauri-stl-viewer-plan.md
```

The first implementation milestone should be only a viewer:

- accept STL files
- load `rose/raw.stl` manually through the app
- render the mesh
- support orbit, pan, and zoom
- fit the camera to the model bounds
- avoid repair, slicing, picking, and mesh editing until the viewer is stable

## Proposed App Location

When implementation starts, put the Tauri app under:

```text
apps/rose-viewer/
```

The intended stack is:

- Tauri 2 for the native Rust shell
- Vite and TypeScript for the frontend
- Three.js for the first renderer
- WebGPU only as a later evaluated renderer path if the STL workload proves to
  need it

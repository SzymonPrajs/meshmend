# MeshMend

This workspace is focused on MeshMend, a native Rust STL viewer for inspecting
AI-generated 3D model meshes.

The previous Python mesh-repair toolchain has been removed from the source
tree. Repair experiments may remain as isolated Rust/C++ CLI code, but the
active app surface is the viewer described in `plan.md`.

## Current Assets

Only current model asset:

```text
rose/raw.stl
```

`rose/raw.stl` is intentionally ignored by Git because this repository should
track code and planning files, not raw model data.

## Current Implementation

The active implementation is the native viewer:

- native `winit` desktop window
- native `wgpu` renderer
- Rust binary STL parsing and validation
- menu-based STL open, save, and save-as/export
- orbit, pan, zoom, fit, and reset camera controls
- top toolbar view modes: rendered, wireframe, surface wire, x-ray wire,
  transparent, normals, studio lighting, and headlight lighting
- compact bottom status bar with file name, triangle count, backend, GPU
  memory, FPS/frame time, current view, and transient status
- screenshot, view-mode, and performance verification commands

No repair, annotation, cross-section, remesh, worker, issue, or project panel is
exposed in the active viewer UI during this reset.

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

Current viewer commands:

```bash
just run
just run-file fixtures/stl/cube_binary.stl
just build
just release
just package
just package-smoke
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
just package-smoke
```

Local large-model checks use the ignored file:

```bash
just verify-rose
just perf rose/raw.stl
```

CLI-only geometry and worker experiments still exist for later reference, but
they are not reachable from the active app UI. Build them explicitly with
`just worker-build` before running worker commands.

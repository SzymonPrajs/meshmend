# Native Renderer

MeshMend now uses a native Rust renderer under `crates/meshmend-render`.

The app creates a `winit` window, initializes `wgpu` against the platform-native
backend, uploads STL triangle chunks as storage buffers, and draws each chunk
with one instanced draw call per chunk. The shader uses `vertex_index` to choose
the STL triangle vertex and `instance_index` to choose the triangle.

Current visible viewer passes:

- grid and axes line pass
- solid shaded triangle pass
- shader barycentric wireframe overlay
- normal debug mode
- hidden `R32Uint` GPU picking pass
- screenshot/readback pass for verification and performance metrics
- `egui` overlay pass for the menu bar, view toolbar, and status bar

The renderer keeps a compact CPU copy of loaded triangles for exact pick-hit
positions after the GPU identifies the visible triangle. Heavier CPU selection
geometry is now built on demand only for CLI diagnostics that explicitly request
hit-stack checks; normal viewer loads do not start that work.

Dormant renderer support for cross-section clipping and label overlays remains
in this crate because verification and later experiments still use it. It is
not exposed from the active viewer UI during the reset.

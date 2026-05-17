# Native Renderer

MeshMend now uses a native Rust renderer under `crates/meshmend-render`.

The app creates a `winit` window, initializes `wgpu` against the platform-native
backend, uploads STL triangle chunks as storage buffers, and draws each chunk
with one instanced draw call per chunk. The shader uses `vertex_index` to choose
the STL triangle vertex and `instance_index` to choose the triangle.

Current passes:

- grid and axes line pass
- solid shaded triangle pass
- shader barycentric wireframe overlay
- normal debug mode
- hidden `R32Uint` GPU picking pass
- screenshot/readback pass for verification and performance metrics
- `egui` overlay pass for controls, stats, and notes

The renderer keeps a compact CPU copy of loaded triangles for exact pick-hit
positions after the GPU identifies the visible triangle.

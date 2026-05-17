# Inspection And Defects

Selection uses two paths:

- normal visible selection can use the renderer picking pass
- x-ray/internal selection uses the Rust `meshmend-geometry` indexed mesh and
  BVH hit stack

The renderer still draws the visible model to an `R32Uint` texture for fast
front-surface picking. Each fragment stores an encoded `TriangleId` using the
chunk and local triangle index. On click, MeshMend decodes the triangle ID and
intersects the camera ray against that triangle to produce a model-space hit
point.

For internal repair work, the loaded STL is also welded into an indexed mesh.
That sidecar builds connectivity, boundary loops, non-manifold edges,
components, and a flattened BVH. X-ray picking casts a CPU ray through the
cursor and returns an ordered hit stack, so the user can select visible,
back-side, or internal triangles.

Cross-section mode applies the same plane test in the visible mesh shader, the
picking shader, and the CPU hit-stack filter. That keeps selection aligned with
the clipped view.

Repair regions are stored as stroke-based surface regions:

- healthy boundary: good surface around a cavity or damaged area that should be
  preserved by local repair
- repair target: the damaged or hollow area to heal
- exclude: nearby surface that should not be pulled into a local repair solve

Brush size is measured in mesh-detail units. When an STL loads, MeshMend samples
triangle edge lengths and uses the average edge length as unit 1. A brush radius
of 10 therefore records a world-space radius of roughly 10 average triangle
edges for that model. This keeps dense models paintable without tying brush
size to screen percentage or window size.

Defect records still exist as a compatibility data format while project
operations become the primary repair history. User-facing workflow should keep
repair, defects, operations, validation, and export language in front of the
legacy data shape.

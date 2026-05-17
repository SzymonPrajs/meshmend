# Inspection And Issues

Selection uses a hidden GPU picking pass, not whole-mesh CPU ray casting.

The renderer draws the visible model to an `R32Uint` texture. Each fragment
stores an encoded `TriangleId` using the chunk and local triangle index. On
click, MeshMend copies one pixel to a readback buffer, decodes the triangle ID,
and intersects the camera ray against that triangle in CPU memory to produce a
model-space hit point.

Cross-section mode applies the same plane test in both the visible mesh shader
and the picking shader. That keeps picking aligned with the clipped view, so
hidden-side triangles are not selectable after the plane is enabled.

Issue sessions are stored as versioned JSON in `crates/meshmend-inspection`.

Brush labels are stored in the same session. They are stroke-based surface
labels, not point issues. The first active label types are:

- healthy boundary: good surface around a cavity or damaged area that should be
  preserved by local repair
- repair target: the damaged or hollow area to heal
- exclude: nearby surface that should not be pulled into a local repair solve

Brush size is measured in mesh-detail units. When an STL loads, MeshMend samples
triangle edge lengths and uses the average edge length as unit 1. A brush radius
of 10 therefore records a world-space radius of roughly 10 average triangle
edges for that model. This keeps dense models paintable without tying labels to
screen percentage or window size.

Current issue fields:

- model file name
- model file size
- issue ID
- issue kind
- triangle ID
- model-space position
- cross-section axis and offset at the time of recording
- cross-section side
- label
- status

Current brush stroke fields:

- stroke ID
- label kind
- brush radius in model-space units
- sampled triangle IDs
- sampled model-space positions

The UI can paint brush labels on the visible mesh, select an issue kind, add an
issue at the selected point, edit labels, frame an issue, delete issues, save
sessions, and load sessions from JSON.

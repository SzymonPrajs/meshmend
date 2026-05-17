# Selection And Notes

Selection uses a hidden GPU picking pass, not whole-mesh CPU ray casting.

The renderer draws the visible model to an `R32Uint` texture. Each fragment
stores an encoded `TriangleId` using the chunk and local triangle index. On
click, MeshMend copies one pixel to a readback buffer, decodes the triangle ID,
and intersects the camera ray against that triangle in CPU memory to produce a
model-space hit point.

Notes are stored as versioned JSON sessions in `crates/meshmend-notes`.

Current session fields:

- model file name
- model file size
- note ID
- triangle ID
- model-space position
- label
- color

The UI can add a note at the selected point, edit labels, frame a note, delete
notes, save notes, and load notes from JSON.

# MeshMend View-Line Cut and Triangulated Cap Plan

## Purpose

MeshMend is not a general Blender replacement. The app is for repairing and preparing AI-generated meshes for resin printing. The cut tool should solve one specific repeated workflow:

- Load an AI-generated STL.
- Draw a simple line across the view where an unwanted part should be cut away.
- Split the mesh through that view line.
- Automatically cap both newly open sides with printable, sculptable triangle grids.
- Let the user keep, hide, delete, or export the resulting pieces.

The existing line selection tool is the wrong abstraction. A line gesture should not select edges or faces as an end in itself. It should define a cut operation.

## Core Product Decision

Remove `Line` from the selection tool group.

Keep selection tools focused on selecting mesh elements:

- Point select.
- Brush select.
- Vertex, edge, and face element modes.
- Front surface and through selection depth.

Add a separate cut tool:

- Name: `Cut`.
- User-facing behavior: draw a line in screen space.
- Geometry behavior: cut the mesh by the view-defined cutting plane created from that line.
- Output behavior: split the mesh and cap both resulting cut boundaries with triangle grids.

This makes the UI match the real workflow. Selecting by line is not useful here; cutting by line is.

## User Workflow

1. The user loads an STL.
2. The user orbits/zooms until the intended cut is visible.
3. The user activates the `Cut` tool from the left tool palette.
4. The cursor changes to a cut-line cursor.
5. The user clicks or drags two points in the viewport.
6. The app previews an infinite line across the full viewport, not only the segment between the two points.
7. The preview highlights the projected intersection path on the visible mesh.
8. The user confirms the cut with release/Enter, or cancels with Escape.
9. The mesh is split along the view-defined cut plane.
10. Both newly open sides are capped with triangle grids whose target edge length is derived from the cut boundary.
11. The app shows the two resulting pieces as selectable pieces.
12. The user can hide/delete one piece, keep both pieces, or export.

For the rose example:

- The user draws a line across the stem.
- The tool cuts the stem through that line.
- The rose-side stump and the removed stem-side stump both get capped.
- The caps are not one giant polygon; they are dense enough triangle grids to sculpt later.

## View-Line Geometry

The cut gesture is a 2D screen-space line, but the actual mesh operation needs a 3D cutting plane.

For the current perspective camera:

1. Convert the two screen points to camera rays.
2. Build a plane from:
   - camera eye position,
   - ray through point A,
   - ray through point B.
3. The plane normal is `normalize(ray_a.direction cross ray_b.direction)`.
4. The plane is infinite.
5. Every mesh edge whose endpoints lie on opposite sides of that plane is cut.

This is mathematically a plane cut, but the user should experience it as drawing a line. We should avoid exposing generic plane-slice controls here because that is not the intended workflow.

If an orthographic camera is added later, the view-line plane should be:

- line direction in world space from the two unprojected points,
- camera forward direction,
- plane normal from `line_direction cross camera_forward`.

## Preview Behavior

The cut preview needs to answer: "What will this line cut?"

During the line gesture:

- Draw the infinite screen-space line across the viewport.
- Compute a lightweight preview of intersected mesh edges/faces.
- Highlight the projected cut path in orange.
- Do not mutate the mesh until confirm.
- Show status text such as `Cut preview: 142 boundary intersections`.

The first version can preview the affected triangles and cut segments using CPU geometry. It does not need to generate caps during hover/drag.

## Mesh Split Algorithm

The cut operation should create real mesh topology, not just visual selection.

Input:

- Current triangle mesh.
- View-defined cut plane.
- Epsilon based on model bounds and local edge size.

Steps:

1. Classify every vertex by signed distance to the cut plane.
2. Snap vertices very close to the plane onto the plane.
3. For each triangle:
   - If all vertices are on the positive side, copy it to positive piece.
   - If all vertices are on the negative side, copy it to negative piece.
   - If it crosses the plane, split it.
4. For crossing triangles:
   - Find intersection points on crossed triangle edges.
   - Reuse each intersection vertex for both adjacent triangles by caching by original edge key.
   - Emit the split triangles on both sides.
   - Record the cut segment created inside that original triangle.
5. Preserve triangle winding.
6. Preserve source IDs where useful for debugging.
7. Build connected components for each side after the split.

Common cases:

- One vertex on one side, two on the other: create one triangle on the single-vertex side and two triangles on the other side.
- One vertex exactly on the plane and the opposite edge crosses: reuse the on-plane vertex plus one new intersection.
- An entire triangle edge lies on the plane: treat it as a boundary segment and avoid duplicate cap edges.

The cut plane can intersect multiple parts of the mesh. That is acceptable. The user is drawing an infinite line through the whole view, so all crossed parts should be cut unless a later masking mode is added.

## Boundary Loop Construction

After splitting, the cut creates open boundary loops. These loops define the caps.

Steps:

1. Collect all cut segments.
2. Weld segment endpoints using a tolerance derived from the model scale.
3. Build an undirected graph of cut vertices and cut edges.
4. Trace closed loops.
5. Mark problematic loops:
   - open chains,
   - branch vertices,
   - self-intersections in the cap plane,
   - extremely short edges,
   - degenerate loops.
6. For valid loops, project vertices into 2D coordinates on the cut plane.

The app should expect multiple loops. A single cut can hit both sides of a hollow region, overlapping petals, supports, or unrelated geometry.

## Cap Target Resolution

The cap must be sculptable after export. It should not be a single n-gon or a fan of long skinny triangles.

Target edge length:

1. For each cap loop, measure the cut boundary edge lengths.
2. Use a robust statistic, not a raw average:
   - start with median boundary edge length,
   - optionally blend with trimmed mean,
   - ignore tiny epsilon edges.
3. Clamp the target against local mesh scale:
   - lower bound avoids millions of cap triangles from tiny sliver edges,
   - upper bound avoids a visibly coarse cap.
4. Use this target length as the desired interior triangle size.

For the rose stem case, the cap grid should roughly match the size of the triangles around the cut perimeter.

## Cap Triangulation

The cap should be generated as a constrained triangle mesh in the cut plane.

Recommended implementation:

- Use the existing C++ worker architecture for the robust triangulation phase.
- Use CGAL constrained triangulation or 2D mesh generation in the cap plane.
- Keep Rust responsible for app state, preview, user interaction, and file/project orchestration.

Cap generation steps:

1. Project each boundary loop to 2D in the cut plane.
2. Insert loop edges as constraints.
3. Generate interior Steiner points at approximately the target edge spacing.
4. Run constrained Delaunay triangulation or CGAL 2D meshing.
5. Keep triangles inside the boundary loops.
6. Convert the 2D cap vertices back into 3D.
7. Orient cap triangles correctly:
   - one cap faces the positive piece exterior,
   - the other cap faces the negative piece exterior.
8. Append the cap triangles to each resulting piece.
9. Optionally run a local remesh/smoothing pass only on cap triangles and the immediate cut rim.

The first complete version should prioritize clean, printable topology over visual smoothness. Later we can add cap smoothing controls.

## Two-Sided Output

The cut produces two sides, and both sides should be valid outputs.

For each resulting side:

- Original triangles on that side.
- Split triangles created by the cut.
- Cap triangles closing the newly open loops.
- Piece ID.
- Bounds.
- Triangle count.
- Cap triangle count.
- Cut-loop diagnostics.

The UI should show the pieces immediately after the cut:

- Piece A and Piece B should be separately selectable.
- Hovering a piece should outline it.
- Clicking a piece should select it.
- Actions should include `Hide`, `Delete`, `Keep Only`, and `Export Piece`.

Deleting the unwanted stem should become a single obvious action after the cut, not a sequence of Blender-style manual steps.

## Undo and Project State

Cutting changes the mesh, so it must use mesh-level undo, not selection undo.

Requirements:

- Before applying a cut, store a mesh revision.
- `Cmd/Ctrl+Z` should undo the cut result.
- Selection undo and mesh undo need clear ownership:
  - selection actions undo selection changes,
  - mesh operations undo mesh revisions.
- The status bar should say what was undone.

The current selection undo stack is not enough for this operation.

## UI Changes

Tool palette:

- Keep element selector for selection modes.
- Keep point and brush selection.
- Remove line selection from the selection tool set.
- Add a separate cut tool icon.

Top toolbar:

- Keep view modes.
- Add no complex cut controls here for the first version.

Cut tool controls:

- Minimal viewport-first behavior.
- Status bar shows preview/apply state.
- Escape cancels active cut gesture.
- Enter applies if two points are present.
- Undo restores previous mesh.

Shortcuts:

- `Q`: point select.
- `W`: brush select.
- `C` or `K`: cut tool.
- `Esc`: cancel active cut gesture, otherwise clear selection.
- `Enter`: apply cut when cut preview is active.
- `Cmd/Ctrl+Z`: undo selection or mesh operation, depending on the latest action.

Visual language:

- Orange is for active cuts, selected boundaries, and highlighted future edits.
- Blue should not be used for active cut geometry.
- Cut preview line should be orange.
- Generated cap area can use a subtle orange fill until accepted.

## Data Model

Add explicit tool mode state separate from element selection:

- `ActiveTool::Select`
- `ActiveTool::BrushSelect`
- `ActiveTool::Cut`

Add cut gesture state:

- first screen point,
- current/second screen point,
- computed cut plane,
- preview segments,
- preview affected triangle count,
- validation status.

Add mesh revision state:

- current mesh,
- revision stack,
- resulting pieces after operations,
- selected piece IDs.

Add cap metadata:

- source cut plane,
- loop count,
- target cap edge length,
- cap triangle count,
- warnings.

## Worker Architecture

The robust cap operation is complex enough to belong in the worker layer.

Rust app responsibilities:

- Gesture handling.
- View-line to plane conversion.
- Preview selection/intersection.
- Request/response orchestration.
- UI state and undo revisions.
- Export/save.

Worker responsibilities:

- Exact mesh splitting.
- Boundary loop tracing.
- Constrained triangulated caps.
- Local cap remeshing.
- Diagnostics.

Worker request should include:

- input mesh path or mesh payload,
- cut plane origin/normal,
- cap target edge length policy,
- weld tolerance,
- output path or response payload path.

Worker response should include:

- output mesh path,
- piece metadata,
- cut loop metadata,
- cap statistics,
- warnings and errors.

The existing coarse `cut` CLI worker should not be treated as the finished tool. It should be replaced or upgraded to produce the two-sided, triangulated-cap result described here.

## File Saving and Export

After a cut:

- The in-app mesh should update to the cut result.
- Save should write the current full mesh state.
- Export Piece should write only the selected piece.
- Export All Pieces should write separate STL files.
- Reports should describe cut plane, cap edge length, cap loops, cap triangles, and any warnings.

STL has no native piece metadata, so piece separation should be managed in app state and export naming.

## Edge Cases

The implementation must handle:

- Cut plane misses the mesh.
- Cut plane touches but does not split a triangle.
- Cut line crosses multiple disconnected components.
- Cut produces multiple loops.
- Cut produces nested loops.
- Mesh has existing holes near the cut.
- Mesh has non-manifold edges.
- Mesh has duplicated triangles.
- Boundary loop self-intersects after projection.
- Very small stem or thin geometry creates tiny cut edges.

For the first implementation, if a cap loop is invalid, the operation should fail before mutating the mesh and explain the problem in the status bar. It should not create a broken output.

## Implementation Phases

### Phase 1: Replace Line Selection With Cut Tool UX

- Remove `Line` from `SelectionTool`.
- Remove line-selection sampling and line-selection tests.
- Add `Cut` as a separate active tool.
- Add cut icon and shortcut.
- Draw two-point cut gesture preview.
- Use orange for preview.
- Escape cancels preview.
- Enter applies only after later phases are ready.

Verification:

- App builds.
- Existing point and brush selection still work.
- No line selection appears in the selection palette.
- Cut gesture can be drawn and canceled.

### Phase 2: View-Line Plane and Preview

- Convert screen line to a view-defined 3D plane.
- Intersect mesh triangles with that plane for preview.
- Highlight cut segments on the mesh.
- Show affected triangle and segment counts in the status bar.
- Keep preview non-mutating.

Verification:

- Unit tests for screen-line to plane conversion.
- Unit tests for triangle-plane intersection.
- Visual check on `rose/raw.stl`.

### Phase 3: Rust Mesh Split Prototype

- Implement deterministic triangle splitting in Rust.
- Preserve winding.
- Cache edge intersections.
- Produce positive and negative side meshes.
- Produce cut segment graph.
- Add tests with simple cubes, cylinders, and slanted cuts.

Verification:

- Cube cut produces two valid open-side meshes before capping.
- Cut segment counts match expected cases.
- No degenerate triangles from normal cases.

### Phase 4: Boundary Loop Tracing

- Weld cut segment endpoints.
- Trace loops.
- Detect invalid open chains and branches.
- Project loops to 2D cut-plane coordinates.
- Compute cap target edge length from loop statistics.

Verification:

- Unit tests for single loop, multiple loops, nested loops, and broken loop detection.
- Rose stem cut preview reports plausible loop counts.

### Phase 5: Triangulated Cap Worker

- Upgrade the C++ CGAL worker for constrained triangulated cap generation.
- Insert boundary constraints.
- Generate interior triangles with target edge length.
- Return cap metadata and warnings.
- Keep both cut sides capped.

Verification:

- Cube cut creates capped closed pieces.
- Cylinder/stem fixture creates many cap triangles with reasonable edge lengths.
- Analysis reports no new open boundary on the cut caps.

### Phase 6: Apply Cut In App

- Add mesh revision before applying cut.
- Send cut request to worker.
- Load cut result back into renderer.
- Show pieces and cap highlights.
- Add progress/status messages.
- Undo restores previous mesh.

Verification:

- Cut can be applied from the viewport.
- Undo restores the uncut model.
- Save/export writes the cut result.

### Phase 7: Piece Selection and Deletion

- Build connected components after cut.
- Let user click a resulting piece.
- Add `Hide`, `Delete`, `Keep Only`, and `Export Piece`.
- Ensure deleted piece removal preserves the cap on the kept side.

Verification:

- Rose stem can be cut, selected, hidden/deleted, and exported.
- Remaining rose mesh is closed at the cut.

### Phase 8: Quality Controls

- Add optional cap density controls:
  - automatic,
  - finer,
  - coarser.
- Add cap smoothing option constrained to cap region.
- Add diagnostics overlay for cap loops.

Verification:

- Cap edge length distribution is close to target.
- Cap triangles remain sculptable and not stretched.

## Acceptance Criteria

The feature is complete when:

- There is no line selection tool.
- The user can activate a cut tool.
- The user can draw a line across the view.
- The line defines a full view-through cut.
- The mesh is split into pieces.
- Both sides of the cut are capped.
- Caps are triangulated with roughly local mesh-sized triangles.
- The user can remove the unwanted piece.
- The kept piece remains closed and printable.
- The result can be saved/exported as STL.
- Undo restores the previous mesh state.

## Non-Goals

- General-purpose modeling.
- Arbitrary Blender-style edit mode.
- Manual edge/vertex cleanup workflows.
- Complex multi-plane CAD cutting.
- N-gon cap output.
- A line selection tool as a final feature.

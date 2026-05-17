# MeshMend Master Plan

Status: authoritative product and implementation plan
Date: 2026-05-17
Scope: native Rust MeshMend app, C++ geometry workers, resin-print repair workflows

This is the single planning document for MeshMend. Do not create parallel plan
files. Existing architecture documents are current-state references only; when
they conflict with this document, this document wins.

## End Goal

MeshMend should become a practical native repair workstation for STL meshes
created by AI 3D generation tools and prepared for resin printing.

The app must let a user load a problematic mesh, inspect it clearly, detect
print-breaking defects, repair those defects locally or automatically, preview
the result, and export a repaired mesh. The output is not an annotation JSON
file. The output is a repaired mesh plus optional project history and repair
report.

The primary rose-class failure cases are:

- closed-looking surfaces with tiny openings into large internal cavities
- secondary internal shells or cave surfaces that would trap resin
- genuinely open holes and boundary loops
- non-manifold edges, duplicate faces, degenerate triangles, flipped normals,
  self-intersections, and disconnected fragments
- models that are much denser than the physical printer resolution justifies
- unwanted model parts that need fast printable cutting and automatic capping

## Product Principles

- Repair-first, not annotation-first. Any mark the user makes must feed an
  operation: inspect, select, repair, cut, remesh, export.
- The viewport is the app. Panels should support the model, not surround it
  with scattered controls.
- Prefer clear tool modes over exposed implementation toggles.
- Every destructive operation needs preview, apply, undo, and saved output.
- Long geometry jobs must run off the UI thread with progress and cancellation.
- Physical scale matters. Resolution and remeshing must be expressed in real
  units once the user calibrates the model.
- Native Rust remains the shell, renderer, project system, and orchestration
  layer. Heavy geometry repair can use isolated C++ workers when the libraries
  are better there.
- The archived Python repair pipeline is reference material only until each
  useful behavior is ported. It must not remain active product code.

## Current UI Problems To Fix

The current UI spreads small text controls across a top strip, a model stats
panel, and an inspection panel. It has no real tool palette, no visible repair
workflow, no icons, and it presents labels/issues as if the app were an
annotation tool. Common view modes such as normals and wireframe are hidden as
small checkboxes, and the lighting makes the back side hard to inspect.

Replace this with a tool-led shell:

- central full-height viewport
- left vertical tool palette with discoverable icon buttons
- top contextual tool options for the active tool
- compact viewport mode strip for rendered, normal, wire, x-ray, section, and
  lighting modes
- right repair/analysis panel that changes with the active tool
- bottom status and job progress bar
- collapsible model stats instead of a permanent large stats wall

Recommended control sizes:

- 36 to 40 px square tool buttons
- 22 to 26 px icons
- short tooltips on hover
- text labels only where the choice is not obvious from an icon

The UI should use a proper icon asset path, not text-only buttons. Start with a
small vendored SVG/icon-font set for file, cursor, orbit, hand/pan, cross
section, brush, heal, cut, measure, remesh, export, normals, wire, x-ray,
solid/rendered, undo, redo, cancel, and apply.

## Target User Workflows

### 1. Load And Inspect

- Open STL by drag-drop, file picker, recent files, or command line.
- Auto-fit the camera.
- Show triangle count, physical scale status, bounds, component count, and
  initial validation warnings.
- Default to a camera-headlight rendered view so all sides remain readable.

### 2. Analyze

- Run automatic mesh analysis.
- Populate a defect list with grouped, selectable findings:
  - open boundaries
  - non-manifold edges
  - self-intersections
  - disconnected components
  - internal shells
  - likely trapped cavities
  - thin walls relative to print scale
  - degenerate or duplicate triangles
- Clicking a finding frames it and activates a useful view mode.

### 3. Inspect Inside

- Cross-section through X, Y, or Z with draggable plane.
- X-ray wireframe mode that shows inside and back-side surfaces.
- Pick-through selection in x-ray mode, with an intersection stack so the user
  can choose front, back, or internal hit points.
- Internal surfaces and open boundary loops should render with distinctive
  overlays.

### 4. Local Cavity Repair

- User paints repair anchors on healthy outside surface.
- User paints or selects the cavity/opening/target area.
- App proposes the local repair volume.
- Preview removes the inner cavity surface and replaces the damaged outer area
  with a smooth printable patch tied to the healthy anchors.
- Apply writes a new mesh state and keeps the operation in undo history.

### 5. Hole Repair

- User selects an open edge loop or a detected hole.
- App can auto-close simple holes.
- Larger holes get a gridded/remeshed patch, not a single stretched fan, so the
  repaired area remains sculptable in Blender.
- Preview shows cap topology and target edge length before apply.

### 6. Cut Away Unwanted Parts

- Straight cut tool: draw a line in the viewport, generating a cut plane from
  screen line plus camera direction.
- Preview the two sides.
- User clicks the side to delete.
- App clips the mesh, caps the remaining open surface, remeshes the cap, and
  validates printability.
- Later freehand knife/lasso: draw a path, create an extruded cutting surface,
  split, select side, cap, and validate.

### 7. Scale And Remesh For Resin Printing

- Measure two points and assign a real distance, for example 10 mm.
- Store the model scale and unit.
- Add printer profiles:
  - XY pixel size, for example 20 microns
  - layer height
  - minimum wall/detail thresholds
- Remesh or simplify to the useful physical resolution while preserving visible
  silhouette and sharp features.
- Show before/after triangle count and estimated error.

### 8. Export

- Save a MeshMend project file with operations, previews, logs, source hash,
  scale, and output references.
- Export repaired STL.
- Export an optional repair report with defect counts, operations applied,
  final triangle count, and validation status.

## View Modes

Implement view modes as first-class modes, not loose checkboxes.

- Rendered: shaded surface for normal work.
- Rendered Headlight: default rendered mode with a camera-attached light.
- Studio: multiple soft fixed lights for final visual checks.
- Normal: surface normals as color; keep this easy to access because it is
  useful for finding bad geometry.
- Surface Wire: depth-tested wire overlay projected onto the visible surface.
- X-Ray Wire: translucent surface plus non-depth-tested wire to see through the
  model and select internal or back-side points.
- Transparent: solid transparent shell for spatial context.
- Cross Section: clipped view with plane guide and section boundary emphasis.
- Defect Overlay: colors boundary edges, non-manifold regions, internal shells,
  and selected analysis results.
- Thickness/Resolution Overlay: after scale calibration, colors areas below
  printer-relevant thickness or detail thresholds.

Implementation notes:

- Keep the existing barycentric wire overlay for Surface Wire.
- Add a separate x-ray line pass with configurable depth behavior.
- Add render uniforms for view mode, lighting mode, and camera light direction.
- Default lighting should include a camera-attached headlight plus ambient fill.
- Keep fixed-light and studio-light modes available for checking shape.

## Picking And Selection

The current hidden GPU picking pass only selects the visible triangle. That is
not enough for x-ray repair work.

Add a mesh selection core:

- Build a CPU acceleration structure over loaded triangles.
- Raycast from the camera through the cursor.
- Return an ordered intersection stack, not just the first visible triangle.
- Respect cross-section clipping when the section tool is active.
- In x-ray mode, let the user cycle through intersections or choose from a small
  stack popup.
- Store selections as operation inputs, not as permanent annotation issues.

This should live outside the renderer so repair operations, analysis findings,
and CLI diagnostics can reuse it.

## Tool Palette

Initial palette order:

- Select
- Orbit/Pan navigation
- Analyze
- Cross Section
- X-Ray Inspect
- Repair Brush
- Hole Fill
- Cut/Bisect
- Measure/Scale
- Remesh
- Export

Contextual controls:

- active view mode
- brush radius in mesh-detail units and physical units when scale is known
- repair preview/apply/cancel
- cut side selection
- remesh target resolution
- analysis filters

Rename or remove current UI concepts:

- Remove "Labels" as a user-facing section.
- Remove "Issues" as a user-facing output workflow.
- Replace them with "Repair", "Defects", "Operations", and "Validation".
- The saved JSON issue format can remain temporarily for migration tests, but
  the product should move to a project/operation format.

## Data Model

Add a project model that separates source mesh, current mesh, operations, and
exports.

Suggested crates:

- `meshmend-geometry`: indexed mesh, connectivity, boundary loops, components,
  BVH/raycasting, topology queries
- `meshmend-analysis`: defect detection and printability checks
- `meshmend-project`: project file, operation history, undo/redo, output paths
- `meshmend-repair`: Rust-side operation orchestration and previews
- `meshmend-worker-api`: stable request/response schema for native workers
- `meshmend-render`: rendering, overlays, view modes, screenshots
- `meshmend-stl`: STL parsing and writing

Project state should include:

- source file path and hash
- source mesh metadata
- calibrated unit and scale
- printer profile
- current mesh state
- undoable operations
- operation parameters
- worker logs and progress summaries
- exported file paths

## Geometry And Worker Architecture

Use Rust for:

- app shell and UI
- file IO and project state
- STL load/write
- renderer and GPU resources
- mesh selection, simple topology, and operation orchestration
- progress UI, cancellation, and validation summaries

Use C++ workers where mature geometry libraries are needed:

- CGAL worker for polygon mesh processing: mesh cleanup, connected components,
  hole filling, clipping/corefinement where appropriate, remeshing, and
  self-intersection checks.
- OpenVDB worker for voxel/SDF wrapping: local implicit rebuilds, cavity fills,
  and volume-to-mesh outputs.
- Optional Manifold or libigl experiments only after CGAL/OpenVDB are evaluated
  against the rose-class cases.

Prefer an external worker process before direct in-process FFI:

- process isolation protects the UI from C++ crashes and long-running jobs
- stdout/stderr can stream progress JSON and logs
- temp mesh files avoid unstable pointer ownership between Rust and C++
- cancellation can terminate the worker safely

Use `cxx` or a C ABI only for small, stable calls after the process protocol is
proven. Do not block the UI thread on geometry repair.

Every worker request should include:

- operation type
- input mesh path or mesh cache ID
- selected ROI or boundary loops
- physical scale
- target edge length/resolution
- preview or apply mode
- output path

Every worker response should include:

- success/failure
- progress events
- output mesh path
- changed region bounds
- topology metrics
- warnings
- validation summary

## Automatic Defect Detection

Build analysis in layers.

### Topology Pass

- count connected components
- find open boundary loops
- identify non-manifold edges and vertices
- identify degenerate triangles and duplicate triangles
- detect inconsistent orientation and flipped normals
- find tiny disconnected fragments

### Intersection Pass

- broad-phase triangle acceleration
- self-intersection candidates
- exact C++ confirmation for hard cases
- group intersections spatially for UI display

### Internal Shell And Cavity Pass

- connected component classification
- signed containment checks using winding/ray tests
- voxel outside flood fill to identify enclosed voids
- detect small openings into large cavities
- estimate trapped resin volume
- flag internal secondary surfaces for deletion or local wrapping

### Printability Pass

- after scale calibration, compute minimum local thickness and feature size
- compare target mesh density with printer XY/layer resolution
- flag areas that are too dense to matter physically
- flag areas too thin or likely to fail

## Repair Operations

### Clean Mesh Operation

Purpose: safe automatic cleanup before specific repair.

Steps:

- remove degenerate triangles
- merge duplicate vertices within tolerance
- remove duplicate faces
- orient components when possible
- split or flag non-manifold regions that need manual choice

### Hole Fill Operation

Purpose: close genuine open boundaries.

Steps:

- select boundary loop manually or from analysis
- classify simple versus complex loops
- triangulate simple holes
- refine and remesh larger patches to target edge length
- fair/smooth patch while preserving boundary
- validate no new non-manifold edges

### Local Cavity Replacement Operation

Purpose: eliminate cave-like internal voids without remeshing the whole model.

Inputs:

- healthy anchor stroke or selected anchor region
- target/cavity opening region
- optional exclude region
- target physical resolution when scale exists

Steps:

- extract ROI around target and anchors
- identify and remove interior shell surfaces inside ROI
- build a local implicit/voxel repair volume
- reconstruct a patch constrained by healthy anchors
- blend patch into existing surface
- remesh patch to target edge length
- validate boundaries, intersections, and visible deviation

### Surface Wrap Operation

Purpose: rebuild a selected local area from the outside shape.

Steps:

- create local signed distance field
- preserve outer visible surface and anchors
- close internal cavities and tunnels
- extract new surface
- trim and stitch into original mesh
- remesh transition band

### Cut/Bisect Operation

Purpose: fast printable chopping.

Straight cut:

- draw screen line
- derive cut plane
- preview kept/deleted side
- split mesh by plane
- delete chosen side
- cap boundary loops
- remesh cap
- validate output

Freehand cut:

- draw screen path
- generate cutting surface from camera rays
- split by surface
- choose kept side
- cap and remesh

### Remesh/Simplify Operation

Purpose: make mesh density match physical printer usefulness.

Inputs:

- calibrated scale
- printer profile
- target accuracy, for example 20 microns
- preserve-boundary and preserve-feature options

Steps:

- compute current average edge length in physical units
- choose target edge length from printer profile and user tolerance
- decimate areas denser than useful resolution
- remesh repaired patches
- preserve silhouette and high-curvature features
- report estimated deviation and triangle count reduction

## Save, Export, And Undo

Add explicit project and output handling.

- `Save Project`: writes `.meshmend` project metadata and operation history.
- `Export STL`: writes the current repaired mesh.
- `Export Report`: writes optional JSON/Markdown repair report.
- `Undo/Redo`: operation-level mesh state history.
- `Snapshots`: allow named before/after mesh states for risky operations.

The app must never overwrite the source STL without explicit export path
confirmation.

## CLI And Automation

Keep the Rust CLI, but make it serve real repair workflows instead of only
inspection.

Planned commands:

- `meshmend inspect input.stl`
- `meshmend analyze input.stl --output report.json`
- `meshmend repair input.stl --preset clean --output repaired.stl`
- `meshmend hole-fill input.stl --loop LOOP_ID --output repaired.stl`
- `meshmend cut input.stl --plane ... --keep positive --output cut.stl`
- `meshmend remesh input.stl --scale ... --target-microns 20 --output remesh.stl`
- `meshmend project validate file.meshmend`

The GUI should use the same operation engine as the CLI.

## Replace NPM With Just

This is now a Rust project. Remove the npm workflow after adding a `Justfile`.

Required just targets:

- `just run`: run `rose/raw.stl` if present, otherwise open the app
- `just run-file path`: run a specific STL
- `just build`: debug build
- `just release`: release build
- `just test`: workspace tests
- `just lint`: format check and clippy
- `just verify`: fixture render and cross-section verification
- `just verify-rose`: local large-model checks when `rose/raw.stl` exists
- `just perf path`: performance report
- `just clean`: clean generated outputs, not source assets
- `just worker-build`: build C++ workers

Update `.codex/environments/environment.toml` so the Run action uses
`just run`. Remove `package.json` and `scripts/run-meshmend.sh` once the Just
workflow is working.

## Archive Cleanup Plan

The archive must be removed from active source control after useful behavior is
ported.

Inventory:

- `archive/python-resinmesh/src/resinmesh/diagnostics.py`: port useful mesh
  metrics, report ideas, and screenshot/contact-sheet concepts.
- `archive/python-resinmesh/src/resinmesh/voxel.py`: replace with OpenVDB/local
  SDF worker implementation.
- `archive/python-resinmesh/src/resinmesh/roi.py`: replace with native
  selection, BVH, and repair ROI extraction.
- `archive/python-resinmesh/src/resinmesh/roi_ui.py` and `roi_3d_ui.py`: delete
  after native UI tools exist.
- `archive/python-resinmesh/src/resinmesh/cli.py`: port useful command
  semantics into Rust CLI operations.
- `archive/python-resinmesh/tests`: convert relevant tests into Rust fixtures,
  synthetic meshes, and worker golden tests.

Cleanup stages:

1. Extract an inventory checklist into this master plan implementation tracking.
2. Port each useful behavior into Rust or C++ workers.
3. Add equivalent tests before deleting the Python reference.
4. Delete `archive/python-resinmesh`, Python requirements, and archive README
   references.
5. Verify no Python repair code remains in active product paths.

## Implementation Phases

Do this in commit-sized stages. Do not split this into separate plan files.

### Phase 0: Plan Consolidation

- Add this master plan.
- Confirm no other `*plan*.md` files remain.
- Keep architecture docs only as references.

Acceptance:

- `docs/meshmend-master-plan.md` is the single plan file.
- No obsolete plan markdown remains.

### Phase 1: Just Workflow And Run Action

- Add `Justfile`.
- Move Run action to `just run`.
- Update README and AGENTS command references.
- Remove npm scripts and package file after parity.

Acceptance:

- `just run` launches the app.
- `just verify` runs current verification.
- Codex Run action launches MeshMend.
- `npm` is no longer required.

### Phase 2: UI Shell And Tool Palette

- Replace top checkbox strip with icon tool palette and view mode strip.
- Move model stats into collapsible panel.
- Replace Labels/Issues panels with Defects/Repair/Operations panels.
- Add tooltips and keyboard shortcuts.

Acceptance:

- Normal, rendered, wire, x-ray, cross-section, and lighting modes are visible
  as discoverable controls.
- Repair tools are reachable from the palette.
- The viewport is visually dominant.

### Phase 3: View Modes And Lighting

- Add headlight lighting tied to camera.
- Add studio/fixed light modes.
- Add surface wire and x-ray wire as distinct modes.
- Improve normal mode access.
- Add transparent shell rendering.

Acceptance:

- Back-side inspection remains readable while orbiting.
- Surface wire only shows visible surface.
- X-ray wire shows internal/back-side structure.
- Normal mode is one click or shortcut away.

### Phase 4: Selection Core

- Add CPU BVH/intersection stack.
- Add x-ray pick-through selection.
- Add cross-section-aware picking.
- Add selection overlays for boundary loops and components.

Acceptance:

- User can pick visible, back-side, and internal hits in x-ray mode.
- Picking remains responsive on `rose/raw.stl`.

### Phase 5: Project And Operation State

- Introduce project model and operation history.
- Replace annotation-first session UI.
- Add save project, export STL, undo, redo, and operation logs.

Acceptance:

- A repair operation can be previewed, applied, undone, and exported.
- JSON labels/issues are no longer the main product output.

### Phase 6: Mesh Analysis

- Add topology defect detection.
- Add automatic defect list and viewport overlays.
- Add CLI `analyze`.
- Add synthetic fixture meshes for holes, non-manifold edges, internal shells,
  and self-intersections.

Acceptance:

- The app identifies open holes and simple topology errors automatically.
- Analysis results are grouped, selectable, and frameable.

### Phase 7: Worker Framework

- Add worker request/response schema.
- Add worker process runner with progress and cancellation.
- Add C++ worker build path.
- Add initial CGAL mesh load/validate smoke operation.

Acceptance:

- UI can run a background worker without freezing.
- Worker progress appears in the job bar.
- Worker failure is reported without crashing the app.

### Phase 8: Hole Fill And Simple Repair

- Implement boundary-loop selection.
- Implement simple hole fill worker.
- Add gridded/refined patch output for larger holes.
- Validate repaired mesh.

Acceptance:

- Open holes can be filled and exported.
- Filled patches remain sculptable, not single stretched fans.

### Phase 9: Local Cavity Repair

- Implement anchor/target repair brush as operation input.
- Detect and remove internal ROI surfaces.
- Generate local wrap/replacement patch.
- Preview and apply local repair.

Acceptance:

- Rose-class cavity workflow can remove an internal cave and replace the local
  surface while preserving surrounding healthy surface.

### Phase 10: Cut/Bisect

- Implement straight line cut.
- Add side preview and side deletion.
- Cap and remesh cut surface.
- Add later freehand knife path.

Acceptance:

- User can chop away unwanted mesh parts quickly and export a closed result.

### Phase 11: Physical Scale And Remesh

- Add two-point measure tool.
- Add scale assignment.
- Add printer profile.
- Add remesh/simplify operation based on target microns.

Acceptance:

- User can set a 1 cm reference distance.
- App can remesh toward a 20 micron style target and report triangle reduction
  and estimated deviation.

### Phase 12: Export, Reports, And Validation

- Export repaired STL.
- Save project.
- Export repair report.
- Add final validation pass before export.

Acceptance:

- Exported mesh is validated for boundary loops, non-manifold edges, internal
  shells, and scale metadata/report consistency.

### Phase 13: Archive Deletion

- Port useful Python behavior.
- Convert needed tests.
- Delete `archive/python-resinmesh`.
- Remove Python-specific references and dependencies.

Acceptance:

- No active or archived Python repair pipeline remains in the repo.
- Replacement Rust/C++ tests cover the useful behavior that was kept.

### Phase 14: Packaging And Performance

- Package the native app for macOS first.
- Keep large mesh load and interaction performance measured.
- Add worker progress regression tests where possible.

Acceptance:

- Large STL viewing remains responsive.
- Repair jobs show progress and can be cancelled.
- Release build and verification pass.

## Verification Strategy

Every phase needs relevant verification. Core commands remain:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace --release
cargo run -p meshmend -- --verify-render fixtures/stl/cube_binary.stl
cargo run -p meshmend -- --verify-cross-section fixtures/stl/cube_binary.stl
```

After the Just migration, these become:

```bash
just lint
just test
just release
just verify
```

Add repair-specific tests:

- synthetic open hole mesh
- synthetic non-manifold mesh
- synthetic internal shell mesh
- synthetic small-opening-large-cavity mesh
- plane-cut fixture
- scale/remesh fixture
- rose local verification when `rose/raw.stl` exists

Repair validation metrics:

- boundary loop count
- non-manifold edge count
- component count
- self-intersection count
- internal shell/cavity estimate
- triangle count before/after
- physical target edge length and deviation
- render nonblank screenshot

## External References To Evaluate

- CGAL Polygon Mesh Processing for hole filling, remeshing, connected
  components, and mesh repair operations:
  https://doc.cgal.org/latest/Polygon_mesh_processing/group__PkgPolygonMeshProcessingRef.html
- OpenVDB for sparse volume and level-set workflows:
  https://www.openvdb.org/documentation/doxygen/index.html
- Rust `cxx` for safe Rust/C++ interop if process workers later need a direct
  API:
  https://cxx.rs/

Complete licensing and packaging checks before shipping any C++ dependency.

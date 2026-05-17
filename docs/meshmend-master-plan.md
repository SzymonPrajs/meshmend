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

## Detailed Research And Implementation Design

This section expands the hard parts of the plan. It should be used before
implementation starts on each phase, especially the repair and worker phases.

### Research Summary

The geometry plan should use a staged toolchain rather than betting on one
library to solve every case.

- Rust remains the owner of the app, UI, project state, rendering, selection,
  lightweight topology, and CLI.
- CGAL Polygon Mesh Processing is the first candidate for exact polygon mesh
  operations: connected components, orientation, stitching, hole filling,
  clipping/splitting, remeshing, simplification, and self-intersection checks.
  Its Polygon Mesh Processing and Surface Mesh Simplification packages are GPL,
  so licensing must be treated as a release blocker before distributing
  binaries.
- OpenVDB is the first candidate for local SDF/voxel wrap operations. It
  directly supports mesh-to-level-set conversion, signed/unsigned distance
  fields, cancellation callbacks for conversion, level-set filtering, and
  volume-to-mesh extraction.
- Manifold is a serious optional candidate for fast booleans and cut results
  when the input can first be made manifold. It explicitly requires manifold
  input for its guarantee and offers only limited help for slightly
  non-manifold meshes, so it is not the first bad-input repair tool.
- libigl remains optional research material. Its core is MPL2, but many useful
  mesh operations live under copyleft dependency folders, so include choices
  must be audited carefully.
- Rust BVH/spatial tooling should be evaluated for interactive picking:
  `parry3d::shape::TriMesh` has built-in BVH construction, ray casts,
  connected components, plane intersections, and split helpers; `bvh` is a
  focused ray/BVH crate; `rstar` is useful for broad-phase AABB candidate
  queries.

Research sources:

- CGAL PMP reference: https://doc.cgal.org/latest/Polygon_mesh_processing/group__PkgPolygonMeshProcessingRef.html
- CGAL PMP manual: https://doc.cgal.org/latest/Polygon_mesh_processing/index.html
- CGAL simplification manual: https://doc.cgal.org/latest/Surface_mesh_simplification/index.html
- CGAL license: https://www.cgal.org/license.html
- OpenVDB mesh-to-volume: https://www.openvdb.org/documentation/doxygen/MeshToVolume_8h.html
- OpenVDB volume-to-mesh: https://www.openvdb.org/documentation/doxygen/VolumeToMesh_8h_source.html
- OpenVDB transforms: https://www.openvdb.org/documentation/doxygen/transformsAndMaps.html
- OpenVDB dependencies: https://www.openvdb.org/documentation/doxygen/dependencies.html
- OpenVDB license: https://www.openvdb.org/license/
- Manifold docs: https://manifoldcad.org/docs/html/index.html
- libigl: https://libigl.github.io/
- Rust `cxx`: https://cxx.rs/
- Just manual: https://just.systems/man/en/
- parry3d `TriMesh`: https://docs.rs/parry3d/latest/parry3d/shape/struct.TriMesh.html
- Rust `bvh`: https://docs.rs/bvh/latest/bvh/
- Rust `rstar`: https://docs.rs/rstar/latest/rstar/struct.RTree.html

### Phase 1 Research: Just Workflow

`just` is a command runner, not a build system. That is exactly the right
replacement for `package.json` scripts in this Rust-native repo because it
stores project commands as recipes, supports recipe arguments, can list recipes,
and can be invoked from subdirectories.

Implementation:

- Add a root `Justfile`.
- Keep shell logic inline for small recipes only; use scripts only when logic
  becomes complex enough to test separately.
- Make the first/default recipe list commands with `just --list`.
- Use `just run` as the Codex Run action target.
- Keep `cargo` as the actual Rust build/test engine.
- Remove `package.json` and `scripts/run-meshmend.sh` only after command parity
  is proven.

Initial recipes:

```just
default:
    just --list

run:
    if [ -f rose/raw.stl ]; then cargo run -p meshmend -- rose/raw.stl; else cargo run -p meshmend; fi

run-file path:
    cargo run -p meshmend -- "{{path}}"

lint:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings

test:
    cargo test --workspace

build:
    cargo build --workspace

release:
    cargo build --workspace --release

verify:
    cargo run -p meshmend -- --verify-render fixtures/stl/cube_binary.stl
    cargo run -p meshmend -- --verify-cross-section fixtures/stl/cube_binary.stl

verify-rose:
    test -f rose/raw.stl
    cargo run -p meshmend -- inspect rose/raw.stl --parallel
    cargo run -p meshmend -- --verify-render rose/raw.stl
```

### Phase 2 Research: UI Shell And Tool Palette

The current UI is built from `egui` panels and checkboxes. It can support the
new shell without changing GUI libraries.

Implementation:

- Use a fixed-width left `SidePanel` for the tool palette.
- Use a top `TopBottomPanel` for active tool options and view modes.
- Use a right `SidePanel` only for contextual repair/analysis details.
- Use a bottom `TopBottomPanel` for status, progress, and job cancellation.
- Make the viewport a `CentralPanel` and add it last.
- Replace checkbox clusters with segmented mode buttons.
- Use icon buttons with tooltips; in current egui versions this can be done with
  image buttons or custom-painted icon glyphs. Prefer vendored SVG/icon assets
  converted into texture handles so the UI is not dependent on font glyphs.

Tool state model:

- Add `ToolMode`: select, navigate, analyze, cross_section, inspect_xray,
  repair_brush, hole_fill, cut, measure, remesh, export.
- Add `ViewMode`: rendered, headlight, studio, normals, surface_wire, xray_wire,
  transparent, cross_section, defect_overlay, thickness_overlay.
- Add `OperationPanelState`: idle, analysis_result, repair_preview,
  worker_running, export_ready, error.

Acceptance details:

- View modes must be visible even when no repair tool is active.
- The repair panel must contain action buttons: Preview, Apply, Cancel, Export.
- Model stats must be collapsible and not claim permanent viewport width.
- Labels/issues must no longer be the main visible product language.

### Phase 3 Research: View Modes And Lighting

The renderer already has solid shading, barycentric wire, normal debug,
cross-section clipping, and line overlays. The problem is that these are exposed
as scattered checkboxes and the lighting is fixed.

Implementation:

- Add a render uniform for `view_mode`.
- Add a render uniform for `lighting_mode`.
- Compute camera forward/right/up vectors from the camera each frame.
- Headlight mode: use a diffuse/specular light direction derived from camera
  forward plus ambient fill.
- Studio mode: use two or three fixed lights around the mesh plus ambient fill.
- Surface Wire: keep the current depth-tested barycentric overlay.
- X-Ray Wire: add a second mesh/wire pass with depth testing disabled or relaxed,
  low alpha surface, and stronger internal wire lines.
- Transparent mode: sorted transparency is expensive; start with one-pass alpha
  blending for inspection, not final-quality transparency.
- Defect Overlay: draw analysis result lines/points after mesh passes.

Risks:

- Full-order-independent transparency is not required for the first x-ray mode.
- X-ray mode must pair with pick-through selection; otherwise it is only visual.

### Phase 4 Research: Selection Core

The existing GPU picking pass returns only the front-most visible triangle. That
is good for normal surface selection but not for x-ray/internal repair.

Implementation options:

- `parry3d::shape::TriMesh` can build a BVH during mesh construction and offers
  ray casting, connected components, plane intersections, splitting, and
  optional topology flags. Prototype this first for pick-through and plane
  intersection because it gives the broadest immediate API surface.
- `bvh` is a focused ray/BVH crate and is a good fallback if `parry3d` memory or
  coordinate-type choices are not a fit.
- `rstar` is better for broad-phase spatial queries and candidate overlap
  detection than for exact ray intersection stacks.

Selection data structure:

- Build an indexed mesh from STL triangle soup.
- Keep mapping back to original chunk/local triangle IDs.
- Build an acceleration structure once per mesh state.
- Raycast from screen cursor and return all intersections sorted by distance.
- Filter by cross-section plane if active.
- In x-ray mode, show a small hit stack popup: front, inner, back, component id,
  distance.

Implementation stages:

1. Convert loaded STL to indexed vertices plus triangle indices.
2. Build a selection BVH sidecar.
3. Implement exact ray-triangle intersection and sorted hit list.
4. Swap x-ray selection to CPU hit-stack while keeping GPU picking for normal
   visible picking.
5. Add tests with nested cubes/spheres where the correct hit order is known.

Performance target:

- `rose/raw.stl` has about 1.95 million triangles. BVH build can be seconds if
  done after load, but cursor pick should be interactive after the BVH exists.
- Cache BVHs per mesh revision and rebuild in the background after repair
  operations.

### Phase 5 Research: Project And Operation State

The current issue JSON is not enough because repairs need source mesh,
generated mesh states, previews, worker logs, scale, exports, and undo history.

Project format:

- Use a directory-backed project first, optionally zipped later.
- `project.meshmend.json`: metadata, source hash, unit, scale, printer profile,
  current mesh revision, operations.
- `meshes/source.stl`: optional copy or hash reference to source.
- `meshes/rev-0001.stl`, `meshes/rev-0002.stl`: saved mesh revisions.
- `previews/`: temporary preview meshes and screenshots.
- `logs/`: worker JSONL logs.
- `reports/`: validation and export reports.

Operation record:

- operation id
- operation kind
- input mesh revision
- output mesh revision
- parameters
- ROI/selection references
- preview mesh path if any
- worker command and version
- start/end timestamps
- status and warnings
- validation summary

Undo model:

- For early versions, undo by switching current revision pointer.
- Later optimize storage with mesh deltas only if revision storage becomes too
  large.

### Phase 6 Research: Mesh Analysis

Analysis must be split into cheap Rust passes and exact/expensive worker passes.

Rust topology pass:

- Build quantized vertex map for STL triangle soup.
- Build undirected edge map keyed by vertex pair.
- Boundary edge: edge used by exactly one face.
- Non-manifold edge: edge used by more than two faces.
- Duplicate face: same sorted vertex triplet appears more than once.
- Degenerate triangle: repeated vertices or area below tolerance.
- Connected components: union faces across manifold shared edges.
- Boundary loops: trace boundary edges into loops and open chains.

Rust geometry pass:

- Compute bounds, area, volume estimate for closed oriented components.
- Compute average, min, p95, and p99 edge lengths.
- Group defects spatially by component and bounding box.
- Flag tiny disconnected components by face count and area.

CGAL confirmation pass:

- Use `does_self_intersect()` for boolean self-intersection status.
- Use `self_intersections()` to report face pairs for UI grouping.
- Use orientation functions for closed component orientation.
- Use connected components and volume connected components to cross-check Rust
  component classification on closed surfaces.

Internal shell and cavity pass:

- First classify disconnected components by containment. For a closed component,
  sample points and test inside/outside against larger closed components.
- Then voxelize the selected ROI or whole mesh at analysis resolution.
- Run outside flood fill in empty voxels from the grid boundary.
- Any empty region not reached is an enclosed void.
- Empty regions reached only through a narrow throat are cave-like cavities:
  report throat area, max interior radius, and approximate trapped volume.
- Candidate internal shell triangles are triangles adjacent to internal voids or
  components classified as contained inside another component.

Important distinction:

- A fully enclosed void and a cave connected by a tiny opening are different
  topologically but both can trap resin. The UI should call both "cavity"
  findings and show whether each is enclosed or throat-connected.

Analysis output schema:

- defect id
- defect kind
- severity
- component id
- triangle/edge ids
- world-space bounds
- recommended tool/action
- confidence
- validation notes

### Phase 7 Research: Worker Framework

The first worker API should be process-based, not FFI. Process workers isolate
crashes, make cancellation simple, and avoid Rust/C++ ownership problems while
we are still changing the data model.

Worker protocol:

- Rust writes a request JSON file.
- Rust launches worker binary with `--request request.json`.
- Worker writes progress events as newline-delimited JSON to stdout.
- Worker writes final response JSON to a known path.
- Rust streams stdout, updates progress UI, and can terminate the process on
  cancel.

Request fields:

- `schema_version`
- `operation`
- `input_mesh`
- `output_mesh`
- `preview`
- `scale`
- `target_edge_length`
- `roi_bounds`
- `selected_faces`
- `boundary_loops`
- `strokes`
- `options`

Progress event fields:

- `event`: started, phase, progress, warning, artifact, done, error
- `operation_id`
- `phase`
- `current`
- `total`
- `message`
- `artifact_path`

Build approach:

- Add `workers/cpp/CMakeLists.txt`.
- Add one binary per backend at first:
  - `meshmend-cgal-worker`
  - `meshmend-openvdb-worker`
  - `meshmend-manifold-worker` only if needed
- Add `just worker-build` to configure and build workers.
- Add worker discovery in Rust: look next to app binary, then `target/workers`,
  then environment override.

Dependency strategy:

- macOS local development can start with Homebrew packages where available.
- CI/release needs pinned dependency setup, probably CMake plus vcpkg or vendored
  source builds.
- CGAL licensing must be decided before distributing worker binaries. Because
  PMP and simplification are GPL packages, commercial licensing or GPL release
  implications must be resolved early.
- OpenVDB core uses C++17, TBB, and optional compression dependencies; this is
  heavier than CGAL and should stay in a separate worker binary.

When to use `cxx`:

- Only after request/response shapes stabilize.
- Use it for small stable APIs, not for long-running repair jobs.
- Keep process workers as the default for heavy operations.

### Phase 8 Research: Hole Fill And Simple Repair

Hole filling is the first real repair feature because it has clear geometry and
clear validation.

Detection:

- Rust finds boundary edges and traces loops.
- Filter loops by length, area, and whether the loop is open/ambiguous.
- Highlight the selected loop in the viewport.

Worker operation:

- Convert the mesh into `CGAL::Surface_mesh`.
- For simple loops, call `triangulate_hole()`.
- For most user-facing repairs, call `triangulate_and_refine_hole()` or
  `triangulate_refine_and_fair_hole()` so the patch is useful for sculpting.
- For large loops, set a timeout/progress visitor because CGAL hole filling cost
  depends on boundary vertex count.
- After filling, run local `isotropic_remeshing()` over the patch and transition
  ring with boundary constraints protected.

Patch quality rules:

- Avoid single-fan caps except for tiny holes.
- Patch target edge length comes from mesh-detail unit or physical target once
  scale is known.
- Validate that no new boundary loops, non-manifold edges, or self-intersections
  were introduced.
- Show patch triangle count and target edge length before apply.

Fallback:

- If CGAL patch self-intersects or times out, offer a planar/projected cap for
  simple near-planar holes and mark the operation as lower confidence.

### Phase 9 Research: Local Cavity Repair

This is the hardest rose-class feature. It should not be attempted as one giant
operation. Build it as three increasingly capable prototypes.

Prototype A: manual local patch replacement

- User paints healthy anchor ring around the area.
- User paints repair target/opening.
- App derives an ROI and boundary loop.
- Worker removes target faces and fills the boundary with a refined/fair patch.
- This solves visible holes and gives the UI/operation model a real repair path.

Prototype B: internal shell removal inside ROI

- Use x-ray selection and analysis to identify triangles that are inside the
  selected cavity/ROI.
- Remove internal shell triangles while preserving healthy exterior faces.
- Fill any resulting boundary loops.
- Validate no disconnected internal component remains in the ROI.

Prototype C: local SDF wrap

- Extract ROI mesh with margin.
- Add temporary seal geometry over the target opening when needed so the SDF
  represents the desired repaired outside, not the open cave.
- Convert ROI mesh/points to an OpenVDB level set at target voxel size.
- Use level-set filtering/smoothing within a mask, not across protected anchors.
- Extract a replacement mesh with `volumeToMesh()`.
- Trim replacement to the ROI boundary.
- Stitch or cap the replacement into the original mesh.
- Remesh the transition band.

OpenVDB design details:

- Use `Transform::createLinearTransform(voxel_size)` so voxel size is explicit
  in model units.
- Voxel size comes from physical target when scale exists; otherwise use mesh
  detail unit.
- Use narrow bands wide enough for smoothing and extraction; start with 3 to 5
  voxels on each side.
- Use interrupters/cancellation support in mesh-to-level-set conversion.
- Keep this worker ROI-local; whole-model voxelization at resin resolution will
  be too memory-heavy for dense models.

Validation:

- Compare repaired region against protected anchor samples.
- Check patch boundary, self-intersections, and component count.
- Re-run cavity detection inside the ROI.
- Render before/after preview from saved camera.

### Phase 10 Research: Cut/Bisect

The cut tool is both a UI operation and a mesh operation.

UI-to-plane conversion:

- User draws a screen-space line.
- Unproject both line endpoints into near/far rays.
- Define cut plane from endpoint rays and camera direction.
- Show the plane and both sides immediately using GPU clipping.
- User clicks a side to delete.

Worker implementation:

- For clean closed meshes, use CGAL `clip()` or `split()` against a halfspace.
- If using a cutter volume rather than a plane, use CGAL corefinement/boolean
  difference only after self-intersections are checked.
- CGAL booleans require non-self-intersecting input and manifold output, so the
  cut operation should run a preflight analysis before apply.
- After clipping, find boundary loops on the remaining mesh and use the hole
  fill operation to cap them.
- Remesh the cap and transition ring.

Preview:

- GPU preview comes first and is allowed to be approximate.
- Worker preview generates an actual mesh before apply.
- Side selection should be reversible until Apply.

Freehand knife later:

- Convert screen path to a ruled/extruded cutting surface.
- Split with the cutting surface.
- Require more validation because self-intersecting cut surfaces are easy to
  draw by accident.

### Phase 11 Research: Physical Scale And Remesh

STL does not carry reliable unit metadata, so scale must be a project property.

Measure/scale tool:

- Pick point A and point B on the mesh.
- Show model-space distance.
- User enters real distance and unit.
- Store `model_units_per_mm` or equivalent in the project.
- Derive model bounds in physical units.

Printer profile:

- XY pixel size in microns
- layer height in microns
- minimum wall thickness
- target surface tolerance
- target edge length multiplier

Resolution policy:

- Printer XY pixel size is not automatically the mesh edge length. Use it as
  the lower bound for meaningful surface accuracy.
- Start with target edge length around 2x to 3x the printer XY pixel size for
  broad remesh/simplification, with an advanced override for 1x when needed.
- For repaired patches, use smaller target edge length near high curvature and
  larger target edge length on flat/smooth areas.

Implementation:

- Use CGAL `isotropic_remeshing()` for local repaired regions and caps.
- Use CGAL Surface Mesh Simplification `edge_collapse()` for whole-model
  triangle reduction when the mesh is already a valid oriented 2-manifold.
- Use ACVD or Delaunay surface remeshing only after tests show they preserve the
  topology and visual details needed for organic AI meshes.
- Always compute deviation metrics after simplification; do not treat triangle
  count reduction as success by itself.

Validation:

- before/after triangle count
- average/p95 edge length in physical units
- approximate Hausdorff/deviation sampling
- normals and silhouette screenshot comparison
- boundary/non-manifold/self-intersection checks

### Phase 12 Research: Export, Reports, And Validation

Export should run final validation, not just write triangles.

Required final validation:

- STL write/read round trip
- boundary loop count
- non-manifold edge count
- component count
- self-intersection status
- internal shell/cavity estimate
- triangle count
- physical dimensions when scale exists
- repair operations applied

Output formats:

- STL is required because the current workflow is resin printing.
- Project file stores scale and operation history because STL cannot be trusted
  to preserve that context.
- Consider 3MF later for richer manufacturing metadata, but do not block STL
  export on 3MF support.

Report:

- JSON for automation
- Markdown for human reading
- include source hash, exported file hash, scale, operations, validation
  metrics, warnings, and screenshots if generated

### Phase 13 Research: Archive Deletion

Delete the archive only after porting useful behavior. Do not keep Python as a
parallel fallback.

Port mapping:

- diagnostics: Rust `meshmend-analysis` metrics and report generation.
- voxel: OpenVDB worker prototypes.
- ROI: Rust selection/BVH/ROI extraction.
- ROI UI: replaced by native tool palette, x-ray selection, and repair brush.
- CLI: Rust CLI repair commands.
- tests: synthetic Rust fixtures and worker golden tests.

Deletion gate:

- No current command references the Python archive.
- Useful tests are ported.
- Replacement repair worker can run at least one real operation.
- README/AGENTS/docs no longer describe archive code as a future source of
  product behavior.

### Phase 14 Research: Packaging And Performance

Packaging is complicated because of C++ worker dependencies.

Performance targets:

- `rose/raw.stl` opens and renders.
- viewport interaction stays responsive after BVH build.
- background worker jobs never block UI redraw.
- worker cancellation is reliable.
- each repair operation emits progress.

Packaging stages:

1. Package Rust app only.
2. Package app plus worker binaries built locally.
3. Package app plus pinned worker dependencies.
4. Add codesigning/notarization only after binary layout stabilizes.

Worker binary layout:

```text
MeshMend.app/
  Contents/MacOS/meshmend
  Contents/Resources/workers/meshmend-cgal-worker
  Contents/Resources/workers/meshmend-openvdb-worker
```

Release blocker list:

- CGAL licensing decision.
- OpenVDB dependency bundle strategy.
- worker crash/error reporting.
- output validation.
- large-model performance report.

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

## Research Sources And Licensing Gates

Use primary documentation when implementing or changing a worker dependency.

- CGAL Polygon Mesh Processing:
  https://doc.cgal.org/latest/Polygon_mesh_processing/group__PkgPolygonMeshProcessingRef.html
- CGAL Polygon Mesh Processing manual:
  https://doc.cgal.org/latest/Polygon_mesh_processing/index.html
- CGAL Surface Mesh Simplification:
  https://doc.cgal.org/latest/Surface_mesh_simplification/index.html
- CGAL licensing:
  https://www.cgal.org/license.html
- OpenVDB mesh-to-volume:
  https://www.openvdb.org/documentation/doxygen/MeshToVolume_8h.html
- OpenVDB volume-to-mesh:
  https://www.openvdb.org/documentation/doxygen/VolumeToMesh_8h_source.html
- OpenVDB transforms:
  https://www.openvdb.org/documentation/doxygen/transformsAndMaps.html
- OpenVDB license:
  https://www.openvdb.org/license/
- Manifold:
  https://manifoldcad.org/docs/html/index.html
- libigl:
  https://libigl.github.io/
- Rust `cxx`:
  https://cxx.rs/
- Just:
  https://just.systems/man/en/
- parry3d `TriMesh`:
  https://docs.rs/parry3d/latest/parry3d/shape/struct.TriMesh.html
- Rust `bvh`:
  https://docs.rs/bvh/latest/bvh/
- Rust `rstar`:
  https://docs.rs/rstar/latest/rstar/struct.RTree.html

Complete licensing and packaging checks before shipping any C++ dependency.
CGAL is the most urgent legal check because the plan currently depends on CGAL
GPL packages for several repair operations.

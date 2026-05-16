# Tauri STL Viewer Plan

## Objective

Build a clean desktop viewer for STL models before returning to repair tools.
The first useful version should do one thing well: load an STL file and let the
user inspect it with orbit, pan, and zoom controls.

The active test asset is:

```text
rose/raw.stl
```

This is now the only model asset the viewer should refer to. It is around 93 MB
and contains about 1.95 million triangles, so the viewer must be designed for a
large real test file from the start.

## Non-Goals For The First Demo

The first demo should not attempt any of the earlier repair functionality.

Do not include:

- mesh repair
- shrink wrapping
- voxel reconstruction
- remeshing
- smoothing
- slicing
- defect detection
- point selection
- persistent annotations

Those are later stages. The first stage is only a reliable STL viewer.

## Cleanup Plan

The old Python repair code has been archived under:

```text
archive/python-resinmesh/
```

Keep this as reference material only. It may still be useful for later ideas:

- mesh statistics
- diagnostic rendering
- cross-section experiments
- report formats
- validation terminology

The active tree should stay focused:

```text
README.md
AGENTS.md
docs/tauri-stl-viewer-plan.md
rose/raw.stl
archive/python-resinmesh/
```

Generated app outputs, `node_modules`, Rust `target`, and generated STL outputs
should remain ignored by Git.

## Recommended Stack

Use Tauri 2 for the desktop app shell and Rust packaging layer.

Use a Vite TypeScript frontend with Three.js as the first renderer.

Use WebGL first through Three.js. WebGPU should be treated as an evaluation path
after the viewer exists, not as the first implementation choice.

Reasons:

- Three.js already has `STLLoader`, `OrbitControls`, mature camera handling,
  material support, clipping planes, helpers, and later ray picking.
- The first bottleneck is likely STL parsing and geometry memory, not raw GPU
  shader throughput.
- WebGPU support is improving, but the first viewer benefits more from the
  stable Three.js WebGL path.
- A direct Rust `wgpu` viewer is possible, but it would mean rebuilding common
  viewer behavior that Three.js already provides.

## Architecture

Proposed app location:

```text
apps/rose-viewer/
```

Proposed structure:

```text
apps/rose-viewer/
  package.json
  vite.config.ts
  tsconfig.json
  src/
    main.ts
    viewer/
      createScene.ts
      loadStl.ts
      fitCamera.ts
      stats.ts
    styles.css
  src-tauri/
    Cargo.toml
    tauri.conf.json
    src/
      main.rs
```

Rust responsibilities:

- app shell
- packaging
- future native file dialogs if needed
- future background processes if repair tools are reintroduced

Frontend responsibilities:

- STL loading
- rendering
- camera controls
- viewport layout
- mesh stats
- future picking and annotations

For milestone 1, avoid a frontend framework. Use Vite with plain TypeScript.
That keeps the viewer small and makes the Three.js lifecycle easier to control.
A UI framework can be added later if annotation panels and workflow state become
large enough to justify it.

## STL Input Policy

The app should accept STL only.

For the first version, use a standard file input:

```text
Open STL -> user selects rose/raw.stl
```

This is better than hardcoding access to `rose/raw.stl` because it avoids early
Tauri filesystem permission and path-scope complexity.

Later, add:

- drag and drop
- recent files
- native Tauri file dialog
- last-opened file persistence

The loader should reject non-STL files clearly.

## Milestone 0: Done Cleanup And Asset Preparation

Completed:

- old Python toolchain archived under `archive/python-resinmesh/`
- failed repair STL outputs removed
- old experiment outputs removed
- `rose/raw.stl` retained as the only current model input

Current conversion result:

```text
rose/raw.stl
faces: 1,949,244
vertices: 974,605
size: about 93 MB
```

## Milestone 1: Minimal Viewer

Goal: open the Tauri app, choose an STL, inspect it.

Implementation tasks:

1. Create `apps/rose-viewer/` with Tauri, Vite, and TypeScript.
2. Install Three.js.
3. Build a full-window viewport.
4. Add a small top toolbar with:
   - open STL button
   - mesh name
   - triangle count
   - vertex count
   - bounds
5. Load STL from a browser `File` object.
6. Parse the file with Three.js `STLLoader`.
7. Build a `Mesh` using a neutral material.
8. Compute bounds.
9. Fit the camera to the model after loading.
10. Enable `OrbitControls`:
    - left drag orbit
    - right drag or modifier drag pan
    - scroll zoom
11. Add basic lighting:
    - ambient light
    - directional key light
    - optional hemisphere light
12. Add optional helpers:
    - grid
    - axes
    - model bounding box
13. Add a loading state while the 93 MB STL parses.
14. Add an error state for invalid files.

Definition of done:

- `npm run tauri dev` opens the app.
- `rose/raw.stl` loads from disk.
- the mesh is visible.
- orbit, pan, and zoom work.
- camera fit works.
- the viewport resizes correctly.
- no repair features are present.

## Milestone 1 Commands

These are the intended implementation commands when we move from plan to code:

```bash
mkdir -p apps
cd apps
npm create tauri-app@latest rose-viewer
cd rose-viewer
npm install three @types/three
npm run tauri dev
```

During scaffolding, choose:

- Tauri 2
- Vite
- TypeScript
- Vanilla frontend

Exact prompts may vary slightly by the current Tauri scaffolder.

## Milestone 2: Large STL Usability

The rose STL is large enough that milestone 2 should focus on responsiveness.

Tasks:

- move STL parsing into a Web Worker if the UI freezes
- show parse progress where possible
- preserve the previous camera state when reloading the same file
- add a reset-camera button
- add shaded, matcap-like, and wireframe display modes
- add backface display toggle
- add opacity slider for future internal inspection
- add a stats panel with:
  - triangle count
  - vertex count
  - bounding box dimensions
  - center point
  - estimated memory use
- add a warning if the file is too large for comfortable interactive viewing

Potential optimization:

- leave geometry non-indexed for first correctness
- evaluate `BufferGeometryUtils.mergeVertices` later
- do not simplify geometry in the viewer unless the user explicitly asks for a
  temporary preview mode

The viewer should not reduce detail silently.

## Milestone 3: Viewer Quality

Once basic loading is stable, improve inspection quality.

Tasks:

- better material presets:
  - clay
  - glossy gray
  - dark wireframe overlay
  - normals debug
- clipping plane preview controls
- screenshot export
- persistent UI preferences
- model unit display as "source units" until units are known
- optional model normalization only for camera framing, not for changing the
  mesh data

## Milestone 4: Marking And Analysis Foundation

Only after the viewer is stable, add the interaction needed for the original
hole and tunnel workflow.

Tasks:

- face picking
- multi-point markers
- marker list
- marker labels
- delete and edit markers
- save marker sessions as JSON
- load marker sessions
- render marker screenshots
- export a local region selection around a marker

Recommended dependency for later picking:

- `three-mesh-bvh`, if ray picking against a 1.95 million triangle STL becomes
  too slow

## Milestone 5: Cross-Sections

Cross-sections should come after the viewer and marker layer.

Start with visual GPU clipping:

- X, Y, and Z clipping planes
- adjustable plane position
- optional second plane for slabs
- cap rendering only if straightforward

Then evaluate real mesh section extraction:

- CPU slicing in JavaScript for display only
- Rust-side slicing for robust export
- Python archived algorithms only as references, not active dependencies

The first slicing goal should be "inspect internal structure", not "repair it".

## WebGPU Evaluation

Do not start with WebGPU.

Evaluate it after milestone 1 if one of these happens:

- the 93 MB STL cannot orbit smoothly in Three.js WebGL
- future cross-section or analysis views need GPU compute
- we need very large multi-model rendering

Possible paths:

- Three.js WebGPU renderer, if it is stable enough for the target environment
- direct Rust `wgpu`, if we decide to make rendering native instead of webview
- hybrid Tauri plus WebGPU frontend, keeping Three.js abstractions where useful

Expected tradeoff:

- WebGPU may improve future rendering and compute options.
- WebGPU increases implementation risk for the first viewer.
- Three.js WebGL is the pragmatic first milestone.

## Rust-Only Or Rust-Heavy Alternative

A Rust-native viewer is possible, but it is not the recommended first path.

Possible Rust stacks:

- `wgpu` for rendering
- `egui` or `iced` for UI
- `stl_io` or similar crates for STL parsing
- `nalgebra` or `glam` for math

Cost:

- orbit camera controls must be implemented or integrated
- picking must be implemented or integrated
- clipping and helper views require more custom work
- UI iteration is slower than a web frontend

This may be worth revisiting only if Tauri plus Three.js cannot handle the real
rose STL acceptably.

## Performance Risks

Known risks:

- `rose/raw.stl` is large.
- STL has no scene hierarchy, materials, units, or object names.
- Binary STL is still much bigger and less efficient than formats like GLB.
- Three.js `STLLoader` returns geometry suitable for display, but not an
  optimized application-specific mesh structure.
- Loading may require hundreds of MB of transient memory.

Mitigations:

- use binary STL input
- avoid copying geometry buffers unnecessarily
- parse in a worker if needed
- keep only one active mesh in memory
- dispose old geometry and materials on reload
- add a memory warning for very large files

Possible later improvement:

- internally convert STL to GLB cache for faster reloads, while keeping STL as
  the required user input format

## Viewer UX

First screen:

- full-window empty viewer
- centered open button
- small toolbar at top once loaded
- no marketing page
- no repair language

Controls:

- left mouse: orbit
- middle mouse or right mouse: pan
- wheel: zoom
- double click or toolbar button: fit to view

Display:

- dark neutral background
- gray clay material
- optional grid on by default
- axes helper small and unobtrusive

The app should feel like a practical inspection tool, not a landing page.

## Testing Plan

Automated tests for milestone 1 should be light:

- TypeScript type check
- Vite build
- Tauri build smoke test if local dependencies allow it

Manual verification:

- launch `npm run tauri dev`
- open `rose/raw.stl`
- confirm camera fit
- orbit around the rose
- zoom into petals
- pan across the model
- resize the window
- reload another STL if available

Visual verification:

- capture a screenshot of the loaded rose
- confirm the model is not blank
- confirm the model is not clipped at default fit
- confirm controls remain responsive

## Later Repair Tool Integration

The viewer should eventually become the front end for defect marking, not the
repair engine itself.

Longer-term shape:

```text
Tauri viewer
  -> user opens STL
  -> user marks suspicious region
  -> app saves marker JSON
  -> repair backend receives STL plus marker JSON
  -> backend creates candidate patch or repaired STL
  -> viewer compares before and after
```

This keeps the viewing and repair responsibilities separate.

## References Checked

- [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/)
- [Tauri create project](https://v2.tauri.app/start/create-project/)
- [Tauri architecture](https://v2.tauri.app/concept/architecture/)
- [Three.js STLLoader](https://threejs.org/docs/#examples/en/loaders/STLLoader)
- [Three.js OrbitControls](https://threejs.org/docs/#examples/en/controls/OrbitControls)
- [MDN WebGPU API](https://developer.mozilla.org/en-US/docs/Web/API/WebGPU_API)

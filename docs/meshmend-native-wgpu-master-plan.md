# MeshMend Native WGPU Master Plan

## Purpose

MeshMend is a desktop tool for inspecting and eventually repairing
AI-generated 3D models. The current Three.js viewer proved the first product
shape, but it is not the final architecture. The final direction should be a
native Rust renderer built on `wgpu`, with a Rust mesh engine, native GPU
rendering, CPU-parallel STL loading, GPU picking, notes, and later repair tools.

This plan is the master implementation plan. It covers:

- removing the current Three.js viewer
- replacing the rendering layer with native Rust `wgpu`
- parsing large STL files efficiently
- using all practical CPU cores for loading and preprocessing
- using the native GPU backend on macOS and Windows
- implementing camera controls, shading, wireframe, selection, notes, and
  validation
- committing work periodically as each milestone is completed
- leaving the repository in the final intended shape

## Core Decision

Use native Rust `wgpu` instead of browser WebGPU inside a Tauri webview.

Rationale:

- `wgpu` is a cross-platform Rust graphics API that runs natively over Metal,
  D3D12, Vulkan, and other backends.
- It gives us the WebGPU-style modern rendering model without depending on
  WebView2 or WKWebView WebGPU availability.
- It keeps the renderer under our control: buffers, shaders, picking,
  chunking, memory use, and later repair visualization.
- It lets Rust own the hot path: parsing, validation, threading, GPU uploads,
  camera math, and selection.

This means the final app should not be a webview-rendered Three.js app. It
should be a native Rust desktop app. Tauri can remain a packaging or launcher
option later, but it should not own the main renderer.

## Final Product Shape

The final product is a cross-platform native desktop application:

```text
MeshMend
  native Rust app
  winit window/event loop
  wgpu renderer
  egui overlay UI
  Rust mesh core
  Rust STL parser
  Rust selection and notes model
```

Supported first platforms:

- macOS Apple Silicon and Intel where practical
- Windows 10/11

Target GPU backends:

- macOS: Metal through `wgpu`
- Windows: D3D12 through `wgpu`
- fallback where appropriate: Vulkan or GL, but only after the primary paths
  are working

## Non-Goals For The Replacement Phase

Do not implement repair algorithms in the first native renderer milestone.

Do not implement:

- mesh repair
- shrink wrapping
- voxel reconstruction
- automatic hole filling
- smoothing
- remeshing
- slicer export

The first native milestone is a high-performance inspection tool. Repair comes
after loading, rendering, selection, notes, and validation are solid.

## Repository End State

The repository should end up as a Rust workspace with separate crates:

```text
.
  Cargo.toml
  AGENTS.md
  README.md
  docs/
    meshmend-native-wgpu-master-plan.md
    architecture/
      renderer.md
      stl-loading.md
      selection-and-notes.md
      verification.md
  apps/
    meshmend/
      Cargo.toml
      src/
        main.rs
        app.rs
        ui.rs
        input.rs
  crates/
    meshmend-core/
      Cargo.toml
      src/
        lib.rs
        mesh.rs
        bounds.rs
        units.rs
    meshmend-stl/
      Cargo.toml
      src/
        lib.rs
        binary.rs
        parse.rs
        validate.rs
    meshmend-render/
      Cargo.toml
      src/
        lib.rs
        renderer.rs
        camera.rs
        pipelines.rs
        buffers.rs
        picking.rs
        shaders/
          mesh.wgsl
          picking.wgsl
          grid.wgsl
    meshmend-notes/
      Cargo.toml
      src/
        lib.rs
        note.rs
        session.rs
    meshmend-io/
      Cargo.toml
      src/
        lib.rs
        file_dialog.rs
        project.rs
  fixtures/
    stl/
      cube_binary.stl
      open_hole_binary.stl
      malformed_header.stl
      invalid_count.stl
  assets/
    icons/
  scripts/
    generate_icons.py
```

Raw user data stays ignored:

```text
rose/
data/
raw/
outputs/
experiments/
screenshots/
```

The ignored local test file remains:

```text
rose/raw.stl
```

The test file is useful locally, but the app must load arbitrary user-selected
STL files.

## What To Remove

Remove the current Three.js/Tauri webview implementation once the replacement
branch begins.

Delete or replace:

- `apps/meshmend/index.html`
- `apps/meshmend/package.json`
- `apps/meshmend/package-lock.json`
- `apps/meshmend/tsconfig.json`
- `apps/meshmend/vite.config.ts`
- `apps/meshmend/src/`
- `apps/meshmend/src-tauri/` if the final app is native `winit` rather than
  Tauri
- `node_modules/`
- `dist/`
- any Three.js, Vite, or browser viewer wiring

Keep or migrate:

- app name: `MeshMend`
- generated app icon concept and source script
- documentation about STL-only input
- the ignored local `rose/raw.stl` test asset

After removal, commit immediately:

```text
chore: remove webview STL viewer
```

## Technology Stack

Core app:

- Rust stable
- Cargo workspace
- `winit` for windows and event loop
- `wgpu` for rendering
- `egui`, `egui-winit`, and `egui-wgpu` for UI overlay
- `glam` for vectors and matrices
- `rayon` for parallel parsing and preprocessing
- `memmap2` for large STL file reads
- `rfd` for native file dialogs
- `bytemuck` for GPU-safe buffer structs
- `tracing` and `tracing-subscriber` for diagnostics
- `anyhow` or `thiserror` for application and parser errors
- `serde` and `serde_json` for notes/session files

Optional later crates:

- `criterion` for performance benchmarks
- `insta` for snapshot tests
- `proptest` for parser property tests
- `cargo-fuzz` for fuzzing STL parser boundaries
- a BVH crate or custom BVH for CPU picking and fast spatial queries

## Application Modules

### `meshmend-core`

Owns data structures that are independent of file format and rendering.

Responsibilities:

- mesh metadata
- bounds and centers
- unit policy
- axis conventions
- chunk IDs
- triangle IDs
- model IDs
- shared error types where appropriate

Initial types:

```rust
pub struct MeshBounds {
    pub min: glam::Vec3,
    pub max: glam::Vec3,
}

pub struct MeshStats {
    pub triangle_count: u64,
    pub vertex_position_count: u64,
    pub bounds: MeshBounds,
    pub source_bytes: u64,
}

pub struct TriangleId {
    pub chunk: u32,
    pub local_index: u32,
}
```

### `meshmend-stl`

Owns STL parsing and validation.

Responsibilities:

- binary STL detection
- ASCII STL rejection or later support
- file size validation
- triangle count validation
- parallel parsing
- chunking
- bounds computation
- packed GPU-friendly triangle output
- parser tests

Initial policy:

- binary STL only
- reject ASCII STL with a clear error
- reject files whose length does not match `84 + triangle_count * 50`
- parse to triangle chunks
- do not deduplicate vertices in the first renderer

### `meshmend-render`

Owns native GPU rendering.

Responsibilities:

- `wgpu::Instance`
- adapter and device selection
- surface creation and configuration
- depth buffer
- render pipelines
- shader modules
- GPU buffers
- chunked mesh upload
- camera uniform
- solid shaded pass
- wireframe overlay
- grid and axes
- picking pass
- screenshot readback
- renderer tests where feasible

### `meshmend-notes`

Owns user annotations.

Responsibilities:

- note sessions
- selected points
- selected triangles
- labels
- marker colors
- saved JSON files
- note anchoring
- versioned session format

Initial session shape:

```json
{
  "version": 1,
  "model_file_name": "raw.stl",
  "model_file_size": 97462284,
  "notes": [
    {
      "id": "uuid",
      "triangle": { "chunk": 12, "local_index": 3456 },
      "position": [0.1, 0.2, 0.3],
      "label": "possible tunnel",
      "color": "#ffb347"
    }
  ]
}
```

### `meshmend-io`

Owns cross-platform file IO behavior.

Responsibilities:

- open STL dialog
- save note session dialog
- load note session dialog
- recent files
- project file format later

## Renderer Architecture

### Window And Event Loop

Use `winit` for the native window.

Event responsibilities:

- resize
- redraw request
- mouse down/up/move
- scroll wheel
- keyboard modifiers
- file drop
- close request
- platform DPI changes

The app should redraw continuously only while interacting or loading. When idle,
it should redraw on demand.

### GPU Initialization

Startup steps:

1. Create `wgpu::Instance`.
2. Create `wgpu::Surface` from the `winit` window.
3. Request high-performance adapter.
4. Prefer native backends:
   - Metal on macOS
   - D3D12 on Windows
5. Request device and queue with required limits.
6. Configure surface format and present mode.
7. Create depth texture.
8. Create render pipelines.
9. Create camera uniform buffer.

Log:

- selected backend
- adapter name
- device limits
- surface format
- present mode
- max buffer sizes

### GPU Mesh Representation

For STL, start with triangle chunks rather than indexed vertex meshes.

Use a storage buffer layout:

```rust
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuTriangle {
    pub p0: [f32; 4],
    pub p1: [f32; 4],
    pub p2: [f32; 4],
    pub normal: [f32; 4],
}
```

Why:

- STL is already triangle soup.
- No expensive deduplication is required before first render.
- Triangle IDs are stable.
- Picking can map directly to chunk and triangle index.
- Chunked uploads are straightforward.

Draw method:

```text
for each chunk:
  bind triangle storage buffer
  draw(vertex_count = 3, instance_count = triangle_count)
```

WGSL uses:

- `vertex_index` to pick p0/p1/p2
- `instance_index` to pick triangle
- model-view-projection matrix from camera uniform
- normal for simple lighting

### Shading

First material:

- neutral clay gray
- one directional light
- ambient fill
- optional hemisphere approximation
- depth testing
- backface toggle

Later:

- matcap-style material
- normal debug mode
- curvature or cavity debug mode
- clipping plane highlight

### Wireframe

Do not rely on GPU polygon line mode for cross-platform correctness.

Preferred approach:

- derive barycentric coordinates in shader from `vertex_index`
- draw a wireframe overlay with screen-space edge width
- expose a wireframe toggle in the UI

Alternative:

- create line-list buffers per chunk, but this doubles upload and memory
  pressure.

### Grid And Axes

Implement grid and axes as separate simple pipelines.

Requirements:

- toggle grid
- toggle axes
- keep scale tied to model bounds
- do not obscure the model

### Depth And Clipping

Initial:

- depth buffer only
- dynamic near/far based on model bounds

Later:

- shader clipping planes
- X/Y/Z clipping controls
- slab mode
- cap rendering if needed

## STL Loading Architecture

### File Open

The user must be able to load any STL file.

Supported first workflow:

```text
File -> Load STL -> choose .stl
```

Also support drag/drop onto the app window.

### Binary STL Validation

Binary STL format:

```text
80 byte header
4 byte little-endian triangle count
50 bytes per triangle
```

Validation:

```text
expected_size = 84 + triangle_count * 50
actual_size == expected_size
```

Reject:

- empty file
- file smaller than 84 bytes
- count that overflows expected size
- count that does not match file length
- ASCII STL in first version

### Multi-Threaded Parsing

Use `memmap2` for local file mapping.

Use `rayon` for parallel parsing:

1. Memory-map file.
2. Validate header and triangle count.
3. Divide triangle records into chunks.
4. Parse chunks in parallel.
5. Each worker computes:
   - packed `GpuTriangle` vector
   - local bounds
   - local triangle count
   - local error if malformed
6. Reduce chunk bounds into global bounds.
7. Send chunks to renderer upload queue.

Chunk target:

- start with 100,000 triangles per chunk
- benchmark 50,000, 100,000, 250,000, and 500,000
- choose based on upload smoothness and memory use

### Progress And Cancellation

The load pipeline must report:

- validating
- parsing chunk N of M
- uploading chunk N of M
- finished
- failed

Cancellation:

- if user opens another STL while loading, cancel current load
- drop pending chunks
- release old GPU buffers after the new load succeeds or after explicit cancel

### Memory Policy

Avoid avoidable copies.

Target flow:

```text
memmap bytes
  -> parallel chunk parse
  -> packed triangle Vec per chunk
  -> GPU buffer upload
  -> drop CPU chunk data if not needed for picking
```

For picking and notes, keep either:

- CPU triangle chunks, if memory allows
- a compact CPU copy of positions only
- or re-read relevant chunks from the memory map

Initial version can keep CPU chunks for simplicity. Optimize after measuring.

## Camera And Navigation

Implement CAD-style orbit camera.

Controls:

- left drag: orbit
- middle drag or right drag: pan
- wheel: zoom
- double click or button: fit view
- keyboard `F`: fit view
- keyboard `R`: reset view

Camera state:

```rust
pub struct Camera {
    pub target: Vec3,
    pub distance: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub fov_y: f32,
    pub near: f32,
    pub far: f32,
}
```

Fit algorithm:

1. Compute global bounds.
2. Compute center and bounding sphere radius.
3. Set target to center.
4. Set distance from FOV and radius.
5. Set near/far from radius.
6. Clamp pitch to avoid inversion.

Pan:

- convert mouse delta to camera right/up vectors
- scale by distance and FOV

Zoom:

- exponential zoom factor
- clamp min/max distance based on bounds

## Selection And Notes

Selection is central to the later repair workflow.

### Point Selection

Use GPU picking, not CPU raycasting first.

Picking pass:

1. Render model to hidden `R32Uint` texture.
2. Encode triangle ID into pixel.
3. On click, copy 1 pixel to readback buffer.
4. Decode chunk and local triangle ID.
5. Compute exact hit position by CPU ray-triangle intersection against that
   triangle.
6. Add marker.

Why:

- GPU already knows what is visible.
- It handles overlapping petals correctly from the current camera.
- It gives the visible front triangle, which is what the user clicked.

### Multiple Notes

Notes requirements:

- add note at selected point
- edit note label
- delete note
- list notes in UI
- click note in list to frame it
- save notes as JSON
- load notes from JSON

Note anchor:

- model-space position
- triangle ID
- optional barycentric coordinates later

### Box And Lasso Selection

Later selection modes:

- rectangle selection
- lasso selection
- paint selection

Implementation:

- read a region of the picking texture
- collect unique triangle IDs
- show selected IDs as highlight overlay

## UI Plan

Use `egui` overlay.

Panels:

- top toolbar
- left file/model panel
- right notes panel
- bottom status/progress bar

Top toolbar:

- Load STL
- Fit
- Reset
- Wireframe toggle
- Backfaces toggle
- Grid toggle
- Screenshot

Model panel:

- file name
- file size
- triangle count
- chunk count
- bounds
- backend
- parse time
- upload time
- frame time

Notes panel:

- notes list
- selected note details
- save notes
- load notes

Status bar:

- loading progress
- errors
- warnings
- frame timing

## Rendering Verification

Every renderer milestone must include automated or semi-automated verification.

### Nonblank Render Check

Create a command or test mode:

```bash
cargo run -p meshmend -- --verify-render fixtures/stl/cube_binary.stl
```

It should:

1. Open an offscreen or hidden render target where supported.
2. Render a known STL.
3. Read back pixels.
4. Confirm:
   - image is not blank
   - depth has non-background content
   - rendered bounding box is inside frame

If fully headless GPU is unreliable on CI, keep this as a local verification
command and run it before commits touching renderer code.

### Screenshot Verification

Add:

```bash
cargo run -p meshmend -- --screenshot fixtures/stl/cube_binary.stl outputs/cube.png
```

Local rose verification:

```bash
cargo run -p meshmend -- --screenshot rose/raw.stl outputs/rose.png
```

Do not commit `outputs/`.

### Pixel Checks

For each verification screenshot:

- compute non-background pixel percentage
- compute approximate silhouette bounds
- confirm no full-black or full-transparent output
- confirm model is not clipped by frame

## Test Plan

### Unit Tests

`meshmend-stl`:

- parse valid binary cube
- reject empty file
- reject too-small file
- reject invalid triangle count
- reject ASCII STL in first version
- parse bounds correctly
- parse triangle normals correctly

`meshmend-core`:

- bounds union
- center calculation
- radius calculation
- triangle ID encode/decode

`meshmend-render`:

- camera fit math
- camera pan math
- camera zoom clamp
- picking ID encode/decode

`meshmend-notes`:

- save/load note session JSON
- preserve note IDs
- reject incompatible session version

### Integration Tests

Use small committed STL fixtures:

- binary cube
- two disconnected triangles
- malformed count
- large synthetic generated file, created during test and not committed

Integration tests:

- parse file
- compute stats
- create chunks
- upload to renderer in a local verification command
- render screenshot locally

### Performance Benchmarks

Use `criterion`:

- parse 10k triangles
- parse 100k triangles
- parse 1m triangles generated fixture
- bounds reduction
- chunk packing
- note session serialization

Local benchmark with ignored rose file:

```bash
cargo bench --bench stl_parse -- --rose rose/raw.stl
```

Do not require `rose/raw.stl` in CI.

### Manual Verification

Manual checks for every major renderer milestone:

- load `rose/raw.stl`
- confirm triangle count
- confirm bounds
- orbit smoothly
- pan smoothly
- zoom into dense petal areas
- toggle wireframe
- toggle backfaces
- fit/reset camera
- click visible triangles
- add several notes
- save and reload notes
- close and reopen app

## Review Plan

Every milestone gets a review before the next milestone starts.

Review checklist:

- Is the architecture still aligned with native `wgpu`?
- Is the hot path in Rust?
- Are large files handled without unnecessary copies?
- Are raw data files still ignored?
- Are tests present for pure logic?
- Was renderer behavior verified with screenshot or pixel checks?
- Are performance numbers recorded when relevant?
- Are docs updated?
- Was the work committed after verification?

## Commit Cadence

Commit after each coherent verified milestone. Do not wait for a giant final
commit.

Suggested sequence:

1. `docs: add native wgpu master plan`
2. `chore: remove webview stl viewer`
3. `build: create rust workspace`
4. `feat(stl): parse binary stl files`
5. `test(stl): add parser fixtures and malformed file tests`
6. `feat(app): open native window`
7. `feat(render): initialize wgpu surface`
8. `feat(render): draw chunked stl triangles`
9. `feat(camera): add orbit pan zoom and fit view`
10. `feat(ui): add load controls stats and progress`
11. `feat(render): add shading grid and wireframe`
12. `feat(selection): add gpu triangle picking`
13. `feat(notes): add point notes and session save load`
14. `test(render): add screenshot and nonblank render verification`
15. `docs: document native renderer workflow`

Commit rule:

- commit only after build and relevant tests pass
- include verification evidence in the commit message body for renderer work
- keep generated data out of Git

## Implementation Milestones

### Milestone 1: Remove Webview Viewer

Goal:

- remove Three.js/Vite/Tauri-webview renderer
- leave docs and assets in a clean state

Tasks:

- delete Node frontend files
- delete Three.js dependencies
- delete Vite config
- delete Tauri webview scaffold if not reused
- move icons to `assets/icons/`
- update `.gitignore`
- update README

Verification:

- `git status --ignored` shows raw data ignored
- no `three`, `vite`, or `node_modules` in active source
- docs point to native `wgpu`

Commit:

```text
chore: remove webview stl viewer
```

### Milestone 2: Rust Workspace

Goal:

- create final Cargo workspace shape

Tasks:

- root `Cargo.toml`
- app crate
- core crate
- STL crate
- renderer crate
- notes crate
- IO crate
- shared lint and formatting config

Verification:

```bash
cargo fmt --all --check
cargo check --workspace
cargo test --workspace
```

Commit:

```text
build: create native rust workspace
```

### Milestone 3: Binary STL Parser

Goal:

- parse binary STL quickly and safely

Tasks:

- implement header validation
- implement triangle count validation
- implement chunked parser
- implement bounds computation
- implement serial mesh stats
- add fixtures
- add unit tests

Verification:

```bash
cargo test -p meshmend-stl
cargo run -p meshmend -- inspect fixtures/stl/cube_binary.stl
```

Local verification:

```bash
cargo run -p meshmend -- inspect rose/raw.stl
```

Expected local rose stats:

```text
triangles: 1,949,244
source bytes: 97,462,284
```

Commit:

```text
feat(stl): parse binary stl files
```

### Milestone 4: Parallel Loading

Goal:

- use CPU cores for large STL parse and preprocessing

Tasks:

- add `memmap2`
- add `rayon`
- split triangle records into chunks
- parse chunks in parallel
- reduce bounds
- report progress
- support cancellation

Verification:

```bash
cargo test -p meshmend-stl
cargo bench -p meshmend-stl
```

Local verification:

```bash
cargo run -p meshmend -- inspect rose/raw.stl --parallel
```

Record:

- parse time
- chunk count
- CPU thread count
- memory use if available

Commit:

```text
feat(stl): add parallel chunked loading
```

### Milestone 5: Native Window

Goal:

- open a native cross-platform app window

Tasks:

- add `winit`
- implement event loop
- handle resize
- handle close
- handle redraw request
- basic app state

Verification:

```bash
cargo run -p meshmend
```

Commit:

```text
feat(app): open native desktop window
```

### Milestone 6: WGPU Initialization

Goal:

- create native GPU renderer

Tasks:

- create `wgpu::Instance`
- create surface
- request adapter
- request device and queue
- configure surface
- create depth texture
- clear screen
- log backend and adapter

Verification:

```bash
cargo run -p meshmend
```

Expected:

- window opens
- nonblack clear color or grid appears
- logs show Metal on macOS or D3D12 on Windows where available

Commit:

```text
feat(render): initialize native wgpu renderer
```

### Milestone 7: Chunked Triangle Rendering

Goal:

- render STL triangle chunks

Tasks:

- define `GpuTriangle`
- write WGSL mesh shader
- create render pipeline
- upload chunk buffers
- draw each chunk
- compute camera fit from model bounds

Verification:

```bash
cargo run -p meshmend -- rose/raw.stl
```

Expected:

- model visible
- no crash on 93 MB STL
- stats shown
- GPU memory stable after load

Commit:

```text
feat(render): draw chunked stl triangles
```

### Milestone 8: Camera Controls

Goal:

- inspect the model comfortably

Tasks:

- orbit camera
- pan
- zoom
- fit view
- reset view
- dynamic near/far
- DPI-correct mouse input

Verification:

- load cube fixture
- load `rose/raw.stl`
- orbit/pan/zoom
- zoom into dense areas
- fit view from several camera positions

Commit:

```text
feat(camera): add orbit pan zoom and fit view
```

### Milestone 9: UI Overlay

Goal:

- expose viewer controls and stats

Tasks:

- add `egui`
- add top toolbar
- add file load button
- add model stats panel
- add loading progress
- add error display
- add backend/device info

Verification:

- file dialog works on macOS
- file dialog works on Windows
- drag/drop works where supported
- UI remains responsive during load

Commit:

```text
feat(ui): add load controls stats and progress
```

### Milestone 10: Shading And Display Modes

Goal:

- make inspection useful

Tasks:

- clay shading
- directional light
- ambient fill
- grid
- axes
- wireframe overlay
- backface toggle
- normal debug mode

Verification:

- screenshot cube
- screenshot local rose test model
- nonblank pixel check
- wireframe visible
- no full-frame clipping

Commit:

```text
feat(render): add shading grid and wireframe
```

### Milestone 11: GPU Picking

Goal:

- select visible triangles accurately

Tasks:

- create picking texture
- render triangle IDs
- read one pixel on click
- decode triangle ID
- compute hit point
- show marker

Verification:

- click cube faces
- click dense local STL areas
- confirm selected triangle changes with camera
- confirm hidden back triangles are not selected through front surfaces

Commit:

```text
feat(selection): add gpu triangle picking
```

### Milestone 12: Notes

Goal:

- support user defect marking

Tasks:

- note model
- add note at selected point
- edit note label
- delete note
- frame note
- save notes JSON
- load notes JSON

Verification:

- add multiple notes
- save
- reload
- notes appear in correct positions
- invalid JSON produces clear error

Commit:

```text
feat(notes): add point notes and sessions
```

### Milestone 13: Screenshot And Render Verification

Goal:

- prove rendering automatically

Tasks:

- screenshot command
- nonblank pixel checker
- silhouette bounds checker
- local verification script
- documentation

Verification:

```bash
cargo run -p meshmend -- --screenshot fixtures/stl/cube_binary.stl outputs/cube.png
cargo run -p meshmend -- --verify-render fixtures/stl/cube_binary.stl
```

Local large model:

```bash
cargo run -p meshmend -- --verify-render rose/raw.stl
```

Commit:

```text
test(render): add screenshot verification
```

### Milestone 14: Performance Instrumentation

Goal:

- make load, upload, render, navigation, memory, and screenshot performance
  measurable before CI/CD and packaging work begins

This milestone must happen before release infrastructure. The renderer should
not be packaged until we can measure whether it is actually fast enough.

Tasks:

- add a performance metrics module
- measure STL file read time
- measure STL validation time
- measure STL parse time
- measure per-chunk parse time
- measure GPU upload time
- measure first-frame time
- measure total time to interactive
- measure frame time and FPS while idle
- measure frame time and FPS while orbiting
- measure frame time and FPS while panning
- measure frame time and FPS while zooming
- measure screenshot render/readback time
- track CPU memory use where platform APIs allow it
- track GPU buffer allocation totals from our renderer
- show live metrics in a debug overlay
- write metrics to JSON for repeatable comparison

Metrics JSON shape:

```json
{
  "version": 1,
  "app_version": "0.1.0",
  "platform": "macos",
  "gpu_backend": "metal",
  "adapter": "Apple M-series GPU",
  "file": {
    "name": "raw.stl",
    "bytes": 97462284,
    "triangles": 1949244
  },
  "timings_ms": {
    "file_map": 0.0,
    "validate": 0.0,
    "parse_total": 0.0,
    "gpu_upload_total": 0.0,
    "first_frame": 0.0,
    "time_to_interactive": 0.0,
    "screenshot": 0.0
  },
  "frame_stats": {
    "idle_fps_avg": 0.0,
    "orbit_fps_avg": 0.0,
    "pan_fps_avg": 0.0,
    "zoom_fps_avg": 0.0,
    "p95_frame_ms": 0.0,
    "p99_frame_ms": 0.0
  },
  "memory": {
    "cpu_rss_mb": 0.0,
    "gpu_buffer_mb": 0.0,
    "chunk_count": 0
  }
}
```

Performance commands:

```bash
cargo run -p meshmend -- perf fixtures/stl/cube_binary.stl \
  --output outputs/perf-cube.json

cargo run -p meshmend -- perf rose/raw.stl \
  --output outputs/perf-rose.json
```

The ignored local rose file should be used for real performance work, but it
must never be required in CI.

Verification:

- run performance command on cube fixture
- run performance command on local `rose/raw.stl`
- inspect debug overlay while orbiting
- save screenshot and metrics JSON
- record key numbers in the milestone notes

Commit:

```text
feat(perf): add renderer performance metrics
```

### Milestone 15: Performance Optimization Loop

Goal:

- use measured data to improve the renderer before investing in CI/CD and
  release packaging

This is a repeated loop, not a single one-off task.

Loop:

1. Load fixture STL and local `rose/raw.stl`.
2. Capture metrics JSON.
3. Capture screenshot.
4. Identify the worst bottleneck.
5. Make one focused optimization.
6. Re-run the same metrics.
7. Compare before/after.
8. Keep the change only if metrics improve without breaking visual correctness.
9. Commit the optimization with before/after numbers in the commit body.

Optimization candidates:

- chunk size tuning
- parallel parse scheduling
- avoiding duplicate CPU copies
- streaming chunks to GPU earlier
- merging very small chunks
- staging buffer reuse
- reducing per-frame allocations
- avoiding redraws while idle
- cache camera matrices
- separate solid and wireframe passes efficiently
- limit expensive readbacks to explicit verification or picking operations
- optimize picking readback path
- reduce UI overhead during orbit
- lazy-load optional debug views

Required comparison evidence:

```text
before:
  parse_total_ms:
  gpu_upload_total_ms:
  first_frame_ms:
  orbit_fps_avg:
  p95_frame_ms:
  cpu_rss_mb:
  gpu_buffer_mb:

after:
  parse_total_ms:
  gpu_upload_total_ms:
  first_frame_ms:
  orbit_fps_avg:
  p95_frame_ms:
  cpu_rss_mb:
  gpu_buffer_mb:
```

Optimization guardrails:

- do not reduce visual fidelity silently
- do not simplify or decimate the model as a hidden performance trick
- do not make notes or picking unstable
- do not remove validation checks to make parsing look faster
- do not optimize for the cube fixture at the expense of the local large STL
- do not keep optimizations that make the code substantially more complex
  without measurable benefit

Performance acceptance before CI/CD:

- `rose/raw.stl` loads successfully on the development Mac
- metrics JSON is produced
- screenshot is produced
- camera interaction is measurable
- obvious hot-path waste has been removed
- remaining bottlenecks are documented

Commit:

```text
perf(render): optimize large stl interaction
```

## Performance Targets

Initial targets for `rose/raw.stl` on a powerful Mac:

- parse and validate: measured and recorded, target under a few seconds after
  parallel loading
- first visible render: as soon as first chunks can be uploaded, if streaming is
  implemented
- final full model upload: measured and recorded
- orbit interaction: responsive at normal inspection zoom
- memory use: no unbounded copies and no duplicate full-file buffers unless
  explicitly justified

Do not promise fixed frame rates until measured. Record actual hardware,
backend, triangle count, and timings.

The performance loop must run before CI/CD and release packaging. CI/CD should
package a viewer whose performance characteristics are known, not a renderer we
have only proven to compile.

## Documentation Requirements

Docs to update as implementation proceeds:

- README with run/build commands
- architecture renderer doc
- STL loading doc
- selection and notes doc
- verification doc
- troubleshooting doc for GPU backend selection
- performance methodology doc
- latest local performance report summary

Each renderer milestone must include:

- what changed
- how to run it
- how it was verified
- known limitations
- relevant performance impact if the milestone touches loading, GPU upload,
  rendering, camera interaction, picking, or screenshots

## CI/CD And Release Plan

CI/CD comes after the native viewer core is working locally and after the
performance instrumentation and optimization loop exists. Do not spend the early
renderer milestones fighting packaging infrastructure before the app can load,
render, navigate, select, save notes, and report performance.

The final CI/CD shape should include:

- pull request checks
- Linux build
- Windows build
- disabled macOS build workflow
- release workflow
- downloadable GitHub release artifacts
- late-stage signing and notarization

### CI Principles

CI must prove the code is healthy without relying on ignored local data.

CI should use committed fixtures only:

```text
fixtures/stl/cube_binary.stl
fixtures/stl/open_hole_binary.stl
fixtures/stl/malformed_header.stl
fixtures/stl/invalid_count.stl
```

CI must not require:

```text
rose/raw.stl
```

That file is local, large, ignored, and only for manual/performance validation.

### Pull Request Checks

Run on every pull request and push to the main branch:

```text
.github/workflows/ci.yml
```

Jobs:

- format check
- clippy
- unit tests
- parser integration tests
- camera math tests
- notes serialization tests
- build Linux app
- build Windows app

Commands:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace --release
```

Renderer verification in regular CI should start conservative:

- run parser and logic tests always
- run screenshot/nonblank GPU verification only if the platform runner can do it
  reliably
- keep local GPU verification mandatory before renderer commits even if CI
  cannot prove it yet

If Linux GPU verification is needed later, investigate:

- software Vulkan through Lavapipe
- `xvfb`
- `wgpu` headless/offscreen rendering where supported

### Linux Build Job

Linux CI should build on Ubuntu.

Outputs:

- native binary archive first
- later AppImage
- later `.deb`

Initial artifact:

```text
MeshMend-linux-x86_64.tar.gz
```

Later packaging options:

- AppImage for broad user download
- `.deb` for Debian/Ubuntu users
- optional Flatpak only if distribution becomes important

Linux signing can be deferred. Package checksums are enough for early releases.

### Windows Build Job

Windows CI should build on the latest stable Windows runner.

Outputs:

- `.exe` binary archive first
- later MSI or installer

Initial artifact:

```text
MeshMend-windows-x86_64.zip
```

Later packaging options:

- MSI installer
- MSIX only if store-style packaging becomes useful

Windows code signing should be a late packaging milestone because it requires a
certificate, secret handling, and release process discipline.

### macOS Build Job

Mac builds are expensive and may consume limited GitHub credits. Do not enable
macOS builds by default.

Write the workflow, but keep it disabled until release time.

Options:

1. `workflow_dispatch` only
2. run only when manually triggered
3. run only for release tags after explicitly enabling it

Suggested file:

```text
.github/workflows/macos-release.disabled.yml
```

or:

```yaml
on:
  workflow_dispatch:
```

Do not include macOS in ordinary PR CI.

macOS outputs:

- `.app` bundle
- `.dmg` later
- zipped `.app` for early release testing

Initial artifact:

```text
MeshMend-macos-universal.zip
```

Later:

```text
MeshMend-macos-universal.dmg
```

Universal builds are ideal eventually, but arm64-only is acceptable for early
local testing if that is what the development hardware supports.

### Release Marking

Use Git tags as the release marker.

Release flow:

1. Update version numbers.
2. Update changelog.
3. Commit release prep.
4. Create annotated tag:

```bash
git tag -a v0.1.0 -m "MeshMend v0.1.0"
git push origin main --tags
```

5. GitHub release workflow builds artifacts.
6. Workflow uploads artifacts to GitHub Releases.
7. Release notes list:
   - supported platforms
   - known limitations
   - verification status
   - whether packages are signed

Release workflow:

```text
.github/workflows/release.yml
```

Trigger:

```yaml
on:
  push:
    tags:
      - "v*"
```

Release artifacts:

- `MeshMend-linux-x86_64.tar.gz`
- `MeshMend-windows-x86_64.zip`
- `MeshMend-macos-universal.zip` only when macOS release job is enabled
- checksums file
- optional SBOM later

### Downloadable GitHub Packages

The GitHub Releases page should be the first distribution channel.

Each release should provide:

- platform artifact
- SHA256 checksum
- release notes
- install/run notes
- signing status

Early release status may be:

```text
Unsigned developer preview
```

Later release status should become:

```text
Signed and notarized where applicable
```

### Signing Plan

Signing is a late-stage packaging milestone. Do not block early viewer
implementation on it.

#### Windows Signing

Needed for proper installable Windows packages.

Tasks:

- choose signing certificate provider
- buy or provision code signing certificate
- store certificate securely as GitHub Actions secret
- add signing step to release workflow
- sign `.exe` and installer
- verify signature in CI

Risks:

- certificate cost
- secret handling
- timestamp server reliability

#### macOS Signing And Notarization

Needed for normal macOS distribution outside local developer machines.

Tasks:

- Apple Developer account
- Developer ID Application certificate
- store certificate and password as GitHub secrets
- sign `.app`
- create `.dmg`
- sign `.dmg`
- notarize with Apple notary service
- staple notarization ticket
- verify Gatekeeper acceptance

Because macOS runners are expensive, notarization should happen only in release
workflow, not in PR CI.

#### Linux Signing

Linux signing can wait.

Initial Linux releases can use:

- checksums
- GitHub release provenance

Later options:

- sign checksums with GPG or Sigstore
- sign AppImage if the chosen packaging route supports it cleanly

### Release Milestones

#### Release Milestone 1: CI Without Packaging

Goal:

- make every PR prove code health

Tasks:

- add `.github/workflows/ci.yml`
- run fmt
- run clippy
- run tests
- build Linux release binary
- build Windows release binary
- leave macOS disabled

Commit:

```text
ci: add linux and windows checks
```

#### Release Milestone 2: Artifact Builds

Goal:

- produce downloadable artifacts from CI

Tasks:

- upload Linux artifact
- upload Windows artifact
- add checksums
- keep artifacts on PR builds for debugging
- do not publish releases yet

Commit:

```text
ci: upload linux and windows build artifacts
```

#### Release Milestone 3: Release Tags

Goal:

- publish GitHub Releases from tags

Tasks:

- add `.github/workflows/release.yml`
- trigger on `v*` tags
- create release notes from checked-in changelog or manual body
- upload Linux and Windows artifacts
- upload checksums
- document release process

Commit:

```text
ci: publish tagged release artifacts
```

#### Release Milestone 4: Disabled macOS Release Workflow

Goal:

- keep macOS release logic ready but not consuming credits

Tasks:

- add disabled/manual macOS workflow
- document exactly how to run it
- ensure it does not run on PRs or normal pushes
- upload `.app` or `.dmg` only when manually triggered

Commit:

```text
ci: add manual macos release workflow
```

#### Release Milestone 5: Signing And Notarization

Goal:

- make release packages installable without warnings where practical

Tasks:

- add Windows signing
- add macOS signing
- add macOS notarization
- verify signatures
- document secret setup
- keep unsigned developer release path for local builds

Commit:

```text
ci: add release package signing
```

## Final Acceptance Criteria

The native MeshMend viewer is acceptable when:

- current Three.js viewer has been removed
- repo is a Rust workspace
- app opens on macOS
- app opens on Windows
- user can load arbitrary binary STL files
- `rose/raw.stl` loads locally
- triangle count and bounds are correct
- model renders with native `wgpu`
- camera orbit, pan, zoom, fit, and reset work
- shaded and wireframe modes work
- user can select visible triangles/points
- user can create, edit, save, and load notes
- screenshot verification works
- parser tests pass
- camera tests pass
- notes tests pass
- renderer verification has local evidence
- generated assets and raw data remain ignored
- each major milestone has been committed
- Linux CI builds pass
- Windows CI builds pass
- macOS release workflow exists but is disabled or manual by default
- tagged releases publish downloadable GitHub artifacts
- signing/notarization plan is documented and ready for late-stage execution

## Known Risks

Risk: native app takes longer than webview viewer.

Mitigation:

- keep milestones small
- commit after each verified step
- delay repair tools until viewer core is stable

Risk: `wgpu` and `egui` version churn.

Mitigation:

- pin versions in `Cargo.lock`
- update deliberately
- keep renderer code isolated from app/UI code

Risk: headless GPU verification differs across CI platforms.

Mitigation:

- keep parser and camera tests in CI
- keep GPU screenshot verification as local required evidence
- add CI GPU checks only when stable

Risk: STL triangle soup is memory-heavy.

Mitigation:

- chunk from the start
- avoid deduplication in first pass
- benchmark parse/upload
- only add optimization after measurement

## References

- `wgpu` documentation: https://wgpu.rs/doc/wgpu/index.html
- `wgpu` backends: https://wgpu.rs/doc/wgpu/struct.Backends.html
- `winit` documentation: https://docs.rs/winit/latest/winit/
- `egui` documentation: https://docs.rs/egui
- `egui-wgpu` documentation: https://docs.rs/egui-wgpu/latest/egui_wgpu/
- Tauri sidecars, if a wrapper is later needed:
  https://v2.tauri.app/develop/sidecar/

# MeshMend Command, Scenario, and Visual Control Plan

## Purpose

MeshMend is being developed through tight visual iteration. That means every user-visible tool needs a way to be exercised without manual clicking, and every repair workflow needs a way to produce repeatable evidence:

- a render or screenshot of the viewport,
- a structured state report,
- output STL files when geometry changes,
- enough command coverage that bugs can be reproduced from a script.

The current app has useful CLI commands for geometry operations and render checks, but the desktop UI has started to move faster than the command surface. The goal of this plan is to close that gap by adding a shared command layer, a batch scenario runner, deterministic viewport rendering, and later an optional live local control socket.

This is not a web server project. It should remain a Rust desktop app and Rust CLI. Any live control mode should be local-only, lightweight, and off by default.

## Current Baseline

The repo already has these useful foundations:

- `apps/meshmend/src/main.rs` uses `clap` and exposes commands such as `inspect`, `analyze`, `cut`, `remesh`, `export`, `perf`, `worker-smoke`, and project validation.
- Hidden verification flags already exercise some renderer paths:
  - `--screenshot STL PNG`
  - `--verify-render STL`
  - `--verify-cross-section STL`
  - `--verify-view-modes STL`
  - `--verify-hit-stack STL`
  - `--smoke-window`
- `apps/meshmend/src/app.rs` already contains the desktop event loop, viewer rendering, camera operations, cut preview/application, object selection, object hide/delete/keep/export, and screenshot capture.
- `crates/meshmend-render` already supports off-screen screenshots through the same WGPU renderer used by the app.
- `just verify`, `just smoke`, and `just repair-smoke` already prove a subset of behavior.

The main gap is that many UI actions are not yet represented as reusable commands. The UI currently owns too much behavior directly.

## Product Decision

Every meaningful UI action should have a corresponding command that can be run from tests or scripts.

The UI should become one front end for commands. The batch scenario runner and optional live control socket should be two more front ends for the same commands.

The implementation should therefore introduce a command/session architecture:

```text
UI button / keyboard shortcut
Batch scenario step
Live control socket request
        |
        v
Shared AppCommand executor
        |
        v
Mesh session + renderer session
        |
        v
State report, viewport image, STL output
```

If the UI can do something important, a scenario should eventually be able to do it too.

## Key Concepts

### Mesh Session

The mesh session owns logical document state:

- loaded STL path and display name,
- current triangle data,
- cut objects,
- selected object,
- hidden objects,
- undo history for geometry operations,
- dirty state,
- export paths,
- geometry reports.

This should be independent of egui and winit.

### View Session

The view session owns viewport and display state:

- viewport width and height,
- camera eye/target/up or orbit parameters,
- view mode,
- lighting mode,
- display toggles,
- active cut preview overlay,
- active selection/object overlay,
- screenshot output settings.

Some of this state will still be applied through `WgpuRenderer`, but the desired state should be serializable so a scenario can reproduce it.

### App Command

An app command is a single operation that changes the mesh session, the view session, the renderer, or the output artifacts.

Examples:

- `LoadStl`
- `SetViewMode`
- `FitCamera`
- `SetCamera`
- `SetTool`
- `SetCutOptions`
- `PreviewViewLineCut`
- `ApplyCut`
- `SelectObject`
- `SelectObjectAt`
- `HideSelectedObject`
- `DeleteSelectedObject`
- `KeepOnlySelectedObject`
- `ExportVisible`
- `Screenshot`

### Scenario

A scenario is a JSON or TOML file containing a deterministic sequence of commands. It should be runnable from the CLI and should produce screenshots, state reports, and exported meshes.

### Live Control Socket

The live control socket is optional and later-stage. It allows another process to send commands to a running app instance. It should use local IPC, not HTTP and not Node.

On macOS this should be a Unix domain socket. Messages should be JSON lines or a small JSON-RPC-like format.

## Non-Goals

- Do not add a Node server.
- Do not add a network listener by default.
- Do not make a remote-control API that is exposed outside the local machine.
- Do not make UI automation depend on fragile mouse coordinates when a semantic command exists.
- Do not require exact pixel-perfect screenshot comparison in the first version.
- Do not rewrite the renderer for this; use the existing WGPU renderer.

## Phase 1: Inventory and Command Coverage Map

### Goal

Create a complete map from UI functionality to commands, so we know what needs parity.

### Work

Audit current UI actions in `apps/meshmend/src/app.rs`:

- menu actions,
- top toolbar actions,
- context toolbar actions,
- left palette actions,
- keyboard shortcuts,
- mouse gestures,
- cut preview and apply flow,
- object selection and object operations,
- save/export actions.

Create a table in the plan or a new internal tracking comment that maps:

```text
UI action -> existing function -> desired AppCommand -> CLI/scenario support
```

Initial command coverage should include:

| Area | UI Behavior | Command Required |
| --- | --- | --- |
| File | open STL | `LoadStl` |
| File | save/export STL | `ExportVisible` |
| View | rendered/wire/x-ray/normals/etc. | `SetViewMode` |
| View | fit/reset | `FitCamera`, `ResetCamera` |
| Camera | orbit/pan/zoom | `SetCamera`, `OrbitCamera`, `PanCamera`, `ZoomCamera` |
| Tools | point/brush/cut | `SetTool` |
| Selection | vertex/edge/face | `SetSelectionElement` |
| Selection | front/through | `SetSelectionDepth` |
| Cut | cap density/smooth | `SetCutOptions` |
| Cut | draw line | `PreviewViewLineCut` |
| Cut | apply | `ApplyCut` |
| Objects | select object | `SelectObject`, `SelectObjectAt` |
| Objects | hide/delete/keep/export | matching object commands |
| Rendering | screenshot | `Screenshot` |
| Reporting | inspect state | `StateSnapshot` |

### Acceptance Criteria

- There is a clear list of all currently exposed UI actions.
- Each UI action has a command name, even if not implemented yet.
- Missing CLI/scenario support is explicitly visible.

## Phase 2: Extract Mesh Session

### Goal

Move document and geometry operation logic out of the UI event loop so it can be called by UI, CLI, and scenarios.

### New Module

Create a module such as:

```text
apps/meshmend/src/session.rs
```

or split it further later:

```text
apps/meshmend/src/session/document.rs
apps/meshmend/src/session/commands.rs
apps/meshmend/src/session/reports.rs
```

Start simple. Do not over-split before the interfaces settle.

### MeshSession Responsibilities

`MeshSession` should own:

- `model_info`,
- `mesh_document`,
- selected object,
- hidden object state,
- dirty state,
- undo stack for mesh operations,
- status text or structured last operation result.

It should expose methods like:

```rust
load_stl(path) -> Result<SessionReport>
apply_view_line_cut(cut_plane, cut_options, preferred_pick) -> Result<SessionReport>
select_object(index) -> Result<SessionReport>
select_object_for_render_triangle(render_triangle_index) -> Result<SessionReport>
hide_selected_object() -> Result<SessionReport>
delete_selected_object() -> Result<SessionReport>
keep_only_selected_object() -> Result<SessionReport>
export_visible(path) -> Result<SessionReport>
export_object(index, path) -> Result<SessionReport>
undo() -> Result<SessionReport>
```

### Important Detail

The session should not know about egui. It may know about renderer-independent screen coordinates only where needed for semantic operations, but it should not draw or receive raw UI events.

The UI should become thinner:

- translate mouse/keyboard/ui widgets into commands,
- call the session/command executor,
- ask renderer to update buffers or overlays,
- display the resulting status/report.

### Acceptance Criteria

- Existing UI behavior still works.
- Existing tests still pass.
- Object operations can be unit-tested without a running window.
- The geometry state after a cut can be inspected without reading UI state.

## Phase 3: Define Serializable Commands and State Reports

### Goal

Introduce a stable command schema that can be used by batch scenarios and later by a live control socket.

### AppCommand Enum

Create a serializable command enum:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum AppCommand {
    LoadStl { path: PathBuf },
    SetViewMode { mode: ViewModeName },
    FitCamera,
    ResetCamera,
    SetCamera { camera: CameraState },
    OrbitCamera { delta: [f32; 2] },
    PanCamera { delta: [f32; 2] },
    ZoomCamera { delta: f32 },
    SetTool { tool: ToolName },
    SetSelectionElement { element: SelectionElementName },
    SetSelectionDepth { depth: SelectionDepthName },
    SetCutOptions { cap_density: CapDensityName, smooth_cap: bool },
    PreviewViewLineCut { start: [f32; 2], end: [f32; 2] },
    ApplyCut,
    CancelCut,
    SelectObject { index: usize },
    SelectObjectAt { position: [f32; 2] },
    HideSelectedObject,
    DeleteSelectedObject,
    KeepOnlySelectedObject,
    ShowAllObjects,
    ExportVisible { path: PathBuf },
    ExportObject { index: usize, path: PathBuf },
    Screenshot { path: PathBuf },
    StateReport { path: Option<PathBuf> },
}
```

Names can be adjusted during implementation, but commands should be semantic and not UI-widget-specific.

### State Snapshot

Create a serializable snapshot containing:

- loaded file,
- triangle count,
- bounds,
- current view mode,
- camera state,
- active tool,
- active cut preview summary,
- object count,
- visible object count,
- selected object,
- per-object triangle count,
- per-object cap triangle count,
- per-object bounds,
- hidden state,
- latest status,
- latest error if any.

Example:

```json
{
  "file": "rose/raw.stl",
  "triangles": 1949244,
  "viewMode": "rendered",
  "tool": "cut",
  "cutPreview": {
    "active": true,
    "segments": 48,
    "affectedTriangles": 48
  },
  "objects": [
    {
      "index": 0,
      "name": "Object 1",
      "visible": true,
      "selected": false,
      "triangles": 103928,
      "capTriangles": 824,
      "bounds": {
        "min": [-0.1, -0.2, -0.3],
        "max": [0.2, 0.1, 0.4]
      }
    }
  ]
}
```

### Acceptance Criteria

- Commands serialize and deserialize from JSON.
- State reports serialize to JSON.
- Unit tests cover command parsing for representative commands.
- UI code can call command executor directly for at least one command before moving all commands.

## Phase 4: Renderer Session and Deterministic Screenshots

### Goal

Make viewport rendering scriptable and deterministic enough for repeatable visual checks.

### Work

Introduce a renderer/session bridge that can:

- create a hidden window with fixed size,
- load a mesh,
- apply a view mode,
- apply camera state,
- apply overlays,
- render one or more frames,
- write a PNG screenshot,
- return image statistics.

Existing screenshot code in `run_capture_with_options` should be refactored so it is reusable by scenarios.

### Camera State

Define serializable camera state:

```json
{
  "eye": [0.0, 0.0, 2.5],
  "target": [0.0, 0.0, 0.0],
  "up": [0.0, 1.0, 0.0]
}
```

Also support simple camera operations in scenario steps:

- `fitCamera`
- `resetCamera`
- `orbitCamera`
- `panCamera`
- `zoomCamera`

The important capability is to launch into a reproducible view for a screenshot.

### CLI Command

Add a first-class render command:

```bash
meshmend render rose/raw.stl \
  --output outputs/rose-rendered.png \
  --width 1600 \
  --height 1000 \
  --view rendered \
  --camera outputs/camera.json \
  --state outputs/rose-rendered-state.json
```

### Acceptance Criteria

- CLI can produce a PNG of a fixed-size viewport.
- CLI can use a saved camera state.
- CLI can output a state report alongside the PNG.
- `just verify` can include at least one deterministic screenshot check.

## Phase 5: Batch Scenario Runner

### Goal

Add a CLI command that runs a scripted sequence of operations and emits screenshots/reports at each important stage.

### CLI Command

```bash
meshmend scenario tests/scenarios/rose-stem-cut.json --output-dir outputs/rose-stem-cut
```

### Scenario Format

Use JSON first because the command schema can share serde structures. TOML can be added later if useful.

Example:

```json
{
  "name": "rose stem cut",
  "input": "rose/raw.stl",
  "viewport": {
    "width": 1600,
    "height": 1000
  },
  "steps": [
    {
      "type": "fit-camera"
    },
    {
      "type": "set-view-mode",
      "mode": "rendered"
    },
    {
      "type": "screenshot",
      "path": "01-loaded.png"
    },
    {
      "type": "set-tool",
      "tool": "cut"
    },
    {
      "type": "set-cut-options",
      "capDensity": "automatic",
      "smoothCap": false
    },
    {
      "type": "preview-view-line-cut",
      "start": [360, 650],
      "end": [860, 535]
    },
    {
      "type": "screenshot",
      "path": "02-cut-preview.png"
    },
    {
      "type": "apply-cut"
    },
    {
      "type": "state-report",
      "path": "03-cut-applied.json"
    },
    {
      "type": "screenshot",
      "path": "03-cut-applied.png"
    },
    {
      "type": "select-object",
      "index": 0
    },
    {
      "type": "delete-selected-object"
    },
    {
      "type": "screenshot",
      "path": "04-after-delete.png"
    },
    {
      "type": "export-visible",
      "path": "rose-stem-removed.stl"
    }
  ],
  "assertions": [
    {
      "type": "object-count-at-least",
      "count": 2
    },
    {
      "type": "screenshot-nonblank",
      "path": "03-cut-applied.png"
    },
    {
      "type": "export-reloads",
      "path": "rose-stem-removed.stl"
    }
  ]
}
```

### Scenario Output Directory

Each run should write:

```text
outputs/rose-stem-cut/
  scenario-input.json
  run-report.json
  01-loaded.png
  02-cut-preview.png
  03-cut-applied.json
  03-cut-applied.png
  04-after-delete.png
  rose-stem-removed.stl
```

### Assertions

Start with practical assertions:

- screenshot exists,
- screenshot is nonblank,
- object count equals or exceeds expected count,
- selected object exists,
- triangle count changed,
- exported STL reloads,
- exported STL has no open boundary loops where expected,
- command result status is success.

Do not start with exact image matching. Use image stats first.

### Acceptance Criteria

- A scenario can reproduce a basic cut workflow.
- A scenario can produce before/preview/after screenshots.
- A scenario can export an STL and validate it reloads.
- A scenario failure identifies the failed step and prints the state snapshot.

## Phase 6: UI Parity Migration

### Goal

Change UI event handlers so all meaningful actions call `AppCommand`.

### Migration Order

1. View commands:
   - `SetViewMode`
   - `FitCamera`
   - `ResetCamera`
2. Tool commands:
   - `SetTool`
   - `SetSelectionElement`
   - `SetSelectionDepth`
3. Cut commands:
   - `SetCutOptions`
   - `PreviewViewLineCut`
   - `ApplyCut`
   - `CancelCut`
4. Object commands:
   - `SelectObject`
   - `SelectObjectAt`
   - `HideSelectedObject`
   - `DeleteSelectedObject`
   - `KeepOnlySelectedObject`
   - `ShowAllObjects`
5. Export commands:
   - `ExportVisible`
   - `ExportObject`
   - `ExportAllObjects`

### UI Rule

The UI can still own layout, mouse input interpretation, and shortcuts. It should not own core operation semantics.

For example:

```text
Mouse drag in Cut tool
  -> UI converts drag to start/end screen coordinates
  -> AppCommand::PreviewViewLineCut
```

```text
Apply Cut button
  -> AppCommand::ApplyCut
```

```text
Object dropdown changed
  -> AppCommand::SelectObject
```

### Acceptance Criteria

- UI behavior stays the same or improves.
- The same command used by a UI button can also be used in a scenario.
- New commands include unit tests.
- No command requires egui types.

## Phase 7: Live Local Control Socket

### Goal

Allow a running app instance to receive commands from a separate process. This enables interactive debugging and external automation while preserving the desktop app experience.

### Launch Mode

Add:

```bash
meshmend rose/raw.stl --control-socket /tmp/meshmend.sock
```

or:

```bash
meshmend --control-socket auto rose/raw.stl
```

If `auto`, the app can create a socket path under:

```text
target/meshmend-control/meshmend-<pid>.sock
```

### Client Command

Either add a subcommand to the same binary:

```bash
meshmend control --socket /tmp/meshmend.sock screenshot outputs/live.png
meshmend control --socket /tmp/meshmend.sock preview-cut 360 650 860 535
meshmend control --socket /tmp/meshmend.sock apply-cut
meshmend control --socket /tmp/meshmend.sock state
```

or create a second binary later:

```text
meshmendctl
```

Start with a `meshmend control` subcommand to avoid extra packaging work.

### Protocol

Use JSON lines over a Unix domain socket:

Request:

```json
{"id":1,"command":{"type":"screenshot","path":"outputs/live.png"}}
```

Response:

```json
{"id":1,"ok":true,"state":{"objectsVisible":2,"selectedObject":0}}
```

Error:

```json
{"id":1,"ok":false,"error":"no object selected"}
```

### Event Loop Integration

The socket listener should run on a background thread and forward commands into the winit event loop through `EventLoopProxy`.

The main app thread applies commands because it owns the renderer and window state.

The response can be sent back through a oneshot channel managed by the listener. If that is too complex for the first version, the first live control mode can be fire-and-report-to-log, but the target should be request/response.

### Security and Safety

- Local-only Unix domain socket.
- No TCP listener.
- Socket disabled unless explicitly requested.
- If the socket path already exists, refuse unless a `--replace-control-socket` flag is passed.
- Clean up socket file on normal exit.
- Never allow arbitrary shell commands through the protocol.

### Acceptance Criteria

- Launching with `--control-socket` starts the app normally and creates a local socket.
- `meshmend control state` returns a state snapshot.
- `meshmend control screenshot` writes a PNG.
- `meshmend control preview-cut` and `apply-cut` work on the running app.
- The app remains responsive while the socket is active.

## Phase 8: Visual Regression Harness

### Goal

Use the scenario runner to produce repeatable screenshots and structured reports that can catch regressions.

### Test Strategy

Start with image statistics and semantic state checks:

- non-background pixel coverage,
- selected object overlay coverage,
- cut preview overlay coverage,
- object count,
- visible object count,
- triangle count,
- export reload success,
- analysis defect count.

Avoid exact pixel comparison until the renderer stabilizes more.

### Example Regression Scenarios

Create scenarios under:

```text
tests/scenarios/
```

Initial scenarios:

```text
tests/scenarios/cube-view-modes.json
tests/scenarios/cube-view-line-cut.json
tests/scenarios/cube-two-cuts.json
tests/scenarios/cube-delete-object.json
tests/scenarios/rose-load-render.json
```

The rose scenario should be optional because `rose/raw.stl` is ignored by Git. It can be used by `just verify-rose`.

### Just Recipes

Add:

```text
scenario-smoke:
    cargo run -p meshmend -- scenario tests/scenarios/cube-view-line-cut.json --output-dir outputs/scenario-cube-view-line-cut

scenario-rose:
    test -f rose/raw.stl
    cargo run -p meshmend -- scenario tests/scenarios/rose-load-render.json --output-dir outputs/scenario-rose-load-render
```

Then update:

```text
verify:
    ...
    just scenario-smoke
```

### Acceptance Criteria

- `just scenario-smoke` produces screenshots and reports.
- Scenario output can be inspected manually.
- CI/local verification can fail on blank screenshots or broken command results.
- The scenario runner gives enough information to debug the failing step.

## Phase 9: Repair Workflow Parity

### Goal

As repair tools are added, they must be added to the command layer and scenario runner immediately.

### Required Future Commands

For brush repair:

- `SetBrushRadius`
- `PaintRepairBoundary`
- `ClearRepairBoundary`
- `PreviewLocalWrap`
- `ApplyLocalWrap`
- `ExportRepairPatch`

For cavity workflows:

- `AnalyzeInternalCavities`
- `ShowCavity`
- `SelectCavity`
- `SealCavityFromBoundary`
- `RemoveHiddenInteriorShells`

For remeshing:

- `SetModelScale`
- `SetTargetPrinterResolution`
- `PreviewRemesh`
- `ApplyRemesh`

Each command should support:

- state report output,
- screenshot output,
- undo where appropriate,
- export/reload validation.

### Acceptance Criteria

- No new major repair UI feature is merged without at least one scenario.
- Scenario output proves before/preview/after states.
- CLI can reproduce the workflow without manual UI interaction.

## Phase 10: Documentation

### User Documentation

Document:

- `meshmend render`
- `meshmend scenario`
- `meshmend control`
- screenshot outputs,
- state report schema,
- scenario examples,
- common debugging workflows.

### Developer Documentation

Document rules for adding new commands:

1. Add the command enum variant.
2. Add command execution.
3. Add UI wiring.
4. Add scenario parsing.
5. Add state report fields if needed.
6. Add tests.
7. Add one scenario if it is user-visible.

### Acceptance Criteria

- A future agent can add a new tool without guessing the command architecture.
- The repo explains how to run scripted visual tests.

## Proposed File Structure

The final structure can evolve, but the target should be close to:

```text
apps/meshmend/src/
  main.rs
  app.rs
  icons.rs
  input.rs
  session.rs
  commands.rs
  scenario.rs
  control.rs
  render_script.rs

tests/scenarios/
  cube-view-line-cut.json
  cube-two-cuts.json
  cube-delete-object.json
  rose-load-render.json

outputs/
  scenario-.../
```

If `session.rs` becomes too large:

```text
apps/meshmend/src/session/
  mod.rs
  document.rs
  executor.rs
  snapshot.rs
```

## Implementation Order

1. Add command and snapshot types without changing UI behavior.
2. Extract mesh document operations into `MeshSession`.
3. Add deterministic render CLI with fixed camera/view/viewport.
4. Add scenario runner for load/view/screenshot.
5. Add cut preview/apply to scenario runner.
6. Add object select/hide/delete/keep/export to scenario runner.
7. Move UI actions to call shared commands.
8. Add scenario smoke tests to `just verify`.
9. Add optional live local control socket.
10. Expand scenario coverage as repair tools are added.

This order matters because batch scenarios will give most of the testing value before the live socket exists.

## Success Criteria

This work is complete when:

- Any major UI action has a matching command.
- A scripted scenario can load an STL, position the camera, preview a cut, apply it, select an object, delete or keep it, export the result, and save screenshots at each stage.
- The same command executor is used by UI and scenarios.
- A hidden renderer can rasterize the same viewport state into a PNG.
- `just verify` includes at least one scenario smoke test.
- Optional live control can drive a running app through a local Unix socket.

## Practical First Scenario

The first scenario should be cube-based because it is tracked and deterministic:

```json
{
  "name": "cube view line cut",
  "input": "fixtures/stl/cube_binary.stl",
  "viewport": {
    "width": 1280,
    "height": 800
  },
  "steps": [
    {
      "type": "fit-camera"
    },
    {
      "type": "set-view-mode",
      "mode": "surface-wire"
    },
    {
      "type": "screenshot",
      "path": "01-loaded.png"
    },
    {
      "type": "set-tool",
      "tool": "cut"
    },
    {
      "type": "preview-view-line-cut",
      "start": [500, 120],
      "end": [780, 680]
    },
    {
      "type": "screenshot",
      "path": "02-preview.png"
    },
    {
      "type": "apply-cut"
    },
    {
      "type": "state-report",
      "path": "03-state.json"
    },
    {
      "type": "screenshot",
      "path": "03-applied.png"
    },
    {
      "type": "select-object",
      "index": 0
    },
    {
      "type": "delete-selected-object"
    },
    {
      "type": "export-visible",
      "path": "cube-cut-visible.stl"
    }
  ],
  "assertions": [
    {
      "type": "object-count-at-least",
      "count": 2
    },
    {
      "type": "screenshot-nonblank",
      "path": "02-preview.png"
    },
    {
      "type": "export-reloads",
      "path": "cube-cut-visible.stl"
    }
  ]
}
```

## Notes on Existing App Patterns

The current app already proves that this direction is realistic:

- The renderer can run hidden and create screenshots.
- The CLI already runs geometry operations.
- `just repair-smoke` already validates exported STL files.
- Cut operations already return structured counts such as loops and cap triangles.

The main work is not inventing new geometry. The main work is separating command semantics from UI event handling and making the renderer scriptable.

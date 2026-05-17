# MeshMend Reset Plan

## Purpose

MeshMend has become too broad too quickly. The next implementation pass must
reduce the app back to a simple, reliable STL viewer before any repair,
selection, brush, remesh, analysis, or worker features are exposed again.

The active product goal is:

- open the native app quickly
- load a binary STL from disk
- save or export the current STL, even when unmodified
- orbit, pan, and zoom smoothly
- switch between clear view modes from a top toolbar
- show simple status information in a bottom status bar
- keep the UI modern, simple, and consistent

Everything else is out of scope until this viewer foundation is boring and
reliable.

## Non-Goals For This Reset

Remove or hide these from the active UI and interaction model:

- repair brush
- defect recording
- issue/session editing
- analysis tools
- cross-section controls
- X-ray hit-stack selection
- hole fill
- local wrap
- remesh
- cut/cap
- measure/scale
- worker launch buttons
- right inspection/repair panel
- operation history UI
- project revision UI

The underlying crates and C++ workers may remain if they are isolated and not
reachable from the viewer UI. Code that keeps being accidentally picked up by
the app should be deleted or moved behind an inactive boundary.

## Target App Shape

The reset app has three visible zones.

1. System-style top menu

Use normal app commands rather than custom repair-workflow buttons:

- File
- Open STL
- Save
- Save As / Export STL
- Quit
- View
- Reset View
- Frame Mesh
- Shortcuts

The first pass can implement this as an egui menu bar. Native macOS menu
integration can come later.

2. Top view toolbar

The view toolbar is the main control surface. It should use larger,
easy-to-read buttons with a consistent icon set and short labels.

Required initial view modes:

- Rendered
- Wireframe
- Surface Wire
- X-Ray Wire
- Transparent
- Normals
- Studio / Matcap-style lighting
- Headlight

Candidate diagnostic view modes after the reset is stable:

- Height Map: color by axis/elevation.
- Radial Distance: color by distance from model center.
- Normal Variation: color by local normal/angle change. This requires
  adjacency data, so it should not block the reset.
- Overdraw / Backface View: emphasize overlapping or back-facing surfaces.

3. Bottom status bar

The status bar should be the only always-visible information surface:

- file name
- triangle count
- GPU/backend summary or compact renderer status
- FPS or frame time
- current view mode
- short transient status messages

No right panel should be visible in this reset.

## Icon Direction

Use one consistent visual language across the app:

- modern monoline icons
- 20-24 px base size
- 1.75-2.0 px stroke
- rounded caps and joins
- no mixed filled/outlined styles
- no decorative gradients
- labels beside or below icons where ambiguity is likely

Implementation preference:

- First pass: keep a small internal icon module that draws egui vector icons.
- Use a Lucide-style vocabulary without adding a large UI framework.
- If an icon crate is introduced later, replace the internal drawings in one
  place rather than scattering icon code through the UI.

Initial icon mapping:

- Open: folder-open
- Save: save/disk
- Export: arrow-out-to-line or box-arrow-up
- Rendered: shaded cube
- Wireframe: cube wire
- Surface Wire: cube with surface grid
- X-Ray Wire: transparent cube with internal dashed line
- Transparent: overlapping translucent squares/cube
- Normals: axis/normal rays from a surface
- Studio: sparkle/light rig around cube
- Headlight: lamp/camera light
- Frame/Fit: focus corners
- Reset View: rotate-ccw
- Shortcuts: keyboard

All icon drawing should live in one file or module. Toolbars should reference
semantic icon names, not custom drawing per button.

## Keyboard And Shortcuts

Keep shortcut behavior simple and predictable.

Initial defaults:

- `O`: Open STL
- `Cmd+S`: Save
- `Cmd+Shift+S`: Save As / Export STL
- `F`: frame mesh
- `Home`: reset view
- left drag: orbit
- `Shift` + left drag: pan
- middle drag: pan
- right drag: pan
- mouse wheel / trackpad scroll: zoom

View shortcuts should be visible in tooltips and eventually editable:

- `Z`: open a small view-mode switcher, inspired by Blender
- `1`: Rendered
- `2`: Wireframe
- `3`: Surface Wire
- `4`: X-Ray Wire
- `5`: Transparent
- `N`: Normals

Do not build the editable shortcut system during the reset unless the viewer is
already stable. For now, centralize shortcut definitions so future editing is
possible.

## Implementation Phases

### Phase 1: Remove Active Feature Surface

Goal: the visible app becomes a viewer, not a repair workstation.

Tasks:

- Remove the right side panel from `draw_ui`.
- Remove the vertical tool palette from `draw_ui`.
- Remove active UI entry points for Analyze, Cross Section, X-Ray Inspect,
  Repair Brush, Hole Fill, Cut, Measure, Remesh, Export panel, operation
  history, and project controls.
- Keep only menu commands, view toolbar, viewport, and status bar.
- Ensure no mouse path starts brush strokes or defect recording.
- Ensure no worker operation can be launched from the viewer UI.

Verification:

- `just lint`
- `just build`
- `just smoke`
- manually run `just run` and confirm no right panel appears.

### Phase 2: Simplify App State

Goal: reduce the state that the UI actively carries.

Tasks:

- Remove active `ToolMode` usage from the UI.
- Keep a small `ViewMode` state.
- Keep `ModelInfo`, current STL path, current status message, and renderer
  settings.
- Keep camera input state.
- Remove issue-session mutation paths from the app.
- Remove repair/project operation action variants from `UiAction`.
- Keep save/export actions only where needed for STL copy/export.

Important boundary:

- `meshmend-project`, `meshmend-inspection`, `meshmend-analysis`, and
  `meshmend-worker-api` can remain in the workspace, but the simple viewer app
  should not import them unless still needed for a visible viewer feature.

Verification:

- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`

### Phase 3: File Menu And Save/Export

Goal: file operations feel like normal desktop app operations.

Tasks:

- Add a top menu bar with `File`, `View`, and `Help` or `Shortcuts`.
- Implement `Open STL` with the existing native file picker.
- Implement `Save`:
  - If the current mesh came from disk and is unmodified, saving can simply
    report that the file is already saved.
  - If future modifications exist, save should write over the current output
    target.
- Implement `Save As / Export STL`:
  - For now, copy the current STL to the chosen destination and validate it by
    reloading it.
- Keep source overwrite protection when exporting to a different path.

Verification:

- Open fixture STL.
- Export it to `outputs/`.
- Reload exported STL.
- Confirm triangle count remains identical.

### Phase 4: View Toolbar

Goal: view switching is the main app interaction after loading.

Tasks:

- Create a reusable toolbar button component.
- Create a central icon module.
- Build view buttons with icon, label, tooltip, and shortcut.
- Make buttons large enough to read and click comfortably.
- Keep all view settings in one mapping function from `ViewMode` to
  `DisplaySettings`.
- Remove confusing duplicate view controls.

Initial toolbar order:

1. Rendered
2. Wireframe
3. Surface Wire
4. X-Ray Wire
5. Transparent
6. Normals
7. Studio
8. Headlight

Verification:

- `just verify`
- manual screenshot pass on cube and rose.

### Phase 5: Camera And Interaction Polish

Goal: viewing feels stable before any tools return.

Tasks:

- Keep orbit, pan, and zoom behavior.
- Confirm `Shift` + left drag pans reliably.
- Confirm right/middle drag pan.
- Confirm trackpad/mouse wheel zoom does not invert unexpectedly.
- Add `Frame Mesh` and `Reset View` toolbar/menu commands.
- Ensure egui controls do not leak clicks into the viewport.

Verification:

- Manual run with `rose/raw.stl`.
- Smoke check with `--smoke-window`.
- Existing camera tests.

### Phase 6: Status Bar And Metrics

Goal: replace the old right panel with compact, useful status.

Tasks:

- Show file name.
- Show triangle count.
- Show current view mode.
- Show renderer backend.
- Add frame timing/FPS.
- Show transient status messages from load/save/export.

Keep model stats collapsed out of sight for now. If deeper stats are needed
later, add a temporary menu dialog, not a persistent right panel.

Verification:

- Load cube fixture.
- Load rose.
- Confirm status bar updates correctly.

### Phase 7: Remove Or Fence Dormant Complexity

Goal: prevent future agents from accidentally continuing repair work.

Tasks:

- Remove unused imports from the app crate.
- Move large dormant repair UI functions out of `app.rs` or delete them.
- If keeping repair/worker code, ensure it is reachable only from CLI commands
  or isolated crates, not from viewer UI.
- Update README and AGENTS to describe the reset scope.
- Keep architecture docs only if they describe current code accurately. Delete
  or rewrite stale repair-first docs.

Verification:

- `rg` for removed UI labels such as `Repair Brush`, `Hole Fill`, `Remesh`,
  `Defects`, `Operation History`.
- Confirm those labels do not appear in active app UI code.
- `just lint`
- `just test`
- `just verify`

## Expected End State

At the end of this reset, MeshMend should feel like a small native STL viewer:

- open STL
- save/export STL
- orbit/pan/zoom
- fit/reset camera
- switch modern view modes from a top toolbar
- read basic status from the bottom bar

No repair workflow should be visible. No annotation workflow should be visible.
No right panel should exist. Any future repair work must be reintroduced one
tool at a time after the viewer is stable.

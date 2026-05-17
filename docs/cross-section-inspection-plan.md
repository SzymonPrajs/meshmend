# Cross-Section Inspection Plan

## Purpose

MeshMend needs a way to see internal gaps, tunnels, cavities, overlapping
sheets, and other hidden defects before any repair workflow can be credible.
The next product step is a cross-section inspection tool that cuts the rendered
view with a movable plane. This is a viewing tool only: it does not export
slices, modify geometry, repair meshes, or classify defects automatically.

## Target Experience

Replace the current right-side Notes panel with an Inspection panel.

The Inspection panel should contain:

- Cross-section enable toggle.
- Axis selector for X, Y, or Z.
- Offset slider bounded by the loaded model bounds on that axis.
- Numeric offset display in model units.
- Flip-side toggle to inspect either side of the plane.
- Reset-to-center action.
- Issue list for defects found during inspection.

When cross-section mode is enabled, MeshMend should render only one side of the
active plane and draw a visible plane guide through the model. The user should
be able to orbit, pan, zoom, fit, and select while moving the plane.

## Product Language

Use "Cross Section" or "Inspection" in the UI. Avoid "slice" as the primary
term because this feature is not a slicer and does not generate printable layer
output.

## First Milestone

The first cross-section milestone is complete when:

- The right panel is an Inspection panel rather than a Notes panel.
- A loaded STL can be clipped by an X, Y, or Z plane.
- The plane offset can be dragged interactively across the model bounds.
- The visible clipped side can be flipped.
- The active plane is shown clearly enough to understand where the model is cut.
- Picking respects the clipped view, so hidden triangles are not selectable.
- Camera controls remain smooth while cross-section mode is active.
- The issue list can record manually identified defects against a selected
  point.
- The issue list displays each recorded issue with kind, position, and actions
  to frame or delete it.

## Issue List

The issue list replaces the old note workflow in the right panel. The first
version should stay manual and simple: the app helps the user record things
they can now see, but it does not claim to detect defects by itself.

Initial issue kinds:

- Internal gap
- Tunnel or cavity
- Open boundary
- Overlapping sheet
- Detached shell
- Thin or fragile area
- Other

Issue records should include:

- stable ID
- issue kind
- selected triangle ID
- model-space position
- active cross-section axis and offset when recorded
- short label
- optional status, starting with `open`

The current `meshmend-notes` crate can either be renamed later or replaced with
an inspection-session crate. The important boundary is that the UI should stop
presenting this as generic notes and start presenting it as mesh-inspection
issues.

## Implementation Stages

### 1. State Model

Add cross-section state to the app and renderer boundary:

- enabled
- axis: X, Y, or Z
- offset in model units
- flip side
- show plane guide

Initialize offset from the loaded model bounds center. Clamp offset to the model
bounds whenever a model is loaded or the axis changes.

### 2. Right Panel Replacement

Replace `egui::SidePanel::right("notes_panel")` in
`apps/meshmend/src/app.rs` with an Inspection panel.

Controls:

- segmented or radio-style axis selector
- bounded `egui::Slider` for offset
- flip-side checkbox
- reset-to-center button
- "Add Issue" action enabled only when a visible triangle is selected
- issue kind selector
- issue list with frame/delete actions

Keep the left model panel for model stats, GPU info, and current selected point.

### 3. Renderer Clipping

Pass the active plane to `crates/meshmend-render`.

Preferred first implementation:

- extend the existing camera/display uniform or add a small scene uniform
- encode plane normal and signed offset
- discard fragments on the hidden side in the mesh shader
- apply the same plane test in the picking shader so picking matches the view

This first pass does not need to cap cut surfaces. Seeing the exposed interior
and hidden geometry boundaries is more important than producing a closed visual
cross-section.

### 4. Plane Guide

Draw a translucent rectangular outline or lightweight guide plane at the active
offset. The guide should use model bounds to size itself and should be axis
colored:

- X: red
- Y: green
- Z: blue

If a filled translucent plane is too intrusive in the first pass, start with an
outline plus center line.

### 5. Issue Session

Replace generic note data with issue-oriented data. Keep save/load support only
if it remains cheap; otherwise make persistence a follow-up after the visible
inspection workflow works.

Migration is not required for old note files unless they are still useful. This
app is still pre-release and the right panel is changing purpose.

### 6. Verification

Add checks that prove the cross-section path works:

- unit tests for axis/offset clamping and plane equation construction
- renderer verification with cross-section enabled on the cube fixture
- picking test or smoke path that confirms a hidden-side triangle cannot be
  selected
- screenshot output for at least one axis and offset

Manual local check:

```bash
npm run dev
```

Then load `rose/raw.stl`, enable Cross Section, move X/Y/Z planes through the
rose, and record several visible internal issues.

## Out Of Scope

- Automatic defect detection.
- Mesh repair.
- Boolean cutting or geometry generation.
- Printable slicing/layer export.
- Cross-section capping surfaces.
- Multi-plane clipping.
- ROI tools.
- Defect severity scoring.

These can come later after single-plane inspection is usable.

# Mesh Repair Research Log

## Problem Statement

The rose source is visually plausible but structurally unreliable. The repair
target is not resizing, simplification, or making a low-poly model. The target is
to preserve the first visible exterior surface while producing a printable solid
that removes bad internal geometry, accidental tunnels, open boundary holes,
self-intersections, detached shells, and narrow-access AI artifacts.

The primary test case is:

```text
rose/raw.stl
```

Baseline topology from `uv run resinmesh inspect rose/raw.stl --dimension shortest --size-mm 10`:

```text
faces: 1,949,244
vertices: 974,605
boundary edges: 8
non-manifold edges: 6
non-manifold vertices: 7
two-manifold: false
```

## Defect Classes

Do not use one repair operation for all visible openings. The same visible
screen-space "hole" can be one of several different defects:

- Open boundary hole: missing patch in the triangle mesh; boundary edges exist.
- Watertight tunnel: visible passage through the model, but no boundary edges.
- Narrow-access internal cavity: a small throat opens into a large amount of
  generated internal geometry.
- Self-intersection or overlapping sheets: surfaces cross or occupy nearly the
  same region.
- Detached shell/component: a separate closed component, often tiny or internal.
- Sub-resolution texture: geometry below the intended resin-print resolution.

## Sources Reviewed

### CGAL Alpha Wrapping

Source: https://cgal.geometryfactory.com/CGAL/doc/main/Alpha_wrap_3/index.html

CGAL's Alpha Wrapping package is directly relevant because it is designed to
create valid, watertight, intersection-free, orientable 2-manifold enclosing
surfaces from triangle soups and defective meshes. The documentation explicitly
mentions gaps, self-intersections, degeneracies, non-manifold features,
unnecessary inner components, and fine cavities.

Key decision:

- Alpha wrapping is a candidate repair generator, not a validator.
- `alpha` controls which holes/straits the wrap can traverse.
- `offset` controls tightness.
- The documentation warns about two-sided wraps when alpha is small relative to
  holes, so any alpha-wrap sweep must be scored with visual/depth validation.

### CGAL Polygon Mesh Processing

Source: https://doc.cgal.org/latest/Polygon_mesh_processing/index.html

CGAL Polygon Mesh Processing provides the conceptual checklist for true mesh
repair: hole filling, stitching, orientation, self-intersection detection, and
repair of polygon soups. This is useful for classifying defects. It is not by
itself enough for AI organic models, because fixing individual topological
defects can preserve semantically bad internal geometry.

Key decision:

- Use hole filling only for true boundary holes.
- Do not expect hole filling to fix watertight tunnels or internal chambers.

### Open3D Raycasting and Distance Queries

Source: https://www.open3d.org/docs/release/tutorial/geometry/distance_queries.html

Open3D's `RaycastingScene` supports raycasting and distance queries. The docs
state that unsigned distance can be computed generally, while signed distance
and occupancy require a watertight mesh with a clear inside/outside.

Key decision:

- Use raycasting on raw broken models for validation and depth-complexity maps.
- Use signed distance/occupancy only after a candidate is watertight.

### libigl Winding Numbers

Source: https://libigl.github.io/tutorial/

libigl documents generalized winding numbers as a robust way to determine
inside/outside for triangle soups, and includes self-intersection and distance
query concepts. This is relevant to a future solid-extraction method, especially
if alpha wrapping and voxel repair are insufficient.

Key decision:

- Treat winding-number reconstruction as a high-value research branch, but not
  the first implementation because Python integration is less direct here.

### Trimesh Repair and Voxel Helpers

Sources:

- https://trimesh.org/trimesh.repair.html
- https://trimesh.org/trimesh.voxel.html

Trimesh provides STL loading, basic repair helpers, ray helpers, and
voxelization utilities.

Key decision:

- Use Trimesh for synthetic tests, component logic, ray helpers where useful,
  and voxel experiments.

### Manifold3D

Source: https://pypi.org/project/manifold3d/

Manifold3D is relevant for robust manifold/boolean experiments and for SDF level
set style construction. It expects or constructs valid manifold solids, so it is
more likely to be useful after an initial exterior reconstruction or for
synthetic tests.

Key decision:

- Keep Manifold3D as a candidate backend for later repair generation, not as the
  initial diagnostic backend.

### OpenVDB Mesh To Volume

Source: https://www.openvdb.org/documentation/doxygen/MeshToVolume_8h.html

OpenVDB mesh-to-volume is the reference family for converting mesh geometry to a
level set or volume. The core idea is relevant even if we do not use OpenVDB
directly from Python: mesh to volume, exterior flood fill, morphological closing,
then surface extraction.

Key decision:

- Implement a Python approximation with Open3D/Trimesh/scikit-image first.
- Consider OpenVDB later if Python voxel experiments are too slow or too crude.

### Blender 3D Print Toolbox

Source: https://docs.blender.org/manual/en/latest/addons/mesh/3d_print_toolbox.html

Blender's 3D Print Toolbox is useful as a validation checklist: non-manifold
geometry, intersecting geometry, thickness, overhangs, and other printability
concerns. Blender is not currently available on PATH in this workspace, so the
initial automation should not depend on it.

Key decision:

- Mirror the relevant checks in Python first.
- Add Blender integration later only if it becomes available and useful.

### Tripo / AI-Generated Mesh Failure Notes

Sources:

- https://www.tripo3d.ai/blog/explore/how-to-fix-non-manifold-geometry-from-ai-outputs
- https://www.tripo3d.ai/3d-print/ai-generated-3d-models-repairing-holes-for-3d-printing

These sources are partly vendor material, so treat them as context rather than
independent validation. They are still useful because they explicitly name the
same defect classes seen here: non-manifold geometry, naked edges, internal
faces, floating vertices, and visually plausible outputs that are not physically
valid for printing.

Key decision:

- Do not trust an AI model because it looks correct in Blender.
- Repair should be driven by measured topology, multi-view visual comparison,
  and volumetric/accessibility checks.

## Current Automated Diagnostic Method

Implemented command:

```bash
uv run resinmesh diagnose rose/raw.stl --views 16 --image-size 128 --max-hits 8 --output-dir rose/experiments/diagnose_raw_baseline
```

The command writes:

- `diagnostics.json`
- `diagnostics.md`
- `contact_sheet.png`
- per-view mask, depth, normal, and hit-count images

Current baseline ray metrics:

```text
multi-hit fraction: 0.3374
three-plus-hit fraction: 0.1614
max hit count: 8
```

Interpretation:

- The raw model has true open boundary holes and non-manifold defects.
- A large fraction of exterior rays hit multiple surfaces.
- This is expected for a rose to some extent because petals overlap, but the
  three-plus-hit fraction is high enough to justify depth-complexity scoring in
  any repair loop.

Visual inspection of `contact_sheet.png`:

- The normal/depth views preserve the expected rose silhouette and petal layout.
- Hit-count heat maps show concentrated high-depth regions through the rose head
  and petal folds.
- Some high depth complexity is legitimate petal layering; localized validation
  is needed before declaring it bad internal geometry.

## Implemented Tool Suite

Implemented commands:

```bash
uv run resinmesh inspect rose/raw.stl --dimension shortest --size-mm 10
uv run resinmesh diagnose rose/raw.stl --views 16 --image-size 128 --max-hits 8 --output-dir rose/experiments/diagnose_raw_baseline
uv run resinmesh repair-sweep rose/raw.stl --output-dir rose/experiments/<run>
uv run resinmesh validate rose/raw.stl <candidate.stl> --output-dir rose/experiments/<run>/validation
uv run resinmesh voxel-audit <candidate.stl> --output-dir rose/experiments/<run>/voxel
```

The commands produce JSON, Markdown, and PNG contact sheets. Generated files
belong under `rose/experiments/`.

Validation signals:

- MeshLab topology: boundary edges, non-manifold edges/vertices, components.
- MeshLab self-intersection face selection where requested.
- Open3D multi-view raycasting: silhouette overlap, depth delta, normal/depth
  sheets, repeated ray-hit depth complexity.
- Voxel occupancy on watertight candidates: exterior flood fill, sealed voids,
  and morphological throat-closing checks.

## Rose Case Study Results

### Alpha Wrap Initial Sweep

Command:

```bash
uv run resinmesh repair-sweep rose/raw.stl \
  --output-dir rose/experiments/20260516_alpha_initial \
  --alpha-values-mm 0.04,0.05,0.075 \
  --offset-factors 0.20 \
  --no-hybrid \
  --validate-views 8 \
  --validate-image-size 96 \
  --validate-max-hits 8 \
  --diagnose-views 8 \
  --diagnose-image-size 96 \
  --diagnose-max-hits 8
```

Results:

| Candidate | Topology | Components | Silhouette IoU | Three-plus hits | Notes |
|---|---|---:|---:|---:|---|
| `alpha_0p04mm_offset_0p2x.stl` | pass | 1 | 0.9941 | 0.1587 | Best visual preservation. |
| `alpha_0p05mm_offset_0p2x.stl` | pass | 1 | 0.9927 | 0.1567 | Middle ground. |
| `alpha_0p075mm_offset_0p2x.stl` | pass | 1 | 0.9894 | 0.1510 | Stronger repair, more visual change. |
| `close_boundary_holes.stl` | fail | 1 | 1.0000 | 0.1661 | Closes holes but leaves non-manifold vertices; not sufficient. |

Higher-resolution validation confirmed the tradeoff:

```text
alpha_0p04:  IoU 0.9943, depth delta/diagonal 0.001194, three-plus delta -0.0071
alpha_0p075: IoU 0.9895, depth delta/diagonal 0.002227, three-plus delta -0.0149
```

Self-intersection checks on both alpha candidates selected zero faces.

### Visibility-Filtered Hybrid Sweep

Command:

```bash
uv run resinmesh repair-sweep rose/raw.stl \
  --output-dir rose/experiments/20260516_hybrid_initial \
  --no-alpha \
  --no-close-holes \
  --hybrid-pixel-mm 0.04,0.06 \
  --hybrid-alpha-mm 0.05,0.075 \
  --offset-factors 0.20 \
  --hybrid-views 48 \
  --validate-views 8 \
  --validate-image-size 96 \
  --diagnose-views 8 \
  --diagnose-image-size 96
```

Result: rejected for this rose. The candidates were topologically valid, but
their three-plus-hit fractions rose to roughly `0.317`, indicating the cull-then
wrap path increased layered depth complexity.

### Voxel Accessibility Audit

Commands:

```bash
uv run resinmesh voxel-audit rose/experiments/20260516_alpha_initial/candidates/alpha_0p075mm_offset_0p2x.stl \
  --output-dir rose/experiments/20260516_alpha_initial/voxel_alpha_0p075_pitch0p05 \
  --pitch-mm 0.05 \
  --throat-mm 0.10
```

At `0.05 mm` virtual pitch and `0.10 mm` throat closing:

| Candidate | Sealed void voxels | Narrow-access void voxels |
|---|---:|---:|
| `alpha_0p04mm_offset_0p2x.stl` | 1 | 0 |
| `alpha_0p075mm_offset_0p2x.stl` | 0 | 0 |

The one-voxel sealed signal in the `0.04 mm` candidate is below practical
significance at this pitch, but the `0.075 mm` candidate is cleaner on the
volumetric audit.

## Current Recommendation

Recommended printable candidate:

```text
rose/experiments/20260516_alpha_initial/candidates/alpha_0p075mm_offset_0p2x.stl
```

Polished candidate for Blender/print review:

```text
rose/rose_repaired_printable_polished.stl
```

This is generated from the `0.075 mm` alpha-wrap candidate by Taubin smoothing,
isotropic remeshing, and a short final Taubin pass. It is intended to reduce the
alpha-wrap triangle ridges while keeping the mesh closed.

Reason:

- One connected component.
- No boundary edges.
- No non-manifold edges or vertices.
- No MeshLab-selected self-intersecting faces.
- No sealed or narrow-access voxel voids at `0.05 mm` pitch with `0.10 mm`
  throat closing.
- Still has high exterior visual similarity (`0.9895` high-resolution
  silhouette IoU).

Polished candidate checks:

- `117,592` faces and `58,798` vertices.
- One connected component.
- No boundary edges, holes, non-manifold edges, or non-manifold vertices.
- No MeshLab-selected self-intersecting faces.
- No sealed or narrow-access voxel voids at `0.05 mm` pitch with `0.10 mm`
  throat closing.
- Validation against raw: silhouette IoU `0.9923`, depth delta/diagonal
  `0.002112`, three-plus-hit delta `-0.0230`.

Conservative visual alternative:

```text
rose/experiments/20260516_alpha_initial/candidates/alpha_0p04mm_offset_0p2x.stl
```

Use this if Blender inspection shows the `0.075 mm` candidate has lost too much
petal separation. It preserves the exterior more closely but is slightly weaker
on voxel printability.

## Working Strategy

1. Build diagnostics first.
2. Score raw and all candidates with the same validation harness.
3. Only then run repair sweeps.
4. Reject repairs that look topologically valid but fail exterior preservation
   or retain suspicious depth complexity.
5. Rank multiple candidates rather than assuming one parameter is correct.

## Candidate Repair Branches

### Branch A: True Boundary Repair

Purpose: close missing boundary faces and repair non-manifold defects.

Risk: does not remove tunnels, internal cavities, or self-intersections.

### Branch B: Alpha Wrap Sweep

Purpose: generate conservative enclosing surfaces with controlled throat size.

Risk: small alpha can create two-sided wraps; large alpha can erase petal gaps.

Validation required:

- silhouette/depth preservation
- hit-count reduction
- topology validity
- local review of petal gaps

### Branch C: Exterior Visibility Extraction

Purpose: identify faces that are first-visible from outside views and suppress
surfaces that are only visible through narrow/deep paths.

Risk: concave but legitimate petal surfaces can be under-sampled.

Validation required:

- view count and image resolution sensitivity
- preservation of known exterior petal regions

### Branch D: Voxel/SDF Exterior Reconstruction

Purpose: reconstruct a printable solid from a volumetric field, using exterior
flood fill and optionally morphological closing.

Risk: voxel pitch trades detail for robustness; too coarse destroys rose petals,
too fine preserves bad geometry.

Validation required:

- physical pitch sweep relative to 20-50 um print precision at 10 mm target
- depth/silhouette comparison
- topology and component checks

### Branch E: Winding Number / Solid Extraction

Purpose: robust inside/outside classification for triangle soups and
self-intersecting surfaces.

Risk: integration and performance complexity.

Validation required:

- compare against alpha wrap and voxel approaches on synthetic defect cases.

## Next Implementation Steps

1. Add synthetic defect tests:
   - open boundary hole
   - watertight tunnel
   - internal shell
   - detached component
   - narrow-access cavity
   - overlapping/self-intersecting sheets
2. Add candidate comparison command:
   - compare raw vs candidate contact sheets
   - silhouette IoU
   - depth difference
   - hit-count reduction
   - topology score
3. Add repair sweeps only after validation is stable.

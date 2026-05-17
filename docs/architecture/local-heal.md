# Local Heal Direction

Local repair should start from painted labels, not from global mesh heuristics.

The intended workflow is:

- Paint a healthy boundary around the visible good surface that must be
  preserved.
- Paint the repair target around the internal cavity, tunnel, or damaged area.
- Optionally paint excluded nearby surface that should not be pulled into the
  solve.
- Build a local repair volume from the labeled area plus a small margin.
- Choose repair resolution from the brush radius, mesh-detail unit, and local
  triangle density.
- Reconstruct only the local target volume, then blend the patch back into the
  healthy boundary.

The label brush stores each stroke radius in model-space units. The UI radius is
scaled from the loaded mesh's average triangle edge length, so a radius of 10 is
about 10 local detail units on the current model. Local repair should use that
stored radius as the first ROI expansion distance before choosing voxel or
shrink-wrap resolution.

The first implemented local shrink-wrap path is the OpenVDB `local_sdf_wrap`
worker operation. It rebuilds a mesh through a local-detail voxel surface
extraction, and the next refinement is to constrain that operation to painted
healthy, target, and exclude regions instead of running on the whole mesh file.

Diagnostics, ROI concepts, voxel concepts, and CLI semantics from the old
Python experiments have been ported or superseded. Do not recreate a Python
repair pipeline.

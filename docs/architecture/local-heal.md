# Local Heal Direction

Local repair should start from painted labels, not from global mesh heuristics.

The intended workflow is:

- Paint a healthy boundary around the visible good surface that must be
  preserved.
- Paint the repair target around the internal cavity, tunnel, or damaged area.
- Optionally paint excluded nearby surface that should not be pulled into the
  solve.
- Build a local repair volume from the labeled area plus a small margin.
- Choose repair resolution from the brush radius and local triangle density.
- Reconstruct only the local target volume, then blend the patch back into the
  healthy boundary.

The first implementation target is a local shrink-wrap or voxel-wrap prototype
that eliminates the internal cavity without changing the full model. Global
repair, full-model remeshing, and printable slicing remain out of scope for this
native viewer path.

The archived Python pipeline may be used as reference material for diagnostics
and experiments, but it should not be revived as active product code.

"""Experiment runners for AI-mesh repair studies."""

from __future__ import annotations

from dataclasses import dataclass
import json
import time
from pathlib import Path
from typing import Any, Callable

from .core import (
    ComponentCleanConfig,
    DimensionMode,
    HoleCloseConfig,
    SmoothConfig,
    VisibilityCullConfig,
    WrapConfig,
    close_boundary_holes,
    cull_hidden_geometry,
    inspect_model,
    print_scale_for,
    remove_small_components,
    smooth_mesh,
    wrap_mesh,
)
from .diagnostics import RayDiagnosticConfig, compare_meshes, diagnose_mesh

LogFn = Callable[[str], None]


@dataclass(frozen=True)
class RepairSweepConfig:
    dimension: DimensionMode = DimensionMode.SHORTEST
    size_mm: float = 10.0
    tolerance_um: float = 20.0
    alpha_values_mm: tuple[float, ...] = (0.04, 0.05, 0.075, 0.10)
    offset_factors: tuple[float, ...] = (0.20,)
    hybrid_pixel_mm: tuple[float, ...] = (0.04, 0.06)
    hybrid_alpha_mm: tuple[float, ...] = (0.05, 0.075)
    include_close_holes: bool = True
    include_alpha: bool = True
    include_hybrid: bool = True
    close_hole_edges: int = 2000
    hybrid_views: int = 48
    hybrid_min_visible_views: int = 1
    clean_min_faces: int = 1000
    smooth_steps: int = 0
    keep_intermediates: bool = False
    validate: RayDiagnosticConfig = RayDiagnosticConfig(view_count=12, image_size=96, max_hits=8)
    diagnose: RayDiagnosticConfig = RayDiagnosticConfig(view_count=12, image_size=96, max_hits=8)


def run_repair_sweep(
    model: Path,
    output_dir: Path,
    config: RepairSweepConfig,
    *,
    log: LogFn | None = None,
) -> dict[str, Any]:
    """Run repair candidates and rank them with the validation harness."""

    started = time.time()
    output_dir.mkdir(parents=True, exist_ok=True)
    candidates_dir = output_dir / "candidates"
    diagnostics_dir = output_dir / "diagnostics"
    validations_dir = output_dir / "validations"
    intermediates_dir = output_dir / "intermediates"
    for directory in (candidates_dir, diagnostics_dir, validations_dir, intermediates_dir):
        directory.mkdir(parents=True, exist_ok=True)

    metrics = inspect_model(
        model,
        dimension=config.dimension,
        target_mm=config.size_mm,
        tolerance_um=config.tolerance_um,
    )
    scale = print_scale_for(metrics["bounds"], config.dimension, config.size_mm, config.tolerance_um)

    if log:
        log("Diagnosing raw reference")
    reference_diagnostic = diagnose_mesh(model, output_dir / "reference_diagnostics", config.diagnose)

    generated: list[dict[str, Any]] = []
    if config.include_close_holes:
        generated.append(
            _run_close_holes_candidate(
                model=model,
                output=candidates_dir / "close_boundary_holes.stl",
                scale=scale,
                max_hole_edges=config.close_hole_edges,
                log=log,
            )
        )

    if config.include_alpha:
        for alpha_mm in config.alpha_values_mm:
            for offset_factor in config.offset_factors:
                label = f"alpha_{slug_number(alpha_mm)}mm_offset_{slug_number(offset_factor)}x"
                generated.append(
                    _run_alpha_candidate(
                        model=model,
                        output=candidates_dir / f"{label}.stl",
                        scale=scale,
                        alpha_mm=alpha_mm,
                        offset_factor=offset_factor,
                        clean_min_faces=config.clean_min_faces,
                        smooth_steps=config.smooth_steps,
                        intermediates_dir=intermediates_dir / label,
                        keep_intermediates=config.keep_intermediates,
                        log=log,
                    )
                )

    if config.include_hybrid:
        for pixel_mm in config.hybrid_pixel_mm:
            for alpha_mm in config.hybrid_alpha_mm:
                for offset_factor in config.offset_factors:
                    label = (
                        f"visible_{slug_number(pixel_mm)}mm_"
                        f"alpha_{slug_number(alpha_mm)}mm_offset_{slug_number(offset_factor)}x"
                    )
                    generated.append(
                        _run_visibility_alpha_candidate(
                            model=model,
                            output=candidates_dir / f"{label}.stl",
                            scale=scale,
                            pixel_mm=pixel_mm,
                            alpha_mm=alpha_mm,
                            offset_factor=offset_factor,
                            view_count=config.hybrid_views,
                            min_visible_views=config.hybrid_min_visible_views,
                            clean_min_faces=config.clean_min_faces,
                            smooth_steps=config.smooth_steps,
                            intermediates_dir=intermediates_dir / label,
                            keep_intermediates=config.keep_intermediates,
                            log=log,
                        )
                    )

    evaluated = []
    for row in generated:
        if row.get("error"):
            evaluated.append(row)
            continue
        candidate_path = Path(row["output"])
        label = row["label"]
        if log:
            log(f"Validating {label}")
        comparison = compare_meshes(model, candidate_path, validations_dir / label, config.validate)
        diagnostic = diagnose_mesh(candidate_path, diagnostics_dir / label, config.diagnose)
        score = score_candidate(reference_diagnostic, comparison, diagnostic)
        row.update(
            {
                "score": score,
                "comparison": comparison,
                "diagnostic": diagnostic,
                "comparison_report": comparison["markdown"],
                "comparison_sheet": comparison["contact_sheet"],
                "diagnostic_report": diagnostic["markdown"],
                "diagnostic_sheet": diagnostic["contact_sheet"],
            }
        )
        evaluated.append(row)

    ranked = sorted(
        evaluated,
        key=lambda row: (
            row.get("score", {}).get("total", -1_000_000),
            -1 if row.get("error") else 0,
        ),
        reverse=True,
    )
    summary = {
        "model": str(model),
        "output_dir": str(output_dir),
        "metrics": metrics,
        "config": repair_sweep_config_to_dict(config),
        "reference_diagnostic": reference_diagnostic,
        "candidates": ranked,
        "best": ranked[0] if ranked else None,
        "seconds": time.time() - started,
    }

    summary_json = output_dir / "repair_sweep.json"
    summary_json.write_text(json.dumps(_jsonable(summary), indent=2) + "\n")
    summary_md = output_dir / "repair_sweep.md"
    summary_md.write_text(render_repair_sweep_markdown(summary) + "\n")
    summary["report"] = str(summary_json)
    summary["markdown"] = str(summary_md)

    if not config.keep_intermediates:
        _remove_empty_tree(intermediates_dir)
    return summary


def _run_close_holes_candidate(
    *,
    model: Path,
    output: Path,
    scale: Any,
    max_hole_edges: int,
    log: LogFn | None,
) -> dict[str, Any]:
    label = output.stem
    try:
        if log:
            log(f"Generating {label}")
        result = close_boundary_holes(
            model,
            HoleCloseConfig(
                scale=scale,
                max_hole_edges=max_hole_edges,
                refine=True,
                repair_non_manifold=True,
                keep_source_scale=True,
                exact_final_size=False,
            ),
            output=output,
            log=log,
        )
        return {"label": label, "method": "close_boundary_holes", "output": str(output), "generation": result}
    except Exception as exc:
        return {"label": label, "method": "close_boundary_holes", "output": str(output), "error": str(exc)}


def _run_alpha_candidate(
    *,
    model: Path,
    output: Path,
    scale: Any,
    alpha_mm: float,
    offset_factor: float,
    clean_min_faces: int,
    smooth_steps: int,
    intermediates_dir: Path,
    keep_intermediates: bool,
    log: LogFn | None,
) -> dict[str, Any]:
    label = output.stem
    intermediates_dir.mkdir(parents=True, exist_ok=True)
    wrapped = intermediates_dir / f"{label}_wrapped.stl"
    cleaned = intermediates_dir / f"{label}_cleaned.stl"
    try:
        if log:
            log(f"Generating {label}")
        wrap_result = wrap_mesh(
            model,
            WrapConfig(
                scale=scale,
                close_below_mm=alpha_mm,
                offset_mm=alpha_mm * offset_factor,
                keep_source_scale=True,
                exact_final_size=False,
            ),
            output=wrapped,
            log=log,
        )
        clean_result = remove_small_components(
            wrapped,
            ComponentCleanConfig(min_faces=clean_min_faces, keep_source_scale=True),
            output=cleaned if smooth_steps > 0 else output,
            log=log,
        )
        smooth_result = None
        if smooth_steps > 0:
            smooth_result = smooth_mesh(
                cleaned,
                SmoothConfig(
                    scale=scale,
                    steps=smooth_steps,
                    keep_source_scale=True,
                    exact_final_size=False,
                ),
                output=output,
                log=log,
            )
        if not keep_intermediates:
            _unlink_paths([wrapped, cleaned])
            _remove_empty_tree(intermediates_dir)
        return {
            "label": label,
            "method": "alpha_wrap",
            "output": str(output),
            "parameters": {
                "alpha_mm": alpha_mm,
                "offset_factor": offset_factor,
                "offset_mm": alpha_mm * offset_factor,
                "clean_min_faces": clean_min_faces,
                "smooth_steps": smooth_steps,
            },
            "generation": {
                "wrap": wrap_result,
                "clean": clean_result,
                "smooth": smooth_result,
            },
        }
    except Exception as exc:
        return {"label": label, "method": "alpha_wrap", "output": str(output), "error": str(exc)}


def _run_visibility_alpha_candidate(
    *,
    model: Path,
    output: Path,
    scale: Any,
    pixel_mm: float,
    alpha_mm: float,
    offset_factor: float,
    view_count: int,
    min_visible_views: int,
    clean_min_faces: int,
    smooth_steps: int,
    intermediates_dir: Path,
    keep_intermediates: bool,
    log: LogFn | None,
) -> dict[str, Any]:
    label = output.stem
    intermediates_dir.mkdir(parents=True, exist_ok=True)
    visible = intermediates_dir / f"{label}_visible.stl"
    wrapped = intermediates_dir / f"{label}_wrapped.stl"
    cleaned = intermediates_dir / f"{label}_cleaned.stl"
    try:
        if log:
            log(f"Generating {label}")
        cull_result = cull_hidden_geometry(
            model,
            VisibilityCullConfig(
                scale=scale,
                pixel_mm=pixel_mm,
                depth_tolerance_mm=pixel_mm * 0.5,
                view_count=view_count,
                min_visible_views=min_visible_views,
                keep_source_scale=True,
                exact_final_size=False,
            ),
            output=visible,
            log=log,
        )
        wrap_result = wrap_mesh(
            visible,
            WrapConfig(
                scale=scale,
                close_below_mm=alpha_mm,
                offset_mm=alpha_mm * offset_factor,
                keep_source_scale=True,
                exact_final_size=False,
            ),
            output=wrapped,
            log=log,
        )
        clean_result = remove_small_components(
            wrapped,
            ComponentCleanConfig(min_faces=clean_min_faces, keep_source_scale=True),
            output=cleaned if smooth_steps > 0 else output,
            log=log,
        )
        smooth_result = None
        if smooth_steps > 0:
            smooth_result = smooth_mesh(
                cleaned,
                SmoothConfig(
                    scale=scale,
                    steps=smooth_steps,
                    keep_source_scale=True,
                    exact_final_size=False,
                ),
                output=output,
                log=log,
            )
        if not keep_intermediates:
            _unlink_paths([visible, wrapped, cleaned])
            _remove_empty_tree(intermediates_dir)
        return {
            "label": label,
            "method": "visibility_alpha_wrap",
            "output": str(output),
            "parameters": {
                "pixel_mm": pixel_mm,
                "depth_tolerance_mm": pixel_mm * 0.5,
                "view_count": view_count,
                "min_visible_views": min_visible_views,
                "alpha_mm": alpha_mm,
                "offset_factor": offset_factor,
                "offset_mm": alpha_mm * offset_factor,
                "clean_min_faces": clean_min_faces,
                "smooth_steps": smooth_steps,
            },
            "generation": {
                "cull": cull_result,
                "wrap": wrap_result,
                "clean": clean_result,
                "smooth": smooth_result,
            },
        }
    except Exception as exc:
        return {"label": label, "method": "visibility_alpha_wrap", "output": str(output), "error": str(exc)}


def score_candidate(
    reference_diagnostic: dict[str, Any],
    comparison: dict[str, Any],
    diagnostic: dict[str, Any],
) -> dict[str, Any]:
    """Score a candidate. Higher is better; components explain the penalty."""

    candidate_topology = comparison["candidate_mesh"].get("topology") or {}
    compare = comparison["comparison"]["aggregate"]
    raw_ray = reference_diagnostic["visibility"]["aggregate"]
    candidate_ray = diagnostic["visibility"]["aggregate"]

    boundary_edges = int(candidate_topology.get("boundary_edges") or 0)
    non_manifold_edges = int(candidate_topology.get("non_two_manifold_edges") or 0)
    non_manifold_vertices = int(candidate_topology.get("non_two_manifold_vertices") or 0)
    components = int(candidate_topology.get("connected_components_number") or 0)
    topology_ok = boundary_edges == 0 and non_manifold_edges == 0 and non_manifold_vertices == 0
    component_ok = components in (0, 1)

    silhouette_iou = float(compare.get("mean_silhouette_iou") or 0.0)
    reference_only = float(compare.get("mean_reference_only_fraction") or 0.0)
    candidate_only = float(compare.get("mean_candidate_only_fraction") or 0.0)
    depth_delta = float(compare.get("mean_abs_depth_delta_fraction_diagonal") or 0.0)
    raw_three_plus = float(raw_ray.get("three_plus_hit_fraction") or 0.0)
    candidate_three_plus = float(candidate_ray.get("three_plus_hit_fraction") or 0.0)
    complexity_reduction = max(raw_three_plus - candidate_three_plus, 0.0)
    complexity_increase = max(candidate_three_plus - raw_three_plus, 0.0)

    total = 100.0
    penalties: dict[str, float] = {}
    bonuses: dict[str, float] = {}
    if not topology_ok:
        penalties["topology_defects"] = 35.0
    if not component_ok:
        penalties["extra_components"] = min(20.0, max(components - 1, 0) * 4.0)
    penalties["silhouette_loss"] = min(60.0, (1.0 - silhouette_iou) * 260.0)
    penalties["reference_surface_loss"] = min(45.0, reference_only * 180.0)
    penalties["candidate_extra_surface"] = min(25.0, candidate_only * 100.0)
    penalties["depth_shift"] = min(30.0, depth_delta * 220.0)
    penalties["new_depth_complexity"] = min(35.0, complexity_increase * 140.0)
    bonuses["depth_complexity_reduction"] = min(20.0, complexity_reduction * 120.0)

    total -= sum(penalties.values())
    total += sum(bonuses.values())
    return {
        "total": round(total, 3),
        "topology_ok": topology_ok,
        "component_ok": component_ok,
        "boundary_edges": boundary_edges,
        "non_manifold_edges": non_manifold_edges,
        "non_manifold_vertices": non_manifold_vertices,
        "components": components,
        "silhouette_iou": silhouette_iou,
        "reference_only_fraction": reference_only,
        "candidate_only_fraction": candidate_only,
        "depth_delta_fraction_diagonal": depth_delta,
        "raw_three_plus_fraction": raw_three_plus,
        "candidate_three_plus_fraction": candidate_three_plus,
        "three_plus_reduction": complexity_reduction,
        "penalties": penalties,
        "bonuses": bonuses,
    }


def render_repair_sweep_markdown(summary: dict[str, Any]) -> str:
    lines = [
        "# Repair Sweep",
        "",
        f"Model: `{summary['model']}`",
        f"Output directory: `{summary['output_dir']}`",
        "",
        "## Ranking",
        "",
        "| Rank | Score | Candidate | Method | Topology | Components | Silhouette IoU | Three-plus hits | Output |",
        "|---:|---:|---|---|---|---:|---:|---:|---|",
    ]
    for rank, row in enumerate(summary.get("candidates", []), start=1):
        if row.get("error"):
            lines.append(
                f"| {rank} | error | `{row['label']}` | `{row['method']}` | error | | | | `{row.get('error')}` |"
            )
            continue
        score = row["score"]
        topology = "ok" if score["topology_ok"] else "fail"
        lines.append(
            "| "
            f"{rank} | "
            f"{score['total']:.3f} | "
            f"`{row['label']}` | "
            f"`{row['method']}` | "
            f"{topology} | "
            f"{score['components']} | "
            f"{score['silhouette_iou']:.4f} | "
            f"{score['candidate_three_plus_fraction']:.4f} | "
            f"`{row['output']}` |"
        )
    lines.extend(["", "## Candidate Details", ""])
    for row in summary.get("candidates", []):
        lines.append(f"### {row['label']}")
        lines.append("")
        if row.get("error"):
            lines.append(f"- Error: `{row['error']}`")
            lines.append("")
            continue
        score = row["score"]
        lines.extend(
            [
                f"- Output: `{row['output']}`",
                f"- Comparison sheet: `{row['comparison_sheet']}`",
                f"- Diagnostic sheet: `{row['diagnostic_sheet']}`",
                f"- Topology OK: `{score['topology_ok']}`",
                f"- Boundary edges: `{score['boundary_edges']}`",
                f"- Non-manifold edges: `{score['non_manifold_edges']}`",
                f"- Non-manifold vertices: `{score['non_manifold_vertices']}`",
                f"- Components: `{score['components']}`",
                f"- Silhouette IoU: `{score['silhouette_iou']:.4f}`",
                f"- Reference-only surface fraction: `{score['reference_only_fraction']:.4f}`",
                f"- Candidate-only surface fraction: `{score['candidate_only_fraction']:.4f}`",
                f"- Depth delta / diagonal: `{score['depth_delta_fraction_diagonal']:.6f}`",
                f"- Candidate three-plus-hit fraction: `{score['candidate_three_plus_fraction']:.4f}`",
                f"- Three-plus-hit reduction vs raw: `{score['three_plus_reduction']:.4f}`",
                "",
            ]
        )
    lines.extend(
        [
            "## Notes",
            "",
            "- Scores are decision aids, not proof of semantic correctness.",
            "- Topology and boundary checks are strong automated signals.",
            "- Silhouette/depth/contact sheets still need human review for petal identity and rose character.",
        ]
    )
    return "\n".join(lines)


def repair_sweep_config_to_dict(config: RepairSweepConfig) -> dict[str, Any]:
    return {
        "dimension": config.dimension.value,
        "size_mm": config.size_mm,
        "tolerance_um": config.tolerance_um,
        "alpha_values_mm": list(config.alpha_values_mm),
        "offset_factors": list(config.offset_factors),
        "hybrid_pixel_mm": list(config.hybrid_pixel_mm),
        "hybrid_alpha_mm": list(config.hybrid_alpha_mm),
        "include_close_holes": config.include_close_holes,
        "include_alpha": config.include_alpha,
        "include_hybrid": config.include_hybrid,
        "close_hole_edges": config.close_hole_edges,
        "hybrid_views": config.hybrid_views,
        "hybrid_min_visible_views": config.hybrid_min_visible_views,
        "clean_min_faces": config.clean_min_faces,
        "smooth_steps": config.smooth_steps,
        "keep_intermediates": config.keep_intermediates,
        "validate": config.validate.__dict__,
        "diagnose": config.diagnose.__dict__,
    }


def slug_number(value: float) -> str:
    return f"{value:g}".replace("-", "m").replace(".", "p")


def _unlink_paths(paths: list[Path]) -> None:
    for path in paths:
        path.unlink(missing_ok=True)


def _remove_empty_tree(path: Path) -> None:
    if not path.exists():
        return
    for child in sorted(path.rglob("*"), reverse=True):
        if child.is_dir():
            try:
                child.rmdir()
            except OSError:
                pass
    try:
        path.rmdir()
    except OSError:
        pass


def _jsonable(value: Any) -> Any:
    if isinstance(value, dict):
        return {str(key): _jsonable(item) for key, item in value.items()}
    if isinstance(value, list):
        return [_jsonable(item) for item in value]
    if isinstance(value, tuple):
        return [_jsonable(item) for item in value]
    if isinstance(value, Path):
        return str(value)
    if hasattr(value, "item"):
        return value.item()
    return value

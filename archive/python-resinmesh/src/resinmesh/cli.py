"""Typer CLI for adaptive resin-print mesh simplification."""

from __future__ import annotations

from pathlib import Path
from datetime import datetime
from typing import Optional

import typer
from rich.console import Console
from rich.table import Table

from .core import (
    ComponentCleanConfig,
    DimensionMode,
    HoleCloseConfig,
    PolishConfig,
    SimplifyConfig,
    SmoothConfig,
    VisibilityCullConfig,
    WrapConfig,
    close_boundary_holes,
    cull_hidden_geometry,
    default_output_path,
    diagnostic_prefix_for,
    fail_diagnostic_prefix_for,
    inspect_model,
    optimize_face_count,
    polish_mesh,
    print_scale_for,
    report_path_for,
    remove_small_components,
    simplify_candidate,
    smooth_mesh,
    topology_for_path,
    wrap_mesh,
    write_json,
)
from .diagnostics import RayDiagnosticConfig, compare_meshes, diagnose_mesh
from .experiments import RepairSweepConfig, run_repair_sweep
from .roi import RoiProbeConfig, probe_roi, render_roi_views
from .roi_3d_ui import launch_roi_3d_ui
from .roi_ui import launch_roi_ui
from .voxel import VoxelAuditConfig, voxel_audit_mesh

app = typer.Typer(
    help="Mesh diagnostics and repair tools for resin-printable AI meshes.",
    no_args_is_help=True,
)
console = Console()


def _echo(message: str) -> None:
    console.print(message)


def _build_scale(
    model: Path,
    dimension: DimensionMode,
    size_mm: float,
    tolerance_um: float,
):
    metrics = inspect_model(model, dimension=dimension, target_mm=size_mm, tolerance_um=tolerance_um)
    bounds = metrics["bounds"]
    return metrics, print_scale_for(bounds, dimension, size_mm, tolerance_um)


def _build_config(
    scale,
    hausdorff_samples: int,
    hausdorff_maxdist_percent: float,
    quality_threshold: float,
    planar_quadric: bool,
    keep_source_scale: bool,
    diagnostic_top_points: int,
    save_diagnostic_pointclouds: bool,
) -> SimplifyConfig:
    return SimplifyConfig(
        scale=scale,
        hausdorff_samples=hausdorff_samples,
        hausdorff_maxdist_percent=hausdorff_maxdist_percent,
        quality_threshold=quality_threshold,
        planar_quadric=planar_quadric,
        keep_source_scale=keep_source_scale,
        diagnostic_top_points=diagnostic_top_points,
        save_diagnostic_pointclouds=save_diagnostic_pointclouds,
    )


def _build_wrap_config(
    scale,
    close_below_mm: float,
    offset_mm: float | None,
    keep_source_scale: bool,
    exact_final_size: bool,
) -> WrapConfig:
    offset = close_below_mm / 5.0 if offset_mm is None else offset_mm
    return WrapConfig(
        scale=scale,
        close_below_mm=close_below_mm,
        offset_mm=offset,
        keep_source_scale=keep_source_scale,
        exact_final_size=exact_final_size,
    )


def _build_visibility_config(
    scale,
    pixel_mm: float,
    depth_tolerance_mm: float,
    view_count: int,
    min_visible_views: int,
    normal_threshold: float | None,
    keep_source_scale: bool,
    exact_final_size: bool,
) -> VisibilityCullConfig:
    return VisibilityCullConfig(
        scale=scale,
        pixel_mm=pixel_mm,
        depth_tolerance_mm=depth_tolerance_mm,
        view_count=view_count,
        min_visible_views=min_visible_views,
        normal_threshold=normal_threshold,
        keep_source_scale=keep_source_scale,
        exact_final_size=exact_final_size,
    )


def _build_smooth_config(
    scale,
    steps: int,
    lambda_: float,
    mu: float,
    keep_source_scale: bool,
    exact_final_size: bool,
) -> SmoothConfig:
    return SmoothConfig(
        scale=scale,
        steps=steps,
        lambda_=lambda_,
        mu=mu,
        keep_source_scale=keep_source_scale,
        exact_final_size=exact_final_size,
    )


def _build_hole_close_config(
    scale,
    max_hole_edges: int,
    refine: bool,
    repair_non_manifold: bool,
    keep_source_scale: bool,
    exact_final_size: bool,
) -> HoleCloseConfig:
    return HoleCloseConfig(
        scale=scale,
        max_hole_edges=max_hole_edges,
        refine=refine,
        repair_non_manifold=repair_non_manifold,
        keep_source_scale=keep_source_scale,
        exact_final_size=exact_final_size,
    )


def _print_metrics(metrics: dict) -> None:
    bounds = metrics["bounds"]
    scale = metrics["scale"]
    table = Table(title="Mesh")
    table.add_column("Metric")
    table.add_column("Value")
    table.add_row("source", metrics["source"])
    table.add_row("faces", f"{metrics['faces']:,}")
    table.add_row("vertices", f"{metrics['vertices']:,}")
    table.add_row("dims source units", " x ".join(f"{dim:.9g}" for dim in bounds["dims"]))
    table.add_row("selected dimension", scale["dimension"])
    table.add_row("selected source units", f"{scale['selected_source_units']:.9g}")
    table.add_row("target size", f"{scale['target_mm']:g} mm")
    table.add_row("scale", f"{scale['mm_per_source_unit']:.9g} mm/source-unit")
    table.add_row("tolerance", f"{scale['tolerance_um']:g} um")
    table.add_row("tolerance source units", f"{scale['tolerance_source_units']:.9g}")

    topology = metrics.get("topology")
    if topology:
        table.add_row("two-manifold", str(topology.get("is_mesh_two_manifold")))
        table.add_row("boundary edges", str(topology.get("boundary_edges")))
        table.add_row("non-manifold edges", str(topology.get("non_two_manifold_edges")))
        table.add_row("non-manifold vertices", str(topology.get("non_two_manifold_vertices")))
    console.print(table)


def _print_result(result: dict, label: str = "result") -> None:
    status = "pass" if result["within_tolerance"] else "fail"
    console.print(
        f"{label}: {result['actual_faces']:,} faces, "
        f"{result['actual_vertices']:,} vertices, "
        f"max {result['max_error_um']:.3f} um, "
        f"mean {result['mean_error_um']:.3f} um: {status}"
    )


def _print_wrap_result(result: dict, label: str = "wrap") -> None:
    topology = result.get("output_topology") or result.get("wrapped_topology_source") or {}
    console.print(
        f"{label}: {result['wrapped_faces']:,} faces, "
        f"{result['wrapped_vertices']:,} vertices, "
        f"two-manifold={topology.get('is_mesh_two_manifold')}, "
        f"boundary_edges={topology.get('boundary_edges')}, "
        f"non_manifold_edges={topology.get('non_two_manifold_edges')}"
    )


def _print_cull_result(result: dict, label: str = "cull-hidden") -> None:
    topology = result.get("output_topology") or result.get("culled_topology_source") or {}
    console.print(
        f"{label}: kept {result['kept_faces']:,}/{result['source_faces']:,} faces "
        f"({result['removed_face_fraction']:.1%} removed), "
        f"vertices {result['kept_vertices']:,}, "
        f"boundary_edges={topology.get('boundary_edges')}, "
        f"non_manifold_edges={topology.get('non_two_manifold_edges')}"
    )


def _print_smooth_result(result: dict, label: str = "smooth") -> None:
    topology = result.get("output_topology") or result.get("smoothed_topology_source") or {}
    console.print(
        f"{label}: {result['smoothed_faces']:,} faces, "
        f"{result['smoothed_vertices']:,} vertices, "
        f"two-manifold={topology.get('is_mesh_two_manifold')}, "
        f"boundary_edges={topology.get('boundary_edges')}, "
        f"non_manifold_edges={topology.get('non_two_manifold_edges')}"
    )


def _print_hole_close_result(result: dict, label: str = "close-holes") -> None:
    topology = result.get("output_topology") or result.get("closed_topology_source") or {}
    before = result.get("boundary_edges_before")
    after = result.get("boundary_edges_after")
    console.print(
        f"{label}: {result['closed_faces']:,} faces "
        f"({result['added_faces']:+,} faces), "
        f"boundary_edges {before} -> {after}, "
        f"two-manifold={topology.get('is_mesh_two_manifold')}, "
        f"components={topology.get('connected_components_number')}"
    )


def _print_component_clean_result(result: dict, label: str = "clean-components") -> None:
    topology = result.get("output_topology") or result.get("cleaned_topology") or {}
    console.print(
        f"{label}: {result['cleaned_faces']:,} faces "
        f"({-result['removed_faces']:+,} faces removed), "
        f"components={topology.get('connected_components_number')}, "
        f"boundary_edges={topology.get('boundary_edges')}, "
        f"non_manifold_edges={topology.get('non_two_manifold_edges')}"
    )


def _print_polish_result(result: dict, label: str = "polish") -> None:
    topology = result.get("output_topology") or result.get("polished_topology") or {}
    console.print(
        f"{label}: {result['polished_faces']:,} faces, "
        f"{result['polished_vertices']:,} vertices, "
        f"two-manifold={topology.get('is_mesh_two_manifold')}, "
        f"boundary_edges={topology.get('boundary_edges')}, "
        f"non_manifold_edges={topology.get('non_two_manifold_edges')}"
    )


def _pipeline_path(output: Path, suffix: str) -> Path:
    base = output.with_suffix("")
    return base.with_name(f"{base.name}_{suffix}.stl")


def _default_experiment_dir(model: Path, label: str) -> Path:
    stamp = datetime.now().strftime("%Y%m%d_%H%M%S")
    return model.parent / "experiments" / f"{stamp}_{model.stem}_{label}"


def _parse_float_list(text: str, option_name: str) -> tuple[float, ...]:
    values = []
    for part in text.split(","):
        stripped = part.strip()
        if not stripped:
            continue
        try:
            value = float(stripped)
        except ValueError as exc:
            raise typer.BadParameter(f"{option_name} must be a comma-separated list of numbers") from exc
        if value <= 0:
            raise typer.BadParameter(f"{option_name} values must be positive")
        values.append(value)
    if not values:
        raise typer.BadParameter(f"{option_name} must contain at least one number")
    return tuple(values)


def _parse_vector3(text: str) -> tuple[float, float, float]:
    parts = [part.strip() for part in text.split(",") if part.strip()]
    if len(parts) != 3:
        raise typer.BadParameter("camera direction must be formatted as x,y,z")
    try:
        values = tuple(float(part) for part in parts)
    except ValueError as exc:
        raise typer.BadParameter("camera direction values must be numbers") from exc
    return values  # type: ignore[return-value]


@app.command("inspect")
def inspect_command(
    model: Path = typer.Argument(..., exists=True, readable=True, help="Input mesh path."),
    dimension: DimensionMode = typer.Option(DimensionMode.SHORTEST, "--dimension", "-d"),
    size_mm: float = typer.Option(10.0, "--size-mm", help="Target printed size for the selected dimension."),
    tolerance_um: float = typer.Option(20.0, "--tolerance-um", help="Allowed print-scale error."),
    report: Optional[Path] = typer.Option(None, "--report", help="Optional JSON report path."),
) -> None:
    """Inspect a mesh and compute print-scale tolerance."""
    metrics = inspect_model(model, dimension=dimension, target_mm=size_mm, tolerance_um=tolerance_um)
    _print_metrics(metrics)
    if report:
        write_json(report, metrics)
        console.print(f"Wrote {report}")


@app.command("diagnose")
def diagnose_command(
    model: Path = typer.Argument(..., exists=True, readable=True, help="Input mesh path."),
    output_dir: Optional[Path] = typer.Option(None, "--output-dir", help="Experiment output directory."),
    views: int = typer.Option(16, "--views", help="Number of exterior raycast views."),
    image_size: int = typer.Option(128, "--image-size", help="Square diagnostic image size per view."),
    max_hits: int = typer.Option(6, "--max-hits", help="Maximum repeated ray hits to count per pixel."),
    self_intersections: bool = typer.Option(
        False,
        "--self-intersections/--no-self-intersections",
        help="Run MeshLab self-intersection face selection. This can be slow on dense meshes.",
    ),
) -> None:
    """Write topology, depth-complexity, and multi-view raycast diagnostics."""
    output_dir = output_dir or _default_experiment_dir(model, "diagnose")
    config = RayDiagnosticConfig(
        view_count=views,
        image_size=image_size,
        max_hits=max_hits,
        include_self_intersections=self_intersections,
    )
    console.print(f"Writing diagnostics to {output_dir}")
    report = diagnose_mesh(model, output_dir, config)
    topology = report["mesh"].get("topology") or {}
    aggregate = report["visibility"]["aggregate"]

    table = Table(title="Diagnostics")
    table.add_column("Metric")
    table.add_column("Value")
    table.add_row("faces", f"{report['mesh']['faces']:,}")
    table.add_row("vertices", f"{report['mesh']['vertices']:,}")
    table.add_row("boundary edges", str(topology.get("boundary_edges")))
    table.add_row("non-manifold edges", str(topology.get("non_two_manifold_edges")))
    table.add_row("non-manifold vertices", str(topology.get("non_two_manifold_vertices")))
    table.add_row("two-manifold", str(topology.get("is_mesh_two_manifold")))
    table.add_row("views", str(aggregate.get("views")))
    table.add_row("multi-hit fraction", f"{aggregate.get('multi_hit_fraction', 0):.4f}")
    table.add_row("three-plus-hit fraction", f"{aggregate.get('three_plus_hit_fraction', 0):.4f}")
    table.add_row("max hit count", str(aggregate.get("max_hit_count")))
    console.print(table)
    for note in report.get("interpretation", []):
        console.print(f"- {note}")
    console.print(f"Wrote {report['report']}")
    console.print(f"Wrote {report['markdown']}")
    console.print(f"Wrote {report['contact_sheet']}")


@app.command("validate")
def validate_command(
    reference: Path = typer.Argument(..., exists=True, readable=True, help="Reference/raw mesh path."),
    candidate: Path = typer.Argument(..., exists=True, readable=True, help="Candidate repaired mesh path."),
    output_dir: Optional[Path] = typer.Option(None, "--output-dir", help="Experiment output directory."),
    views: int = typer.Option(16, "--views", help="Number of exterior raycast views."),
    image_size: int = typer.Option(128, "--image-size", help="Square diagnostic image size per view."),
    max_hits: int = typer.Option(8, "--max-hits", help="Maximum repeated ray hits to count per pixel."),
) -> None:
    """Compare a candidate against a reference using common raycast cameras."""
    output_dir = output_dir or _default_experiment_dir(candidate, "validate")
    config = RayDiagnosticConfig(view_count=views, image_size=image_size, max_hits=max_hits)
    console.print(f"Writing validation to {output_dir}")
    report = compare_meshes(reference, candidate, output_dir, config)
    topology = report["candidate_mesh"].get("topology") or {}
    aggregate = report["comparison"]["aggregate"]

    table = Table(title="Validation")
    table.add_column("Metric")
    table.add_column("Value")
    table.add_row("candidate faces", f"{report['candidate_mesh']['faces']:,}")
    table.add_row("candidate vertices", f"{report['candidate_mesh']['vertices']:,}")
    table.add_row("candidate components", str(topology.get("connected_components_number")))
    table.add_row("candidate boundary edges", str(topology.get("boundary_edges")))
    table.add_row("candidate non-manifold edges", str(topology.get("non_two_manifold_edges")))
    table.add_row("mean silhouette IoU", f"{aggregate.get('mean_silhouette_iou', 0):.4f}")
    table.add_row("mean depth delta / diagonal", f"{aggregate.get('mean_abs_depth_delta_fraction_diagonal') or 0:.6f}")
    table.add_row("three-plus-hit fraction delta", f"{aggregate.get('mean_three_plus_fraction_delta', 0):.4f}")
    console.print(table)
    for note in report.get("interpretation", []):
        console.print(f"- {note}")
    console.print(f"Wrote {report['report']}")
    console.print(f"Wrote {report['markdown']}")
    console.print(f"Wrote {report['contact_sheet']}")


@app.command("roi-views")
def roi_views_command(
    model: Path = typer.Argument(..., exists=True, readable=True, help="Input mesh path."),
    output_dir: Optional[Path] = typer.Option(None, "--output-dir", help="Experiment output directory."),
    views: int = typer.Option(24, "--views", help="Numbered camera views to render."),
    image_size: int = typer.Option(512, "--image-size", help="Square image size per view."),
    padding: float = typer.Option(0.08, "--padding"),
) -> None:
    """Render numbered views so a defect can be marked by image coordinates."""
    output_dir = output_dir or _default_experiment_dir(model, "roi_views")
    report = render_roi_views(model, output_dir, views, image_size, padding)
    console.print(f"Wrote {report['contact_sheet']}")
    console.print(f"Wrote {report['report']}")


@app.command("roi-probe")
def roi_probe_command(
    model: Path = typer.Argument(..., exists=True, readable=True, help="Input mesh path."),
    output_dir: Optional[Path] = typer.Option(None, "--output-dir", help="Experiment output directory."),
    views: int = typer.Option(24, "--views", help="Number of generated camera views."),
    view_index: int = typer.Option(0, "--view-index", help="View index from roi-views."),
    camera_direction: Optional[str] = typer.Option(
        None,
        "--camera-direction",
        help="Optional explicit camera direction as x,y,z; overrides --view-index.",
    ),
    image_size: int = typer.Option(768, "--image-size", help="Square render size."),
    max_hits: int = typer.Option(12, "--max-hits", help="Ray hits to collect through the marked circle."),
    circle_x: Optional[float] = typer.Option(None, "--circle-x", help="Circle center x in pixels."),
    circle_y: Optional[float] = typer.Option(None, "--circle-y", help="Circle center y in pixels."),
    circle_radius: Optional[float] = typer.Option(None, "--circle-radius", help="Circle radius in pixels."),
    section_size: int = typer.Option(768, "--section-size", help="Cross-section image size."),
    local_expand: float = typer.Option(
        1.8,
        "--local-expand",
        help="Expand the selected local volume around ROI hit points.",
    ),
) -> None:
    """Back-project a rendered circle into 3D and generate local cross-sections."""
    output_dir = output_dir or _default_experiment_dir(model, "roi_probe")
    config = RoiProbeConfig(
        view_count=views,
        view_index=view_index,
        image_size=image_size,
        max_hits=max_hits,
        circle_x=circle_x,
        circle_y=circle_y,
        circle_radius=circle_radius,
        camera_direction=_parse_vector3(camera_direction) if camera_direction else None,
        section_size=section_size,
        local_expand=local_expand,
    )
    report = probe_roi(model, output_dir, config)
    roi = report["roi"]
    table = Table(title="ROI Probe")
    table.add_column("Metric")
    table.add_column("Value")
    table.add_row("unique hit faces", f"{roi['unique_hit_faces']:,}")
    table.add_row("local volume faces", f"{roi['local_volume_faces']:,}")
    table.add_row("overlay sheet", report["images"]["overlay_sheet"])
    table.add_row("cross sections", report["images"]["cross_sections"])
    table.add_row("local mesh", roi["local_volume_mesh"])
    console.print(table)
    console.print(f"Wrote {report['report']}")
    console.print(f"Wrote {report['markdown']}")


@app.command("roi-ui")
def roi_ui_command(
    model: Path = typer.Argument(..., exists=True, readable=True, help="Input mesh path."),
    output_dir: Optional[Path] = typer.Option(None, "--output-dir", help="Experiment output directory."),
    views: int = typer.Option(24, "--views", help="Numbered camera views to render."),
    image_size: int = typer.Option(768, "--image-size", help="Square image size for interactive view."),
    host: str = typer.Option("127.0.0.1", "--host"),
    port: int = typer.Option(8765, "--port"),
) -> None:
    """Launch a local browser UI for marking and probing mesh defects."""
    output_dir = output_dir or _default_experiment_dir(model, "roi_ui")
    launch_roi_ui(
        model,
        output_dir,
        host=host,
        port=port,
        view_count=views,
        image_size=image_size,
    )


@app.command("roi-3d-ui")
def roi_3d_ui_command(
    model: Path = typer.Argument(..., exists=True, readable=True, help="Input mesh path."),
    output_dir: Optional[Path] = typer.Option(None, "--output-dir", help="Experiment output directory."),
    preview_faces: int = typer.Option(250_000, "--preview-faces", help="Target face count for the browser preview mesh."),
    radius_fraction: float = typer.Option(
        0.035,
        "--radius-fraction",
        help="Default point-analysis radius as a fraction of the model diagonal.",
    ),
    host: str = typer.Option("127.0.0.1", "--host"),
    port: int = typer.Option(8766, "--port"),
) -> None:
    """Launch a Three.js 3D viewer for selecting multiple repair points."""
    output_dir = output_dir or _default_experiment_dir(model, "roi_3d_ui")
    launch_roi_3d_ui(
        model,
        output_dir,
        host=host,
        port=port,
        preview_faces=preview_faces,
        default_radius_fraction=radius_fraction,
    )


@app.command("repair-sweep")
def repair_sweep_command(
    model: Path = typer.Argument(..., exists=True, readable=True, help="Input mesh path."),
    output_dir: Optional[Path] = typer.Option(None, "--output-dir", help="Experiment output directory."),
    dimension: DimensionMode = typer.Option(DimensionMode.SHORTEST, "--dimension", "-d"),
    size_mm: float = typer.Option(
        10.0,
        "--size-mm",
        help="Virtual print size for translating physical repair thresholds; output stays in source units.",
    ),
    tolerance_um: float = typer.Option(20.0, "--tolerance-um", help="Only used for scale reporting."),
    alpha_values_mm: str = typer.Option(
        "0.04,0.05,0.075,0.10",
        "--alpha-values-mm",
        help="Comma-separated alpha-wrap throat sizes to test.",
    ),
    offset_factors: str = typer.Option(
        "0.20",
        "--offset-factors",
        help="Comma-separated offsets as fractions of each alpha value.",
    ),
    hybrid_pixel_mm: str = typer.Option(
        "0.04,0.06",
        "--hybrid-pixel-mm",
        help="Comma-separated projection pixel sizes for visibility-filtered candidates.",
    ),
    hybrid_alpha_mm: str = typer.Option(
        "0.05,0.075",
        "--hybrid-alpha-mm",
        help="Comma-separated alpha values for visibility-filtered candidates.",
    ),
    include_close_holes: bool = typer.Option(True, "--close-holes/--no-close-holes"),
    include_alpha: bool = typer.Option(True, "--alpha/--no-alpha"),
    include_hybrid: bool = typer.Option(True, "--hybrid/--no-hybrid"),
    close_hole_edges: int = typer.Option(2000, "--close-hole-edges"),
    hybrid_views: int = typer.Option(48, "--hybrid-views"),
    clean_min_faces: int = typer.Option(1000, "--clean-min-faces"),
    smooth_steps: int = typer.Option(0, "--smooth-steps"),
    validate_views: int = typer.Option(12, "--validate-views"),
    validate_image_size: int = typer.Option(96, "--validate-image-size"),
    validate_max_hits: int = typer.Option(8, "--validate-max-hits"),
    diagnose_views: int = typer.Option(12, "--diagnose-views"),
    diagnose_image_size: int = typer.Option(96, "--diagnose-image-size"),
    diagnose_max_hits: int = typer.Option(8, "--diagnose-max-hits"),
    keep_intermediates: bool = typer.Option(False, "--keep-intermediates"),
) -> None:
    """Generate, validate, and rank reusable repair candidates."""
    output_dir = output_dir or _default_experiment_dir(model, "repair_sweep")
    config = RepairSweepConfig(
        dimension=dimension,
        size_mm=size_mm,
        tolerance_um=tolerance_um,
        alpha_values_mm=_parse_float_list(alpha_values_mm, "--alpha-values-mm"),
        offset_factors=_parse_float_list(offset_factors, "--offset-factors"),
        hybrid_pixel_mm=_parse_float_list(hybrid_pixel_mm, "--hybrid-pixel-mm"),
        hybrid_alpha_mm=_parse_float_list(hybrid_alpha_mm, "--hybrid-alpha-mm"),
        include_close_holes=include_close_holes,
        include_alpha=include_alpha,
        include_hybrid=include_hybrid,
        close_hole_edges=close_hole_edges,
        hybrid_views=hybrid_views,
        clean_min_faces=clean_min_faces,
        smooth_steps=smooth_steps,
        keep_intermediates=keep_intermediates,
        validate=RayDiagnosticConfig(
            view_count=validate_views,
            image_size=validate_image_size,
            max_hits=validate_max_hits,
        ),
        diagnose=RayDiagnosticConfig(
            view_count=diagnose_views,
            image_size=diagnose_image_size,
            max_hits=diagnose_max_hits,
        ),
    )
    console.print(f"Writing repair sweep to {output_dir}")
    report = run_repair_sweep(model, output_dir, config, log=_echo)
    best = report.get("best") or {}

    table = Table(title="Repair Sweep")
    table.add_column("Rank")
    table.add_column("Score")
    table.add_column("Candidate")
    table.add_column("Topology")
    table.add_column("IoU")
    table.add_column("3+ hit")
    for rank, row in enumerate(report.get("candidates", [])[:8], start=1):
        if row.get("error"):
            table.add_row(str(rank), "error", row["label"], row.get("error", ""), "", "")
            continue
        score = row["score"]
        table.add_row(
            str(rank),
            f"{score['total']:.3f}",
            row["label"],
            "ok" if score["topology_ok"] else "fail",
            f"{score['silhouette_iou']:.4f}",
            f"{score['candidate_three_plus_fraction']:.4f}",
        )
    console.print(table)
    if best and not best.get("error"):
        console.print(f"Best candidate: {best['output']}")
        console.print(f"Best comparison sheet: {best['comparison_sheet']}")
        console.print(f"Best diagnostic sheet: {best['diagnostic_sheet']}")
    console.print(f"Wrote {report['report']}")
    console.print(f"Wrote {report['markdown']}")


@app.command("voxel-audit")
def voxel_audit_command(
    model: Path = typer.Argument(..., exists=True, readable=True, help="Input watertight mesh path."),
    output_dir: Optional[Path] = typer.Option(None, "--output-dir", help="Experiment output directory."),
    dimension: DimensionMode = typer.Option(DimensionMode.SHORTEST, "--dimension", "-d"),
    size_mm: float = typer.Option(
        10.0,
        "--size-mm",
        help="Virtual print size for translating voxel pitch; output files do not rescale the mesh.",
    ),
    tolerance_um: float = typer.Option(20.0, "--tolerance-um", help="Only used for scale reporting."),
    pitch_mm: float = typer.Option(0.075, "--pitch-mm", help="Voxel pitch at the virtual print scale."),
    throat_mm: float = typer.Option(
        0.15,
        "--throat-mm",
        help="Morphological closing diameter for narrow-access void detection.",
    ),
    padding_voxels: int = typer.Option(3, "--padding-voxels"),
    batch_size: int = typer.Option(1_000_000, "--batch-size"),
) -> None:
    """Voxelize a watertight candidate and audit sealed or narrow-access voids."""
    output_dir = output_dir or _default_experiment_dir(model, "voxel_audit")
    config = VoxelAuditConfig(
        dimension=dimension,
        size_mm=size_mm,
        tolerance_um=tolerance_um,
        pitch_mm=pitch_mm,
        throat_mm=throat_mm,
        padding_voxels=padding_voxels,
        batch_size=batch_size,
    )
    console.print(f"Writing voxel audit to {output_dir}")
    report = voxel_audit_mesh(model, output_dir, config)
    occupancy = report["occupancy"]
    table = Table(title="Voxel Audit")
    table.add_column("Metric")
    table.add_column("Value")
    table.add_row("grid", " x ".join(str(v) for v in report["grid"]["shape"]))
    table.add_row("solid voxels", f"{occupancy['solid_voxels']:,}")
    table.add_row("sealed void voxels", f"{occupancy['sealed_void_voxels']:,}")
    table.add_row(
        "narrow-access void voxels",
        f"{occupancy['narrow_access_void_voxels_after_closing']:,}",
    )
    table.add_row("slice sheet", report["slice_sheet"])
    console.print(table)
    for note in report.get("interpretation", []):
        console.print(f"- {note}")
    console.print(f"Wrote {report['report']}")
    console.print(f"Wrote {report['markdown']}")


@app.command("repair-cavities")
def repair_cavities_command(
    model: Path = typer.Argument(..., exists=True, readable=True, help="Input mesh path."),
    output: Optional[Path] = typer.Option(None, "--output", "-o", help="Final output mesh path."),
    dimension: DimensionMode = typer.Option(DimensionMode.SHORTEST, "--dimension", "-d"),
    size_mm: float = typer.Option(10.0, "--size-mm", help="Target printed size for the selected dimension."),
    tolerance_um: float = typer.Option(20.0, "--tolerance-um", help="Only used for scale reporting."),
    aperture_mm: float = typer.Option(
        0.10,
        "--aperture-mm",
        help="Entrances/narrow projections at or below this physical scale are treated as non-useful cavities.",
    ),
    wrap_offset_mm: Optional[float] = typer.Option(
        None,
        "--wrap-offset-mm",
        help="Alpha-wrap offset. Defaults to aperture_mm / 5.",
    ),
    visibility_pixel_mm: Optional[float] = typer.Option(
        None,
        "--visibility-pixel-mm",
        help="Projection pixel size for hidden-geometry culling. Defaults to aperture_mm.",
    ),
    depth_tolerance_mm: Optional[float] = typer.Option(
        None,
        "--depth-tolerance-mm",
        help="Visible depth tolerance. Defaults to aperture_mm / 2.",
    ),
    views: int = typer.Option(96, "--views"),
    smooth_steps: int = typer.Option(20, "--smooth-steps"),
    keep_intermediates: bool = typer.Option(False, "--keep-intermediates"),
    report: Optional[Path] = typer.Option(None, "--report"),
) -> None:
    """Cull narrow-access interior junk, wrap the exterior, then smooth."""
    metrics, scale = _build_scale(model, dimension, size_mm, tolerance_um)
    _print_metrics(metrics)

    output = output or default_output_path(model, scale=scale, label=f"cavity_repair_{aperture_mm:g}mm")
    report = report or report_path_for(output)
    visible_path = _pipeline_path(output, "visible")
    wrapped_path = _pipeline_path(output, "wrapped")

    visibility_pixel = aperture_mm if visibility_pixel_mm is None else visibility_pixel_mm
    depth_tolerance = aperture_mm / 2.0 if depth_tolerance_mm is None else depth_tolerance_mm
    offset = aperture_mm / 5.0 if wrap_offset_mm is None else wrap_offset_mm

    console.print("Stage 1/3: projection visibility cull")
    cull_config = _build_visibility_config(
        scale,
        visibility_pixel,
        depth_tolerance,
        views,
        1,
        None,
        False,
        True,
    )
    cull_result = cull_hidden_geometry(model, cull_config, output=visible_path, log=_echo)
    _print_cull_result(cull_result, "cull")

    console.print("Stage 2/3: alpha wrap the culled exterior")
    wrap_metrics, wrap_scale = _build_scale(visible_path, dimension, size_mm, tolerance_um)
    wrap_config = _build_wrap_config(wrap_scale, aperture_mm, offset, False, True)
    wrap_result = wrap_mesh(visible_path, wrap_config, output=wrapped_path, log=_echo)
    _print_wrap_result(wrap_result, "wrap")

    console.print("Stage 3/3: Taubin smooth the wrapped shell")
    smooth_metrics, smooth_scale = _build_scale(wrapped_path, dimension, size_mm, tolerance_um)
    smooth_config = _build_smooth_config(smooth_scale, smooth_steps, 0.5, -0.53, False, True)
    smooth_result = smooth_mesh(wrapped_path, smooth_config, output=output, log=_echo)
    _print_smooth_result(smooth_result, "smooth")

    report_data = {
        "metrics": metrics,
        "parameters": {
            "aperture_mm": aperture_mm,
            "visibility_pixel_mm": visibility_pixel,
            "depth_tolerance_mm": depth_tolerance,
            "views": views,
            "wrap_offset_mm": offset,
            "smooth_steps": smooth_steps,
        },
        "intermediates": {
            "visible": str(visible_path),
            "wrapped": str(wrapped_path),
        },
        "cull": cull_result,
        "wrap_metrics": wrap_metrics,
        "wrap": wrap_result,
        "smooth_metrics": smooth_metrics,
        "smooth": smooth_result,
        "output": str(output),
    }
    write_json(report, report_data)
    if not keep_intermediates:
        for path in (visible_path, report_path_for(visible_path), wrapped_path, report_path_for(wrapped_path)):
            path.unlink(missing_ok=True)
    console.print(f"Wrote {output}")
    console.print(f"Wrote {report}")


@app.command("repair-scan")
def repair_scan_command(
    model: Path = typer.Argument(..., exists=True, readable=True, help="Input mesh path."),
    output_dir: Optional[Path] = typer.Option(None, "--output-dir", help="Directory for variant files."),
    summary_report: Optional[Path] = typer.Option(None, "--summary-report", help="Aggregate JSON report path."),
    dimension: DimensionMode = typer.Option(DimensionMode.SHORTEST, "--dimension", "-d"),
    size_mm: float = typer.Option(10.0, "--size-mm", help="Target printed size for the selected dimension."),
    tolerance_um: float = typer.Option(20.0, "--tolerance-um", help="Only used for scale reporting."),
    apertures_mm: str = typer.Option(
        "0.05,0.075,0.10,0.125",
        "--apertures-mm",
        help="Comma-separated aperture thresholds to test.",
    ),
    views: int = typer.Option(72, "--views"),
    smooth_steps: int = typer.Option(20, "--smooth-steps"),
    keep_intermediates: bool = typer.Option(False, "--keep-intermediates"),
) -> None:
    """Generate a scale-space set of cavity-repair candidates."""
    metrics, scale = _build_scale(model, dimension, size_mm, tolerance_um)
    _print_metrics(metrics)
    output_dir = output_dir or model.parent
    output_dir.mkdir(parents=True, exist_ok=True)
    summary_report = summary_report or output_dir / f"{model.stem}_{size_mm:g}mm_{dimension.value}_repair_scan.report.json"

    aperture_items = []
    for part in apertures_mm.split(","):
        text = part.strip()
        if not text:
            continue
        aperture_items.append((float(text), text.lower().replace("+", "").replace(".", "p")))
    if not aperture_items:
        raise typer.BadParameter("--apertures-mm must contain at least one number")
    apertures = [aperture for aperture, _label in aperture_items]
    if any(aperture <= 0 for aperture in apertures):
        raise typer.BadParameter("--apertures-mm values must be positive")

    rows = []
    for aperture, label in aperture_items:
        output = output_dir / f"{model.stem}_{size_mm:g}mm_{dimension.value}_cavity_repair_{label}mm_smooth{smooth_steps}.stl"
        report = report_path_for(output)
        console.rule(f"aperture {aperture:g} mm")
        cull_config = _build_visibility_config(
            scale,
            aperture,
            aperture / 2.0,
            views,
            1,
            None,
            False,
            True,
        )
        visible_path = _pipeline_path(output, "visible")
        wrapped_path = _pipeline_path(output, "wrapped")
        cull_result = cull_hidden_geometry(model, cull_config, output=visible_path, log=_echo)
        _print_cull_result(cull_result, "cull")

        wrap_metrics, wrap_scale = _build_scale(visible_path, dimension, size_mm, tolerance_um)
        wrap_config = _build_wrap_config(wrap_scale, aperture, aperture / 5.0, False, True)
        wrap_result = wrap_mesh(visible_path, wrap_config, output=wrapped_path, log=_echo)
        _print_wrap_result(wrap_result, "wrap")

        smooth_metrics, smooth_scale = _build_scale(wrapped_path, dimension, size_mm, tolerance_um)
        smooth_config = _build_smooth_config(smooth_scale, smooth_steps, 0.5, -0.53, False, True)
        smooth_result = smooth_mesh(wrapped_path, smooth_config, output=output, log=_echo)
        _print_smooth_result(smooth_result, "smooth")

        write_json(
            report,
            {
                "metrics": metrics,
                "parameters": {
                    "aperture_mm": aperture,
                    "visibility_pixel_mm": aperture,
                    "depth_tolerance_mm": aperture / 2.0,
                    "views": views,
                    "wrap_offset_mm": aperture / 5.0,
                    "smooth_steps": smooth_steps,
                },
                "intermediates": {
                    "visible": str(visible_path),
                    "wrapped": str(wrapped_path),
                },
                "cull": cull_result,
                "wrap_metrics": wrap_metrics,
                "wrap": wrap_result,
                "smooth_metrics": smooth_metrics,
                "smooth": smooth_result,
                "output": str(output),
            },
        )
        if not keep_intermediates:
            for path in (visible_path, report_path_for(visible_path), wrapped_path, report_path_for(wrapped_path)):
                path.unlink(missing_ok=True)

        topology = smooth_result.get("output_topology") or {}
        rows.append(
            {
                "aperture_mm": aperture,
                "output": str(output),
                "report": str(report),
                "removed_face_fraction": cull_result["removed_face_fraction"],
                "faces": smooth_result["smoothed_faces"],
                "vertices": smooth_result["smoothed_vertices"],
                "connected_components": topology.get("connected_components_number"),
                "boundary_edges": topology.get("boundary_edges"),
                "non_manifold_edges": topology.get("non_two_manifold_edges"),
            }
        )

    table = Table(title="Repair Scan")
    table.add_column("Aperture")
    table.add_column("Faces")
    table.add_column("Components")
    table.add_column("Boundary")
    table.add_column("Non-manifold")
    table.add_column("Output")
    for row in rows:
        table.add_row(
            f"{row['aperture_mm']:g} mm",
            f"{row['faces']:,}",
            str(row["connected_components"]),
            str(row["boundary_edges"]),
            str(row["non_manifold_edges"]),
            row["output"],
        )
    console.print(table)
    write_json(
        summary_report,
        {
            "metrics": metrics,
            "parameters": {
                "apertures_mm": apertures,
                "views": views,
                "smooth_steps": smooth_steps,
                "visibility_pixel_mm": "same as aperture_mm",
                "depth_tolerance_mm": "aperture_mm / 2",
                "wrap_offset_mm": "aperture_mm / 5",
            },
            "variants": rows,
        },
    )
    console.print(f"Wrote {summary_report}")


@app.command("close-holes")
def close_holes_command(
    model: Path = typer.Argument(..., exists=True, readable=True, help="Input mesh path."),
    output: Optional[Path] = typer.Option(None, "--output", "-o", help="Output mesh path."),
    dimension: DimensionMode = typer.Option(DimensionMode.SHORTEST, "--dimension", "-d"),
    size_mm: float = typer.Option(10.0, "--size-mm", help="Target printed size for the selected dimension."),
    tolerance_um: float = typer.Option(20.0, "--tolerance-um", help="Only used for scale reporting."),
    max_hole_edges: int = typer.Option(
        1000,
        "--max-hole-edges",
        help="Close boundary holes with up to this many edges around the rim.",
    ),
    refine: bool = typer.Option(False, "--refine", help="Refine the patch triangles generated across holes."),
    repair_non_manifold: bool = typer.Option(
        True,
        "--repair-non-manifold/--no-repair-non-manifold",
        help="Repair non-manifold edges/vertices before closing holes.",
    ),
    keep_source_scale: bool = typer.Option(False, "--keep-source-scale"),
    exact_final_size: bool = typer.Option(
        True,
        "--exact-final-size/--source-scale-after-close",
        help="Rescale the result so the selected final dimension equals --size-mm.",
    ),
    report: Optional[Path] = typer.Option(None, "--report"),
) -> None:
    """Patch real open boundary holes without doing cavity shrinkwrapping."""
    metrics, scale = _build_scale(model, dimension, size_mm, tolerance_um)
    _print_metrics(metrics)
    config = _build_hole_close_config(
        scale,
        max_hole_edges,
        refine,
        repair_non_manifold,
        keep_source_scale,
        exact_final_size,
    )
    output = output or default_output_path(model, scale=scale, label=f"closed_holes_{max_hole_edges}edges")
    report = report or report_path_for(output)

    result = close_boundary_holes(model, config, output=output, log=_echo)
    _print_hole_close_result(result)
    write_json(report, {"metrics": metrics, "result": result})
    console.print(f"Wrote {output}")
    console.print(f"Wrote {report}")


@app.command("clean-components")
def clean_components_command(
    model: Path = typer.Argument(..., exists=True, readable=True, help="Input mesh path."),
    output: Optional[Path] = typer.Option(None, "--output", "-o", help="Output mesh path."),
    min_faces: int = typer.Option(
        100,
        "--min-faces",
        help="Remove disconnected components with fewer than this many faces.",
    ),
    report: Optional[Path] = typer.Option(None, "--report"),
) -> None:
    """Remove tiny disconnected shells after wrapping."""
    output = output or model.with_name(f"{model.stem}_components_cleaned{model.suffix}")
    report = report or report_path_for(output)
    config = ComponentCleanConfig(min_faces=min_faces)
    result = remove_small_components(model, config, output=output, log=_echo)
    _print_component_clean_result(result)
    write_json(report, {"result": result})
    console.print(f"Wrote {output}")
    console.print(f"Wrote {report}")


@app.command("smooth")
def smooth_command(
    model: Path = typer.Argument(..., exists=True, readable=True, help="Input mesh path."),
    output: Optional[Path] = typer.Option(None, "--output", "-o", help="Output mesh path."),
    dimension: DimensionMode = typer.Option(DimensionMode.SHORTEST, "--dimension", "-d"),
    size_mm: float = typer.Option(10.0, "--size-mm", help="Target printed size for the selected dimension."),
    tolerance_um: float = typer.Option(20.0, "--tolerance-um", help="Only used for scale reporting."),
    steps: int = typer.Option(20, "--steps", help="Taubin smoothing iterations."),
    lambda_: float = typer.Option(0.5, "--lambda", help="Taubin lambda value."),
    mu: float = typer.Option(-0.53, "--mu", help="Taubin mu value."),
    keep_source_scale: bool = typer.Option(False, "--keep-source-scale"),
    exact_final_size: bool = typer.Option(
        True,
        "--exact-final-size/--source-scale-after-smooth",
        help="Rescale the smoothed result so the selected final dimension equals --size-mm.",
    ),
    report: Optional[Path] = typer.Option(None, "--report"),
) -> None:
    """Smooth zipper/facet artifacts while preserving topology."""
    metrics, scale = _build_scale(model, dimension, size_mm, tolerance_um)
    _print_metrics(metrics)
    config = _build_smooth_config(scale, steps, lambda_, mu, keep_source_scale, exact_final_size)
    output = output or default_output_path(model, scale=scale, label=f"smooth_{steps}")
    report = report or report_path_for(output)
    result = smooth_mesh(model, config, output=output, log=_echo)
    _print_smooth_result(result)
    write_json(report, {"metrics": metrics, "result": result})
    console.print(f"Wrote {output}")
    console.print(f"Wrote {report}")


@app.command("polish")
def polish_command(
    model: Path = typer.Argument(..., exists=True, readable=True, help="Input watertight mesh path."),
    output: Optional[Path] = typer.Option(None, "--output", "-o", help="Output mesh path."),
    pre_taubin_steps: int = typer.Option(20, "--pre-taubin-steps"),
    isotropic_iterations: int = typer.Option(8, "--isotropic-iterations"),
    targetlen_percent: float = typer.Option(
        0.4,
        "--targetlen-percent",
        help="Isotropic remesh target edge length as percentage of bounding-box diagonal.",
    ),
    max_surface_dist_percent: float = typer.Option(
        0.3,
        "--max-surface-dist-percent",
        help="Maximum reproject distance as percentage of bounding-box diagonal.",
    ),
    feature_deg: float = typer.Option(30.0, "--feature-deg"),
    post_taubin_steps: int = typer.Option(5, "--post-taubin-steps"),
    report: Optional[Path] = typer.Option(None, "--report"),
) -> None:
    """Smooth and regularize a closed alpha-wrap candidate without rescaling."""
    output = output or model.with_name(f"{model.stem}_polished{model.suffix}")
    report = report or report_path_for(output)
    config = PolishConfig(
        pre_taubin_steps=pre_taubin_steps,
        isotropic_iterations=isotropic_iterations,
        targetlen_percent=targetlen_percent,
        max_surface_dist_percent=max_surface_dist_percent,
        feature_deg=feature_deg,
        post_taubin_steps=post_taubin_steps,
    )
    result = polish_mesh(model, config, output=output, log=_echo)
    _print_polish_result(result)
    write_json(report, {"result": result})
    console.print(f"Wrote {output}")
    console.print(f"Wrote {report}")


@app.command("cull-hidden")
def cull_hidden_command(
    model: Path = typer.Argument(..., exists=True, readable=True, help="Input mesh path."),
    output: Optional[Path] = typer.Option(None, "--output", "-o", help="Output mesh path."),
    dimension: DimensionMode = typer.Option(DimensionMode.SHORTEST, "--dimension", "-d"),
    size_mm: float = typer.Option(10.0, "--size-mm", help="Target printed size for the selected dimension."),
    tolerance_um: float = typer.Option(20.0, "--tolerance-um", help="Only used for scale reporting."),
    visibility_pixel_mm: float = typer.Option(
        0.05,
        "--visibility-pixel-mm",
        help="Projection pixel size. Narrower openings than this are unlikely to reveal internal surfaces.",
    ),
    depth_tolerance_mm: float = typer.Option(
        0.02,
        "--depth-tolerance-mm",
        help="Keep faces within this depth of the frontmost projected surface.",
    ),
    views: int = typer.Option(96, "--views", help="Number of exterior view directions."),
    min_visible_views: int = typer.Option(
        1,
        "--min-visible-views",
        help="Require a face to be visible from at least this many exterior views.",
    ),
    normal_threshold: Optional[float] = typer.Option(
        None,
        "--normal-threshold",
        help="Optional dot(normal, view_direction) threshold. Leave unset when normals may be unreliable.",
    ),
    keep_source_scale: bool = typer.Option(False, "--keep-source-scale"),
    exact_final_size: bool = typer.Option(
        True,
        "--exact-final-size/--source-scale-after-cull",
        help="Rescale the culled result so the selected final dimension equals --size-mm.",
    ),
    report: Optional[Path] = typer.Option(None, "--report"),
) -> None:
    """Remove faces that are never visible from exterior projections."""
    metrics, scale = _build_scale(model, dimension, size_mm, tolerance_um)
    _print_metrics(metrics)
    config = _build_visibility_config(
        scale,
        visibility_pixel_mm,
        depth_tolerance_mm,
        views,
        min_visible_views,
        normal_threshold,
        keep_source_scale,
        exact_final_size,
    )
    output = output or default_output_path(model, scale=scale, label=f"visible_{visibility_pixel_mm:g}mm")
    report = report or report_path_for(output)

    result = cull_hidden_geometry(model, config, output=output, log=_echo)
    _print_cull_result(result)
    write_json(report, {"metrics": metrics, "result": result})
    console.print(f"Wrote {output}")
    console.print(f"Wrote {report}")


@app.command("wrap")
def wrap_command(
    model: Path = typer.Argument(..., exists=True, readable=True, help="Input mesh path."),
    output: Optional[Path] = typer.Option(None, "--output", "-o", help="Output mesh path."),
    dimension: DimensionMode = typer.Option(DimensionMode.SHORTEST, "--dimension", "-d"),
    size_mm: float = typer.Option(10.0, "--size-mm", help="Target printed size for the selected dimension."),
    tolerance_um: float = typer.Option(20.0, "--tolerance-um", help="Only used for scale reporting."),
    close_below_mm: float = typer.Option(
        0.05,
        "--close-below-mm",
        help="Approximate physical feature/gap scale to bridge or ignore.",
    ),
    offset_mm: Optional[float] = typer.Option(
        None,
        "--offset-mm",
        help="Wrapper offset from the source surface. Defaults to close_below_mm / 5.",
    ),
    keep_source_scale: bool = typer.Option(False, "--keep-source-scale"),
    exact_final_size: bool = typer.Option(
        True,
        "--exact-final-size/--source-scale-after-wrap",
        help="Rescale the wrapped result so the selected final dimension equals --size-mm.",
    ),
    report: Optional[Path] = typer.Option(None, "--report"),
) -> None:
    """Create a watertight shrinkwrap-style shell around a model."""
    metrics, scale = _build_scale(model, dimension, size_mm, tolerance_um)
    _print_metrics(metrics)
    config = _build_wrap_config(scale, close_below_mm, offset_mm, keep_source_scale, exact_final_size)
    output = output or default_output_path(model, scale=scale, label=f"wrap_{close_below_mm:g}mm")
    report = report or report_path_for(output)

    result = wrap_mesh(model, config, output=output, log=_echo)
    _print_wrap_result(result)
    write_json(report, {"metrics": metrics, "result": result})
    console.print(f"Wrote {output}")
    console.print(f"Wrote {report}")


@app.command("simplify")
def simplify_command(
    model: Path = typer.Argument(..., exists=True, readable=True, help="Input mesh path."),
    target_faces: int = typer.Option(..., "--target-faces", "-f", help="Face count to request."),
    output: Optional[Path] = typer.Option(None, "--output", "-o", help="Output mesh path."),
    dimension: DimensionMode = typer.Option(DimensionMode.SHORTEST, "--dimension", "-d"),
    size_mm: float = typer.Option(10.0, "--size-mm", help="Target printed size for the selected dimension."),
    tolerance_um: float = typer.Option(20.0, "--tolerance-um", help="Allowed print-scale error."),
    hausdorff_samples: int = typer.Option(1_000_000, "--hausdorff-samples"),
    hausdorff_maxdist_percent: float = typer.Option(5.0, "--hausdorff-maxdist-percent"),
    quality_threshold: float = typer.Option(0.3, "--quality-threshold"),
    planar_quadric: bool = typer.Option(False, "--planar-quadric", help="Better for planar CAD-like meshes."),
    keep_source_scale: bool = typer.Option(False, "--keep-source-scale"),
    diagnostics: bool = typer.Option(True, "--diagnostics/--no-diagnostics"),
    diagnostic_top_points: int = typer.Option(100, "--diagnostic-top-points"),
    save_diagnostic_pointclouds: bool = typer.Option(False, "--save-diagnostic-pointclouds"),
    allow_over_tolerance: bool = typer.Option(False, "--allow-over-tolerance"),
    report: Optional[Path] = typer.Option(None, "--report"),
) -> None:
    """Run one adaptive simplification target and verify it."""
    metrics, scale = _build_scale(model, dimension, size_mm, tolerance_um)
    _print_metrics(metrics)
    config = _build_config(
        scale,
        hausdorff_samples,
        hausdorff_maxdist_percent,
        quality_threshold,
        planar_quadric,
        keep_source_scale,
        diagnostic_top_points,
        save_diagnostic_pointclouds,
    )
    output = output or default_output_path(model, scale=scale, label="simplified", target_faces=target_faces)
    report = report or report_path_for(output)
    diagnostic_prefix = diagnostic_prefix_for(output) if diagnostics else None
    result = simplify_candidate(model, target_faces, config, output=output, diagnostic_prefix=diagnostic_prefix, log=_echo)
    _print_result(result, "simplify")
    if not result["within_tolerance"] and not allow_over_tolerance:
        write_json(report, {"metrics": metrics, "result": result})
        raise typer.Exit(code=2)
    write_json(report, {"metrics": metrics, "result": result})
    console.print(f"Wrote {output}")
    console.print(f"Wrote {report}")


@app.command("optimize")
def optimize_command(
    model: Path = typer.Argument(..., exists=True, readable=True, help="Input mesh path."),
    output: Optional[Path] = typer.Option(None, "--output", "-o", help="Output mesh path."),
    dimension: DimensionMode = typer.Option(DimensionMode.SHORTEST, "--dimension", "-d"),
    size_mm: float = typer.Option(10.0, "--size-mm", help="Target printed size for the selected dimension."),
    tolerance_um: float = typer.Option(20.0, "--tolerance-um", help="Allowed print-scale error."),
    min_faces: int = typer.Option(250_000, "--min-faces", help="Lower search bound."),
    max_faces: Optional[int] = typer.Option(None, "--max-faces", help="Upper search bound. Defaults to original face count."),
    step_faces: int = typer.Option(5_000, "--step-faces", help="Stop when pass/fail bounds are this close."),
    hausdorff_samples: int = typer.Option(1_000_000, "--hausdorff-samples"),
    hausdorff_maxdist_percent: float = typer.Option(5.0, "--hausdorff-maxdist-percent"),
    quality_threshold: float = typer.Option(0.3, "--quality-threshold"),
    planar_quadric: bool = typer.Option(False, "--planar-quadric", help="Better for planar CAD-like meshes."),
    keep_source_scale: bool = typer.Option(False, "--keep-source-scale"),
    diagnostics: bool = typer.Option(True, "--diagnostics/--no-diagnostics"),
    diagnose_last_fail: bool = typer.Option(True, "--diagnose-last-fail/--no-diagnose-last-fail"),
    diagnostic_top_points: int = typer.Option(100, "--diagnostic-top-points"),
    save_diagnostic_pointclouds: bool = typer.Option(False, "--save-diagnostic-pointclouds"),
    report: Optional[Path] = typer.Option(None, "--report"),
) -> None:
    """Search for the smallest passing adaptive simplification."""
    metrics, scale = _build_scale(model, dimension, size_mm, tolerance_um)
    _print_metrics(metrics)
    original_faces = int(metrics["faces"])
    max_faces = max_faces or original_faces
    max_faces = min(max_faces, original_faces)
    if min_faces >= max_faces:
        raise typer.BadParameter("min_faces must be lower than max_faces after clamping to original face count")

    topology = metrics.get("topology") or {}
    if topology and not topology.get("is_mesh_two_manifold", True):
        console.print(
            "Source is not fully manifold; curvature-field weighting is skipped. "
            "Using adaptive QEM plus measured error diagnostics."
        )

    config = _build_config(
        scale,
        hausdorff_samples,
        hausdorff_maxdist_percent,
        quality_threshold,
        planar_quadric,
        keep_source_scale,
        diagnostic_top_points,
        save_diagnostic_pointclouds,
    )
    output = output or default_output_path(model, scale=scale, label="adaptive")
    report = report or report_path_for(output)

    best_target, history, last_fail = optimize_face_count(
        model,
        config,
        min_faces=min_faces,
        max_faces=max_faces,
        step_faces=step_faces,
        log=_echo,
    )
    for row in history:
        _print_result(row, f"checked {row['target_faces']:,}")

    diagnostic_prefix = diagnostic_prefix_for(output) if diagnostics else None
    console.print(f"Writing best passing target {best_target:,} faces")
    final = simplify_candidate(model, best_target, config, output=output, diagnostic_prefix=diagnostic_prefix, log=_echo)
    _print_result(final, "final")

    fail_diagnostic = None
    if diagnose_last_fail and last_fail is not None and diagnostics:
        fail_target = int(last_fail["target_faces"])
        console.print(f"Writing diagnostics for nearest failing target {fail_target:,} faces")
        fail_diagnostic = simplify_candidate(
            model,
            fail_target,
            config,
            diagnostic_prefix=fail_diagnostic_prefix_for(output, fail_target),
            log=_echo,
        )
        _print_result(fail_diagnostic, "nearest fail")

    report_data = {
        "metrics": metrics,
        "search": {
            "min_faces": min_faces,
            "max_faces": max_faces,
            "step_faces": step_faces,
            "selected_target_faces": best_target,
            "history": history,
        },
        "final": final,
        "nearest_fail_diagnostic": fail_diagnostic,
    }
    write_json(report, report_data)
    console.print(f"Wrote {output}")
    console.print(f"Wrote {report}")


@app.command("topology")
def topology_command(
    model: Path = typer.Argument(..., exists=True, readable=True, help="Input mesh path."),
    report: Optional[Path] = typer.Option(None, "--report", help="Optional JSON report path."),
) -> None:
    """Report MeshLab topology measures."""
    topology = topology_for_path(model)
    if not topology:
        console.print("Topology measures unavailable.")
        raise typer.Exit(code=1)
    table = Table(title="Topology")
    table.add_column("Metric")
    table.add_column("Value")
    for key, value in topology.items():
        table.add_row(str(key), str(value))
    console.print(table)
    if report:
        write_json(report, {"source": str(model), "topology": topology})
        console.print(f"Wrote {report}")

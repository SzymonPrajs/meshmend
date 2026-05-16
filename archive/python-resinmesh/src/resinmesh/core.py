"""Core mesh operations for the resinmesh CLI."""

from __future__ import annotations

from dataclasses import dataclass
from enum import Enum
import csv
import json
import re
import time
from pathlib import Path
from typing import Any, Callable

import numpy as np
import pymeshlab
from pymeshlab import PercentageValue

LogFn = Callable[[str], None]


class DimensionMode(str, Enum):
    SHORTEST = "shortest"
    LONGEST = "longest"
    X = "x"
    Y = "y"
    Z = "z"


@dataclass(frozen=True)
class PrintScale:
    dimension: DimensionMode
    target_mm: float
    tolerance_um: float
    selected_source_units: float
    mm_per_source_unit: float
    tolerance_source_units: float


@dataclass(frozen=True)
class SimplifyConfig:
    scale: PrintScale
    hausdorff_samples: int = 1_000_000
    hausdorff_maxdist_percent: float = 5.0
    quality_threshold: float = 0.3
    planar_quadric: bool = False
    keep_source_scale: bool = False
    diagnostic_top_points: int = 100
    save_diagnostic_pointclouds: bool = False


@dataclass(frozen=True)
class WrapConfig:
    scale: PrintScale
    close_below_mm: float
    offset_mm: float
    keep_source_scale: bool = False
    exact_final_size: bool = True


@dataclass(frozen=True)
class VisibilityCullConfig:
    scale: PrintScale
    pixel_mm: float = 0.05
    depth_tolerance_mm: float = 0.02
    view_count: int = 96
    min_visible_views: int = 1
    normal_threshold: float | None = None
    keep_source_scale: bool = False
    exact_final_size: bool = True


@dataclass(frozen=True)
class SmoothConfig:
    scale: PrintScale
    steps: int = 20
    lambda_: float = 0.5
    mu: float = -0.53
    keep_source_scale: bool = False
    exact_final_size: bool = True


@dataclass(frozen=True)
class ComponentCleanConfig:
    min_faces: int = 100
    keep_source_scale: bool = True


@dataclass(frozen=True)
class HoleCloseConfig:
    scale: PrintScale
    max_hole_edges: int = 1000
    refine: bool = False
    repair_non_manifold: bool = True
    keep_source_scale: bool = False
    exact_final_size: bool = True


@dataclass(frozen=True)
class PolishConfig:
    pre_taubin_steps: int = 20
    isotropic_iterations: int = 8
    targetlen_percent: float = 0.4
    max_surface_dist_percent: float = 0.3
    feature_deg: float = 30.0
    post_taubin_steps: int = 5


def bounds_for(mesh: pymeshlab.Mesh) -> dict[str, Any]:
    box = mesh.bounding_box()
    dims = [float(box.dim_x()), float(box.dim_y()), float(box.dim_z())]
    return {
        "min": [float(v) for v in box.min()],
        "max": [float(v) for v in box.max()],
        "dims": dims,
        "shortest_axis": min(dims),
        "longest_axis": max(dims),
    }


def selected_dimension_units(bounds: dict[str, Any], dimension: DimensionMode) -> float:
    dims = bounds["dims"]
    if dimension == DimensionMode.SHORTEST:
        return float(min(dims))
    if dimension == DimensionMode.LONGEST:
        return float(max(dims))
    if dimension == DimensionMode.X:
        return float(dims[0])
    if dimension == DimensionMode.Y:
        return float(dims[1])
    if dimension == DimensionMode.Z:
        return float(dims[2])
    raise ValueError(f"Unknown dimension mode: {dimension}")


def print_scale_for(
    bounds: dict[str, Any],
    dimension: DimensionMode,
    target_mm: float,
    tolerance_um: float,
) -> PrintScale:
    if target_mm <= 0:
        raise ValueError("target_mm must be positive")
    if tolerance_um <= 0:
        raise ValueError("tolerance_um must be positive")

    selected_source_units = selected_dimension_units(bounds, dimension)
    if selected_source_units <= 0:
        raise ValueError(f"Selected {dimension.value} dimension has non-positive size")
    mm_per_source_unit = target_mm / selected_source_units
    tolerance_source_units = (tolerance_um / 1000.0) / mm_per_source_unit
    return PrintScale(
        dimension=dimension,
        target_mm=target_mm,
        tolerance_um=tolerance_um,
        selected_source_units=selected_source_units,
        mm_per_source_unit=mm_per_source_unit,
        tolerance_source_units=tolerance_source_units,
    )


def topology_for(ms: pymeshlab.MeshSet) -> dict[str, Any] | None:
    try:
        topology = ms.get_topological_measures()
    except Exception:
        return None
    return dict(topology)


def topology_for_path(path: Path) -> dict[str, Any] | None:
    ms = pymeshlab.MeshSet()
    ms.load_new_mesh(str(path))
    return topology_for(ms)


def component_summary_for_path(path: Path) -> list[dict[str, Any]]:
    ms = pymeshlab.MeshSet()
    ms.load_new_mesh(str(path))
    ms.generate_splitting_by_connected_components(delete_source_mesh=False)
    components: list[dict[str, Any]] = []
    for mesh_id in range(1, ms.mesh_number()):
        ms.set_current_mesh(mesh_id)
        mesh = ms.current_mesh()
        bounds = bounds_for(mesh)
        components.append(
            {
                "mesh_id": mesh_id,
                "faces": int(mesh.face_number()),
                "vertices": int(mesh.vertex_number()),
                "bounds": bounds,
            }
        )
    components.sort(key=lambda row: row["faces"], reverse=True)
    return components


def load_source(source: Path) -> tuple[pymeshlab.MeshSet, int, pymeshlab.Mesh, dict[str, Any]]:
    ms = pymeshlab.MeshSet()
    ms.load_new_mesh(str(source))
    original_id = ms.current_mesh_id()
    original = ms.current_mesh()
    return ms, original_id, original, bounds_for(original)


def inspect_model(
    source: Path,
    dimension: DimensionMode = DimensionMode.SHORTEST,
    target_mm: float = 10.0,
    tolerance_um: float = 20.0,
) -> dict[str, Any]:
    ms, _original_id, mesh, bounds = load_source(source)
    scale = print_scale_for(bounds, dimension, target_mm, tolerance_um)
    return {
        "source": str(source),
        "faces": int(mesh.face_number()),
        "vertices": int(mesh.vertex_number()),
        "bounds": bounds,
        "topology": topology_for(ms),
        "scale": scale_to_dict(scale),
    }


def scale_to_dict(scale: PrintScale) -> dict[str, Any]:
    return {
        "dimension": scale.dimension.value,
        "target_mm": scale.target_mm,
        "tolerance_um": scale.tolerance_um,
        "selected_source_units": scale.selected_source_units,
        "mm_per_source_unit": scale.mm_per_source_unit,
        "tolerance_source_units": scale.tolerance_source_units,
    }


def um_from_units(source_units: float, scale: PrintScale) -> float:
    return source_units * scale.mm_per_source_unit * 1000.0


def source_units_from_mm(mm: float, scale: PrintScale) -> float:
    return mm / scale.mm_per_source_unit


def percentage_of_diagonal(source_units: float, bounds: dict[str, Any]) -> float:
    dims = bounds["dims"]
    diagonal = float(np.linalg.norm(np.asarray(dims, dtype=float)))
    if diagonal <= 0:
        raise ValueError("Mesh diagonal has non-positive size")
    return source_units / diagonal * 100.0


def wrap_mesh(
    source: Path,
    config: WrapConfig,
    *,
    output: Path | None = None,
    log: LogFn | None = None,
) -> dict[str, Any]:
    """Create an alpha-wrap shell around a model.

    Alpha wrapping is intentionally a geometry-changing repair step: it creates
    a watertight outer surface and bridges cavities/openings below a selected
    physical size. Larger petal gaps can survive if the close-below threshold is
    chosen below their width.
    """
    if config.close_below_mm <= 0:
        raise ValueError("close_below_mm must be positive")
    if config.offset_mm < 0:
        raise ValueError("offset_mm must not be negative")

    started = time.time()
    ms, original_id, original, source_bounds = load_source(source)
    source_topology = topology_for(ms)

    close_below_source_units = source_units_from_mm(config.close_below_mm, config.scale)
    offset_source_units = source_units_from_mm(config.offset_mm, config.scale)
    alpha_percent = percentage_of_diagonal(close_below_source_units, source_bounds)
    offset_percent = percentage_of_diagonal(offset_source_units, source_bounds)

    if log:
        log(
            f"Alpha wrapping with close-below {config.close_below_mm:g} mm "
            f"({alpha_percent:.4g}% of source diagonal) and offset {config.offset_mm:g} mm"
        )

    wrap_started = time.time()
    ms.generate_alpha_wrap(
        alpha=PercentageValue(alpha_percent),
        offset=PercentageValue(offset_percent),
    )
    wrap_seconds = time.time() - wrap_started
    wrapped_id = ms.current_mesh_id()
    wrapped = ms.current_mesh()
    wrapped_faces = int(wrapped.face_number())
    wrapped_vertices = int(wrapped.vertex_number())
    wrapped_bounds_source = bounds_for(wrapped)
    wrapped_topology_source = topology_for(ms)

    output_bounds = None
    output_topology = None
    output_scale_factor = 1.0
    if output is not None:
        output.parent.mkdir(parents=True, exist_ok=True)
        ms.set_current_mesh(wrapped_id)
        if not config.keep_source_scale:
            if config.exact_final_size:
                selected_wrapped_units = selected_dimension_units(wrapped_bounds_source, config.scale.dimension)
                output_scale_factor = config.scale.target_mm / selected_wrapped_units
            else:
                output_scale_factor = config.scale.mm_per_source_unit
            ms.compute_matrix_from_translation_rotation_scale(
                scalex=output_scale_factor,
                scaley=output_scale_factor,
                scalez=output_scale_factor,
                freeze=True,
            )
        output_bounds = bounds_for(ms.current_mesh())
        output_topology = topology_for(ms)
        ms.save_current_mesh(str(output))

    return {
        "source": str(source),
        "output": str(output) if output is not None else None,
        "output_units": "source" if config.keep_source_scale else "millimetres",
        "source_faces": int(original.face_number()),
        "source_vertices": int(original.vertex_number()),
        "source_bounds": source_bounds,
        "source_topology": source_topology,
        "scale": scale_to_dict(config.scale),
        "method": "alpha_wrap",
        "close_below_mm": config.close_below_mm,
        "offset_mm": config.offset_mm,
        "close_below_source_units": close_below_source_units,
        "offset_source_units": offset_source_units,
        "alpha_percent_of_diagonal": alpha_percent,
        "offset_percent_of_diagonal": offset_percent,
        "wrapped_faces": wrapped_faces,
        "wrapped_vertices": wrapped_vertices,
        "wrapped_bounds_source": wrapped_bounds_source,
        "wrapped_topology_source": wrapped_topology_source,
        "exact_final_size": config.exact_final_size,
        "output_scale_factor": output_scale_factor,
        "output_bounds": output_bounds,
        "output_topology": output_topology,
        "wrap_seconds": wrap_seconds,
        "total_seconds": time.time() - started,
    }


def cull_hidden_geometry(
    source: Path,
    config: VisibilityCullConfig,
    *,
    output: Path | None = None,
    log: LogFn | None = None,
) -> dict[str, Any]:
    """Keep faces visible in at least one exterior projection.

    This is a projection-space approximation of "can any outside camera see
    this surface?" It is deliberately controlled in millimetres so tiny slots
    can be treated as closed while larger petal gaps remain visible.
    """
    if config.pixel_mm <= 0:
        raise ValueError("pixel_mm must be positive")
    if config.depth_tolerance_mm < 0:
        raise ValueError("depth_tolerance_mm must not be negative")
    if config.view_count <= 0:
        raise ValueError("view_count must be positive")
    if config.min_visible_views <= 0:
        raise ValueError("min_visible_views must be positive")

    started = time.time()
    ms, _original_id, mesh, source_bounds = load_source(source)
    source_topology = topology_for(ms)
    vertices = np.asarray(mesh.vertex_matrix(), dtype=np.float64)
    faces = np.asarray(mesh.face_matrix(), dtype=np.int32)
    face_count = int(faces.shape[0])

    if face_count == 0:
        raise ValueError("Mesh has no faces")

    tri_vertices = vertices[faces]
    centers = tri_vertices.mean(axis=1)
    normals = np.cross(tri_vertices[:, 1] - tri_vertices[:, 0], tri_vertices[:, 2] - tri_vertices[:, 0])
    normal_lengths = np.linalg.norm(normals, axis=1)
    valid_normals = normal_lengths > 0
    normals[valid_normals] /= normal_lengths[valid_normals, None]
    normals[~valid_normals] = 0

    pixel_source_units = source_units_from_mm(config.pixel_mm, config.scale)
    depth_tolerance_source_units = source_units_from_mm(config.depth_tolerance_mm, config.scale)
    directions = fibonacci_sphere(config.view_count)
    visible_counts = np.zeros(face_count, dtype=np.uint16)

    if log:
        log(
            f"Projection culling {face_count:,} faces from {config.view_count} views "
            f"at {config.pixel_mm:g} mm/pixel"
        )

    cull_started = time.time()
    for view_index, direction in enumerate(directions, start=1):
        if log and (view_index == 1 or view_index == len(directions) or view_index % 12 == 0):
            log(f"  view {view_index}/{len(directions)}")
        newly_visible = visible_faces_for_direction(
            centers=centers,
            normals=normals,
            direction=direction,
            pixel_source_units=pixel_source_units,
            depth_tolerance_source_units=depth_tolerance_source_units,
            normal_threshold=config.normal_threshold,
        )
        visible_counts[newly_visible] += 1

    cull_seconds = time.time() - cull_started
    keep_mask = visible_counts >= config.min_visible_views
    kept_face_count = int(np.count_nonzero(keep_mask))
    if kept_face_count == 0:
        raise RuntimeError("Projection cull removed every face; reduce pixel size or view constraints")

    kept_vertices, kept_faces = compact_faces(vertices, faces[keep_mask])
    culled_mesh = pymeshlab.Mesh(
        vertex_matrix=kept_vertices.astype(np.float64, copy=False),
        face_matrix=kept_faces.astype(np.int32, copy=False),
    )
    out_ms = pymeshlab.MeshSet()
    out_ms.add_mesh(culled_mesh, "exterior_visible_faces")
    out_ms.compute_normal_per_face()
    out_ms.meshing_remove_duplicate_vertices()
    out_ms.meshing_remove_duplicate_faces()
    out_ms.meshing_remove_unreferenced_vertices()
    culled = out_ms.current_mesh()
    culled_bounds_source = bounds_for(culled)
    culled_topology_source = topology_for(out_ms)

    output_bounds = None
    output_topology = None
    output_scale_factor = 1.0
    if output is not None:
        output.parent.mkdir(parents=True, exist_ok=True)
        if not config.keep_source_scale:
            if config.exact_final_size:
                selected_culled_units = selected_dimension_units(culled_bounds_source, config.scale.dimension)
                output_scale_factor = config.scale.target_mm / selected_culled_units
            else:
                output_scale_factor = config.scale.mm_per_source_unit
            out_ms.compute_matrix_from_translation_rotation_scale(
                scalex=output_scale_factor,
                scaley=output_scale_factor,
                scalez=output_scale_factor,
                freeze=True,
            )
        output_bounds = bounds_for(out_ms.current_mesh())
        output_topology = topology_for(out_ms)
        out_ms.save_current_mesh(str(output))

    visible_histogram = visible_count_histogram(visible_counts)
    return {
        "source": str(source),
        "output": str(output) if output is not None else None,
        "output_units": "source" if config.keep_source_scale else "millimetres",
        "source_faces": face_count,
        "source_vertices": int(vertices.shape[0]),
        "source_bounds": source_bounds,
        "source_topology": source_topology,
        "scale": scale_to_dict(config.scale),
        "method": "projection_visibility_cull",
        "pixel_mm": config.pixel_mm,
        "depth_tolerance_mm": config.depth_tolerance_mm,
        "view_count": config.view_count,
        "min_visible_views": config.min_visible_views,
        "normal_threshold": config.normal_threshold,
        "pixel_source_units": pixel_source_units,
        "depth_tolerance_source_units": depth_tolerance_source_units,
        "kept_faces": int(culled.face_number()),
        "kept_vertices": int(culled.vertex_number()),
        "removed_faces": int(face_count - kept_face_count),
        "removed_face_fraction": float((face_count - kept_face_count) / face_count),
        "visible_count_histogram": visible_histogram,
        "culled_bounds_source": culled_bounds_source,
        "culled_topology_source": culled_topology_source,
        "exact_final_size": config.exact_final_size,
        "output_scale_factor": output_scale_factor,
        "output_bounds": output_bounds,
        "output_topology": output_topology,
        "cull_seconds": cull_seconds,
        "total_seconds": time.time() - started,
    }


def smooth_mesh(
    source: Path,
    config: SmoothConfig,
    *,
    output: Path | None = None,
    log: LogFn | None = None,
) -> dict[str, Any]:
    """Apply non-shrinking Taubin smoothing and keep print scale exact."""
    if config.steps <= 0:
        raise ValueError("steps must be positive")

    started = time.time()
    ms, _original_id, mesh, source_bounds = load_source(source)
    source_topology = topology_for(ms)

    if log:
        log(f"Taubin smoothing {mesh.face_number():,} faces for {config.steps} steps")

    smooth_started = time.time()
    ms.apply_coord_taubin_smoothing(
        lambda_=config.lambda_,
        mu=config.mu,
        stepsmoothnum=config.steps,
    )
    smooth_seconds = time.time() - smooth_started
    smoothed = ms.current_mesh()
    smoothed_bounds_source = bounds_for(smoothed)
    smoothed_topology_source = topology_for(ms)

    output_bounds = None
    output_topology = None
    output_scale_factor = 1.0
    if output is not None:
        output.parent.mkdir(parents=True, exist_ok=True)
        if not config.keep_source_scale:
            if config.exact_final_size:
                selected_smoothed_units = selected_dimension_units(smoothed_bounds_source, config.scale.dimension)
                output_scale_factor = config.scale.target_mm / selected_smoothed_units
            else:
                output_scale_factor = config.scale.mm_per_source_unit
            ms.compute_matrix_from_translation_rotation_scale(
                scalex=output_scale_factor,
                scaley=output_scale_factor,
                scalez=output_scale_factor,
                freeze=True,
            )
        output_bounds = bounds_for(ms.current_mesh())
        output_topology = topology_for(ms)
        ms.save_current_mesh(str(output))

    return {
        "source": str(source),
        "output": str(output) if output is not None else None,
        "output_units": "source" if config.keep_source_scale else "millimetres",
        "source_faces": int(mesh.face_number()),
        "source_vertices": int(mesh.vertex_number()),
        "source_bounds": source_bounds,
        "source_topology": source_topology,
        "scale": scale_to_dict(config.scale),
        "method": "taubin_smoothing",
        "steps": config.steps,
        "lambda": config.lambda_,
        "mu": config.mu,
        "smoothed_faces": int(smoothed.face_number()),
        "smoothed_vertices": int(smoothed.vertex_number()),
        "smoothed_bounds_source": smoothed_bounds_source,
        "smoothed_topology_source": smoothed_topology_source,
        "exact_final_size": config.exact_final_size,
        "output_scale_factor": output_scale_factor,
        "output_bounds": output_bounds,
        "output_topology": output_topology,
        "smooth_seconds": smooth_seconds,
        "total_seconds": time.time() - started,
    }


def remove_small_components(
    source: Path,
    config: ComponentCleanConfig,
    *,
    output: Path | None = None,
    log: LogFn | None = None,
) -> dict[str, Any]:
    """Remove disconnected mesh components below a face-count threshold."""
    if config.min_faces <= 0:
        raise ValueError("min_faces must be positive")

    started = time.time()
    before_components = component_summary_for_path(source)
    ms, _original_id, original, source_bounds = load_source(source)
    source_topology = topology_for(ms)
    source_faces = int(original.face_number())
    source_vertices = int(original.vertex_number())

    if log:
        log(f"Removing connected components with fewer than {config.min_faces:,} faces")

    ms.meshing_remove_connected_component_by_face_number(
        mincomponentsize=config.min_faces,
        removeunref=True,
    )
    cleaned = ms.current_mesh()
    cleaned_faces = int(cleaned.face_number())
    cleaned_vertices = int(cleaned.vertex_number())
    cleaned_bounds = bounds_for(cleaned)
    cleaned_topology = topology_for(ms)

    output_bounds = None
    output_topology = None
    if output is not None:
        output.parent.mkdir(parents=True, exist_ok=True)
        output_bounds = cleaned_bounds
        output_topology = cleaned_topology
        ms.save_current_mesh(str(output))

    after_components = component_summary_for_path(output) if output is not None else []
    return {
        "source": str(source),
        "output": str(output) if output is not None else None,
        "source_faces": source_faces,
        "source_vertices": source_vertices,
        "source_bounds": source_bounds,
        "source_topology": source_topology,
        "method": "remove_small_connected_components",
        "min_faces": config.min_faces,
        "before_components": before_components,
        "after_components": after_components,
        "removed_faces": source_faces - cleaned_faces,
        "removed_vertices": source_vertices - cleaned_vertices,
        "cleaned_faces": cleaned_faces,
        "cleaned_vertices": cleaned_vertices,
        "cleaned_bounds": cleaned_bounds,
        "cleaned_topology": cleaned_topology,
        "output_bounds": output_bounds,
        "output_topology": output_topology,
        "total_seconds": time.time() - started,
    }


def close_boundary_holes(
    source: Path,
    config: HoleCloseConfig,
    *,
    output: Path | None = None,
    log: LogFn | None = None,
) -> dict[str, Any]:
    """Patch open boundary loops up to a maximum edge count."""
    if config.max_hole_edges <= 0:
        raise ValueError("max_hole_edges must be positive")

    started = time.time()
    ms, _original_id, mesh, source_bounds = load_source(source)
    source_topology = topology_for(ms)
    source_faces = int(mesh.face_number())
    source_vertices = int(mesh.vertex_number())

    if log:
        log(f"Closing boundary holes up to {config.max_hole_edges:,} boundary edges")

    repair_topology = None
    if config.repair_non_manifold:
        if log:
            log("Repairing non-manifold edges/vertices before hole closure")
        try:
            ms.meshing_repair_non_manifold_edges(method="Remove Faces")
            ms.meshing_repair_non_manifold_vertices(vertdispratio=0)
            repair_topology = topology_for(ms)
        except Exception as exc:
            if log:
                log(f"Non-manifold repair skipped/failed: {exc}")

    close_started = time.time()
    ms.meshing_close_holes(
        maxholesize=config.max_hole_edges,
        selected=False,
        newfaceselected=True,
        selfintersection=True,
        refinehole=config.refine,
    )
    if config.repair_non_manifold:
        try:
            ms.meshing_repair_non_manifold_vertices(vertdispratio=0)
            ms.meshing_remove_unreferenced_vertices()
        except Exception as exc:
            if log:
                log(f"Post-close non-manifold cleanup skipped/failed: {exc}")
    close_seconds = time.time() - close_started

    closed = ms.current_mesh()
    closed_faces = int(closed.face_number())
    closed_vertices = int(closed.vertex_number())
    closed_bounds_source = bounds_for(closed)
    closed_topology_source = topology_for(ms)

    output_bounds = None
    output_topology = None
    output_scale_factor = 1.0
    if output is not None:
        output.parent.mkdir(parents=True, exist_ok=True)
        if not config.keep_source_scale:
            if config.exact_final_size:
                selected_closed_units = selected_dimension_units(closed_bounds_source, config.scale.dimension)
                output_scale_factor = config.scale.target_mm / selected_closed_units
            else:
                output_scale_factor = config.scale.mm_per_source_unit
            ms.compute_matrix_from_translation_rotation_scale(
                scalex=output_scale_factor,
                scaley=output_scale_factor,
                scalez=output_scale_factor,
                freeze=True,
            )
        output_bounds = bounds_for(ms.current_mesh())
        output_topology = topology_for(ms)
        ms.save_current_mesh(str(output))

    before_boundary_edges = (source_topology or {}).get("boundary_edges")
    after_boundary_edges = (closed_topology_source or {}).get("boundary_edges")
    return {
        "source": str(source),
        "output": str(output) if output is not None else None,
        "output_units": "source" if config.keep_source_scale else "millimetres",
        "source_faces": source_faces,
        "source_vertices": source_vertices,
        "source_bounds": source_bounds,
        "source_topology": source_topology,
        "method": "close_boundary_holes",
        "max_hole_edges": config.max_hole_edges,
        "refine": config.refine,
        "repair_non_manifold": config.repair_non_manifold,
        "repair_topology": repair_topology,
        "closed_faces": closed_faces,
        "closed_vertices": closed_vertices,
        "added_faces": closed_faces - source_faces,
        "added_vertices": closed_vertices - source_vertices,
        "boundary_edges_before": before_boundary_edges,
        "boundary_edges_after": after_boundary_edges,
        "closed_bounds_source": closed_bounds_source,
        "closed_topology_source": closed_topology_source,
        "exact_final_size": config.exact_final_size,
        "output_scale_factor": output_scale_factor,
        "output_bounds": output_bounds,
        "output_topology": output_topology,
        "close_seconds": close_seconds,
        "total_seconds": time.time() - started,
    }


def polish_mesh(
    source: Path,
    config: PolishConfig,
    *,
    output: Path | None = None,
    log: LogFn | None = None,
) -> dict[str, Any]:
    """Smooth and regularize an already-watertight wrapped mesh.

    This is a second-stage operation for alpha-wrap faceting. It does not try to
    repair topology; it assumes the input is already closed and then preserves
    that topology while evening out local triangle artifacts.
    """
    if config.pre_taubin_steps < 0:
        raise ValueError("pre_taubin_steps must not be negative")
    if config.isotropic_iterations < 0:
        raise ValueError("isotropic_iterations must not be negative")
    if config.targetlen_percent <= 0:
        raise ValueError("targetlen_percent must be positive")
    if config.max_surface_dist_percent <= 0:
        raise ValueError("max_surface_dist_percent must be positive")
    if config.post_taubin_steps < 0:
        raise ValueError("post_taubin_steps must not be negative")

    started = time.time()
    ms, _original_id, mesh, source_bounds = load_source(source)
    source_topology = topology_for(ms)

    if log:
        log(f"Polishing {mesh.face_number():,} faces")

    if config.pre_taubin_steps:
        if log:
            log(f"  pre Taubin smoothing: {config.pre_taubin_steps} steps")
        ms.apply_coord_taubin_smoothing(
            lambda_=0.5,
            mu=-0.53,
            stepsmoothnum=config.pre_taubin_steps,
            selected=False,
        )

    if config.isotropic_iterations:
        if log:
            log(
                "  isotropic remeshing: "
                f"{config.isotropic_iterations} iterations, target {config.targetlen_percent:g}%"
            )
        ms.meshing_isotropic_explicit_remeshing(
            iterations=config.isotropic_iterations,
            adaptive=False,
            selectedonly=False,
            targetlen=PercentageValue(config.targetlen_percent),
            featuredeg=config.feature_deg,
            checksurfdist=True,
            maxsurfdist=PercentageValue(config.max_surface_dist_percent),
            splitflag=True,
            collapseflag=True,
            swapflag=True,
            smoothflag=True,
            reprojectflag=True,
        )

    if config.post_taubin_steps:
        if log:
            log(f"  post Taubin smoothing: {config.post_taubin_steps} steps")
        ms.apply_coord_taubin_smoothing(
            lambda_=0.5,
            mu=-0.53,
            stepsmoothnum=config.post_taubin_steps,
            selected=False,
        )

    ms.meshing_remove_duplicate_vertices()
    ms.meshing_remove_duplicate_faces()
    ms.meshing_remove_unreferenced_vertices()
    polished = ms.current_mesh()
    polished_bounds = bounds_for(polished)
    polished_topology = topology_for(ms)

    output_bounds = None
    output_topology = None
    if output is not None:
        output.parent.mkdir(parents=True, exist_ok=True)
        ms.save_current_mesh(str(output))
        output_bounds = polished_bounds
        output_topology = polished_topology

    return {
        "source": str(source),
        "output": str(output) if output is not None else None,
        "output_units": "source",
        "source_faces": int(mesh.face_number()),
        "source_vertices": int(mesh.vertex_number()),
        "source_bounds": source_bounds,
        "source_topology": source_topology,
        "method": "taubin_isotropic_polish",
        "pre_taubin_steps": config.pre_taubin_steps,
        "isotropic_iterations": config.isotropic_iterations,
        "targetlen_percent": config.targetlen_percent,
        "max_surface_dist_percent": config.max_surface_dist_percent,
        "feature_deg": config.feature_deg,
        "post_taubin_steps": config.post_taubin_steps,
        "polished_faces": int(polished.face_number()),
        "polished_vertices": int(polished.vertex_number()),
        "polished_bounds": polished_bounds,
        "polished_topology": polished_topology,
        "output_bounds": output_bounds,
        "output_topology": output_topology,
        "total_seconds": time.time() - started,
    }


def visible_faces_for_direction(
    *,
    centers: np.ndarray,
    normals: np.ndarray,
    direction: np.ndarray,
    pixel_source_units: float,
    depth_tolerance_source_units: float,
    normal_threshold: float | None,
) -> np.ndarray:
    u, v = orthonormal_basis(direction)
    px = centers @ u
    py = centers @ v
    depth = centers @ direction

    min_x = float(px.min())
    min_y = float(py.min())
    ix = np.floor((px - min_x) / pixel_source_units).astype(np.int64)
    iy = np.floor((py - min_y) / pixel_source_units).astype(np.int64)
    nx = int(ix.max()) + 1
    ny = int(iy.max()) + 1
    keys = ix + iy * nx

    valid = np.ones(len(centers), dtype=bool)
    if normal_threshold is not None:
        valid &= (normals @ direction) >= normal_threshold
    if not np.any(valid):
        return np.zeros(len(centers), dtype=bool)

    valid_keys = keys[valid]
    valid_depth = depth[valid]
    zbuffer = np.full(nx * ny, -np.inf, dtype=np.float64)
    np.maximum.at(zbuffer, valid_keys, valid_depth)

    visible_valid = valid_depth >= (zbuffer[valid_keys] - depth_tolerance_source_units)
    visible = np.zeros(len(centers), dtype=bool)
    valid_indices = np.flatnonzero(valid)
    visible[valid_indices[visible_valid]] = True
    return visible


def orthonormal_basis(direction: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
    direction = np.asarray(direction, dtype=np.float64)
    direction = direction / np.linalg.norm(direction)
    helper = np.array([0.0, 0.0, 1.0])
    if abs(float(direction @ helper)) > 0.9:
        helper = np.array([0.0, 1.0, 0.0])
    u = np.cross(direction, helper)
    u /= np.linalg.norm(u)
    v = np.cross(direction, u)
    v /= np.linalg.norm(v)
    return u, v


def fibonacci_sphere(count: int) -> np.ndarray:
    indices = np.arange(count, dtype=np.float64) + 0.5
    phi = np.arccos(1.0 - 2.0 * indices / count)
    theta = np.pi * (1.0 + 5.0**0.5) * indices
    x = np.cos(theta) * np.sin(phi)
    y = np.sin(theta) * np.sin(phi)
    z = np.cos(phi)
    return np.column_stack([x, y, z]).astype(np.float64)


def compact_faces(vertices: np.ndarray, faces: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
    unique_vertices, inverse = np.unique(faces.reshape(-1), return_inverse=True)
    compact_vertices = vertices[unique_vertices]
    compact_faces_matrix = inverse.reshape((-1, 3))
    return compact_vertices, compact_faces_matrix


def visible_count_histogram(visible_counts: np.ndarray) -> dict[str, int]:
    values, counts = np.unique(visible_counts, return_counts=True)
    return {str(int(value)): int(count) for value, count in zip(values, counts)}


def simplify_candidate(
    source: Path,
    target_faces: int,
    config: SimplifyConfig,
    *,
    output: Path | None = None,
    diagnostic_prefix: Path | None = None,
    log: LogFn | None = None,
) -> dict[str, Any]:
    if target_faces <= 0:
        raise ValueError("target_faces must be positive")
    if config.hausdorff_samples <= 0:
        raise ValueError("hausdorff_samples must be positive")

    started = time.time()
    ms, original_id, _original, source_bounds = load_source(source)

    ms.generate_copy_of_current_mesh()
    candidate_id = ms.current_mesh_id()

    if log:
        log(f"Simplifying to {target_faces:,} faces")
    simplify_started = time.time()
    ms.meshing_decimation_quadric_edge_collapse(
        targetfacenum=target_faces,
        qualitythr=config.quality_threshold,
        preservenormal=True,
        preservetopology=True,
        optimalplacement=True,
        planarquadric=config.planar_quadric,
        autoclean=True,
    )
    simplify_seconds = time.time() - simplify_started
    simplified = ms.current_mesh()
    simplified_faces = int(simplified.face_number())
    simplified_vertices = int(simplified.vertex_number())

    sample_layers: list[tuple[str, list[int]]] = []
    save_samples = diagnostic_prefix is not None
    maxdist = PercentageValue(config.hausdorff_maxdist_percent)

    if log:
        log("Measuring two-way sampled Hausdorff error")
    hausdorff_started = time.time()
    before = ms.mesh_number()
    original_to_simplified = ms.get_hausdorff_distance(
        sampledmesh=original_id,
        targetmesh=candidate_id,
        savesample=save_samples,
        samplevert=True,
        sampleedge=True,
        sampleface=True,
        samplenum=config.hausdorff_samples,
        maxdist=maxdist,
    )
    after = ms.mesh_number()
    if save_samples:
        sample_layers.append(("original_to_simplified", list(range(before, after))))

    before = ms.mesh_number()
    simplified_to_original = ms.get_hausdorff_distance(
        sampledmesh=candidate_id,
        targetmesh=original_id,
        savesample=save_samples,
        samplevert=True,
        sampleedge=True,
        sampleface=True,
        samplenum=config.hausdorff_samples,
        maxdist=maxdist,
    )
    after = ms.mesh_number()
    if save_samples:
        sample_layers.append(("simplified_to_original", list(range(before, after))))
    hausdorff_seconds = time.time() - hausdorff_started

    max_error_source_units = max(original_to_simplified["max"], simplified_to_original["max"])
    mean_error_source_units = max(original_to_simplified["mean"], simplified_to_original["mean"])
    within_tolerance = max_error_source_units <= config.scale.tolerance_source_units

    diagnostic_files: list[str] = []
    if diagnostic_prefix is not None:
        diagnostic_files = save_error_diagnostics(
            ms=ms,
            sample_layers=sample_layers,
            prefix=diagnostic_prefix,
            config=config,
        )

    output_bounds = None
    output_topology = None
    if output is not None:
        output.parent.mkdir(parents=True, exist_ok=True)
        ms.set_current_mesh(candidate_id)
        if not config.keep_source_scale:
            ms.compute_matrix_from_translation_rotation_scale(
                scalex=config.scale.mm_per_source_unit,
                scaley=config.scale.mm_per_source_unit,
                scalez=config.scale.mm_per_source_unit,
                freeze=True,
            )
        output_bounds = bounds_for(ms.current_mesh())
        output_topology = topology_for(ms)
        ms.save_current_mesh(str(output))

    return {
        "target_faces": int(target_faces),
        "actual_faces": simplified_faces,
        "actual_vertices": simplified_vertices,
        "max_error_source_units": float(max_error_source_units),
        "mean_error_source_units": float(mean_error_source_units),
        "max_error_um": float(um_from_units(max_error_source_units, config.scale)),
        "mean_error_um": float(um_from_units(mean_error_source_units, config.scale)),
        "within_tolerance": bool(within_tolerance),
        "original_to_simplified": original_to_simplified,
        "simplified_to_original": simplified_to_original,
        "simplify_seconds": simplify_seconds,
        "hausdorff_seconds": hausdorff_seconds,
        "total_seconds": time.time() - started,
        "source_bounds": source_bounds,
        "scale": scale_to_dict(config.scale),
        "output": str(output) if output is not None else None,
        "output_units": "source" if config.keep_source_scale else "millimetres",
        "output_bounds": output_bounds,
        "output_topology": output_topology,
        "diagnostic_files": diagnostic_files,
    }


def save_error_diagnostics(
    *,
    ms: pymeshlab.MeshSet,
    sample_layers: list[tuple[str, list[int]]],
    prefix: Path,
    config: SimplifyConfig,
) -> list[str]:
    prefix.parent.mkdir(parents=True, exist_ok=True)
    written: list[str] = []
    rows: list[dict[str, Any]] = []

    for direction, layer_ids in sample_layers:
        for layer_ordinal, layer_id in enumerate(layer_ids):
            ms.set_current_mesh(layer_id)
            mesh = ms.current_mesh()
            if mesh.vertex_number() == 0 or not mesh.has_vertex_scalar():
                continue

            if config.save_diagnostic_pointclouds:
                ply_path = prefix.with_name(f"{prefix.name}_{direction}_samples_{layer_ordinal}.ply")
                ms.save_current_mesh(str(ply_path))
                written.append(str(ply_path))

            vertices = np.asarray(mesh.vertex_matrix())
            scalars = np.asarray(mesh.vertex_scalar_array())
            if len(scalars) == 0:
                continue

            order = np.argsort(scalars)[::-1][: config.diagnostic_top_points]
            for rank, idx in enumerate(order, start=1):
                x, y, z = vertices[idx]
                error_source_units = float(scalars[idx])
                rows.append(
                    {
                        "direction": direction,
                        "sample_layer": layer_ordinal,
                        "rank": rank,
                        "x_source": float(x),
                        "y_source": float(y),
                        "z_source": float(z),
                        "x_mm_at_output_scale": float(x * config.scale.mm_per_source_unit),
                        "y_mm_at_output_scale": float(y * config.scale.mm_per_source_unit),
                        "z_mm_at_output_scale": float(z * config.scale.mm_per_source_unit),
                        "error_source_units": error_source_units,
                        "error_um_at_output_scale": um_from_units(error_source_units, config.scale),
                    }
                )

    if rows:
        csv_path = prefix.with_name(f"{prefix.name}_worst_error_points.csv")
        with csv_path.open("w", newline="") as handle:
            writer = csv.DictWriter(handle, fieldnames=list(rows[0].keys()))
            writer.writeheader()
            writer.writerows(rows)
        written.append(str(csv_path))

    return written


def optimize_face_count(
    source: Path,
    config: SimplifyConfig,
    *,
    min_faces: int,
    max_faces: int,
    step_faces: int,
    log: LogFn | None = None,
) -> tuple[int, list[dict[str, Any]], dict[str, Any] | None]:
    if min_faces <= 0 or max_faces <= 0:
        raise ValueError("min_faces and max_faces must be positive")
    if min_faces >= max_faces:
        raise ValueError("min_faces must be lower than max_faces")
    if step_faces <= 0:
        raise ValueError("step_faces must be positive")

    history: list[dict[str, Any]] = []
    last_fail: dict[str, Any] | None = None

    if log:
        log(f"Checking lower bound {min_faces:,} faces")
    low_result = simplify_candidate(source, min_faces, config)
    history.append(low_result)
    if low_result["within_tolerance"]:
        return min_faces, history, None
    last_fail = low_result

    if log:
        log(f"Checking upper bound {max_faces:,} faces")
    high_result = simplify_candidate(source, max_faces, config)
    history.append(high_result)
    if not high_result["within_tolerance"]:
        raise RuntimeError(
            f"max_faces={max_faces:,} still exceeds tolerance; raise max_faces "
            "or relax the tolerance."
        )

    low = min_faces
    high = max_faces
    best = max_faces
    while high - low > step_faces:
        midpoint = (low + high) // 2
        if log:
            log(f"Checking midpoint {midpoint:,} faces")
        result = simplify_candidate(source, midpoint, config)
        history.append(result)
        if result["within_tolerance"]:
            best = midpoint
            high = midpoint
        else:
            last_fail = result
            low = midpoint

    return best, history, last_fail


def default_output_path(
    source: Path,
    *,
    scale: PrintScale,
    label: str,
    target_faces: int | None = None,
    suffix: str = ".stl",
) -> Path:
    size_label = format_number_for_name(scale.target_mm)
    tolerance_label = format_number_for_name(scale.tolerance_um)
    face_label = f"_{target_faces // 1000}k" if target_faces else ""
    return source.with_name(
        f"{source.stem}_{size_label}mm_{scale.dimension.value}_{tolerance_label}um_{label}{face_label}{suffix}"
    )


def format_number_for_name(value: float) -> str:
    text = f"{value:g}"
    return text.replace(".", "p")


def report_path_for(output: Path) -> Path:
    return output.with_suffix(output.suffix + ".report.json")


def diagnostic_prefix_for(output: Path) -> Path:
    return output.with_suffix("")


def fail_diagnostic_prefix_for(output: Path, target_faces: int) -> Path:
    base = output.with_suffix("")
    return base.with_name(f"{base.name}_fail_{target_faces}")


def write_json(path: Path, data: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(jsonable(data), indent=2) + "\n")


def jsonable(value: Any) -> Any:
    if isinstance(value, Enum):
        return value.value
    if isinstance(value, Path):
        return str(value)
    if isinstance(value, np.ndarray):
        return value.tolist()
    if isinstance(value, np.generic):
        return value.item()
    if isinstance(value, dict):
        return {str(key): jsonable(item) for key, item in value.items()}
    if isinstance(value, (list, tuple)):
        return [jsonable(item) for item in value]
    return value


def slugify(value: str) -> str:
    slug = re.sub(r"[^A-Za-z0-9_.-]+", "_", value.strip())
    return slug.strip("_") or "mesh"

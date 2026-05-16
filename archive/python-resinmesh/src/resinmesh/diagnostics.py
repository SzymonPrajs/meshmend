"""Diagnostic rendering and topology reports for mesh repair research."""

from __future__ import annotations

from dataclasses import dataclass
import json
import math
import time
from pathlib import Path
from typing import Any

import imageio.v3 as iio
import matplotlib
import numpy as np
import open3d as o3d
from PIL import Image, ImageDraw
import pymeshlab

from .core import bounds_for, component_summary_for_path, jsonable, topology_for

matplotlib.use("Agg")
from matplotlib import colormaps  # noqa: E402


@dataclass(frozen=True)
class RayDiagnosticConfig:
    view_count: int = 16
    image_size: int = 128
    max_hits: int = 6
    padding: float = 0.08
    hit_epsilon_fraction: float = 1e-5
    include_self_intersections: bool = False


def load_mesh_arrays(source: Path) -> tuple[np.ndarray, np.ndarray, dict[str, Any], dict[str, Any] | None]:
    ms = pymeshlab.MeshSet()
    ms.load_new_mesh(str(source))
    mesh = ms.current_mesh()
    vertices = np.asarray(mesh.vertex_matrix(), dtype=np.float32)
    faces = np.asarray(mesh.face_matrix(), dtype=np.uint32)
    return vertices, faces, bounds_for(mesh), topology_for(ms)


def diagnose_mesh(
    source: Path,
    output_dir: Path,
    config: RayDiagnosticConfig,
) -> dict[str, Any]:
    started = time.time()
    output_dir.mkdir(parents=True, exist_ok=True)
    view_dir = output_dir / "views"
    view_dir.mkdir(parents=True, exist_ok=True)

    vertices, faces, bounds, topology = load_mesh_arrays(source)
    components = component_summary_for_path(source)
    self_intersections = (
        self_intersection_summary(source) if config.include_self_intersections else {"enabled": False}
    )

    scene = build_raycast_scene(vertices, faces)
    directions = fibonacci_sphere(config.view_count)
    face_first_hit_counts = np.zeros(len(faces), dtype=np.uint16)
    view_reports: list[dict[str, Any]] = []
    contact_tiles: list[Image.Image] = []

    for view_index, camera_direction in enumerate(directions):
        result = raycast_view(
            scene=scene,
            vertices=vertices,
            faces=faces,
            camera_direction=camera_direction,
            image_size=config.image_size,
            max_hits=config.max_hits,
            padding=config.padding,
            hit_epsilon_fraction=config.hit_epsilon_fraction,
        )
        valid_first = result["primitive_ids"] >= 0
        if np.any(valid_first):
            ids, counts = np.unique(result["primitive_ids"][valid_first], return_counts=True)
            clamped = np.minimum(counts, np.iinfo(face_first_hit_counts.dtype).max).astype(face_first_hit_counts.dtype)
            face_first_hit_counts[ids] = np.maximum(face_first_hit_counts[ids], clamped)

        images = write_view_images(view_dir, view_index, result, config.max_hits)
        view_reports.append(view_metrics(view_index, result, images))
        contact_tiles.extend(make_contact_tiles(view_index, images))

    visible_face_count = int(np.count_nonzero(face_first_hit_counts))
    aggregate = aggregate_view_metrics(view_reports)
    report = {
        "source": str(source),
        "output_dir": str(output_dir),
        "config": config.__dict__,
        "mesh": {
            "vertices": int(vertices.shape[0]),
            "faces": int(faces.shape[0]),
            "bounds": bounds,
            "topology": topology,
            "components": components,
            "self_intersections": self_intersections,
        },
        "visibility": {
            "sampled_first_visible_faces": visible_face_count,
            "sampled_first_visible_face_fraction": float(visible_face_count / max(len(faces), 1)),
            "aggregate": aggregate,
            "views": view_reports,
        },
        "interpretation": interpret_report(topology, aggregate),
        "seconds": time.time() - started,
    }

    contact_sheet_path = output_dir / "contact_sheet.png"
    make_contact_sheet(contact_tiles, contact_sheet_path)
    report["contact_sheet"] = str(contact_sheet_path)

    report_path = output_dir / "diagnostics.json"
    report_path.write_text(json.dumps(jsonable(report), indent=2) + "\n")
    markdown_path = output_dir / "diagnostics.md"
    markdown_path.write_text(render_markdown_report(report) + "\n")
    report["report"] = str(report_path)
    report["markdown"] = str(markdown_path)
    return report


def compare_meshes(
    reference: Path,
    candidate: Path,
    output_dir: Path,
    config: RayDiagnosticConfig,
) -> dict[str, Any]:
    started = time.time()
    output_dir.mkdir(parents=True, exist_ok=True)
    view_dir = output_dir / "views"
    view_dir.mkdir(parents=True, exist_ok=True)

    ref_vertices, ref_faces, ref_bounds, ref_topology = load_mesh_arrays(reference)
    cand_vertices, cand_faces, cand_bounds, cand_topology = load_mesh_arrays(candidate)
    ref_scene = build_raycast_scene(ref_vertices, ref_faces)
    cand_scene = build_raycast_scene(cand_vertices, cand_faces)

    directions = fibonacci_sphere(config.view_count)
    view_reports: list[dict[str, Any]] = []
    contact_tiles: list[Image.Image] = []

    for view_index, camera_direction in enumerate(directions):
        frame = make_view_frame(
            vertices=ref_vertices,
            camera_direction=camera_direction,
            image_size=config.image_size,
            padding=config.padding,
        )
        ref_result = raycast_frame(
            scene=ref_scene,
            frame=frame,
            max_hits=config.max_hits,
            hit_epsilon_fraction=config.hit_epsilon_fraction,
        )
        cand_result = raycast_frame(
            scene=cand_scene,
            frame=frame,
            max_hits=config.max_hits,
            hit_epsilon_fraction=config.hit_epsilon_fraction,
        )
        images = write_compare_images(view_dir, view_index, ref_result, cand_result, config.max_hits)
        view_report = compare_view_metrics(view_index, ref_result, cand_result, images, float(frame["diagonal"]))
        view_reports.append(view_report)
        contact_tiles.extend(make_compare_contact_tiles(view_index, images))

    aggregate = aggregate_compare_metrics(view_reports)
    report = {
        "reference": str(reference),
        "candidate": str(candidate),
        "output_dir": str(output_dir),
        "config": config.__dict__,
        "reference_mesh": {
            "vertices": int(ref_vertices.shape[0]),
            "faces": int(ref_faces.shape[0]),
            "bounds": ref_bounds,
            "topology": ref_topology,
        },
        "candidate_mesh": {
            "vertices": int(cand_vertices.shape[0]),
            "faces": int(cand_faces.shape[0]),
            "bounds": cand_bounds,
            "topology": cand_topology,
        },
        "comparison": {
            "aggregate": aggregate,
            "views": view_reports,
        },
        "interpretation": interpret_comparison(cand_topology, aggregate),
        "seconds": time.time() - started,
    }

    contact_sheet_path = output_dir / "comparison_sheet.png"
    make_contact_sheet(contact_tiles, contact_sheet_path)
    report["contact_sheet"] = str(contact_sheet_path)
    report_path = output_dir / "comparison.json"
    report_path.write_text(json.dumps(jsonable(report), indent=2) + "\n")
    markdown_path = output_dir / "comparison.md"
    markdown_path.write_text(render_compare_markdown(report) + "\n")
    report["report"] = str(report_path)
    report["markdown"] = str(markdown_path)
    return report


def build_raycast_scene(vertices: np.ndarray, faces: np.ndarray) -> o3d.t.geometry.RaycastingScene:
    scene = o3d.t.geometry.RaycastingScene()
    scene.add_triangles(
        o3d.core.Tensor(vertices.astype(np.float32, copy=False)),
        o3d.core.Tensor(faces.astype(np.uint32, copy=False)),
    )
    return scene


def raycast_view(
    *,
    scene: o3d.t.geometry.RaycastingScene,
    vertices: np.ndarray,
    faces: np.ndarray,
    camera_direction: np.ndarray,
    image_size: int,
    max_hits: int,
    padding: float,
    hit_epsilon_fraction: float,
) -> dict[str, np.ndarray]:
    frame = make_view_frame(
        vertices=vertices,
        camera_direction=camera_direction,
        image_size=image_size,
        padding=padding,
    )
    return raycast_frame(
        scene=scene,
        frame=frame,
        max_hits=max_hits,
        hit_epsilon_fraction=hit_epsilon_fraction,
    )


def make_view_frame(
    *,
    vertices: np.ndarray,
    camera_direction: np.ndarray,
    image_size: int,
    padding: float,
) -> dict[str, np.ndarray | float | int]:
    if image_size <= 0:
        raise ValueError("image_size must be positive")

    camera_direction = normalize(camera_direction.astype(np.float32))
    ray_direction = -camera_direction
    u, v = orthonormal_basis(ray_direction)

    center = vertices.mean(axis=0).astype(np.float32)
    dims = vertices.max(axis=0) - vertices.min(axis=0)
    diagonal = float(np.linalg.norm(dims))
    projected_u = vertices @ u
    projected_v = vertices @ v
    min_u, max_u = float(projected_u.min()), float(projected_u.max())
    min_v, max_v = float(projected_v.min()), float(projected_v.max())
    span_u = max_u - min_u
    span_v = max_v - min_v
    min_u -= span_u * padding
    max_u += span_u * padding
    min_v -= span_v * padding
    max_v += span_v * padding

    xs = np.linspace(min_u, max_u, image_size, dtype=np.float32)
    ys = np.linspace(max_v, min_v, image_size, dtype=np.float32)
    grid_x, grid_y = np.meshgrid(xs, ys)
    origin_plane = center + camera_direction * (diagonal * 2.5)
    origins = origin_plane[None, None, :] + grid_x[..., None] * u + grid_y[..., None] * v
    directions = np.broadcast_to(ray_direction, origins.shape).astype(np.float32)
    return {
        "origins": origins.reshape((-1, 3)).astype(np.float32),
        "directions": directions.reshape((-1, 3)).astype(np.float32),
        "shape": np.array([image_size, image_size], dtype=np.int32),
        "camera_direction": camera_direction,
        "ray_direction": ray_direction,
        "diagonal": float(diagonal),
    }


def raycast_frame(
    *,
    scene: o3d.t.geometry.RaycastingScene,
    frame: dict[str, np.ndarray | float | int],
    max_hits: int,
    hit_epsilon_fraction: float,
) -> dict[str, np.ndarray]:
    if max_hits <= 0:
        raise ValueError("max_hits must be positive")

    flat_origins = np.asarray(frame["origins"], dtype=np.float32)
    flat_dirs = np.asarray(frame["directions"], dtype=np.float32)
    image_size = int(np.asarray(frame["shape"])[0])
    diagonal = float(frame["diagonal"])
    active = np.ones(len(flat_origins), dtype=bool)
    current_origins = flat_origins.copy()
    hit_counts = np.zeros(len(flat_origins), dtype=np.uint8)
    first_depth = np.full(len(flat_origins), np.nan, dtype=np.float32)
    first_primitive = np.full(len(flat_origins), -1, dtype=np.int64)
    first_normal = np.zeros((len(flat_origins), 3), dtype=np.float32)
    epsilon = max(diagonal * hit_epsilon_fraction, np.finfo(np.float32).eps * 100)

    for hit_index in range(max_hits):
        if not np.any(active):
            break
        active_indices = np.flatnonzero(active)
        rays = np.concatenate([current_origins[active_indices], flat_dirs[active_indices]], axis=1)
        result = scene.cast_rays(o3d.core.Tensor(rays, dtype=o3d.core.Dtype.Float32))
        t_hit = result["t_hit"].numpy()
        primitive_ids = result["primitive_ids"].numpy().astype(np.int64)
        primitive_normals = result["primitive_normals"].numpy().astype(np.float32)
        valid = np.isfinite(t_hit)
        if not np.any(valid):
            break

        valid_indices = active_indices[valid]
        hit_counts[valid_indices] += 1
        if hit_index == 0:
            first_depth[valid_indices] = t_hit[valid].astype(np.float32)
            first_primitive[valid_indices] = primitive_ids[valid]
            first_normal[valid_indices] = primitive_normals[valid]
        current_origins[valid_indices] = (
            current_origins[valid_indices] + flat_dirs[valid_indices] * (t_hit[valid, None] + epsilon)
        )
        active[active_indices[~valid]] = False

    shape = (image_size, image_size)
    return {
        "depth": first_depth.reshape(shape),
        "hit_count": hit_counts.reshape(shape),
        "primitive_ids": first_primitive.reshape(shape),
        "primitive_normals": first_normal.reshape((image_size, image_size, 3)),
        "camera_direction": np.asarray(frame["camera_direction"], dtype=np.float32),
        "ray_direction": np.asarray(frame["ray_direction"], dtype=np.float32),
    }


def self_intersection_summary(source: Path) -> dict[str, Any]:
    ms = pymeshlab.MeshSet()
    ms.load_new_mesh(str(source))
    started = time.time()
    try:
        ms.compute_selection_by_self_intersections_per_face()
        mesh = ms.current_mesh()
        selected = np.asarray(mesh.face_selection_array(), dtype=bool)
        return {
            "enabled": True,
            "selected_faces": int(np.count_nonzero(selected)),
            "selected_face_fraction": float(np.count_nonzero(selected) / max(mesh.face_number(), 1)),
            "seconds": time.time() - started,
        }
    except Exception as exc:
        return {"enabled": True, "error": str(exc), "seconds": time.time() - started}


def write_view_images(view_dir: Path, index: int, result: dict[str, np.ndarray], max_hits: int) -> dict[str, str]:
    prefix = view_dir / f"view_{index:03d}"
    hit_mask = np.isfinite(result["depth"])

    mask_image = (hit_mask.astype(np.uint8) * 255)
    depth_image = depth_to_uint8(result["depth"])
    hit_count_image = scalar_to_colormap(result["hit_count"].astype(np.float32), 0, max_hits, "magma")
    normal_image = normals_to_rgb(result["primitive_normals"], hit_mask)

    paths = {
        "mask": str(prefix.with_name(prefix.name + "_mask.png")),
        "depth": str(prefix.with_name(prefix.name + "_depth.png")),
        "hit_count": str(prefix.with_name(prefix.name + "_hit_count.png")),
        "normal": str(prefix.with_name(prefix.name + "_normal.png")),
    }
    iio.imwrite(paths["mask"], mask_image)
    iio.imwrite(paths["depth"], depth_image)
    iio.imwrite(paths["hit_count"], hit_count_image)
    iio.imwrite(paths["normal"], normal_image)
    return paths


def depth_to_uint8(depth: np.ndarray) -> np.ndarray:
    valid = np.isfinite(depth)
    out = np.zeros(depth.shape, dtype=np.uint8)
    if not np.any(valid):
        return out
    values = depth[valid]
    lo, hi = np.percentile(values, [2, 98])
    if hi <= lo:
        hi = float(values.max())
        lo = float(values.min())
    if hi <= lo:
        out[valid] = 255
        return out
    normalized = np.clip((depth[valid] - lo) / (hi - lo), 0, 1)
    out[valid] = (255 - normalized * 255).astype(np.uint8)
    return out


def scalar_to_colormap(values: np.ndarray, lo: float, hi: float, name: str) -> np.ndarray:
    if hi <= lo:
        hi = lo + 1
    normalized = np.clip((values - lo) / (hi - lo), 0, 1)
    rgba = colormaps[name](normalized)
    return (rgba[..., :3] * 255).astype(np.uint8)


def normals_to_rgb(normals: np.ndarray, hit_mask: np.ndarray) -> np.ndarray:
    rgb = np.zeros(normals.shape, dtype=np.uint8)
    encoded = np.clip(normals * 0.5 + 0.5, 0, 1)
    rgb[hit_mask] = (encoded[hit_mask] * 255).astype(np.uint8)
    return rgb


def view_metrics(index: int, result: dict[str, np.ndarray], images: dict[str, str]) -> dict[str, Any]:
    hit_count = result["hit_count"]
    rays = int(hit_count.size)
    hit = hit_count > 0
    multi = hit_count >= 2
    three_plus = hit_count >= 3
    return {
        "index": index,
        "rays": rays,
        "hit_rays": int(np.count_nonzero(hit)),
        "hit_fraction": float(np.count_nonzero(hit) / rays),
        "multi_hit_rays": int(np.count_nonzero(multi)),
        "multi_hit_fraction": float(np.count_nonzero(multi) / rays),
        "three_plus_hit_rays": int(np.count_nonzero(three_plus)),
        "three_plus_hit_fraction": float(np.count_nonzero(three_plus) / rays),
        "mean_hit_count": float(hit_count.mean()),
        "max_hit_count": int(hit_count.max(initial=0)),
        "images": images,
    }


def aggregate_view_metrics(views: list[dict[str, Any]]) -> dict[str, Any]:
    if not views:
        return {}
    rays = sum(view["rays"] for view in views)
    hit_rays = sum(view["hit_rays"] for view in views)
    multi = sum(view["multi_hit_rays"] for view in views)
    three_plus = sum(view["three_plus_hit_rays"] for view in views)
    weighted_hit_count = sum(view["mean_hit_count"] * view["rays"] for view in views)
    return {
        "views": len(views),
        "rays": rays,
        "hit_rays": hit_rays,
        "hit_fraction": float(hit_rays / max(rays, 1)),
        "multi_hit_rays": multi,
        "multi_hit_fraction": float(multi / max(rays, 1)),
        "three_plus_hit_rays": three_plus,
        "three_plus_hit_fraction": float(three_plus / max(rays, 1)),
        "mean_hit_count": float(weighted_hit_count / max(rays, 1)),
        "max_hit_count": max(view["max_hit_count"] for view in views),
    }


def make_contact_tiles(index: int, images: dict[str, str]) -> list[Image.Image]:
    tiles = []
    for label in ("normal", "depth", "hit_count"):
        image = Image.open(images[label]).convert("RGB")
        tiles.append(add_label(image, f"v{index:02d} {label}"))
    return tiles


def write_compare_images(
    view_dir: Path,
    index: int,
    reference: dict[str, np.ndarray],
    candidate: dict[str, np.ndarray],
    max_hits: int,
) -> dict[str, str]:
    prefix = view_dir / f"view_{index:03d}"
    ref_mask = np.isfinite(reference["depth"])
    cand_mask = np.isfinite(candidate["depth"])
    overlay = np.zeros((*ref_mask.shape, 3), dtype=np.uint8)
    overlay[ref_mask & cand_mask] = [220, 220, 220]
    overlay[ref_mask & ~cand_mask] = [255, 64, 64]
    overlay[~ref_mask & cand_mask] = [64, 128, 255]

    both = ref_mask & cand_mask
    depth_delta = np.zeros(ref_mask.shape, dtype=np.float32)
    if np.any(both):
        depth_delta[both] = np.abs(candidate["depth"][both] - reference["depth"][both])
        hi = np.percentile(depth_delta[both], 98)
    else:
        hi = 1.0
    depth_delta_image = scalar_to_colormap(depth_delta, 0, max(float(hi), 1e-9), "viridis")
    depth_delta_image[~both] = 0

    hit_delta = candidate["hit_count"].astype(np.float32) - reference["hit_count"].astype(np.float32)
    hit_delta_image = diverging_hit_delta(hit_delta, max_hits)

    paths = {
        "silhouette_overlay": str(prefix.with_name(prefix.name + "_silhouette_overlay.png")),
        "depth_delta": str(prefix.with_name(prefix.name + "_depth_delta.png")),
        "hit_delta": str(prefix.with_name(prefix.name + "_hit_delta.png")),
    }
    iio.imwrite(paths["silhouette_overlay"], overlay)
    iio.imwrite(paths["depth_delta"], depth_delta_image)
    iio.imwrite(paths["hit_delta"], hit_delta_image)
    return paths


def compare_view_metrics(
    index: int,
    reference: dict[str, np.ndarray],
    candidate: dict[str, np.ndarray],
    images: dict[str, str],
    diagonal: float,
) -> dict[str, Any]:
    ref_mask = np.isfinite(reference["depth"])
    cand_mask = np.isfinite(candidate["depth"])
    intersection = ref_mask & cand_mask
    union = ref_mask | cand_mask
    ref_only = ref_mask & ~cand_mask
    cand_only = cand_mask & ~ref_mask
    depth_delta = np.abs(candidate["depth"][intersection] - reference["depth"][intersection])
    ref_hit = reference["hit_count"].astype(np.float32)
    cand_hit = candidate["hit_count"].astype(np.float32)
    return {
        "index": index,
        "pixels": int(ref_mask.size),
        "silhouette_iou": float(np.count_nonzero(intersection) / max(np.count_nonzero(union), 1)),
        "reference_only_fraction": float(np.count_nonzero(ref_only) / max(np.count_nonzero(ref_mask), 1)),
        "candidate_only_fraction": float(np.count_nonzero(cand_only) / max(np.count_nonzero(cand_mask), 1)),
        "mean_abs_depth_delta": float(depth_delta.mean()) if len(depth_delta) else None,
        "mean_abs_depth_delta_fraction_diagonal": float(depth_delta.mean() / diagonal) if len(depth_delta) else None,
        "reference_mean_hit_count": float(ref_hit.mean()),
        "candidate_mean_hit_count": float(cand_hit.mean()),
        "mean_hit_count_delta": float(cand_hit.mean() - ref_hit.mean()),
        "reference_three_plus_fraction": float(np.count_nonzero(ref_hit >= 3) / ref_hit.size),
        "candidate_three_plus_fraction": float(np.count_nonzero(cand_hit >= 3) / cand_hit.size),
        "three_plus_fraction_delta": float(
            np.count_nonzero(cand_hit >= 3) / cand_hit.size - np.count_nonzero(ref_hit >= 3) / ref_hit.size
        ),
        "images": images,
    }


def aggregate_compare_metrics(views: list[dict[str, Any]]) -> dict[str, Any]:
    if not views:
        return {}
    keys = [
        "silhouette_iou",
        "reference_only_fraction",
        "candidate_only_fraction",
        "mean_hit_count_delta",
        "reference_three_plus_fraction",
        "candidate_three_plus_fraction",
        "three_plus_fraction_delta",
    ]
    aggregate = {"views": len(views)}
    for key in keys:
        aggregate[f"mean_{key}"] = float(np.mean([view[key] for view in views]))
    depth_values = [
        view["mean_abs_depth_delta_fraction_diagonal"]
        for view in views
        if view["mean_abs_depth_delta_fraction_diagonal"] is not None
    ]
    aggregate["mean_abs_depth_delta_fraction_diagonal"] = float(np.mean(depth_values)) if depth_values else None
    return aggregate


def make_compare_contact_tiles(index: int, images: dict[str, str]) -> list[Image.Image]:
    tiles = []
    for label in ("silhouette_overlay", "depth_delta", "hit_delta"):
        image = Image.open(images[label]).convert("RGB")
        tiles.append(add_label(image, f"v{index:02d} {label}"))
    return tiles


def diverging_hit_delta(delta: np.ndarray, max_hits: int) -> np.ndarray:
    limit = max(float(max_hits), 1.0)
    normalized = np.clip(delta / limit, -1, 1)
    image = np.zeros((*delta.shape, 3), dtype=np.uint8)
    pos = normalized > 0
    neg = normalized < 0
    image[..., :] = 24
    image[pos, 0] = (normalized[pos] * 255).astype(np.uint8)
    image[pos, 1] = 64
    image[neg, 2] = (-normalized[neg] * 255).astype(np.uint8)
    image[neg, 1] = 96
    return image


def add_label(image: Image.Image, label: str) -> Image.Image:
    label_height = 18
    out = Image.new("RGB", (image.width, image.height + label_height), "white")
    out.paste(image, (0, label_height))
    draw = ImageDraw.Draw(out)
    draw.text((4, 2), label, fill="black")
    return out


def make_contact_sheet(tiles: list[Image.Image], output: Path, columns: int = 6) -> None:
    if not tiles:
        return
    width = max(tile.width for tile in tiles)
    height = max(tile.height for tile in tiles)
    rows = math.ceil(len(tiles) / columns)
    sheet = Image.new("RGB", (columns * width, rows * height), "white")
    for idx, tile in enumerate(tiles):
        x = (idx % columns) * width
        y = (idx // columns) * height
        sheet.paste(tile, (x, y))
    sheet.save(output)


def interpret_report(topology: dict[str, Any] | None, aggregate: dict[str, Any]) -> list[str]:
    notes: list[str] = []
    if topology:
        if topology.get("boundary_edges", 0) > 0:
            notes.append("Open boundary holes are present; ordinary hole filling may be needed for this defect class.")
        if topology.get("non_two_manifold_edges", 0) > 0 or topology.get("non_two_manifold_vertices", 0) > 0:
            notes.append("Non-manifold elements are present; topology cleanup is needed before boolean-style operations.")
        if topology.get("is_mesh_two_manifold") is True and topology.get("boundary_edges", 0) == 0:
            notes.append("The mesh is topologically closed by MeshLab measures; visible openings may be tunnels/cavities, not boundary holes.")
    if aggregate.get("three_plus_hit_fraction", 0) > 0.05:
        notes.append("Many rays hit three or more surfaces; this suggests high depth complexity or internal/overlapping geometry.")
    elif aggregate.get("multi_hit_fraction", 0) > 0.15:
        notes.append("Many rays hit multiple surfaces; this may be legitimate petals or internal layers and needs localized review.")
    return notes


def interpret_comparison(topology: dict[str, Any] | None, aggregate: dict[str, Any]) -> list[str]:
    notes: list[str] = []
    if topology:
        if topology.get("boundary_edges", 0) == 0 and topology.get("non_two_manifold_edges", 0) == 0:
            notes.append("Candidate has no boundary or non-manifold edges by MeshLab topology measures.")
        else:
            notes.append("Candidate still has boundary or non-manifold edge defects.")
        if topology.get("connected_components_number") not in (None, 1):
            notes.append("Candidate has more than one connected component.")
    if aggregate.get("mean_silhouette_iou", 0) < 0.95:
        notes.append("Candidate silhouette diverges from the reference; inspect red/blue overlay regions.")
    if aggregate.get("mean_three_plus_fraction_delta", 0) < -0.03:
        notes.append("Candidate reduces high depth complexity, which is a positive signal for cavity cleanup.")
    if aggregate.get("mean_three_plus_fraction_delta", 0) > 0.03:
        notes.append("Candidate increases high depth complexity, which is a negative signal.")
    return notes


def render_markdown_report(report: dict[str, Any]) -> str:
    mesh = report["mesh"]
    topology = mesh.get("topology") or {}
    aggregate = report["visibility"]["aggregate"]
    lines = [
        "# Mesh Diagnostics",
        "",
        f"Source: `{report['source']}`",
        f"Contact sheet: `{report['contact_sheet']}`",
        "",
        "## Topology",
        "",
        f"- Faces: `{mesh['faces']:,}`",
        f"- Vertices: `{mesh['vertices']:,}`",
        f"- Components: `{topology.get('connected_components_number')}`",
        f"- Boundary edges: `{topology.get('boundary_edges')}`",
        f"- Non-manifold edges: `{topology.get('non_two_manifold_edges')}`",
        f"- Non-manifold vertices: `{topology.get('non_two_manifold_vertices')}`",
        f"- Two-manifold: `{topology.get('is_mesh_two_manifold')}`",
        "",
        "## Ray Diagnostics",
        "",
        f"- Views: `{aggregate.get('views')}`",
        f"- Rays: `{aggregate.get('rays')}`",
        f"- Hit fraction: `{aggregate.get('hit_fraction'):.4f}`",
        f"- Multi-hit fraction: `{aggregate.get('multi_hit_fraction'):.4f}`",
        f"- Three-plus-hit fraction: `{aggregate.get('three_plus_hit_fraction'):.4f}`",
        f"- Max hit count: `{aggregate.get('max_hit_count')}`",
        "",
        "## Interpretation",
        "",
    ]
    for note in report.get("interpretation", []):
        lines.append(f"- {note}")
    if not report.get("interpretation"):
        lines.append("- No automatic warning generated.")
    return "\n".join(lines)


def render_compare_markdown(report: dict[str, Any]) -> str:
    candidate_topology = report["candidate_mesh"].get("topology") or {}
    aggregate = report["comparison"]["aggregate"]
    lines = [
        "# Mesh Comparison",
        "",
        f"Reference: `{report['reference']}`",
        f"Candidate: `{report['candidate']}`",
        f"Contact sheet: `{report['contact_sheet']}`",
        "",
        "## Candidate Topology",
        "",
        f"- Faces: `{report['candidate_mesh']['faces']:,}`",
        f"- Vertices: `{report['candidate_mesh']['vertices']:,}`",
        f"- Components: `{candidate_topology.get('connected_components_number')}`",
        f"- Boundary edges: `{candidate_topology.get('boundary_edges')}`",
        f"- Non-manifold edges: `{candidate_topology.get('non_two_manifold_edges')}`",
        f"- Non-manifold vertices: `{candidate_topology.get('non_two_manifold_vertices')}`",
        f"- Two-manifold: `{candidate_topology.get('is_mesh_two_manifold')}`",
        "",
        "## Visual/Raycast Comparison",
        "",
        f"- Mean silhouette IoU: `{aggregate.get('mean_silhouette_iou'):.4f}`",
        f"- Mean reference-only fraction: `{aggregate.get('mean_reference_only_fraction'):.4f}`",
        f"- Mean candidate-only fraction: `{aggregate.get('mean_candidate_only_fraction'):.4f}`",
        f"- Mean depth delta / diagonal: `{aggregate.get('mean_abs_depth_delta_fraction_diagonal'):.6f}`",
        f"- Mean hit-count delta: `{aggregate.get('mean_mean_hit_count_delta'):.4f}`",
        f"- Mean three-plus-hit fraction delta: `{aggregate.get('mean_three_plus_fraction_delta'):.4f}`",
        "",
        "## Interpretation",
        "",
    ]
    for note in report.get("interpretation", []):
        lines.append(f"- {note}")
    if not report.get("interpretation"):
        lines.append("- No automatic warning generated.")
    return "\n".join(lines)


def normalize(vector: np.ndarray) -> np.ndarray:
    norm = float(np.linalg.norm(vector))
    if norm == 0:
        raise ValueError("Cannot normalize zero vector")
    return vector / norm


def orthonormal_basis(direction: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
    direction = normalize(direction)
    helper = np.array([0.0, 0.0, 1.0], dtype=np.float32)
    if abs(float(direction @ helper)) > 0.9:
        helper = np.array([0.0, 1.0, 0.0], dtype=np.float32)
    u = normalize(np.cross(direction, helper))
    v = normalize(np.cross(direction, u))
    return u.astype(np.float32), v.astype(np.float32)


def fibonacci_sphere(count: int) -> np.ndarray:
    if count <= 0:
        raise ValueError("count must be positive")
    indices = np.arange(count, dtype=np.float64) + 0.5
    phi = np.arccos(1.0 - 2.0 * indices / count)
    theta = np.pi * (1.0 + 5.0**0.5) * indices
    x = np.cos(theta) * np.sin(phi)
    y = np.sin(theta) * np.sin(phi)
    z = np.cos(phi)
    return np.column_stack([x, y, z]).astype(np.float32)

"""Local render-space region-of-interest probes for mesh defects."""

from __future__ import annotations

from dataclasses import dataclass
import json
from pathlib import Path
from typing import Any

import imageio.v3 as iio
import numpy as np
import open3d as o3d
from PIL import Image, ImageDraw
import trimesh

from .core import jsonable
from .diagnostics import (
    build_raycast_scene,
    depth_to_uint8,
    fibonacci_sphere,
    load_mesh_arrays,
    make_contact_sheet,
    make_view_frame,
    normals_to_rgb,
    orthonormal_basis,
    raycast_frame,
    scalar_to_colormap,
)


@dataclass(frozen=True)
class RoiProbeConfig:
    view_count: int = 24
    view_index: int = 0
    image_size: int = 768
    max_hits: int = 12
    padding: float = 0.08
    hit_epsilon_fraction: float = 1e-5
    circle_x: float | None = None
    circle_y: float | None = None
    circle_radius: float | None = None
    camera_direction: tuple[float, float, float] | None = None
    section_size: int = 768
    local_expand: float = 1.8


def render_roi_views(source: Path, output_dir: Path, view_count: int, image_size: int, padding: float) -> dict[str, Any]:
    output_dir.mkdir(parents=True, exist_ok=True)
    vertices, faces, bounds, topology = load_mesh_arrays(source)
    scene = build_raycast_scene(vertices, faces)
    tiles: list[Image.Image] = []
    view_reports: list[dict[str, Any]] = []
    for index, direction in enumerate(fibonacci_sphere(view_count)):
        frame = make_view_frame(vertices=vertices, camera_direction=direction, image_size=image_size, padding=padding)
        result = raycast_frame(scene=scene, frame=frame, max_hits=1, hit_epsilon_fraction=1e-5)
        mask = np.isfinite(result["depth"])
        normal = normals_to_rgb(result["primitive_normals"], mask)
        path = output_dir / f"view_{index:03d}_normal.png"
        iio.imwrite(path, normal)
        tile = Image.open(path).convert("RGB")
        tiles.append(label_tile(tile, f"view {index:03d}"))
        view_reports.append({"index": index, "camera_direction": [float(v) for v in direction], "normal_image": str(path)})
    sheet = output_dir / "roi_view_sheet.png"
    make_contact_sheet(tiles, sheet, columns=4)
    report = {
        "source": str(source),
        "output_dir": str(output_dir),
        "view_count": view_count,
        "image_size": image_size,
        "bounds": bounds,
        "topology": topology,
        "views": view_reports,
        "contact_sheet": str(sheet),
    }
    report_path = output_dir / "roi_views.json"
    report_path.write_text(json.dumps(jsonable(report), indent=2) + "\n")
    report["report"] = str(report_path)
    return report


def probe_roi(source: Path, output_dir: Path, config: RoiProbeConfig) -> dict[str, Any]:
    output_dir.mkdir(parents=True, exist_ok=True)
    vertices, faces, bounds, topology = load_mesh_arrays(source)
    scene = build_raycast_scene(vertices, faces)
    camera_direction = selected_camera_direction(config)
    frame = make_view_frame(
        vertices=vertices,
        camera_direction=camera_direction,
        image_size=config.image_size,
        padding=config.padding,
    )
    first = raycast_frame(
        scene=scene,
        frame=frame,
        max_hits=config.max_hits,
        hit_epsilon_fraction=config.hit_epsilon_fraction,
    )
    images = write_roi_base_images(output_dir, first, config.max_hits)

    circle = normalized_circle(config, config.image_size)
    roi_mask = circle_mask(config.image_size, circle[0], circle[1], circle[2])
    overlay_paths = write_roi_overlay_images(output_dir, images, circle)

    hits = collect_roi_hits(scene, frame, roi_mask, config.max_hits, config.hit_epsilon_fraction)
    unique_faces = np.unique(hits["primitive_ids"]) if len(hits["primitive_ids"]) else np.array([], dtype=np.int64)
    hit_points = hits["points"]
    roi_center = hit_points.mean(axis=0) if len(hit_points) else vertices.mean(axis=0)
    pixel_step = frame_pixel_step(frame, config.image_size)
    roi_radius_source = float(circle[2] * pixel_step)
    local_vertices, local_faces, local_face_source_ids = local_mesh_for_roi(
        vertices=vertices,
        faces=faces,
        frame=frame,
        roi_mask=roi_mask,
        hit_points=hit_points,
        roi_radius=roi_radius_source,
        expand=config.local_expand,
    )
    selected_path = output_dir / "roi_hit_faces.ply"
    local_path = output_dir / "roi_local_volume_faces.ply"
    export_face_subset(vertices, faces, unique_faces, selected_path)
    export_mesh_arrays(local_vertices, local_faces, local_path)

    section_paths = write_cross_sections(
        output_dir=output_dir,
        vertices=local_vertices,
        faces=local_faces,
        center=roi_center,
        ray_direction=np.asarray(frame["ray_direction"], dtype=np.float32),
        section_size=config.section_size,
    )

    report = {
        "source": str(source),
        "output_dir": str(output_dir),
        "config": config_to_dict(config),
        "bounds": bounds,
        "topology": topology,
        "camera": {
            "view_index": config.view_index,
            "view_count": config.view_count,
            "camera_direction": [float(v) for v in camera_direction],
            "ray_direction": [float(v) for v in np.asarray(frame["ray_direction"])],
            "image_size": config.image_size,
            "pixel_step_source_units": pixel_step,
        },
        "circle": {"x": circle[0], "y": circle[1], "radius": circle[2]},
        "roi": {
            "roi_pixels": int(np.count_nonzero(roi_mask)),
            "hit_records": int(len(hits["primitive_ids"])),
            "unique_hit_faces": int(len(unique_faces)),
            "hit_faces_mesh": str(selected_path),
            "local_volume_faces": int(len(local_faces)),
            "local_volume_source_faces": int(len(local_face_source_ids)),
            "local_volume_mesh": str(local_path),
            "center_source": [float(v) for v in roi_center],
            "radius_source_units": roi_radius_source,
            "hit_depth_min": float(hits["ray_distance"].min()) if len(hits["ray_distance"]) else None,
            "hit_depth_max": float(hits["ray_distance"].max()) if len(hits["ray_distance"]) else None,
        },
        "images": images | overlay_paths | section_paths,
        "interpretation": [
            "Use the overlay image to confirm the circle covers the defect in render space.",
            "Use cross-section images to inspect local internal surfaces before trying a local patch.",
            "The exported local mesh can be opened separately in Blender for close inspection.",
        ],
    }
    report_path = output_dir / "roi_probe.json"
    report_path.write_text(json.dumps(jsonable(report), indent=2) + "\n")
    markdown_path = output_dir / "roi_probe.md"
    markdown_path.write_text(render_roi_markdown(report) + "\n")
    report["report"] = str(report_path)
    report["markdown"] = str(markdown_path)
    return report


def selected_camera_direction(config: RoiProbeConfig) -> np.ndarray:
    if config.camera_direction is not None:
        direction = np.asarray(config.camera_direction, dtype=np.float32)
        norm = float(np.linalg.norm(direction))
        if norm == 0:
            raise ValueError("camera_direction must not be zero")
        return direction / norm
    if config.view_index < 0 or config.view_index >= config.view_count:
        raise ValueError("view_index must be within view_count")
    return fibonacci_sphere(config.view_count)[config.view_index]


def normalized_circle(config: RoiProbeConfig, image_size: int) -> tuple[float, float, float]:
    radius = config.circle_radius if config.circle_radius is not None else image_size * 0.12
    x = config.circle_x if config.circle_x is not None else image_size * 0.5
    y = config.circle_y if config.circle_y is not None else image_size * 0.5
    if radius <= 0:
        raise ValueError("circle_radius must be positive")
    return float(x), float(y), float(radius)


def circle_mask(image_size: int, x: float, y: float, radius: float) -> np.ndarray:
    ys, xs = np.ogrid[:image_size, :image_size]
    return (xs - x) ** 2 + (ys - y) ** 2 <= radius**2


def write_roi_base_images(output_dir: Path, result: dict[str, np.ndarray], max_hits: int) -> dict[str, str]:
    hit_mask = np.isfinite(result["depth"])
    normal = normals_to_rgb(result["primitive_normals"], hit_mask)
    depth = depth_to_uint8(result["depth"])
    hit_count = scalar_to_colormap(result["hit_count"].astype(np.float32), 0, max_hits, "magma")
    paths = {
        "normal": str(output_dir / "view_normal.png"),
        "depth": str(output_dir / "view_depth.png"),
        "hit_count": str(output_dir / "view_hit_count.png"),
    }
    iio.imwrite(paths["normal"], normal)
    iio.imwrite(paths["depth"], depth)
    iio.imwrite(paths["hit_count"], hit_count)
    return paths


def write_roi_overlay_images(output_dir: Path, images: dict[str, str], circle: tuple[float, float, float]) -> dict[str, str]:
    paths = {}
    for key in ("normal", "depth", "hit_count"):
        image = Image.open(images[key]).convert("RGB")
        draw = ImageDraw.Draw(image)
        x, y, radius = circle
        draw.ellipse((x - radius, y - radius, x + radius, y + radius), outline=(255, 32, 32), width=4)
        draw.line((x - 10, y, x + 10, y), fill=(255, 32, 32), width=2)
        draw.line((x, y - 10, x, y + 10), fill=(255, 32, 32), width=2)
        path = output_dir / f"{key}_roi_overlay.png"
        image.save(path)
        paths[f"{key}_overlay"] = str(path)
    sheet = output_dir / "roi_overlay_sheet.png"
    tiles = [label_tile(Image.open(paths[f"{key}_overlay"]).convert("RGB"), key) for key in ("normal", "depth", "hit_count")]
    make_contact_sheet(tiles, sheet, columns=3)
    paths["overlay_sheet"] = str(sheet)
    return paths


def collect_roi_hits(
    scene: o3d.t.geometry.RaycastingScene,
    frame: dict[str, Any],
    roi_mask: np.ndarray,
    max_hits: int,
    hit_epsilon_fraction: float,
) -> dict[str, np.ndarray]:
    shape = tuple(int(v) for v in np.asarray(frame["shape"]))
    flat_mask = roi_mask.reshape(-1)
    flat_origins = np.asarray(frame["origins"], dtype=np.float32)[flat_mask]
    flat_dirs = np.asarray(frame["directions"], dtype=np.float32)[flat_mask]
    pixel_indices = np.flatnonzero(flat_mask)
    current_origins = flat_origins.copy()
    active = np.ones(len(flat_origins), dtype=bool)
    diagonal = float(frame["diagonal"])
    epsilon = max(diagonal * hit_epsilon_fraction, np.finfo(np.float32).eps * 100)
    primitive_ids: list[np.ndarray] = []
    points: list[np.ndarray] = []
    ray_distance: list[np.ndarray] = []
    pixels: list[np.ndarray] = []
    cumulative_distance = np.zeros(len(flat_origins), dtype=np.float32)

    for _hit_index in range(max_hits):
        if not np.any(active):
            break
        active_indices = np.flatnonzero(active)
        rays = np.concatenate([current_origins[active_indices], flat_dirs[active_indices]], axis=1)
        result = scene.cast_rays(o3d.core.Tensor(rays, dtype=o3d.core.Dtype.Float32))
        t_hit = result["t_hit"].numpy().astype(np.float32)
        ids = result["primitive_ids"].numpy().astype(np.int64)
        valid = np.isfinite(t_hit)
        if not np.any(valid):
            break
        valid_indices = active_indices[valid]
        hit_points = current_origins[valid_indices] + flat_dirs[valid_indices] * t_hit[valid, None]
        cumulative_distance[valid_indices] += t_hit[valid]
        primitive_ids.append(ids[valid])
        points.append(hit_points)
        ray_distance.append(cumulative_distance[valid_indices].copy())
        pixels.append(pixel_indices[valid_indices])
        current_origins[valid_indices] = hit_points + flat_dirs[valid_indices] * epsilon
        cumulative_distance[valid_indices] += epsilon
        active[active_indices[~valid]] = False

    if not primitive_ids:
        return {
            "primitive_ids": np.array([], dtype=np.int64),
            "points": np.empty((0, 3), dtype=np.float32),
            "ray_distance": np.array([], dtype=np.float32),
            "pixels": np.array([], dtype=np.int64).reshape((0, 2)),
        }
    flat_pixels = np.concatenate(pixels)
    py, px = np.divmod(flat_pixels, shape[1])
    return {
        "primitive_ids": np.concatenate(primitive_ids),
        "points": np.concatenate(points, axis=0),
        "ray_distance": np.concatenate(ray_distance),
        "pixels": np.column_stack([px, py]),
    }


def frame_pixel_step(frame: dict[str, Any], image_size: int) -> float:
    origins = np.asarray(frame["origins"], dtype=np.float32).reshape((image_size, image_size, 3))
    if image_size < 2:
        return 0.0
    return float(np.linalg.norm(origins[0, 1] - origins[0, 0]))


def local_mesh_for_roi(
    *,
    vertices: np.ndarray,
    faces: np.ndarray,
    frame: dict[str, Any],
    roi_mask: np.ndarray,
    hit_points: np.ndarray,
    roi_radius: float,
    expand: float,
) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    ray_direction = np.asarray(frame["ray_direction"], dtype=np.float32)
    ray_direction = ray_direction / np.linalg.norm(ray_direction)
    u, v = orthonormal_basis(ray_direction)
    origins = np.asarray(frame["origins"], dtype=np.float32)
    selected_origins = origins[roi_mask.reshape(-1)]
    plane_center = selected_origins.mean(axis=0)
    plane_u = float(plane_center @ u)
    plane_v = float(plane_center @ v)

    if len(hit_points):
        depths = hit_points @ ray_direction
        depth_min = float(depths.min() - roi_radius * expand)
        depth_max = float(depths.max() + roi_radius * expand)
    else:
        vertex_depths = vertices @ ray_direction
        depth_min = float(vertex_depths.min())
        depth_max = float(vertex_depths.max())

    centers = vertices[faces].mean(axis=1)
    center_u = centers @ u
    center_v = centers @ v
    radial = np.sqrt((center_u - plane_u) ** 2 + (center_v - plane_v) ** 2)
    center_depth = centers @ ray_direction
    keep = (radial <= roi_radius * expand) & (center_depth >= depth_min) & (center_depth <= depth_max)
    source_ids = np.flatnonzero(keep)
    local_vertices, local_faces = compact_subset(vertices, faces[source_ids])
    return local_vertices, local_faces, source_ids


def export_face_subset(vertices: np.ndarray, faces: np.ndarray, face_ids: np.ndarray, output: Path) -> None:
    if len(face_ids) == 0:
        output.write_text("")
        return
    sub_vertices, sub_faces = compact_subset(vertices, faces[face_ids])
    export_mesh_arrays(sub_vertices, sub_faces, output)


def compact_subset(vertices: np.ndarray, faces: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
    if len(faces) == 0:
        return np.empty((0, 3), dtype=np.float32), np.empty((0, 3), dtype=np.int64)
    unique, inverse = np.unique(faces.reshape(-1), return_inverse=True)
    return vertices[unique], inverse.reshape((-1, 3))


def export_mesh_arrays(vertices: np.ndarray, faces: np.ndarray, output: Path) -> None:
    if len(vertices) == 0 or len(faces) == 0:
        output.write_text("")
        return
    mesh = trimesh.Trimesh(vertices=vertices, faces=faces, process=False)
    mesh.export(output)


def write_cross_sections(
    *,
    output_dir: Path,
    vertices: np.ndarray,
    faces: np.ndarray,
    center: np.ndarray,
    ray_direction: np.ndarray,
    section_size: int,
) -> dict[str, str]:
    ray_direction = ray_direction / np.linalg.norm(ray_direction)
    u, v = orthonormal_basis(ray_direction)
    specs = [
        ("screen_plane", ray_direction, u, v),
        ("depth_horizontal", v, u, ray_direction),
        ("depth_vertical", u, v, ray_direction),
    ]
    paths = {}
    tiles = []
    for name, normal, basis_a, basis_b in specs:
        segments = plane_intersection_segments(vertices, faces, center, normal)
        image = render_segments(segments, center, basis_a, basis_b, section_size)
        path = output_dir / f"section_{name}.png"
        image.save(path)
        paths[f"section_{name}"] = str(path)
        tiles.append(label_tile(image, name))
    sheet = output_dir / "cross_sections.png"
    make_contact_sheet(tiles, sheet, columns=3)
    paths["cross_sections"] = str(sheet)
    return paths


def plane_intersection_segments(
    vertices: np.ndarray,
    faces: np.ndarray,
    plane_origin: np.ndarray,
    plane_normal: np.ndarray,
) -> np.ndarray:
    if len(faces) == 0:
        return np.empty((0, 2, 3), dtype=np.float32)
    tri = vertices[faces].astype(np.float32)
    normal = plane_normal.astype(np.float32)
    normal = normal / np.linalg.norm(normal)
    d = (tri - plane_origin.astype(np.float32)) @ normal
    segment_points: list[np.ndarray] = []
    for edge_start, edge_end in ((0, 1), (1, 2), (2, 0)):
        d0 = d[:, edge_start]
        d1 = d[:, edge_end]
        crosses = (d0 <= 0) & (d1 >= 0) | (d0 >= 0) & (d1 <= 0)
        crosses &= np.abs(d0 - d1) > 1e-12
        t = d0[crosses] / (d0[crosses] - d1[crosses])
        p = tri[crosses, edge_start] + (tri[crosses, edge_end] - tri[crosses, edge_start]) * t[:, None]
        face_indices = np.flatnonzero(crosses)
        segment_points.append(np.column_stack([face_indices, p]).astype(np.float64))
    if not segment_points:
        return np.empty((0, 2, 3), dtype=np.float32)
    all_points = np.concatenate(segment_points, axis=0)
    order = np.argsort(all_points[:, 0])
    all_points = all_points[order]
    face_ids = all_points[:, 0].astype(np.int64)
    unique, starts, counts = np.unique(face_ids, return_index=True, return_counts=True)
    segments = []
    for _face_id, start, count in zip(unique, starts, counts):
        if count >= 2:
            pts = all_points[start : start + count, 1:4]
            segments.append(pts[:2])
    if not segments:
        return np.empty((0, 2, 3), dtype=np.float32)
    return np.asarray(segments, dtype=np.float32)


def render_segments(
    segments: np.ndarray,
    center: np.ndarray,
    basis_a: np.ndarray,
    basis_b: np.ndarray,
    image_size: int,
) -> Image.Image:
    image = Image.new("RGB", (image_size, image_size), (18, 18, 22))
    draw = ImageDraw.Draw(image)
    if len(segments) == 0:
        return image
    rel = segments - center.astype(np.float32)
    xy = np.stack([rel @ basis_a.astype(np.float32), rel @ basis_b.astype(np.float32)], axis=-1)
    values = xy.reshape((-1, 2))
    lo = np.percentile(values, 2, axis=0)
    hi = np.percentile(values, 98, axis=0)
    span = np.maximum(hi - lo, 1e-9)
    pad = span * 0.15
    lo -= pad
    hi += pad
    span = np.maximum(hi - lo, 1e-9)
    px = (xy[..., 0] - lo[0]) / span[0] * (image_size - 1)
    py = (1.0 - (xy[..., 1] - lo[1]) / span[1]) * (image_size - 1)
    coords = np.stack([px, py], axis=-1)
    for segment in coords:
        draw.line((float(segment[0, 0]), float(segment[0, 1]), float(segment[1, 0]), float(segment[1, 1])), fill=(245, 170, 40), width=1)
    draw.line((image_size / 2 - 8, image_size / 2, image_size / 2 + 8, image_size / 2), fill=(255, 48, 48), width=2)
    draw.line((image_size / 2, image_size / 2 - 8, image_size / 2, image_size / 2 + 8), fill=(255, 48, 48), width=2)
    return image


def label_tile(image: Image.Image, label: str) -> Image.Image:
    label_height = 22
    out = Image.new("RGB", (image.width, image.height + label_height), "white")
    out.paste(image, (0, label_height))
    draw = ImageDraw.Draw(out)
    draw.text((5, 4), label, fill="black")
    return out


def render_roi_markdown(report: dict[str, Any]) -> str:
    roi = report["roi"]
    lines = [
        "# ROI Probe",
        "",
        f"Source: `{report['source']}`",
        f"Overlay sheet: `{report['images']['overlay_sheet']}`",
        f"Cross sections: `{report['images']['cross_sections']}`",
        "",
        "## Circle",
        "",
        f"- View index: `{report['camera']['view_index']}` of `{report['camera']['view_count']}`",
        f"- Camera direction: `{report['camera']['camera_direction']}`",
        f"- Circle: `x={report['circle']['x']:.1f}, y={report['circle']['y']:.1f}, r={report['circle']['radius']:.1f}`",
        "",
        "## Selected Region",
        "",
        f"- ROI pixels: `{roi['roi_pixels']:,}`",
        f"- Hit records through ROI: `{roi['hit_records']:,}`",
        f"- Unique hit faces: `{roi['unique_hit_faces']:,}`",
        f"- Local volume faces: `{roi['local_volume_faces']:,}`",
        f"- Hit face mesh: `{roi['hit_faces_mesh']}`",
        f"- Local volume mesh: `{roi['local_volume_mesh']}`",
        f"- Center source units: `{roi['center_source']}`",
        f"- Hit depth range: `{roi['hit_depth_min']}` to `{roi['hit_depth_max']}`",
        "",
        "## Interpretation",
        "",
    ]
    for note in report["interpretation"]:
        lines.append(f"- {note}")
    return "\n".join(lines)


def config_to_dict(config: RoiProbeConfig) -> dict[str, Any]:
    data = config.__dict__.copy()
    if config.camera_direction is not None:
        data["camera_direction"] = list(config.camera_direction)
    return data

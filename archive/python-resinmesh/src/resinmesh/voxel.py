"""Voxel accessibility checks for repaired mesh candidates."""

from __future__ import annotations

from dataclasses import dataclass
import json
import math
from pathlib import Path
from typing import Any

import imageio.v3 as iio
import numpy as np
import open3d as o3d
from PIL import Image, ImageDraw
from scipy import ndimage

from .core import DimensionMode, inspect_model, print_scale_for, source_units_from_mm, topology_for_path
from .diagnostics import build_raycast_scene, load_mesh_arrays, make_contact_sheet


@dataclass(frozen=True)
class VoxelAuditConfig:
    dimension: DimensionMode = DimensionMode.SHORTEST
    size_mm: float = 10.0
    tolerance_um: float = 20.0
    pitch_mm: float = 0.075
    throat_mm: float = 0.15
    padding_voxels: int = 3
    batch_size: int = 1_000_000


def voxel_audit_mesh(source: Path, output_dir: Path, config: VoxelAuditConfig) -> dict[str, Any]:
    if config.pitch_mm <= 0:
        raise ValueError("pitch_mm must be positive")
    if config.throat_mm < 0:
        raise ValueError("throat_mm must not be negative")
    if config.padding_voxels < 1:
        raise ValueError("padding_voxels must be at least 1")

    output_dir.mkdir(parents=True, exist_ok=True)
    vertices, faces, bounds, topology = load_mesh_arrays(source)
    scale = print_scale_for(bounds, config.dimension, config.size_mm, config.tolerance_um)
    pitch_source_units = source_units_from_mm(config.pitch_mm, scale)
    throat_source_units = source_units_from_mm(config.throat_mm, scale) if config.throat_mm > 0 else 0.0
    throat_radius_voxels = int(math.ceil((throat_source_units / pitch_source_units) / 2.0)) if throat_source_units else 0

    grid = make_grid(vertices, pitch_source_units, config.padding_voxels)
    scene = build_raycast_scene(vertices, faces)
    solid = compute_occupancy_grid(scene, grid, config.batch_size)
    exterior, sealed_void = classify_empty_regions(solid)

    closed_solid = None
    closed_exterior = None
    narrow_access_void = None
    if throat_radius_voxels > 0:
        closed_solid = ndimage.binary_closing(
            solid,
            structure=ball_structure(throat_radius_voxels),
            border_value=0,
        )
        closed_exterior, narrow_access_void = classify_empty_regions(closed_solid)

    voxel_volume_source = pitch_source_units**3
    voxel_volume_mm3 = config.pitch_mm**3
    sealed_count = int(np.count_nonzero(sealed_void))
    narrow_count = int(np.count_nonzero(narrow_access_void)) if narrow_access_void is not None else 0
    solid_count = int(np.count_nonzero(solid))
    total_count = int(solid.size)
    report = {
        "source": str(source),
        "output_dir": str(output_dir),
        "config": config.__dict__ | {"dimension": config.dimension.value},
        "topology": topology_for_path(source) or topology,
        "bounds": bounds,
        "scale": {
            "dimension": scale.dimension.value,
            "target_mm": scale.target_mm,
            "mm_per_source_unit": scale.mm_per_source_unit,
            "pitch_source_units": pitch_source_units,
            "throat_source_units": throat_source_units,
            "throat_radius_voxels": throat_radius_voxels,
        },
        "grid": {
            "shape": list(solid.shape),
            "total_voxels": total_count,
            "origin": grid["origin"].tolist(),
            "pitch_source_units": pitch_source_units,
        },
        "occupancy": {
            "solid_voxels": solid_count,
            "solid_fraction": float(solid_count / max(total_count, 1)),
            "sealed_void_voxels": sealed_count,
            "sealed_void_volume_source_units3": float(sealed_count * voxel_volume_source),
            "sealed_void_volume_mm3_at_virtual_scale": float(sealed_count * voxel_volume_mm3),
            "narrow_access_void_voxels_after_closing": narrow_count,
            "narrow_access_void_volume_mm3_at_virtual_scale": float(narrow_count * voxel_volume_mm3),
        },
        "interpretation": interpret_voxel_audit(topology or {}, sealed_count, narrow_count),
    }

    sheet_path = output_dir / "voxel_slices.png"
    write_voxel_slices(
        output=sheet_path,
        solid=solid,
        exterior=exterior,
        sealed_void=sealed_void,
        closed_solid=closed_solid,
        closed_exterior=closed_exterior,
        narrow_access_void=narrow_access_void,
    )
    report["slice_sheet"] = str(sheet_path)
    report_path = output_dir / "voxel_audit.json"
    report_path.write_text(json.dumps(jsonable(report), indent=2) + "\n")
    markdown_path = output_dir / "voxel_audit.md"
    markdown_path.write_text(render_voxel_markdown(report) + "\n")
    report["report"] = str(report_path)
    report["markdown"] = str(markdown_path)
    return report


def make_grid(vertices: np.ndarray, pitch: float, padding_voxels: int) -> dict[str, Any]:
    mins = vertices.min(axis=0).astype(np.float64) - pitch * padding_voxels
    maxs = vertices.max(axis=0).astype(np.float64) + pitch * padding_voxels
    shape = np.ceil((maxs - mins) / pitch).astype(int) + 1
    axes = [mins[i] + np.arange(shape[i], dtype=np.float32) * pitch for i in range(3)]
    return {"origin": mins, "axes": axes, "shape": tuple(int(v) for v in shape), "pitch": pitch}


def compute_occupancy_grid(
    scene: o3d.t.geometry.RaycastingScene,
    grid: dict[str, Any],
    batch_size: int,
) -> np.ndarray:
    if batch_size <= 0:
        raise ValueError("batch_size must be positive")
    xs, ys, zs = grid["axes"]
    shape = grid["shape"]
    total = int(np.prod(shape))
    solid = np.zeros(total, dtype=bool)
    start = 0
    while start < total:
        stop = min(start + batch_size, total)
        flat = np.arange(start, stop, dtype=np.int64)
        ix, rem = np.divmod(flat, shape[1] * shape[2])
        iy, iz = np.divmod(rem, shape[2])
        points = np.column_stack([xs[ix], ys[iy], zs[iz]]).astype(np.float32)
        occupancy = scene.compute_occupancy(o3d.core.Tensor(points, dtype=o3d.core.Dtype.Float32)).numpy()
        solid[start:stop] = occupancy > 0.5
        start = stop
    return solid.reshape(shape)


def classify_empty_regions(solid: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
    empty = ~solid
    seeds = np.zeros_like(empty, dtype=bool)
    seeds[0, :, :] = empty[0, :, :]
    seeds[-1, :, :] = empty[-1, :, :]
    seeds[:, 0, :] = empty[:, 0, :]
    seeds[:, -1, :] = empty[:, -1, :]
    seeds[:, :, 0] = empty[:, :, 0]
    seeds[:, :, -1] = empty[:, :, -1]
    exterior = ndimage.binary_propagation(seeds, mask=empty)
    sealed_void = empty & ~exterior
    return exterior, sealed_void


def ball_structure(radius: int) -> np.ndarray:
    if radius <= 0:
        return np.ones((1, 1, 1), dtype=bool)
    coords = np.arange(-radius, radius + 1)
    x, y, z = np.meshgrid(coords, coords, coords, indexing="ij")
    return (x * x + y * y + z * z) <= radius * radius


def write_voxel_slices(
    *,
    output: Path,
    solid: np.ndarray,
    exterior: np.ndarray,
    sealed_void: np.ndarray,
    closed_solid: np.ndarray | None,
    closed_exterior: np.ndarray | None,
    narrow_access_void: np.ndarray | None,
) -> None:
    tiles: list[Image.Image] = []
    for axis, axis_name in enumerate(("x", "y", "z")):
        for fraction in (0.33, 0.50, 0.67):
            index = min(solid.shape[axis] - 1, max(0, int(round((solid.shape[axis] - 1) * fraction))))
            base = slice_image(solid, exterior, sealed_void, axis, index)
            tiles.append(add_label(base, f"{axis_name} {index} raw"))
            if closed_solid is not None and closed_exterior is not None and narrow_access_void is not None:
                closed = slice_image(closed_solid, closed_exterior, narrow_access_void, axis, index)
                tiles.append(add_label(closed, f"{axis_name} {index} throat"))
    make_contact_sheet(tiles, output, columns=6)


def slice_image(
    solid: np.ndarray,
    exterior: np.ndarray,
    void: np.ndarray,
    axis: int,
    index: int,
) -> Image.Image:
    if axis == 0:
        solid_slice = solid[index, :, :]
        exterior_slice = exterior[index, :, :]
        void_slice = void[index, :, :]
    elif axis == 1:
        solid_slice = solid[:, index, :]
        exterior_slice = exterior[:, index, :]
        void_slice = void[:, index, :]
    else:
        solid_slice = solid[:, :, index]
        exterior_slice = exterior[:, :, index]
        void_slice = void[:, :, index]

    rgb = np.zeros((*solid_slice.shape, 3), dtype=np.uint8)
    rgb[exterior_slice] = [12, 12, 18]
    rgb[solid_slice] = [180, 180, 180]
    rgb[void_slice] = [255, 72, 48]
    image = Image.fromarray(np.flipud(rgb.swapaxes(0, 1)), mode="RGB")
    image.thumbnail((180, 180), Image.Resampling.NEAREST)
    return image


def add_label(image: Image.Image, label: str) -> Image.Image:
    label_height = 18
    out = Image.new("RGB", (image.width, image.height + label_height), "white")
    out.paste(image, (0, label_height))
    draw = ImageDraw.Draw(out)
    draw.text((4, 2), label, fill="black")
    return out


def interpret_voxel_audit(topology: dict[str, Any], sealed_count: int, narrow_count: int) -> list[str]:
    notes: list[str] = []
    if topology.get("boundary_edges", 0) or topology.get("non_two_manifold_edges", 0):
        notes.append("Occupancy is less reliable because the mesh is not topologically closed/manifold.")
    if sealed_count:
        notes.append("Sealed internal empty regions were found at this voxel pitch.")
    else:
        notes.append("No sealed internal empty region was found at this voxel pitch.")
    if narrow_count:
        notes.append("Potential narrow-access void regions were found after morphological throat closing.")
    elif narrow_count == 0:
        notes.append("No narrow-access void region was found after morphological throat closing.")
    return notes


def render_voxel_markdown(report: dict[str, Any]) -> str:
    occupancy = report["occupancy"]
    grid = report["grid"]
    lines = [
        "# Voxel Accessibility Audit",
        "",
        f"Source: `{report['source']}`",
        f"Slice sheet: `{report['slice_sheet']}`",
        "",
        "## Grid",
        "",
        f"- Shape: `{grid['shape']}`",
        f"- Total voxels: `{grid['total_voxels']:,}`",
        f"- Pitch: `{report['config']['pitch_mm']} mm` at virtual print scale",
        f"- Throat closing: `{report['config']['throat_mm']} mm`",
        "",
        "## Occupancy",
        "",
        f"- Solid voxels: `{occupancy['solid_voxels']:,}`",
        f"- Solid fraction: `{occupancy['solid_fraction']:.4f}`",
        f"- Sealed void voxels: `{occupancy['sealed_void_voxels']:,}`",
        f"- Sealed void volume: `{occupancy['sealed_void_volume_mm3_at_virtual_scale']:.6f} mm^3`",
        f"- Narrow-access void voxels after closing: `{occupancy['narrow_access_void_voxels_after_closing']:,}`",
        f"- Narrow-access void volume after closing: `{occupancy['narrow_access_void_volume_mm3_at_virtual_scale']:.6f} mm^3`",
        "",
        "## Interpretation",
        "",
    ]
    for note in report["interpretation"]:
        lines.append(f"- {note}")
    return "\n".join(lines)


def jsonable(value: Any) -> Any:
    if isinstance(value, dict):
        return {str(key): jsonable(item) for key, item in value.items()}
    if isinstance(value, (list, tuple)):
        return [jsonable(item) for item in value]
    if isinstance(value, Path):
        return str(value)
    if isinstance(value, np.ndarray):
        return value.tolist()
    if hasattr(value, "item"):
        return value.item()
    return value

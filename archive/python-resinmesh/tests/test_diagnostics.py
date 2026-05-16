from pathlib import Path

import numpy as np
import trimesh

from resinmesh.diagnostics import (
    RayDiagnosticConfig,
    build_raycast_scene,
    compare_meshes,
    diagnose_mesh,
    fibonacci_sphere,
    raycast_view,
)
from resinmesh.experiments import score_candidate
from resinmesh.core import PolishConfig, polish_mesh
from resinmesh.roi import circle_mask, plane_intersection_segments
from resinmesh.voxel import ball_structure, classify_empty_regions


def test_fibonacci_sphere_returns_unit_vectors():
    directions = fibonacci_sphere(32)

    assert directions.shape == (32, 3)
    assert np.allclose(np.linalg.norm(directions, axis=1), 1.0)


def test_raycast_view_counts_front_and_back_hits_on_closed_sphere():
    mesh = trimesh.creation.icosphere(subdivisions=2, radius=1.0)
    vertices = np.asarray(mesh.vertices, dtype=np.float32)
    faces = np.asarray(mesh.faces, dtype=np.uint32)
    scene = build_raycast_scene(vertices, faces)

    result = raycast_view(
        scene=scene,
        vertices=vertices,
        faces=faces,
        camera_direction=np.array([0.0, 0.0, 1.0], dtype=np.float32),
        image_size=32,
        max_hits=4,
        padding=0.05,
        hit_epsilon_fraction=1e-4,
    )

    assert result["hit_count"].max() >= 2
    assert np.count_nonzero(result["hit_count"] >= 2) > 0
    assert np.count_nonzero(np.isfinite(result["depth"])) > 0


def test_diagnose_mesh_flags_open_boundary_hole(tmp_path: Path):
    mesh = trimesh.creation.box(extents=(1.0, 1.0, 1.0))
    open_box = trimesh.Trimesh(
        vertices=mesh.vertices,
        faces=mesh.faces[:-1],
        process=False,
    )
    source = tmp_path / "open_box.ply"
    open_box.export(source)

    report = diagnose_mesh(
        source,
        tmp_path / "diagnostics",
        RayDiagnosticConfig(view_count=2, image_size=24, max_hits=3),
    )

    topology = report["mesh"]["topology"]
    assert topology["boundary_edges"] > 0
    assert any("Open boundary holes" in note for note in report["interpretation"])
    assert Path(report["contact_sheet"]).exists()


def test_compare_meshes_identical_mesh_has_perfect_silhouette(tmp_path: Path):
    mesh = trimesh.creation.icosphere(subdivisions=1, radius=1.0)
    source = tmp_path / "sphere.ply"
    mesh.export(source)

    report = compare_meshes(
        source,
        source,
        tmp_path / "comparison",
        RayDiagnosticConfig(view_count=3, image_size=24, max_hits=4),
    )

    aggregate = report["comparison"]["aggregate"]
    assert aggregate["mean_silhouette_iou"] == 1.0
    assert aggregate["mean_abs_depth_delta_fraction_diagonal"] == 0.0
    assert Path(report["contact_sheet"]).exists()


def test_nested_internal_shell_has_higher_depth_complexity(tmp_path: Path):
    outer = trimesh.creation.icosphere(subdivisions=2, radius=1.0)
    inner = trimesh.creation.icosphere(subdivisions=1, radius=0.35)
    nested = trimesh.util.concatenate([outer, inner])

    outer_path = tmp_path / "outer.ply"
    nested_path = tmp_path / "nested.ply"
    outer.export(outer_path)
    nested.export(nested_path)

    outer_report = diagnose_mesh(
        outer_path,
        tmp_path / "outer_diag",
        RayDiagnosticConfig(view_count=4, image_size=32, max_hits=6),
    )
    nested_report = diagnose_mesh(
        nested_path,
        tmp_path / "nested_diag",
        RayDiagnosticConfig(view_count=4, image_size=32, max_hits=6),
    )

    assert len(nested_report["mesh"]["components"]) >= 2
    assert (
        nested_report["visibility"]["aggregate"]["three_plus_hit_fraction"]
        > outer_report["visibility"]["aggregate"]["three_plus_hit_fraction"]
    )


def test_score_candidate_rewards_topology_and_penalizes_surface_loss(tmp_path: Path):
    reference = trimesh.creation.icosphere(subdivisions=1, radius=1.0)
    candidate = trimesh.creation.icosphere(subdivisions=1, radius=0.9)
    reference_path = tmp_path / "reference.ply"
    candidate_path = tmp_path / "candidate.ply"
    reference.export(reference_path)
    candidate.export(candidate_path)

    reference_diagnostic = diagnose_mesh(
        reference_path,
        tmp_path / "reference_diag",
        RayDiagnosticConfig(view_count=3, image_size=24, max_hits=4),
    )
    comparison = compare_meshes(
        reference_path,
        candidate_path,
        tmp_path / "comparison",
        RayDiagnosticConfig(view_count=3, image_size=24, max_hits=4),
    )
    candidate_diagnostic = diagnose_mesh(
        candidate_path,
        tmp_path / "candidate_diag",
        RayDiagnosticConfig(view_count=3, image_size=24, max_hits=4),
    )

    score = score_candidate(reference_diagnostic, comparison, candidate_diagnostic)

    assert score["topology_ok"]
    assert score["component_ok"]
    assert score["total"] < 100.0
    assert score["reference_only_fraction"] > 0


def test_voxel_classification_finds_sealed_internal_void():
    solid = np.ones((7, 7, 7), dtype=bool)
    solid[3, 3, 3] = False

    exterior, sealed = classify_empty_regions(solid)

    assert not exterior[3, 3, 3]
    assert sealed[3, 3, 3]
    assert np.count_nonzero(sealed) == 1


def test_ball_structure_contains_center_and_axis_neighbors():
    ball = ball_structure(1)

    assert ball.shape == (3, 3, 3)
    assert ball[1, 1, 1]
    assert ball[0, 1, 1]
    assert not ball[0, 0, 0]


def test_polish_mesh_preserves_closed_box_topology(tmp_path: Path):
    mesh = trimesh.creation.box(extents=(1.0, 1.0, 1.0))
    source = tmp_path / "box.ply"
    output = tmp_path / "box_polished.ply"
    mesh.export(source)

    result = polish_mesh(
        source,
        PolishConfig(
            pre_taubin_steps=1,
            isotropic_iterations=1,
            targetlen_percent=25,
            max_surface_dist_percent=25,
            post_taubin_steps=1,
        ),
        output=output,
    )

    topology = result["polished_topology"]
    assert output.exists()
    assert topology["boundary_edges"] == 0
    assert topology["non_two_manifold_edges"] == 0
    assert topology["connected_components_number"] == 1


def test_circle_mask_selects_center_pixel():
    mask = circle_mask(11, 5, 5, 2)

    assert mask[5, 5]
    assert mask[5, 7]
    assert not mask[0, 0]


def test_plane_intersection_segments_cuts_triangle():
    vertices = np.array(
        [
            [-1.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
        ],
        dtype=np.float32,
    )
    faces = np.array([[0, 1, 2]], dtype=np.int64)

    segments = plane_intersection_segments(
        vertices,
        faces,
        plane_origin=np.array([0.0, 0.0, 0.0], dtype=np.float32),
        plane_normal=np.array([1.0, 0.0, 0.0], dtype=np.float32),
    )

    assert segments.shape == (1, 2, 3)
    assert np.allclose(segments[0, :, 0], 0.0)

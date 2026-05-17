use std::{cmp::Ordering, collections::HashMap};

use glam::Vec3;
use meshmend_core::{MeshBounds, MeshStats, Triangle, TriangleId};
use meshmend_geometry::IndexedMesh;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisReport {
    pub version: u32,
    pub summary: AnalysisSummary,
    pub topology: TopologySummary,
    pub geometry: GeometrySummary,
    pub defects: Vec<DefectFinding>,
}

impl AnalysisReport {
    pub const VERSION: u32 = 1;

    pub fn defect_count(&self, kind: DefectKind) -> usize {
        self.defects
            .iter()
            .filter(|defect| defect.kind == kind)
            .count()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisSummary {
    pub triangle_count: u64,
    pub vertex_count: u64,
    pub indexed_vertex_count: u64,
    pub source_bytes: u64,
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopologySummary {
    pub component_count: u32,
    pub boundary_edge_count: usize,
    pub boundary_loop_count: usize,
    pub non_manifold_edge_count: usize,
    pub duplicate_face_count: usize,
    pub degenerate_face_count: usize,
    pub contained_component_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeometrySummary {
    pub surface_area: f64,
    pub signed_volume: f64,
    pub average_edge_length: f64,
    pub p95_edge_length: f64,
    pub p99_edge_length: f64,
    pub component_summaries: Vec<ComponentSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentSummary {
    pub id: u32,
    pub face_count: u64,
    pub surface_area: f64,
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
    pub contained_in_component: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DefectKind {
    OpenBoundary,
    NonManifoldEdge,
    DuplicateFace,
    DegenerateFace,
    TinyFragment,
    ContainedInternalShell,
    OverDenseMesh,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefectFinding {
    pub id: String,
    pub kind: DefectKind,
    pub severity: Severity,
    pub component_id: Option<u32>,
    pub triangle_ids: Vec<TriangleId>,
    pub edge_vertices: Vec<[u32; 2]>,
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
    pub center: [f32; 3],
    pub recommendation: String,
    pub confidence: f32,
    pub notes: Vec<String>,
}

pub fn analyze_triangles(
    triangles: impl IntoIterator<Item = (TriangleId, Triangle)>,
    stats: MeshStats,
    weld_tolerance: f32,
) -> AnalysisReport {
    let mesh = IndexedMesh::from_triangles(triangles, weld_tolerance);
    analyze_indexed_mesh(mesh, stats)
}

pub fn analyze_indexed_mesh(mesh: IndexedMesh, stats: MeshStats) -> AnalysisReport {
    let component_summaries = component_summaries(&mesh);
    let contained_components = contained_components(&component_summaries);
    let duplicate_faces = duplicate_faces(&mesh);
    let degenerate_faces = degenerate_faces(&mesh);
    let mut defects = Vec::new();

    for (index, boundary_loop) in mesh.connectivity.boundary_loops.iter().enumerate() {
        let bounds = bounds_for_vertices(&mesh, &boundary_loop.vertices);
        defects.push(DefectFinding {
            id: format!("boundary-loop-{index}"),
            kind: DefectKind::OpenBoundary,
            severity: Severity::Error,
            component_id: boundary_loop
                .edges
                .first()
                .and_then(|edge| mesh.connectivity.edges[*edge as usize].faces.first())
                .map(|face| mesh.connectivity.component_ids[*face as usize]),
            triangle_ids: Vec::new(),
            edge_vertices: boundary_loop
                .edges
                .iter()
                .map(|edge| mesh.connectivity.edges[*edge as usize].vertices)
                .collect(),
            bounds_min: bounds.min.to_array(),
            bounds_max: bounds.max.to_array(),
            center: bounds.center().to_array(),
            recommendation: "Use Hole Fill or Local Cavity Repair".to_string(),
            confidence: 0.95,
            notes: vec![format!("{} boundary edges", boundary_loop.edges.len())],
        });
    }

    for (index, edge_index) in mesh.connectivity.non_manifold_edges.iter().enumerate() {
        let edge = &mesh.connectivity.edges[*edge_index as usize];
        let bounds = bounds_for_vertices(&mesh, &edge.vertices);
        defects.push(DefectFinding {
            id: format!("non-manifold-edge-{index}"),
            kind: DefectKind::NonManifoldEdge,
            severity: Severity::Error,
            component_id: edge
                .faces
                .first()
                .map(|face| mesh.connectivity.component_ids[*face as usize]),
            triangle_ids: edge
                .faces
                .iter()
                .map(|face| mesh.faces[*face as usize].source_id)
                .collect(),
            edge_vertices: vec![edge.vertices],
            bounds_min: bounds.min.to_array(),
            bounds_max: bounds.max.to_array(),
            center: bounds.center().to_array(),
            recommendation: "Run Clean Mesh, then inspect the region".to_string(),
            confidence: 0.98,
            notes: vec![format!("edge is shared by {} faces", edge.faces.len())],
        });
    }

    for (index, face_index) in duplicate_faces.iter().enumerate() {
        let face = mesh.faces[*face_index as usize];
        let bounds = bounds_for_vertices(&mesh, &face.indices);
        defects.push(DefectFinding {
            id: format!("duplicate-face-{index}"),
            kind: DefectKind::DuplicateFace,
            severity: Severity::Warning,
            component_id: Some(mesh.connectivity.component_ids[*face_index as usize]),
            triangle_ids: vec![face.source_id],
            edge_vertices: Vec::new(),
            bounds_min: bounds.min.to_array(),
            bounds_max: bounds.max.to_array(),
            center: bounds.center().to_array(),
            recommendation: "Run Clean Mesh".to_string(),
            confidence: 0.99,
            notes: Vec::new(),
        });
    }

    for (index, face_index) in degenerate_faces.iter().enumerate() {
        let face = mesh.faces[*face_index as usize];
        let bounds = bounds_for_vertices(&mesh, &face.indices);
        defects.push(DefectFinding {
            id: format!("degenerate-face-{index}"),
            kind: DefectKind::DegenerateFace,
            severity: Severity::Warning,
            component_id: Some(mesh.connectivity.component_ids[*face_index as usize]),
            triangle_ids: vec![face.source_id],
            edge_vertices: Vec::new(),
            bounds_min: bounds.min.to_array(),
            bounds_max: bounds.max.to_array(),
            center: bounds.center().to_array(),
            recommendation: "Run Clean Mesh".to_string(),
            confidence: 0.99,
            notes: Vec::new(),
        });
    }

    for component in &component_summaries {
        if component.face_count < 16 && component_summaries.len() > 1 {
            let bounds = MeshBounds {
                min: Vec3::from_array(component.bounds_min),
                max: Vec3::from_array(component.bounds_max),
            };
            defects.push(DefectFinding {
                id: format!("tiny-fragment-{}", component.id),
                kind: DefectKind::TinyFragment,
                severity: Severity::Warning,
                component_id: Some(component.id),
                triangle_ids: Vec::new(),
                edge_vertices: Vec::new(),
                bounds_min: component.bounds_min,
                bounds_max: component.bounds_max,
                center: bounds.center().to_array(),
                recommendation: "Inspect and remove if it is not printable geometry".to_string(),
                confidence: 0.8,
                notes: vec![format!("{} faces", component.face_count)],
            });
        }
        if let Some(container) = component.contained_in_component {
            let bounds = MeshBounds {
                min: Vec3::from_array(component.bounds_min),
                max: Vec3::from_array(component.bounds_max),
            };
            defects.push(DefectFinding {
                id: format!("contained-shell-{}", component.id),
                kind: DefectKind::ContainedInternalShell,
                severity: Severity::Error,
                component_id: Some(component.id),
                triangle_ids: Vec::new(),
                edge_vertices: Vec::new(),
                bounds_min: component.bounds_min,
                bounds_max: component.bounds_max,
                center: bounds.center().to_array(),
                recommendation: "Use X-Ray Inspect, then Local Cavity Repair".to_string(),
                confidence: 0.72,
                notes: vec![format!("appears contained in component {container}")],
            });
        }
    }

    let geometry = geometry_summary(&mesh, component_summaries);
    if geometry.average_edge_length > 0.0 {
        let radius = stats.bounds.radius().max(1.0);
        if geometry.average_edge_length < f64::from(radius) * 0.00035
            && stats.triangle_count > 250_000
        {
            defects.push(DefectFinding {
                id: "over-dense-mesh".to_string(),
                kind: DefectKind::OverDenseMesh,
                severity: Severity::Info,
                component_id: None,
                triangle_ids: Vec::new(),
                edge_vertices: Vec::new(),
                bounds_min: stats.bounds.min.to_array(),
                bounds_max: stats.bounds.max.to_array(),
                center: stats.bounds.center().to_array(),
                recommendation: "Calibrate scale and run printer-aware Remesh".to_string(),
                confidence: 0.65,
                notes: vec![format!(
                    "average edge length {:.6} against model radius {:.6}",
                    geometry.average_edge_length, radius
                )],
            });
        }
    }

    AnalysisReport {
        version: AnalysisReport::VERSION,
        summary: AnalysisSummary {
            triangle_count: stats.triangle_count,
            vertex_count: stats.vertex_position_count,
            indexed_vertex_count: mesh.vertices.len() as u64,
            source_bytes: stats.source_bytes,
            bounds_min: stats.bounds.min.to_array(),
            bounds_max: stats.bounds.max.to_array(),
        },
        topology: TopologySummary {
            component_count: mesh.connectivity.component_count,
            boundary_edge_count: mesh.connectivity.boundary_edges.len(),
            boundary_loop_count: mesh.connectivity.boundary_loops.len(),
            non_manifold_edge_count: mesh.connectivity.non_manifold_edges.len(),
            duplicate_face_count: duplicate_faces.len(),
            degenerate_face_count: degenerate_faces.len(),
            contained_component_count: contained_components.len(),
        },
        geometry,
        defects,
    }
}

fn duplicate_faces(mesh: &IndexedMesh) -> Vec<u32> {
    let mut seen = HashMap::<[u32; 3], u32>::new();
    let mut duplicates = Vec::new();
    for (face_index, face) in mesh.faces.iter().enumerate() {
        let mut key = face.indices;
        key.sort_unstable();
        if seen.insert(key, face_index as u32).is_some() {
            duplicates.push(face_index as u32);
        }
    }
    duplicates
}

fn degenerate_faces(mesh: &IndexedMesh) -> Vec<u32> {
    mesh.faces
        .iter()
        .enumerate()
        .filter_map(|(face_index, face)| {
            let [a, b, c] = face.indices.map(|index| mesh.vertices[index as usize]);
            let area = triangle_area(a, b, c);
            (area <= f32::EPSILON
                || face.indices[0] == face.indices[1]
                || face.indices[1] == face.indices[2]
                || face.indices[2] == face.indices[0])
                .then_some(face_index as u32)
        })
        .collect()
}

fn component_summaries(mesh: &IndexedMesh) -> Vec<ComponentSummary> {
    let component_count = mesh.connectivity.component_count as usize;
    let mut bounds = vec![MeshBounds::EMPTY; component_count];
    let mut face_counts = vec![0_u64; component_count];
    let mut surface_areas = vec![0.0_f64; component_count];

    for (face_index, face) in mesh.faces.iter().enumerate() {
        let component = mesh.connectivity.component_ids[face_index] as usize;
        face_counts[component] += 1;
        let vertices = face.indices.map(|index| mesh.vertices[index as usize]);
        surface_areas[component] += f64::from(triangle_area(vertices[0], vertices[1], vertices[2]));
        for vertex in vertices {
            bounds[component].include_point(vertex);
        }
    }

    let mut summaries = (0..component_count)
        .map(|component| ComponentSummary {
            id: component as u32,
            face_count: face_counts[component],
            surface_area: surface_areas[component],
            bounds_min: bounds[component].min.to_array(),
            bounds_max: bounds[component].max.to_array(),
            contained_in_component: None,
        })
        .collect::<Vec<_>>();
    let contained = contained_components(&summaries);
    for (component, container) in contained {
        if let Some(summary) = summaries.iter_mut().find(|summary| summary.id == component) {
            summary.contained_in_component = Some(container);
        }
    }
    summaries
}

fn contained_components(components: &[ComponentSummary]) -> Vec<(u32, u32)> {
    let mut contained = Vec::new();
    for component in components {
        let bounds = bounds_from_summary(component);
        let center = bounds.center();
        for candidate in components {
            if candidate.id == component.id || candidate.face_count <= component.face_count {
                continue;
            }
            let candidate_bounds = bounds_from_summary(candidate);
            if point_inside_bounds(center, candidate_bounds) {
                contained.push((component.id, candidate.id));
                break;
            }
        }
    }
    contained
}

fn geometry_summary(
    mesh: &IndexedMesh,
    component_summaries: Vec<ComponentSummary>,
) -> GeometrySummary {
    let mut edge_lengths = Vec::with_capacity(mesh.faces.len() * 3);
    let mut surface_area = 0.0_f64;
    let mut signed_volume = 0.0_f64;

    for face in &mesh.faces {
        let [a, b, c] = face.indices.map(|index| mesh.vertices[index as usize]);
        edge_lengths.push(f64::from(a.distance(b)));
        edge_lengths.push(f64::from(b.distance(c)));
        edge_lengths.push(f64::from(c.distance(a)));
        surface_area += f64::from(triangle_area(a, b, c));
        signed_volume += f64::from(a.dot(b.cross(c))) / 6.0;
    }
    edge_lengths.sort_by(|left, right| left.partial_cmp(right).unwrap_or(Ordering::Equal));
    let average_edge_length = if edge_lengths.is_empty() {
        0.0
    } else {
        edge_lengths.iter().sum::<f64>() / edge_lengths.len() as f64
    };

    GeometrySummary {
        surface_area,
        signed_volume,
        average_edge_length,
        p95_edge_length: percentile(&edge_lengths, 0.95),
        p99_edge_length: percentile(&edge_lengths, 0.99),
        component_summaries,
    }
}

fn percentile(values: &[f64], percentile: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let index = ((values.len() - 1) as f64 * percentile).round() as usize;
    values[index.min(values.len() - 1)]
}

fn bounds_for_vertices(mesh: &IndexedMesh, vertices: &[u32]) -> MeshBounds {
    let mut bounds = MeshBounds::EMPTY;
    for vertex in vertices {
        bounds.include_point(mesh.vertices[*vertex as usize]);
    }
    bounds
}

fn bounds_from_summary(summary: &ComponentSummary) -> MeshBounds {
    MeshBounds {
        min: Vec3::from_array(summary.bounds_min),
        max: Vec3::from_array(summary.bounds_max),
    }
}

fn point_inside_bounds(point: Vec3, bounds: MeshBounds) -> bool {
    point.x > bounds.min.x
        && point.y > bounds.min.y
        && point.z > bounds.min.z
        && point.x < bounds.max.x
        && point.y < bounds.max.y
        && point.z < bounds.max.z
}

fn triangle_area(a: Vec3, b: Vec3, c: Vec3) -> f32 {
    (b - a).cross(c - a).length() * 0.5
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_open_boundary_loop() {
        let triangles = vec![
            (
                id(0),
                tri(
                    Vec3::new(0.0, 0.0, 0.0),
                    Vec3::new(1.0, 0.0, 0.0),
                    Vec3::new(1.0, 1.0, 0.0),
                ),
            ),
            (
                id(1),
                tri(
                    Vec3::new(0.0, 0.0, 0.0),
                    Vec3::new(1.0, 1.0, 0.0),
                    Vec3::new(0.0, 1.0, 0.0),
                ),
            ),
        ];

        let report = analyze_triangles(triangles, stats(2), 1.0e-6);

        assert_eq!(report.topology.boundary_loop_count, 1);
        assert_eq!(report.defect_count(DefectKind::OpenBoundary), 1);
    }

    #[test]
    fn detects_duplicate_and_degenerate_faces() {
        let triangles = vec![
            (
                id(0),
                tri(
                    Vec3::new(0.0, 0.0, 0.0),
                    Vec3::new(1.0, 0.0, 0.0),
                    Vec3::new(0.0, 1.0, 0.0),
                ),
            ),
            (
                id(1),
                tri(
                    Vec3::new(0.0, 1.0, 0.0),
                    Vec3::new(1.0, 0.0, 0.0),
                    Vec3::new(0.0, 0.0, 0.0),
                ),
            ),
            (
                id(2),
                tri(
                    Vec3::new(0.0, 0.0, 0.0),
                    Vec3::new(0.0, 0.0, 0.0),
                    Vec3::new(1.0, 0.0, 0.0),
                ),
            ),
        ];

        let report = analyze_triangles(triangles, stats(3), 1.0e-6);

        assert_eq!(report.defect_count(DefectKind::DuplicateFace), 1);
        assert_eq!(report.defect_count(DefectKind::DegenerateFace), 1);
    }

    fn id(local_index: u32) -> TriangleId {
        TriangleId {
            chunk: 0,
            local_index,
        }
    }

    fn tri(a: Vec3, b: Vec3, c: Vec3) -> Triangle {
        Triangle {
            normal: (b - a).cross(c - a).normalize_or_zero(),
            vertices: [a, b, c],
        }
    }

    fn stats(triangle_count: u64) -> MeshStats {
        MeshStats {
            triangle_count,
            vertex_position_count: triangle_count * 3,
            bounds: MeshBounds {
                min: Vec3::splat(-1.0),
                max: Vec3::splat(1.0),
            },
            source_bytes: triangle_count * 50 + 84,
        }
    }
}

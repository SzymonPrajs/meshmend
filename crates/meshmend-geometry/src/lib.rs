use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet, VecDeque},
};

use glam::{Vec2, Vec3};
use meshmend_core::{CrossSectionPlane, MeshBounds, Triangle, TriangleId};

const DEFAULT_LEAF_SIZE: usize = 8;
const RAY_EPSILON: f32 = 1.0e-7;

#[derive(Debug, Clone)]
pub struct IndexedMesh {
    pub vertices: Vec<Vec3>,
    pub faces: Vec<IndexedFace>,
    pub bounds: MeshBounds,
    pub connectivity: MeshConnectivity,
}

impl IndexedMesh {
    pub fn from_triangles(
        triangles: impl IntoIterator<Item = (TriangleId, Triangle)>,
        weld_tolerance: f32,
    ) -> Self {
        let tolerance = weld_tolerance.max(f32::EPSILON);
        let mut vertices = Vec::new();
        let mut vertex_lookup = HashMap::<QuantizedVertex, u32>::new();
        let mut faces = Vec::new();
        let mut bounds = MeshBounds::EMPTY;

        for (source_id, triangle) in triangles {
            let indices = triangle.vertices.map(|vertex| {
                bounds.include_point(vertex);
                let key = QuantizedVertex::new(vertex, tolerance);
                *vertex_lookup.entry(key).or_insert_with(|| {
                    let index = vertices.len() as u32;
                    vertices.push(vertex);
                    index
                })
            });
            let face_bounds = Aabb::from_points(triangle.vertices);
            faces.push(IndexedFace {
                indices,
                normal: triangle.normal,
                source_id,
                bounds: face_bounds,
                centroid: face_bounds.center(),
            });
        }

        let connectivity = MeshConnectivity::build(&faces);
        Self {
            vertices,
            faces,
            bounds,
            connectivity,
        }
    }

    pub fn triangle(&self, face_index: u32) -> Option<Triangle> {
        let face = self.faces.get(face_index as usize)?;
        Some(Triangle {
            normal: face.normal,
            vertices: face.indices.map(|index| self.vertices[index as usize]),
        })
    }

    pub fn face_count(&self) -> usize {
        self.faces.len()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct IndexedFace {
    pub indices: [u32; 3],
    pub normal: Vec3,
    pub source_id: TriangleId,
    bounds: Aabb,
    centroid: Vec3,
}

#[derive(Debug, Clone)]
pub struct MeshConnectivity {
    pub edges: Vec<EdgeRecord>,
    pub face_edges: Vec<[u32; 3]>,
    pub boundary_edges: Vec<u32>,
    pub non_manifold_edges: Vec<u32>,
    pub boundary_loops: Vec<BoundaryLoop>,
    pub component_ids: Vec<u32>,
    pub component_count: u32,
}

impl MeshConnectivity {
    fn build(faces: &[IndexedFace]) -> Self {
        let mut edge_lookup = HashMap::<EdgeKey, usize>::new();
        let mut edges = Vec::<EdgeRecord>::new();
        let mut face_edges = vec![[0_u32; 3]; faces.len()];
        let mut union = UnionFind::new(faces.len());

        for (face_index, face) in faces.iter().enumerate() {
            for (edge_slot, [a, b]) in face_index_edges(face.indices).into_iter().enumerate() {
                let key = EdgeKey::new(a, b);
                let edge_index = *edge_lookup.entry(key).or_insert_with(|| {
                    let index = edges.len();
                    edges.push(EdgeRecord {
                        vertices: key.vertices(),
                        faces: Vec::new(),
                    });
                    index
                });
                face_edges[face_index][edge_slot] = edge_index as u32;
                edges[edge_index].faces.push(face_index as u32);
            }
        }

        for edge in &edges {
            if let Some((&first, rest)) = edge.faces.split_first() {
                for &other in rest {
                    union.union(first as usize, other as usize);
                }
            }
        }

        let mut component_lookup = HashMap::<usize, u32>::new();
        let mut component_ids = Vec::with_capacity(faces.len());
        for face_index in 0..faces.len() {
            let root = union.find(face_index);
            let next = component_lookup.len() as u32;
            let component_id = *component_lookup.entry(root).or_insert(next);
            component_ids.push(component_id);
        }

        let boundary_edges = edges
            .iter()
            .enumerate()
            .filter_map(|(index, edge)| (edge.faces.len() == 1).then_some(index as u32))
            .collect::<Vec<_>>();
        let non_manifold_edges = edges
            .iter()
            .enumerate()
            .filter_map(|(index, edge)| (edge.faces.len() > 2).then_some(index as u32))
            .collect::<Vec<_>>();
        let boundary_loops = trace_boundary_loops(&edges, &boundary_edges);

        Self {
            edges,
            face_edges,
            boundary_edges,
            non_manifold_edges,
            boundary_loops,
            component_ids,
            component_count: component_lookup.len() as u32,
        }
    }
}

#[derive(Debug, Clone)]
pub struct EdgeRecord {
    pub vertices: [u32; 2],
    pub faces: Vec<u32>,
}

#[derive(Debug, Clone)]
pub struct BoundaryLoop {
    pub vertices: Vec<u32>,
    pub edges: Vec<u32>,
    pub closed: bool,
}

#[derive(Debug, Clone)]
pub struct SelectionMesh {
    pub mesh: IndexedMesh,
    bvh: Bvh,
    source_faces: HashMap<TriangleId, u32>,
}

impl SelectionMesh {
    pub fn from_triangles(
        triangles: impl IntoIterator<Item = (TriangleId, Triangle)>,
        weld_tolerance: f32,
    ) -> Self {
        let mesh = IndexedMesh::from_triangles(triangles, weld_tolerance);
        let bvh = Bvh::build(&mesh);
        let source_faces = mesh
            .faces
            .iter()
            .enumerate()
            .map(|(index, face)| (face.source_id, index as u32))
            .collect();
        Self {
            mesh,
            bvh,
            source_faces,
        }
    }

    pub fn hit_stack(
        &self,
        ray: Ray,
        clip_plane: Option<CrossSectionPlane>,
    ) -> Vec<IntersectionHit> {
        let mut hits = Vec::new();
        self.bvh.traverse(ray, |face_index| {
            if let Some(hit) = intersect_indexed_face(&self.mesh, face_index, ray) {
                if match clip_plane {
                    Some(plane) => plane.keeps_point(hit.position),
                    None => true,
                } {
                    hits.push(hit);
                }
            }
        });
        hits.sort_by(|left, right| {
            left.distance
                .partial_cmp(&right.distance)
                .unwrap_or(Ordering::Equal)
        });
        hits
    }

    pub fn surface_faces_within_radius(
        &self,
        source_id: TriangleId,
        center: Vec3,
        radius: f32,
        max_faces: usize,
    ) -> Vec<SurfaceBrushFace> {
        let Some(&start_face) = self.source_faces.get(&source_id) else {
            return Vec::new();
        };
        let radius = radius.max(f32::EPSILON);
        let expansion_radius = radius * 1.75;
        let mut visited = vec![false; self.mesh.faces.len()];
        let mut queue = VecDeque::from([start_face]);
        let mut selected = Vec::new();

        while let Some(face_index) = queue.pop_front() {
            let face_slot = face_index as usize;
            if visited[face_slot] {
                continue;
            }
            visited[face_slot] = true;

            let face = self.mesh.faces[face_slot];
            let centroid = face_centroid(&self.mesh, face);
            let distance = centroid.distance(center);
            if face_index == start_face || distance <= radius {
                selected.push(SurfaceBrushFace {
                    source_id: face.source_id,
                    center: centroid,
                    component_id: self.mesh.connectivity.component_ids[face_slot],
                    distance,
                });
                if selected.len() >= max_faces {
                    break;
                }
            }
            if distance > expansion_radius {
                continue;
            }

            for edge_index in self.mesh.connectivity.face_edges[face_slot] {
                for &neighbor in &self.mesh.connectivity.edges[edge_index as usize].faces {
                    if !visited[neighbor as usize] {
                        queue.push_back(neighbor);
                    }
                }
            }
        }

        selected.sort_by(|left, right| {
            left.distance
                .partial_cmp(&right.distance)
                .unwrap_or(Ordering::Equal)
        });
        selected
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IntersectionHit {
    pub face_index: u32,
    pub source_id: TriangleId,
    pub component_id: u32,
    pub distance: f32,
    pub position: Vec3,
    pub normal: Vec3,
    pub front_face: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceBrushFace {
    pub source_id: TriangleId,
    pub center: Vec3,
    pub component_id: u32,
    pub distance: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ray {
    pub origin: Vec3,
    pub direction: Vec3,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CutPlane {
    pub normal: Vec3,
    pub offset: f32,
}

impl CutPlane {
    pub fn from_point_normal(point: Vec3, normal: Vec3) -> Option<Self> {
        let normal = normal.normalize_or_zero();
        (normal.length_squared() > f32::EPSILON).then_some(Self {
            normal,
            offset: point.dot(normal),
        })
    }

    pub fn signed_distance(self, point: Vec3) -> f32 {
        point.dot(self.normal) - self.offset
    }

    pub fn project_point(self, point: Vec3) -> Vec3 {
        point - self.normal * self.signed_distance(point)
    }

    pub fn basis(self) -> (Vec3, Vec3) {
        let reference = if self.normal.y.abs() < 0.9 {
            Vec3::Y
        } else {
            Vec3::X
        };
        let u = reference.cross(self.normal).normalize_or_zero();
        let v = self.normal.cross(u).normalize_or_zero();
        (u, v)
    }
}

pub fn cut_plane_from_view_rays(
    eye: Vec3,
    start_direction: Vec3,
    end_direction: Vec3,
) -> Option<CutPlane> {
    let start_direction = start_direction.normalize_or_zero();
    let end_direction = end_direction.normalize_or_zero();
    CutPlane::from_point_normal(eye, start_direction.cross(end_direction))
}

#[derive(Debug, Clone)]
pub struct CutPreview {
    pub segments: Vec<[Vec3; 2]>,
    pub affected_triangle_count: usize,
}

#[derive(Debug, Clone)]
pub struct CutMeshOptions {
    pub weld_tolerance: f32,
    pub target_edge_length: Option<f32>,
    pub cap_density: CapDensity,
    pub smooth_cap: bool,
}

impl Default for CutMeshOptions {
    fn default() -> Self {
        Self {
            weld_tolerance: 1.0e-5,
            target_edge_length: None,
            cap_density: CapDensity::Automatic,
            smooth_cap: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapDensity {
    Coarse,
    Automatic,
    Fine,
}

impl CapDensity {
    fn multiplier(self) -> f32 {
        match self {
            Self::Coarse => 1.6,
            Self::Automatic => 1.0,
            Self::Fine => 0.55,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CutMeshResult {
    pub combined_triangles: Vec<Triangle>,
    pub pieces: [CutPiece; 2],
    pub loops: Vec<CutLoop>,
    pub target_edge_length: f32,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct CutPiece {
    pub side: CutSide,
    pub triangles: Vec<Triangle>,
    pub cap_triangle_count: usize,
    pub bounds: MeshBounds,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CutSide {
    Positive,
    Negative,
}

#[derive(Debug, Clone)]
pub struct CutLoop {
    pub vertices: Vec<Vec3>,
    pub closed: bool,
    pub length: f32,
}

pub fn preview_cut(triangles: &[Triangle], plane: CutPlane, epsilon: f32) -> CutPreview {
    let mut segments = Vec::new();
    for triangle in triangles {
        if let Some(segment) = triangle_plane_segment(*triangle, plane, epsilon) {
            segments.push(segment);
        }
    }
    CutPreview {
        affected_triangle_count: segments.len(),
        segments,
    }
}

pub fn split_and_cap_mesh(
    triangles: &[Triangle],
    plane: CutPlane,
    options: CutMeshOptions,
) -> Result<CutMeshResult, CutError> {
    let bounds = bounds_for_triangles(triangles);
    let epsilon = options
        .weld_tolerance
        .max(bounds.radius().max(1.0) * 1.0e-6)
        .max(1.0e-7);
    let mut positive = Vec::new();
    let mut negative = Vec::new();
    let mut cut_segments = Vec::new();

    for triangle in triangles {
        let preview_segment = triangle_plane_segment(*triangle, plane, epsilon);
        if let Some(segment) = preview_segment {
            cut_segments.push(segment);
        }
        clip_triangle_to_side(*triangle, plane, epsilon, true, &mut positive);
        clip_triangle_to_side(*triangle, plane, epsilon, false, &mut negative);
    }

    if cut_segments.is_empty() {
        return Err(CutError::NoIntersection);
    }

    let loops = trace_cut_loops(&cut_segments, epsilon * 4.0);
    if loops.iter().any(|cut_loop| !cut_loop.closed) {
        return Err(CutError::OpenCutLoops);
    }

    let target_edge_length = options
        .target_edge_length
        .unwrap_or_else(|| target_edge_length_for_loops(&loops, bounds, options.cap_density))
        .max(epsilon * 8.0);
    let mut cap_warnings = Vec::new();
    let mut cap_triangles = triangulate_caps(&loops, plane, target_edge_length, &mut cap_warnings)?;
    if options.smooth_cap {
        smooth_cap_triangles(&mut cap_triangles, &loops, epsilon * 4.0);
    }
    let positive_cap_count = cap_triangles.len();
    for triangle in &cap_triangles {
        positive.push(reverse_triangle(*triangle));
    }
    for triangle in &cap_triangles {
        negative.push(*triangle);
    }

    let positive_piece = CutPiece {
        side: CutSide::Positive,
        bounds: bounds_for_triangles(&positive),
        triangles: positive,
        cap_triangle_count: positive_cap_count,
    };
    let negative_piece = CutPiece {
        side: CutSide::Negative,
        bounds: bounds_for_triangles(&negative),
        cap_triangle_count: cap_triangles.len(),
        triangles: negative,
    };
    let mut combined_triangles =
        Vec::with_capacity(positive_piece.triangles.len() + negative_piece.triangles.len());
    combined_triangles.extend_from_slice(&positive_piece.triangles);
    combined_triangles.extend_from_slice(&negative_piece.triangles);

    Ok(CutMeshResult {
        combined_triangles,
        pieces: [positive_piece, negative_piece],
        loops,
        target_edge_length,
        warnings: cap_warnings,
    })
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum CutError {
    #[error("cut line does not intersect the mesh")]
    NoIntersection,
    #[error("cut created open or branched boundary loops")]
    OpenCutLoops,
    #[error("cut loop self-intersects in the cap plane")]
    InvalidCutLoop,
    #[error("cap triangulation failed")]
    CapTriangulationFailed,
}

fn triangle_plane_segment(triangle: Triangle, plane: CutPlane, epsilon: f32) -> Option<[Vec3; 2]> {
    let vertices = triangle.vertices;
    let distances = vertices.map(|vertex| plane.signed_distance(vertex));
    if distances.iter().all(|distance| *distance > epsilon)
        || distances.iter().all(|distance| *distance < -epsilon)
    {
        return None;
    }

    let mut points = Vec::with_capacity(3);
    for index in 0..3 {
        let next = (index + 1) % 3;
        let start = vertices[index];
        let end = vertices[next];
        let start_distance = distances[index];
        let end_distance = distances[next];

        if start_distance.abs() <= epsilon {
            push_unique_point(&mut points, plane.project_point(start), epsilon * 4.0);
        }
        if start_distance * end_distance < -epsilon * epsilon {
            let t = start_distance / (start_distance - end_distance);
            push_unique_point(
                &mut points,
                plane.project_point(start.lerp(end, t.clamp(0.0, 1.0))),
                epsilon * 4.0,
            );
        }
        if end_distance.abs() <= epsilon {
            push_unique_point(&mut points, plane.project_point(end), epsilon * 4.0);
        }
    }

    if points.len() >= 2 && points[0].distance_squared(points[1]) > epsilon * epsilon {
        Some([points[0], points[1]])
    } else {
        None
    }
}

fn clip_triangle_to_side(
    triangle: Triangle,
    plane: CutPlane,
    epsilon: f32,
    positive: bool,
    output: &mut Vec<Triangle>,
) {
    let polygon = clip_polygon_to_plane(&triangle.vertices, plane, epsilon, positive);
    if polygon.len() < 3 {
        return;
    }
    for index in 1..polygon.len() - 1 {
        push_triangle(output, [polygon[0], polygon[index], polygon[index + 1]]);
    }
}

fn clip_polygon_to_plane(
    vertices: &[Vec3],
    plane: CutPlane,
    epsilon: f32,
    positive: bool,
) -> Vec<Vec3> {
    let mut output = Vec::new();
    if vertices.is_empty() {
        return output;
    }

    for index in 0..vertices.len() {
        let current = vertices[index];
        let next = vertices[(index + 1) % vertices.len()];
        let current_distance = plane.signed_distance(current);
        let next_distance = plane.signed_distance(next);
        let current_inside = if positive {
            current_distance >= -epsilon
        } else {
            current_distance <= epsilon
        };
        let next_inside = if positive {
            next_distance >= -epsilon
        } else {
            next_distance <= epsilon
        };

        match (current_inside, next_inside) {
            (true, true) => output.push(next),
            (true, false) => {
                let t = current_distance / (current_distance - next_distance);
                output.push(plane.project_point(current.lerp(next, t.clamp(0.0, 1.0))));
            }
            (false, true) => {
                let t = current_distance / (current_distance - next_distance);
                output.push(plane.project_point(current.lerp(next, t.clamp(0.0, 1.0))));
                output.push(next);
            }
            (false, false) => {}
        }
    }

    dedupe_adjacent_points(output, epsilon * 4.0)
}

fn push_triangle(output: &mut Vec<Triangle>, vertices: [Vec3; 3]) {
    let normal = (vertices[1] - vertices[0])
        .cross(vertices[2] - vertices[0])
        .normalize_or_zero();
    if normal.length_squared() <= f32::EPSILON {
        return;
    }
    output.push(Triangle { normal, vertices });
}

fn reverse_triangle(triangle: Triangle) -> Triangle {
    let vertices = [
        triangle.vertices[0],
        triangle.vertices[2],
        triangle.vertices[1],
    ];
    Triangle {
        normal: -triangle.normal,
        vertices,
    }
}

fn bounds_for_triangles(triangles: &[Triangle]) -> MeshBounds {
    triangles
        .iter()
        .flat_map(|triangle| triangle.vertices)
        .fold(MeshBounds::EMPTY, |mut bounds, vertex| {
            bounds.include_point(vertex);
            bounds
        })
}

fn trace_cut_loops(segments: &[[Vec3; 2]], tolerance: f32) -> Vec<CutLoop> {
    let mut vertices = Vec::new();
    let mut lookup = HashMap::<QuantizedPoint, u32>::new();
    let mut edges = Vec::<[u32; 2]>::new();
    for [start, end] in segments {
        let a = vertex_index_for_point(&mut vertices, &mut lookup, *start, tolerance);
        let b = vertex_index_for_point(&mut vertices, &mut lookup, *end, tolerance);
        if a != b {
            let edge = if a < b { [a, b] } else { [b, a] };
            if !edges.contains(&edge) {
                edges.push(edge);
            }
        }
    }

    let mut adjacency = HashMap::<u32, Vec<(u32, u32)>>::new();
    for (edge_index, [a, b]) in edges.iter().copied().enumerate() {
        adjacency.entry(a).or_default().push((b, edge_index as u32));
        adjacency.entry(b).or_default().push((a, edge_index as u32));
    }

    let mut visited = vec![false; edges.len()];
    let mut loops = Vec::new();
    for edge_index in 0..edges.len() {
        if visited[edge_index] {
            continue;
        }
        visited[edge_index] = true;
        let [start, next] = edges[edge_index];
        let mut loop_indices = vec![start, next];
        let mut current = next;
        let mut previous = start;
        let mut closed = false;

        while let Some(candidates) = adjacency.get(&current) {
            if candidates.len() != 2 {
                break;
            }
            let Some((candidate, candidate_edge)) = candidates
                .iter()
                .copied()
                .find(|(_, edge)| !visited[*edge as usize])
            else {
                if current == start {
                    closed = true;
                }
                break;
            };
            if candidate == previous {
                break;
            }
            visited[candidate_edge as usize] = true;
            if candidate == start {
                closed = true;
                break;
            }
            loop_indices.push(candidate);
            previous = current;
            current = candidate;
        }

        let loop_vertices = loop_indices
            .into_iter()
            .map(|index| vertices[index as usize])
            .collect::<Vec<_>>();
        let length = loop_length(&loop_vertices, closed);
        loops.push(CutLoop {
            vertices: loop_vertices,
            closed,
            length,
        });
    }

    loops
}

fn target_edge_length_for_loops(loops: &[CutLoop], bounds: MeshBounds, density: CapDensity) -> f32 {
    let mut lengths = loops
        .iter()
        .flat_map(|cut_loop| {
            cut_loop
                .vertices
                .iter()
                .copied()
                .zip(cut_loop.vertices.iter().copied().cycle().skip(1))
                .take(cut_loop.vertices.len())
                .map(|(a, b)| a.distance(b))
        })
        .filter(|length| length.is_finite() && *length > f32::EPSILON)
        .collect::<Vec<_>>();
    lengths.sort_by(|left, right| left.partial_cmp(right).unwrap_or(Ordering::Equal));
    let median = lengths
        .get(lengths.len().saturating_sub(1) / 2)
        .copied()
        .unwrap_or_else(|| bounds.radius().max(1.0) * 0.05);
    let radius = bounds.radius().max(median);
    (median * density.multiplier()).clamp(radius * 0.002, radius)
}

fn triangulate_caps(
    loops: &[CutLoop],
    plane: CutPlane,
    target_edge_length: f32,
    warnings: &mut Vec<String>,
) -> Result<Vec<Triangle>, CutError> {
    let mut triangles = Vec::new();
    for cut_loop in loops
        .iter()
        .filter(|cut_loop| cut_loop.closed && cut_loop.vertices.len() >= 3)
    {
        let mut loop_2d = project_loop(cut_loop, plane, target_edge_length);
        if loop_2d.len() < 3 {
            continue;
        }
        if polygon_self_intersects(&loop_2d) {
            return Err(CutError::InvalidCutLoop);
        }
        if polygon_area(&loop_2d) < 0.0 {
            loop_2d.reverse();
        }
        triangulate_ring_grid(
            &loop_2d,
            plane,
            target_edge_length,
            &mut triangles,
            warnings,
        );
    }

    if triangles.is_empty() {
        return Err(CutError::CapTriangulationFailed);
    }
    Ok(triangles)
}

fn triangulate_ring_grid(
    boundary: &[Vec2],
    plane: CutPlane,
    target_edge_length: f32,
    output: &mut Vec<Triangle>,
    warnings: &mut Vec<String>,
) {
    let (u, v) = plane.basis();
    let origin = plane.normal * plane.offset;
    let center = boundary.iter().copied().sum::<Vec2>() / boundary.len() as f32;
    let max_radius = boundary
        .iter()
        .map(|point| point.distance(center))
        .fold(0.0, f32::max);
    let ring_count = (max_radius / target_edge_length.max(1.0e-6))
        .ceil()
        .clamp(1.0, 96.0) as usize;
    if ring_count == 96 {
        warnings.push("cap ring count was clamped to keep triangle count bounded".to_string());
    }

    for ring_index in 0..ring_count {
        let outer_scale = 1.0 - ring_index as f32 / ring_count as f32;
        let inner_scale = 1.0 - (ring_index + 1) as f32 / ring_count as f32;
        for index in 0..boundary.len() {
            let next = (index + 1) % boundary.len();
            let outer_a = center + (boundary[index] - center) * outer_scale;
            let outer_b = center + (boundary[next] - center) * outer_scale;
            if inner_scale <= f32::EPSILON {
                push_triangle(
                    output,
                    [
                        point_2d_to_3d(outer_a, origin, u, v),
                        point_2d_to_3d(outer_b, origin, u, v),
                        point_2d_to_3d(center, origin, u, v),
                    ],
                );
            } else {
                let inner_a = center + (boundary[index] - center) * inner_scale;
                let inner_b = center + (boundary[next] - center) * inner_scale;
                push_triangle(
                    output,
                    [
                        point_2d_to_3d(outer_a, origin, u, v),
                        point_2d_to_3d(outer_b, origin, u, v),
                        point_2d_to_3d(inner_b, origin, u, v),
                    ],
                );
                push_triangle(
                    output,
                    [
                        point_2d_to_3d(outer_a, origin, u, v),
                        point_2d_to_3d(inner_b, origin, u, v),
                        point_2d_to_3d(inner_a, origin, u, v),
                    ],
                );
            }
        }
    }
}

fn smooth_cap_triangles(triangles: &mut [Triangle], loops: &[CutLoop], tolerance: f32) {
    let tolerance = tolerance.max(1.0e-7);
    let boundary = loops
        .iter()
        .flat_map(|cut_loop| cut_loop.vertices.iter().copied())
        .map(|point| QuantizedPoint::new(point, tolerance))
        .collect::<HashSet<_>>();
    let mut positions = HashMap::<QuantizedPoint, Vec3>::new();
    let mut adjacency = HashMap::<QuantizedPoint, HashSet<QuantizedPoint>>::new();

    for triangle in triangles.iter() {
        let keys = triangle
            .vertices
            .map(|vertex| QuantizedPoint::new(vertex, tolerance));
        for (key, vertex) in keys.into_iter().zip(triangle.vertices) {
            positions.entry(key).or_insert(vertex);
            adjacency.entry(key).or_default();
        }
        for [a, b] in [[keys[0], keys[1]], [keys[1], keys[2]], [keys[2], keys[0]]] {
            adjacency.entry(a).or_default().insert(b);
            adjacency.entry(b).or_default().insert(a);
        }
    }

    let mut smoothed = HashMap::<QuantizedPoint, Vec3>::new();
    for (key, position) in &positions {
        if boundary.contains(key) {
            continue;
        }
        let Some(neighbors) = adjacency.get(key).filter(|neighbors| !neighbors.is_empty()) else {
            continue;
        };
        let mut sum = Vec3::ZERO;
        let mut count = 0.0_f32;
        for neighbor in neighbors {
            if let Some(position) = positions.get(neighbor) {
                sum += *position;
                count += 1.0;
            }
        }
        if count > 0.0 {
            smoothed.insert(*key, sum / count);
        } else {
            smoothed.insert(*key, *position);
        }
    }

    for triangle in triangles.iter_mut() {
        for vertex in &mut triangle.vertices {
            let key = QuantizedPoint::new(*vertex, tolerance);
            if let Some(position) = smoothed.get(&key) {
                *vertex = *position;
            }
        }
        triangle.normal = (triangle.vertices[1] - triangle.vertices[0])
            .cross(triangle.vertices[2] - triangle.vertices[0])
            .normalize_or_zero();
    }
}

fn point_2d_to_3d(point: Vec2, origin: Vec3, u: Vec3, v: Vec3) -> Vec3 {
    origin + u * point.x + v * point.y
}

fn project_loop(cut_loop: &CutLoop, plane: CutPlane, _target_edge_length: f32) -> Vec<Vec2> {
    let (u, v) = plane.basis();
    cut_loop
        .vertices
        .iter()
        .map(|point| Vec2::new(point.dot(u), point.dot(v)))
        .collect()
}

fn polygon_area(points: &[Vec2]) -> f32 {
    points
        .iter()
        .copied()
        .zip(points.iter().copied().cycle().skip(1))
        .take(points.len())
        .map(|(a, b)| a.perp_dot(b))
        .sum::<f32>()
        * 0.5
}

fn polygon_self_intersects(points: &[Vec2]) -> bool {
    if points.len() < 4 {
        return false;
    }
    for index in 0..points.len() {
        let next = (index + 1) % points.len();
        for other in index + 1..points.len() {
            let other_next = (other + 1) % points.len();
            if index == other
                || next == other
                || index == other_next
                || (index == 0 && other == points.len() - 1)
            {
                continue;
            }
            if segments_intersect_2d(
                points[index],
                points[next],
                points[other],
                points[other_next],
            ) {
                return true;
            }
        }
    }
    false
}

fn segments_intersect_2d(a: Vec2, b: Vec2, c: Vec2, d: Vec2) -> bool {
    const EPSILON: f32 = 1.0e-6;
    let ab_c = (b - a).perp_dot(c - a);
    let ab_d = (b - a).perp_dot(d - a);
    let cd_a = (d - c).perp_dot(a - c);
    let cd_b = (d - c).perp_dot(b - c);

    if ab_c.abs() <= EPSILON && point_on_segment_2d(c, a, b) {
        return true;
    }
    if ab_d.abs() <= EPSILON && point_on_segment_2d(d, a, b) {
        return true;
    }
    if cd_a.abs() <= EPSILON && point_on_segment_2d(a, c, d) {
        return true;
    }
    if cd_b.abs() <= EPSILON && point_on_segment_2d(b, c, d) {
        return true;
    }

    ((ab_c > EPSILON && ab_d < -EPSILON) || (ab_c < -EPSILON && ab_d > EPSILON))
        && ((cd_a > EPSILON && cd_b < -EPSILON) || (cd_a < -EPSILON && cd_b > EPSILON))
}

fn point_on_segment_2d(point: Vec2, start: Vec2, end: Vec2) -> bool {
    let min = start.min(end) - Vec2::splat(1.0e-6);
    let max = start.max(end) + Vec2::splat(1.0e-6);
    point.x >= min.x && point.x <= max.x && point.y >= min.y && point.y <= max.y
}

fn loop_length(vertices: &[Vec3], closed: bool) -> f32 {
    if vertices.len() < 2 {
        return 0.0;
    }
    let mut length = vertices
        .windows(2)
        .map(|segment| segment[0].distance(segment[1]))
        .sum::<f32>();
    if closed {
        length += vertices[vertices.len() - 1].distance(vertices[0]);
    }
    length
}

fn push_unique_point(points: &mut Vec<Vec3>, point: Vec3, tolerance: f32) {
    if points
        .iter()
        .any(|existing| existing.distance_squared(point) <= tolerance * tolerance)
    {
        return;
    }
    points.push(point);
}

fn dedupe_adjacent_points(points: Vec<Vec3>, tolerance: f32) -> Vec<Vec3> {
    let mut deduped = Vec::new();
    for point in points {
        if deduped
            .last()
            .map(|last: &Vec3| last.distance_squared(point) <= tolerance * tolerance)
            .unwrap_or(false)
        {
            continue;
        }
        deduped.push(point);
    }
    if deduped.len() > 1
        && deduped[0].distance_squared(*deduped.last().unwrap()) <= tolerance * tolerance
    {
        deduped.pop();
    }
    deduped
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct QuantizedPoint {
    x: i64,
    y: i64,
    z: i64,
}

impl QuantizedPoint {
    fn new(point: Vec3, tolerance: f32) -> Self {
        Self {
            x: (point.x / tolerance).round() as i64,
            y: (point.y / tolerance).round() as i64,
            z: (point.z / tolerance).round() as i64,
        }
    }
}

fn vertex_index_for_point(
    vertices: &mut Vec<Vec3>,
    lookup: &mut HashMap<QuantizedPoint, u32>,
    point: Vec3,
    tolerance: f32,
) -> u32 {
    let key = QuantizedPoint::new(point, tolerance);
    *lookup.entry(key).or_insert_with(|| {
        let index = vertices.len() as u32;
        vertices.push(point);
        index
    })
}

#[derive(Debug, Clone)]
struct Bvh {
    nodes: Vec<BvhNode>,
    face_indices: Vec<u32>,
}

impl Bvh {
    fn build(mesh: &IndexedMesh) -> Self {
        let mut bvh = Self {
            nodes: Vec::new(),
            face_indices: (0..mesh.faces.len() as u32).collect(),
        };
        if !bvh.face_indices.is_empty() {
            bvh.build_node(mesh, 0, bvh.face_indices.len());
        }
        bvh
    }

    fn build_node(&mut self, mesh: &IndexedMesh, start: usize, end: usize) -> u32 {
        let node_index = self.nodes.len() as u32;
        self.nodes.push(BvhNode::empty());
        let bounds = self.bounds_for_range(mesh, start, end);
        let count = end - start;

        if count <= DEFAULT_LEAF_SIZE {
            self.nodes[node_index as usize] = BvhNode {
                bounds,
                first: start as u32,
                count: count as u32,
                left: u32::MAX,
                right: u32::MAX,
            };
            return node_index;
        }

        let centroid_bounds = self.centroid_bounds_for_range(mesh, start, end);
        let axis = centroid_bounds.longest_axis();
        self.face_indices[start..end].sort_by(|left, right| {
            mesh.faces[*left as usize].centroid[axis]
                .partial_cmp(&mesh.faces[*right as usize].centroid[axis])
                .unwrap_or(Ordering::Equal)
        });
        let mid = start + count / 2;
        let left = self.build_node(mesh, start, mid);
        let right = self.build_node(mesh, mid, end);
        self.nodes[node_index as usize] = BvhNode {
            bounds,
            first: 0,
            count: 0,
            left,
            right,
        };
        node_index
    }

    fn bounds_for_range(&self, mesh: &IndexedMesh, start: usize, end: usize) -> Aabb {
        let mut bounds = Aabb::empty();
        for &face_index in &self.face_indices[start..end] {
            bounds = bounds.union(mesh.faces[face_index as usize].bounds);
        }
        bounds
    }

    fn centroid_bounds_for_range(&self, mesh: &IndexedMesh, start: usize, end: usize) -> Aabb {
        let mut bounds = Aabb::empty();
        for &face_index in &self.face_indices[start..end] {
            bounds.include_point(mesh.faces[face_index as usize].centroid);
        }
        bounds
    }

    fn traverse(&self, ray: Ray, mut visit: impl FnMut(u32)) {
        if self.nodes.is_empty() {
            return;
        }
        let mut stack = vec![0_u32];
        while let Some(node_index) = stack.pop() {
            let node = self.nodes[node_index as usize];
            if node.bounds.intersect_ray(ray).is_none() {
                continue;
            }
            if node.is_leaf() {
                let start = node.first as usize;
                let end = start + node.count as usize;
                for &face_index in &self.face_indices[start..end] {
                    visit(face_index);
                }
            } else {
                stack.push(node.left);
                stack.push(node.right);
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct BvhNode {
    bounds: Aabb,
    first: u32,
    count: u32,
    left: u32,
    right: u32,
}

impl BvhNode {
    fn empty() -> Self {
        Self {
            bounds: Aabb::empty(),
            first: 0,
            count: 0,
            left: u32::MAX,
            right: u32::MAX,
        }
    }

    fn is_leaf(self) -> bool {
        self.count > 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Aabb {
    min: Vec3,
    max: Vec3,
}

impl Aabb {
    fn empty() -> Self {
        Self {
            min: Vec3::splat(f32::INFINITY),
            max: Vec3::splat(f32::NEG_INFINITY),
        }
    }

    fn from_points(points: [Vec3; 3]) -> Self {
        let mut bounds = Self::empty();
        for point in points {
            bounds.include_point(point);
        }
        bounds
    }

    fn include_point(&mut self, point: Vec3) {
        self.min = self.min.min(point);
        self.max = self.max.max(point);
    }

    fn union(mut self, other: Self) -> Self {
        self.include_point(other.min);
        self.include_point(other.max);
        self
    }

    fn center(self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    fn longest_axis(self) -> usize {
        let extent = self.max - self.min;
        if extent.x >= extent.y && extent.x >= extent.z {
            0
        } else if extent.y >= extent.z {
            1
        } else {
            2
        }
    }

    fn intersect_ray(self, ray: Ray) -> Option<f32> {
        let mut t_min = 0.0_f32;
        let mut t_max = f32::INFINITY;
        for axis in 0..3 {
            let origin = ray.origin[axis];
            let direction = ray.direction[axis];
            if direction.abs() < RAY_EPSILON {
                if origin < self.min[axis] || origin > self.max[axis] {
                    return None;
                }
                continue;
            }

            let inverse = 1.0 / direction;
            let mut near = (self.min[axis] - origin) * inverse;
            let mut far = (self.max[axis] - origin) * inverse;
            if near > far {
                std::mem::swap(&mut near, &mut far);
            }
            t_min = t_min.max(near);
            t_max = t_max.min(far);
            if t_min > t_max {
                return None;
            }
        }
        Some(t_min)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct QuantizedVertex {
    x: i64,
    y: i64,
    z: i64,
}

impl QuantizedVertex {
    fn new(vertex: Vec3, tolerance: f32) -> Self {
        Self {
            x: (vertex.x / tolerance).round() as i64,
            y: (vertex.y / tolerance).round() as i64,
            z: (vertex.z / tolerance).round() as i64,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct EdgeKey {
    min: u32,
    max: u32,
}

impl EdgeKey {
    fn new(a: u32, b: u32) -> Self {
        Self {
            min: a.min(b),
            max: a.max(b),
        }
    }

    fn vertices(self) -> [u32; 2] {
        [self.min, self.max]
    }
}

fn face_index_edges(indices: [u32; 3]) -> [[u32; 2]; 3] {
    [
        [indices[0], indices[1]],
        [indices[1], indices[2]],
        [indices[2], indices[0]],
    ]
}

fn face_centroid(mesh: &IndexedMesh, face: IndexedFace) -> Vec3 {
    let [a, b, c] = face.indices.map(|index| mesh.vertices[index as usize]);
    (a + b + c) / 3.0
}

fn trace_boundary_loops(edges: &[EdgeRecord], boundary_edges: &[u32]) -> Vec<BoundaryLoop> {
    let mut vertex_edges = HashMap::<u32, Vec<u32>>::new();
    for &edge_index in boundary_edges {
        let edge = &edges[edge_index as usize];
        vertex_edges
            .entry(edge.vertices[0])
            .or_default()
            .push(edge_index);
        vertex_edges
            .entry(edge.vertices[1])
            .or_default()
            .push(edge_index);
    }

    let mut visited = HashMap::<u32, bool>::new();
    let mut loops = Vec::new();
    for &start_edge in boundary_edges {
        if visited.contains_key(&start_edge) {
            continue;
        }
        let start_vertices = edges[start_edge as usize].vertices;
        let mut vertices = vec![start_vertices[0], start_vertices[1]];
        let mut loop_edges = vec![start_edge];
        visited.insert(start_edge, true);

        let closed = extend_boundary_walk(
            edges,
            &vertex_edges,
            &mut visited,
            &mut vertices,
            &mut loop_edges,
            true,
        ) || extend_boundary_walk(
            edges,
            &vertex_edges,
            &mut visited,
            &mut vertices,
            &mut loop_edges,
            false,
        );

        loops.push(BoundaryLoop {
            vertices,
            edges: loop_edges,
            closed,
        });
    }
    loops
}

fn extend_boundary_walk(
    edges: &[EdgeRecord],
    vertex_edges: &HashMap<u32, Vec<u32>>,
    visited: &mut HashMap<u32, bool>,
    vertices: &mut Vec<u32>,
    loop_edges: &mut Vec<u32>,
    forward: bool,
) -> bool {
    loop {
        let current_vertex = if forward {
            *vertices.last().unwrap()
        } else {
            vertices[0]
        };
        let Some(next_edge) = next_unvisited_boundary_edge(vertex_edges, visited, current_vertex)
        else {
            return false;
        };
        visited.insert(next_edge, true);
        let edge = &edges[next_edge as usize];
        let next_vertex = if edge.vertices[0] == current_vertex {
            edge.vertices[1]
        } else {
            edge.vertices[0]
        };
        let closes_loop = if forward {
            next_vertex == vertices[0]
        } else {
            next_vertex == *vertices.last().unwrap()
        };
        if forward {
            loop_edges.push(next_edge);
        } else {
            loop_edges.insert(0, next_edge);
        }
        if closes_loop {
            return true;
        }
        if forward {
            vertices.push(next_vertex);
        } else {
            vertices.insert(0, next_vertex);
        }
    }
}

fn next_unvisited_boundary_edge(
    vertex_edges: &HashMap<u32, Vec<u32>>,
    visited: &HashMap<u32, bool>,
    vertex: u32,
) -> Option<u32> {
    vertex_edges
        .get(&vertex)?
        .iter()
        .copied()
        .find(|edge| !visited.contains_key(edge))
}

fn intersect_indexed_face(
    mesh: &IndexedMesh,
    face_index: u32,
    ray: Ray,
) -> Option<IntersectionHit> {
    let face = mesh.faces[face_index as usize];
    let [a, b, c] = face.indices.map(|index| mesh.vertices[index as usize]);
    let edge1 = b - a;
    let edge2 = c - a;
    let h = ray.direction.cross(edge2);
    let determinant = edge1.dot(h);
    if determinant.abs() < RAY_EPSILON {
        return None;
    }

    let inverse = 1.0 / determinant;
    let s = ray.origin - a;
    let u = inverse * s.dot(h);
    if !(0.0..=1.0).contains(&u) {
        return None;
    }

    let q = s.cross(edge1);
    let v = inverse * ray.direction.dot(q);
    if v < 0.0 || u + v > 1.0 {
        return None;
    }

    let distance = inverse * edge2.dot(q);
    if distance <= RAY_EPSILON {
        return None;
    }

    let normal = face.normal.normalize_or_zero();
    Some(IntersectionHit {
        face_index,
        source_id: face.source_id,
        component_id: mesh.connectivity.component_ids[face_index as usize],
        distance,
        position: ray.origin + ray.direction * distance,
        normal,
        front_face: normal.dot(ray.direction) < 0.0,
    })
}

#[derive(Debug, Clone)]
struct UnionFind {
    parents: Vec<usize>,
    ranks: Vec<u8>,
}

impl UnionFind {
    fn new(size: usize) -> Self {
        Self {
            parents: (0..size).collect(),
            ranks: vec![0; size],
        }
    }

    fn find(&mut self, index: usize) -> usize {
        let parent = self.parents[index];
        if parent != index {
            let root = self.find(parent);
            self.parents[index] = root;
        }
        self.parents[index]
    }

    fn union(&mut self, a: usize, b: usize) {
        let root_a = self.find(a);
        let root_b = self.find(b);
        if root_a == root_b {
            return;
        }
        match self.ranks[root_a].cmp(&self.ranks[root_b]) {
            Ordering::Less => self.parents[root_a] = root_b,
            Ordering::Greater => self.parents[root_b] = root_a,
            Ordering::Equal => {
                self.parents[root_b] = root_a;
                self.ranks[root_a] += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn welds_vertices_and_finds_single_boundary_loop() {
        let mesh = IndexedMesh::from_triangles(
            vec![
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
            ],
            1.0e-6,
        );

        assert_eq!(mesh.vertices.len(), 4);
        assert_eq!(mesh.connectivity.component_count, 1);
        assert_eq!(mesh.connectivity.boundary_edges.len(), 4);
        assert_eq!(mesh.connectivity.boundary_loops.len(), 1);
        assert!(mesh.connectivity.boundary_loops[0].closed);
    }

    #[test]
    fn reports_non_manifold_edges() {
        let mesh = IndexedMesh::from_triangles(
            vec![
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
                        Vec3::new(1.0, 0.0, 0.0),
                        Vec3::new(0.0, 0.0, 0.0),
                        Vec3::new(0.0, -1.0, 0.0),
                    ),
                ),
                (
                    id(2),
                    tri(
                        Vec3::new(0.0, 0.0, 0.0),
                        Vec3::new(1.0, 0.0, 0.0),
                        Vec3::new(0.0, 0.0, 1.0),
                    ),
                ),
            ],
            1.0e-6,
        );

        assert_eq!(mesh.connectivity.non_manifold_edges.len(), 1);
    }

    #[test]
    fn traces_single_triangle_boundary_loop() {
        let mesh = IndexedMesh::from_triangles(
            vec![(
                id(0),
                tri(
                    Vec3::new(0.0, 0.0, 0.0),
                    Vec3::new(1.0, 0.0, 0.0),
                    Vec3::new(0.0, 1.0, 0.0),
                ),
            )],
            1.0e-6,
        );

        assert_eq!(mesh.connectivity.boundary_edges.len(), 3);
        assert_eq!(mesh.connectivity.boundary_loops.len(), 1);
        assert!(mesh.connectivity.boundary_loops[0].closed);
    }

    #[test]
    fn bvh_returns_ordered_nested_cube_hit_stack() {
        let selection = SelectionMesh::from_triangles(nested_cubes(), 1.0e-6);
        let ray = Ray {
            origin: Vec3::new(0.2, 0.1, 3.0),
            direction: Vec3::new(0.0, 0.0, -1.0),
        };

        let hits = selection.hit_stack(ray, None);

        assert_eq!(hits.len(), 4);
        assert!(hits
            .windows(2)
            .all(|pair| pair[0].distance <= pair[1].distance));
        assert!(hits[0].position.z > hits[1].position.z);
        assert!(hits.iter().any(|hit| !hit.front_face));
    }

    #[test]
    fn hit_stack_respects_cross_section_plane() {
        let selection = SelectionMesh::from_triangles(nested_cubes(), 1.0e-6);
        let ray = Ray {
            origin: Vec3::new(0.2, 0.1, 3.0),
            direction: Vec3::new(0.0, 0.0, -1.0),
        };
        let plane = CrossSectionPlane {
            normal: Vec3::Z,
            offset: 0.0,
        };

        let hits = selection.hit_stack(ray, Some(plane));

        assert_eq!(hits.len(), 2);
        assert!(hits.iter().all(|hit| hit.position.z >= 0.0));
    }

    #[test]
    fn brush_radius_selects_neighboring_surface_faces() {
        let selection = SelectionMesh::from_triangles(
            vec![
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
            ],
            1.0e-6,
        );

        let faces = selection.surface_faces_within_radius(id(0), Vec3::new(0.5, 0.5, 0.0), 0.8, 8);

        assert_eq!(faces.len(), 2);
        assert!(faces.iter().any(|face| face.source_id == id(1)));
    }

    #[test]
    fn cut_preview_finds_cube_midline_segments() {
        let cube = nested_cubes()
            .into_iter()
            .take(12)
            .map(|(_, triangle)| triangle)
            .collect::<Vec<_>>();
        let plane = CutPlane::from_point_normal(Vec3::ZERO, Vec3::X).unwrap();

        let preview = preview_cut(&cube, plane, 1.0e-6);

        assert!(!preview.segments.is_empty());
        assert_eq!(preview.affected_triangle_count, preview.segments.len());
        assert!(preview
            .segments
            .iter()
            .flat_map(|segment| segment.iter())
            .all(|point| point.x.abs() < 1.0e-5));
    }

    #[test]
    fn cut_preview_ignores_single_vertex_plane_touch() {
        let triangle = tri(
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(1.0, 1.0, 0.0),
        );
        let plane = CutPlane::from_point_normal(Vec3::ZERO, Vec3::X).unwrap();

        let preview = preview_cut(&[triangle], plane, 1.0e-6);

        assert!(preview.segments.is_empty());
    }

    #[test]
    fn view_line_plane_contains_eye_and_rays() {
        let eye = Vec3::new(0.0, 0.0, 4.0);
        let start = Vec3::new(-0.25, 0.0, -1.0);
        let end = Vec3::new(0.25, 0.0, -1.0);

        let plane = cut_plane_from_view_rays(eye, start, end).expect("rays define a plane");

        assert!(plane.signed_distance(eye).abs() < 1.0e-6);
        assert!(plane.signed_distance(eye + start.normalize()).abs() < 1.0e-6);
        assert!(plane.signed_distance(eye + end.normalize()).abs() < 1.0e-6);
    }

    #[test]
    fn split_and_cap_cube_creates_two_closed_pieces() {
        let cube = nested_cubes()
            .into_iter()
            .take(12)
            .map(|(_, triangle)| triangle)
            .collect::<Vec<_>>();
        let plane = CutPlane::from_point_normal(Vec3::ZERO, Vec3::X).unwrap();

        let result =
            split_and_cap_mesh(&cube, plane, CutMeshOptions::default()).expect("cube should cut");

        assert_eq!(result.pieces.len(), 2);
        assert_eq!(result.loops.len(), 1);
        assert!(result.loops[0].closed);
        assert!(result.pieces[0].cap_triangle_count > 1);
        assert!(result.pieces[1].cap_triangle_count > 1);
        for piece in &result.pieces {
            let mesh = IndexedMesh::from_triangles(
                piece
                    .triangles
                    .iter()
                    .copied()
                    .enumerate()
                    .map(|(index, triangle)| (id(index as u32), triangle)),
                1.0e-5,
            );
            assert_eq!(mesh.connectivity.boundary_loops.len(), 0);
        }
    }

    #[test]
    fn split_and_cap_cylinder_creates_dense_closed_caps() {
        let cylinder = cylinder(24, 1.0, 1.0);
        let plane = CutPlane::from_point_normal(Vec3::ZERO, Vec3::Z).unwrap();

        let result = split_and_cap_mesh(
            &cylinder,
            plane,
            CutMeshOptions {
                smooth_cap: true,
                ..CutMeshOptions::default()
            },
        )
        .expect("cylinder cut");

        assert_eq!(result.loops.len(), 1);
        for piece in &result.pieces {
            assert!(piece.cap_triangle_count >= 24);
            assert_closed(&piece.triangles);
        }
    }

    #[test]
    fn split_and_cap_cube_handles_slanted_plane() {
        let cube = nested_cubes()
            .into_iter()
            .take(12)
            .map(|(_, triangle)| triangle)
            .collect::<Vec<_>>();
        let plane = CutPlane::from_point_normal(Vec3::ZERO, Vec3::new(1.0, 1.0, 0.35)).unwrap();

        let result =
            split_and_cap_mesh(&cube, plane, CutMeshOptions::default()).expect("slanted cut");

        assert!(!result.loops.is_empty());
        for piece in &result.pieces {
            assert!(piece.cap_triangle_count > 1);
            assert_closed(&piece.triangles);
        }
    }

    #[test]
    fn cut_target_edge_length_tracks_loop_edges() {
        let cut_loop = CutLoop {
            vertices: vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(1.0, 1.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
            ],
            closed: true,
            length: 4.0,
        };
        let bounds = MeshBounds {
            min: Vec3::ZERO,
            max: Vec3::ONE,
        };

        let target = target_edge_length_for_loops(&[cut_loop], bounds, CapDensity::Automatic);

        assert!((0.9..=1.1).contains(&target));
    }

    #[test]
    fn cap_loop_detection_rejects_self_intersections() {
        let bow_tie = vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(0.0, 1.0),
            Vec2::new(1.0, 0.0),
        ];

        assert!(polygon_self_intersects(&bow_tie));
    }

    fn id(local_index: u32) -> TriangleId {
        TriangleId {
            chunk: 0,
            local_index,
        }
    }

    fn tri(a: Vec3, b: Vec3, c: Vec3) -> Triangle {
        let normal = (b - a).cross(c - a).normalize_or_zero();
        Triangle {
            normal,
            vertices: [a, b, c],
        }
    }

    fn assert_closed(triangles: &[Triangle]) {
        let mesh = IndexedMesh::from_triangles(
            triangles
                .iter()
                .copied()
                .enumerate()
                .map(|(index, triangle)| (id(index as u32), triangle)),
            1.0e-5,
        );
        assert_eq!(mesh.connectivity.boundary_loops.len(), 0);
    }

    fn nested_cubes() -> Vec<(TriangleId, Triangle)> {
        let mut triangles = Vec::new();
        append_cube(&mut triangles, 1.0, 0);
        append_cube(&mut triangles, 0.5, 100);
        triangles
    }

    fn append_cube(output: &mut Vec<(TriangleId, Triangle)>, half: f32, id_offset: u32) {
        let p = |x, y, z| Vec3::new(x, y, z) * half;
        let faces = [
            [
                p(-1.0, -1.0, 1.0),
                p(1.0, -1.0, 1.0),
                p(1.0, 1.0, 1.0),
                p(-1.0, 1.0, 1.0),
            ],
            [
                p(1.0, -1.0, -1.0),
                p(-1.0, -1.0, -1.0),
                p(-1.0, 1.0, -1.0),
                p(1.0, 1.0, -1.0),
            ],
            [
                p(-1.0, -1.0, -1.0),
                p(-1.0, -1.0, 1.0),
                p(-1.0, 1.0, 1.0),
                p(-1.0, 1.0, -1.0),
            ],
            [
                p(1.0, -1.0, 1.0),
                p(1.0, -1.0, -1.0),
                p(1.0, 1.0, -1.0),
                p(1.0, 1.0, 1.0),
            ],
            [
                p(-1.0, 1.0, 1.0),
                p(1.0, 1.0, 1.0),
                p(1.0, 1.0, -1.0),
                p(-1.0, 1.0, -1.0),
            ],
            [
                p(-1.0, -1.0, -1.0),
                p(1.0, -1.0, -1.0),
                p(1.0, -1.0, 1.0),
                p(-1.0, -1.0, 1.0),
            ],
        ];

        for (face_index, [a, b, c, d]) in faces.into_iter().enumerate() {
            let first = id_offset + face_index as u32 * 2;
            output.push((id(first), tri(a, b, c)));
            output.push((id(first + 1), tri(a, c, d)));
        }
    }

    fn cylinder(segments: usize, radius: f32, half_height: f32) -> Vec<Triangle> {
        let mut triangles = Vec::new();
        let top_center = Vec3::new(0.0, 0.0, half_height);
        let bottom_center = Vec3::new(0.0, 0.0, -half_height);
        for index in 0..segments {
            let angle = index as f32 / segments as f32 * std::f32::consts::TAU;
            let next_angle = (index + 1) as f32 / segments as f32 * std::f32::consts::TAU;
            let bottom_a = Vec3::new(angle.cos() * radius, angle.sin() * radius, -half_height);
            let bottom_b = Vec3::new(
                next_angle.cos() * radius,
                next_angle.sin() * radius,
                -half_height,
            );
            let top_a = Vec3::new(angle.cos() * radius, angle.sin() * radius, half_height);
            let top_b = Vec3::new(
                next_angle.cos() * radius,
                next_angle.sin() * radius,
                half_height,
            );
            triangles.push(tri(bottom_a, bottom_b, top_b));
            triangles.push(tri(bottom_a, top_b, top_a));
            triangles.push(tri(top_center, top_a, top_b));
            triangles.push(tri(bottom_center, bottom_b, bottom_a));
        }
        triangles
    }
}

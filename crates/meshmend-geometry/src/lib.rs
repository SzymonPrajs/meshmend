use std::{
    cmp::Ordering,
    collections::{HashMap, VecDeque},
};

use glam::Vec3;
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
}

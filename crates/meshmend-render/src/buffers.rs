use meshmend_core::{MeshBounds, Triangle};

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuTriangle {
    pub p0: [f32; 4],
    pub p1: [f32; 4],
    pub p2: [f32; 4],
    pub normal: [f32; 4],
}

impl From<Triangle> for GpuTriangle {
    fn from(triangle: Triangle) -> Self {
        Self {
            p0: triangle.vertices[0].extend(1.0).to_array(),
            p1: triangle.vertices[1].extend(1.0).to_array(),
            p2: triangle.vertices[2].extend(1.0).to_array(),
            normal: triangle.normal.extend(0.0).to_array(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct MeshChunkUpload<'a> {
    pub chunk_index: u32,
    pub start_triangle: u64,
    pub bounds: MeshBounds,
    pub triangles: &'a [Triangle],
}

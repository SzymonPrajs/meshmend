use std::path::Path;

use meshmend_core::{MeshStats, Triangle};

#[derive(Debug, Clone)]
pub struct TriangleChunk {
    pub chunk_index: u32,
    pub start_triangle: u64,
    pub triangles: Vec<Triangle>,
}

#[derive(Debug, Clone)]
pub struct ParsedStl {
    pub file_name: String,
    pub source_bytes: u64,
    pub stats: MeshStats,
    pub chunks: Vec<TriangleChunk>,
}

pub fn load_binary_stl(_path: &Path) -> Result<ParsedStl, StlError> {
    Err(StlError::NotImplemented)
}

#[derive(Debug, thiserror::Error)]
pub enum StlError {
    #[error("binary STL loading is not implemented yet")]
    NotImplemented,
}

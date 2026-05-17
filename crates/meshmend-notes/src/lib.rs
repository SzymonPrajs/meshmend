use meshmend_core::TriangleId;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Note {
    pub id: Uuid,
    pub triangle: TriangleId,
    pub position: [f32; 3],
    pub label: String,
    pub color: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteSession {
    pub version: u32,
    pub model_file_name: String,
    pub model_file_size: u64,
    pub notes: Vec<Note>,
}

impl NoteSession {
    pub const VERSION: u32 = 1;

    pub fn new(model_file_name: impl Into<String>, model_file_size: u64) -> Self {
        Self {
            version: Self::VERSION,
            model_file_name: model_file_name.into(),
            model_file_size,
            notes: Vec::new(),
        }
    }
}

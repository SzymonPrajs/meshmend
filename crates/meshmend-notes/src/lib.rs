use std::{fs, path::Path};

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

    pub fn add_note(&mut self, triangle: TriangleId, position: [f32; 3]) -> Uuid {
        let id = Uuid::new_v4();
        self.notes.push(Note {
            id,
            triangle,
            position,
            label: "Note".to_string(),
            color: "#ffb347".to_string(),
        });
        id
    }

    pub fn remove_note(&mut self, id: Uuid) {
        self.notes.retain(|note| note.id != id);
    }

    pub fn save_to_path(&self, path: &Path) -> Result<(), NoteError> {
        let json = serde_json::to_string_pretty(self)?;
        fs::write(path, json)?;
        Ok(())
    }

    pub fn load_from_path(path: &Path) -> Result<Self, NoteError> {
        let json = fs::read_to_string(path)?;
        let session: Self = serde_json::from_str(&json)?;
        if session.version != Self::VERSION {
            return Err(NoteError::UnsupportedVersion {
                version: session.version,
            });
        }
        Ok(session)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum NoteError {
    #[error("unsupported note session version {version}")]
    UnsupportedVersion { version: u32 },
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use meshmend_core::TriangleId;

    use super::*;

    #[test]
    fn serializes_session() {
        let mut session = NoteSession::new("raw.stl", 123);
        let id = session.add_note(
            TriangleId {
                chunk: 1,
                local_index: 2,
            },
            [0.1, 0.2, 0.3],
        );

        let json = serde_json::to_string(&session).unwrap();
        let loaded: NoteSession = serde_json::from_str(&json).unwrap();

        assert_eq!(loaded.version, NoteSession::VERSION);
        assert_eq!(loaded.notes[0].id, id);
        assert_eq!(loaded.notes[0].triangle.chunk, 1);
    }

    #[test]
    fn rejects_unknown_version() {
        let json = r#"{"version":99,"model_file_name":"raw.stl","model_file_size":1,"notes":[]}"#;
        let session: NoteSession = serde_json::from_str(json).unwrap();

        assert_ne!(session.version, NoteSession::VERSION);
    }
}

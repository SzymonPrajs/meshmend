use std::{fs, path::Path};

use meshmend_core::{CrossSectionAxis, CrossSectionState, TriangleId};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum IssueKind {
    #[default]
    InternalGap,
    TunnelOrCavity,
    OpenBoundary,
    OverlappingSheet,
    DetachedShell,
    ThinOrFragileArea,
    Other,
}

impl IssueKind {
    pub const ALL: [Self; 7] = [
        Self::InternalGap,
        Self::TunnelOrCavity,
        Self::OpenBoundary,
        Self::OverlappingSheet,
        Self::DetachedShell,
        Self::ThinOrFragileArea,
        Self::Other,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::InternalGap => "Internal gap",
            Self::TunnelOrCavity => "Tunnel or cavity",
            Self::OpenBoundary => "Open boundary",
            Self::OverlappingSheet => "Overlapping sheet",
            Self::DetachedShell => "Detached shell",
            Self::ThinOrFragileArea => "Thin or fragile area",
            Self::Other => "Other",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum IssueStatus {
    #[default]
    Open,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BrushLabelKind {
    #[default]
    HealthyBoundary,
    RepairTarget,
    Exclude,
}

impl BrushLabelKind {
    pub const ALL: [Self; 3] = [Self::HealthyBoundary, Self::RepairTarget, Self::Exclude];

    pub fn label(self) -> &'static str {
        match self {
            Self::HealthyBoundary => "Keep ring",
            Self::RepairTarget => "Fill target",
            Self::Exclude => "Protect",
        }
    }

    pub fn color(self) -> [f32; 4] {
        match self {
            Self::HealthyBoundary => [1.0, 0.84, 0.12, 1.0],
            Self::RepairTarget => [1.0, 0.25, 0.16, 1.0],
            Self::Exclude => [0.55, 0.66, 1.0, 1.0],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrushSample {
    pub triangle: TriangleId,
    pub position: [f32; 3],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrushStroke {
    pub id: Uuid,
    pub kind: BrushLabelKind,
    pub radius: f32,
    pub samples: Vec<BrushSample>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Issue {
    pub id: Uuid,
    pub kind: IssueKind,
    pub triangle: TriangleId,
    pub position: [f32; 3],
    pub cross_section_axis: CrossSectionAxis,
    pub cross_section_offset: f32,
    pub cross_section_flipped: bool,
    pub label: String,
    pub status: IssueStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueSession {
    pub version: u32,
    pub model_file_name: String,
    pub model_file_size: u64,
    pub issues: Vec<Issue>,
    #[serde(default)]
    pub label_strokes: Vec<BrushStroke>,
}

impl IssueSession {
    pub const VERSION: u32 = 1;

    pub fn new(model_file_name: impl Into<String>, model_file_size: u64) -> Self {
        Self {
            version: Self::VERSION,
            model_file_name: model_file_name.into(),
            model_file_size,
            issues: Vec::new(),
            label_strokes: Vec::new(),
        }
    }

    pub fn add_issue(
        &mut self,
        kind: IssueKind,
        triangle: TriangleId,
        position: [f32; 3],
        cross_section: CrossSectionState,
    ) -> Uuid {
        let id = Uuid::new_v4();
        self.issues.push(Issue {
            id,
            kind,
            triangle,
            position,
            cross_section_axis: cross_section.axis,
            cross_section_offset: cross_section.offset,
            cross_section_flipped: cross_section.flip_side,
            label: kind.label().to_string(),
            status: IssueStatus::Open,
        });
        id
    }

    pub fn remove_issue(&mut self, id: Uuid) {
        self.issues.retain(|issue| issue.id != id);
    }

    pub fn start_label_stroke(&mut self, kind: BrushLabelKind, radius: f32) -> usize {
        let index = self.label_strokes.len();
        self.label_strokes.push(BrushStroke {
            id: Uuid::new_v4(),
            kind,
            radius,
            samples: Vec::new(),
        });
        index
    }

    pub fn add_label_sample(
        &mut self,
        stroke_index: usize,
        triangle: TriangleId,
        position: [f32; 3],
    ) {
        if let Some(stroke) = self.label_strokes.get_mut(stroke_index) {
            stroke.samples.push(BrushSample { triangle, position });
        }
    }

    pub fn remove_label_stroke(&mut self, index: usize) {
        if index < self.label_strokes.len() {
            self.label_strokes.remove(index);
        }
    }

    pub fn clear_label_strokes(&mut self) {
        self.label_strokes.clear();
    }

    pub fn discard_empty_label_strokes(&mut self) {
        self.label_strokes
            .retain(|stroke| !stroke.samples.is_empty());
    }

    pub fn save_to_path(&self, path: &Path) -> Result<(), SessionError> {
        let json = serde_json::to_string_pretty(self)?;
        fs::write(path, json)?;
        Ok(())
    }

    pub fn load_from_path(path: &Path) -> Result<Self, SessionError> {
        let json = fs::read_to_string(path)?;
        let session: Self = serde_json::from_str(&json)?;
        if session.version != Self::VERSION {
            return Err(SessionError::UnsupportedVersion {
                version: session.version,
            });
        }
        Ok(session)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("unsupported session version {version}")]
    UnsupportedVersion { version: u32 },
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use meshmend_core::{CrossSectionAxis, TriangleId};

    use super::*;

    #[test]
    fn serializes_issue_session() {
        let mut session = IssueSession::new("raw.stl", 123);
        let cross_section = CrossSectionState {
            enabled: true,
            axis: CrossSectionAxis::Y,
            offset: 4.5,
            flip_side: true,
            show_plane_guide: true,
        };
        let id = session.add_issue(
            IssueKind::TunnelOrCavity,
            TriangleId {
                chunk: 1,
                local_index: 2,
            },
            [0.1, 0.2, 0.3],
            cross_section,
        );

        let json = serde_json::to_string(&session).unwrap();
        let loaded: IssueSession = serde_json::from_str(&json).unwrap();

        assert_eq!(loaded.version, IssueSession::VERSION);
        assert_eq!(loaded.issues[0].id, id);
        assert_eq!(loaded.issues[0].kind, IssueKind::TunnelOrCavity);
        assert_eq!(loaded.issues[0].cross_section_axis, CrossSectionAxis::Y);
        assert_eq!(loaded.issues[0].cross_section_offset, 4.5);
        assert!(loaded.issues[0].cross_section_flipped);
    }

    #[test]
    fn serializes_brush_label_strokes() {
        let mut session = IssueSession::new("raw.stl", 123);
        let stroke = session.start_label_stroke(BrushLabelKind::HealthyBoundary, 0.25);
        session.add_label_sample(
            stroke,
            TriangleId {
                chunk: 4,
                local_index: 8,
            },
            [1.0, 2.0, 3.0],
        );

        let json = serde_json::to_string(&session).unwrap();
        let loaded: IssueSession = serde_json::from_str(&json).unwrap();

        assert_eq!(loaded.label_strokes.len(), 1);
        assert_eq!(
            loaded.label_strokes[0].kind,
            BrushLabelKind::HealthyBoundary
        );
        assert_eq!(loaded.label_strokes[0].radius, 0.25);
        assert_eq!(loaded.label_strokes[0].samples[0].triangle.chunk, 4);
    }

    #[test]
    fn rejects_unknown_version() {
        let json = r#"{"version":99,"model_file_name":"raw.stl","model_file_size":1,"issues":[],"label_strokes":[]}"#;
        let session: IssueSession = serde_json::from_str(json).unwrap();

        assert_ne!(session.version, IssueSession::VERSION);
    }
}

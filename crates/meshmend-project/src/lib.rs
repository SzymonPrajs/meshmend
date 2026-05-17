use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use meshmend_core::MeshStats;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const PROJECT_FILE_NAME: &str = "project.meshmend.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshMendProject {
    pub version: u32,
    pub metadata: ProjectMetadata,
    pub source: SourceMesh,
    pub scale: Option<ScaleCalibration>,
    pub printer_profile: PrinterProfile,
    pub mesh_revisions: Vec<MeshRevision>,
    pub current_revision: u32,
    pub undo_stack: Vec<u32>,
    pub redo_stack: Vec<u32>,
    pub operations: Vec<OperationRecord>,
    pub exports: Vec<ExportRecord>,
}

impl MeshMendProject {
    pub const VERSION: u32 = 1;

    pub fn new(
        name: impl Into<String>,
        source_path: impl Into<PathBuf>,
        source_hash: impl Into<String>,
        stats: MeshStats,
    ) -> Self {
        let source_path = source_path.into();
        let source_hash = source_hash.into();
        Self {
            version: Self::VERSION,
            metadata: ProjectMetadata {
                id: Uuid::new_v4(),
                name: name.into(),
                created_at_unix_ms: now_unix_ms(),
                updated_at_unix_ms: now_unix_ms(),
                app_version: env!("CARGO_PKG_VERSION").to_string(),
            },
            source: SourceMesh {
                path: source_path.clone(),
                file_name: source_path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| source_path.display().to_string()),
                hash: source_hash,
                bytes: stats.source_bytes,
                triangle_count: stats.triangle_count,
                bounds_min: stats.bounds.min.to_array(),
                bounds_max: stats.bounds.max.to_array(),
            },
            scale: None,
            printer_profile: PrinterProfile::default(),
            mesh_revisions: vec![MeshRevision {
                id: 0,
                label: "Source".to_string(),
                mesh_path: source_path,
                triangle_count: stats.triangle_count,
                created_by_operation: None,
                created_at_unix_ms: now_unix_ms(),
            }],
            current_revision: 0,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            operations: Vec::new(),
            exports: Vec::new(),
        }
    }

    pub fn record_operation(
        &mut self,
        kind: OperationKind,
        status: OperationStatus,
        parameters: serde_json::Value,
        selection: Vec<SelectionReference>,
    ) -> Uuid {
        let id = Uuid::new_v4();
        self.operations.push(OperationRecord {
            id,
            kind,
            status,
            input_revision: self.current_revision,
            output_revision: None,
            parameters,
            selection,
            validation: ValidationSummary::default(),
            warnings: Vec::new(),
            started_at_unix_ms: now_unix_ms(),
            finished_at_unix_ms: Some(now_unix_ms()),
            worker_log: None,
            preview_mesh: None,
        });
        self.touch();
        id
    }

    pub fn apply_mesh_revision(
        &mut self,
        operation_id: Uuid,
        label: impl Into<String>,
        mesh_path: impl Into<PathBuf>,
        triangle_count: u64,
        validation: ValidationSummary,
    ) -> u32 {
        let revision_id = self
            .mesh_revisions
            .iter()
            .map(|revision| revision.id)
            .max()
            .unwrap_or(0)
            + 1;
        self.undo_stack.push(self.current_revision);
        self.redo_stack.clear();
        self.current_revision = revision_id;
        self.mesh_revisions.push(MeshRevision {
            id: revision_id,
            label: label.into(),
            mesh_path: mesh_path.into(),
            triangle_count,
            created_by_operation: Some(operation_id),
            created_at_unix_ms: now_unix_ms(),
        });
        if let Some(operation) = self
            .operations
            .iter_mut()
            .find(|operation| operation.id == operation_id)
        {
            operation.status = OperationStatus::Applied;
            operation.output_revision = Some(revision_id);
            operation.validation = validation;
            operation.finished_at_unix_ms = Some(now_unix_ms());
        }
        self.touch();
        revision_id
    }

    pub fn undo(&mut self) -> Option<u32> {
        let previous = self.undo_stack.pop()?;
        self.redo_stack.push(self.current_revision);
        self.current_revision = previous;
        self.touch();
        Some(previous)
    }

    pub fn redo(&mut self) -> Option<u32> {
        let next = self.redo_stack.pop()?;
        self.undo_stack.push(self.current_revision);
        self.current_revision = next;
        self.touch();
        Some(next)
    }

    pub fn current_revision(&self) -> Option<&MeshRevision> {
        self.mesh_revisions
            .iter()
            .find(|revision| revision.id == self.current_revision)
    }

    pub fn add_export(
        &mut self,
        kind: ExportKind,
        path: impl Into<PathBuf>,
        validation: ValidationSummary,
    ) -> Uuid {
        let id = Uuid::new_v4();
        self.exports.push(ExportRecord {
            id,
            kind,
            path: path.into(),
            revision: self.current_revision,
            validation,
            created_at_unix_ms: now_unix_ms(),
        });
        self.touch();
        id
    }

    pub fn save_to_dir(&mut self, directory: &Path) -> Result<PathBuf, ProjectError> {
        fs::create_dir_all(directory.join("meshes"))?;
        fs::create_dir_all(directory.join("previews"))?;
        fs::create_dir_all(directory.join("logs"))?;
        fs::create_dir_all(directory.join("reports"))?;
        self.touch();
        let path = directory.join(PROJECT_FILE_NAME);
        fs::write(&path, serde_json::to_string_pretty(self)?)?;
        Ok(path)
    }

    pub fn load_from_dir(directory: &Path) -> Result<Self, ProjectError> {
        let path = if directory.is_dir() {
            directory.join(PROJECT_FILE_NAME)
        } else {
            directory.to_path_buf()
        };
        let json = fs::read_to_string(path)?;
        let project: Self = serde_json::from_str(&json)?;
        if project.version != Self::VERSION {
            return Err(ProjectError::UnsupportedVersion {
                version: project.version,
            });
        }
        Ok(project)
    }

    pub fn write_markdown_report(&mut self, path: &Path) -> Result<(), ProjectError> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, self.markdown_report())?;
        self.add_export(
            ExportKind::ReportMarkdown,
            path,
            ValidationSummary::default(),
        );
        Ok(())
    }

    pub fn markdown_report(&self) -> String {
        let mut report = String::new();
        report.push_str("# MeshMend Repair Report\n\n");
        report.push_str(&format!("Project: {}\n\n", self.metadata.name));
        report.push_str(&format!("Source: {}\n\n", self.source.path.display()));
        report.push_str(&format!("Source hash: `{}`\n\n", self.source.hash));
        report.push_str(&format!("Current revision: {}\n\n", self.current_revision));
        report.push_str("## Mesh\n\n");
        report.push_str(&format!("- Triangles: {}\n", self.source.triangle_count));
        report.push_str(&format!("- Source bytes: {}\n", self.source.bytes));
        if let Some(scale) = &self.scale {
            report.push_str(&format!(
                "- Scale: {:.6} model units/mm\n",
                scale.model_units_per_mm
            ));
        } else {
            report.push_str("- Scale: uncalibrated\n");
        }
        report.push_str("\n## Operations\n\n");
        if self.operations.is_empty() {
            report.push_str("No operations recorded.\n");
        } else {
            for operation in &self.operations {
                report.push_str(&format!(
                    "- {:?} {:?} on revision {}\n",
                    operation.kind, operation.status, operation.input_revision
                ));
            }
        }
        report
    }

    fn touch(&mut self) {
        self.metadata.updated_at_unix_ms = now_unix_ms();
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMetadata {
    pub id: Uuid,
    pub name: String,
    pub created_at_unix_ms: u128,
    pub updated_at_unix_ms: u128,
    pub app_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceMesh {
    pub path: PathBuf,
    pub file_name: String,
    pub hash: String,
    pub bytes: u64,
    pub triangle_count: u64,
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScaleCalibration {
    pub model_units_per_mm: f64,
    pub reference_model_distance: f64,
    pub reference_real_distance_mm: f64,
    pub point_a: [f32; 3],
    pub point_b: [f32; 3],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrinterProfile {
    pub name: String,
    pub xy_pixel_microns: f64,
    pub layer_height_microns: f64,
    pub minimum_wall_microns: f64,
    pub surface_tolerance_microns: f64,
    pub target_edge_multiplier: f64,
}

impl Default for PrinterProfile {
    fn default() -> Self {
        Self {
            name: "Generic resin printer".to_string(),
            xy_pixel_microns: 20.0,
            layer_height_microns: 30.0,
            minimum_wall_microns: 400.0,
            surface_tolerance_microns: 20.0,
            target_edge_multiplier: 2.5,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshRevision {
    pub id: u32,
    pub label: String,
    pub mesh_path: PathBuf,
    pub triangle_count: u64,
    pub created_by_operation: Option<Uuid>,
    pub created_at_unix_ms: u128,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationKind {
    DefectRecord,
    RepairRegionStroke,
    CleanMesh,
    HoleFill,
    LocalCavityReplacement,
    SurfaceWrap,
    Cut,
    ScaleCalibration,
    Remesh,
    Export,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationStatus {
    Planned,
    Previewed,
    Applied,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationRecord {
    pub id: Uuid,
    pub kind: OperationKind,
    pub status: OperationStatus,
    pub input_revision: u32,
    pub output_revision: Option<u32>,
    pub parameters: serde_json::Value,
    pub selection: Vec<SelectionReference>,
    pub validation: ValidationSummary,
    pub warnings: Vec<String>,
    pub started_at_unix_ms: u128,
    pub finished_at_unix_ms: Option<u128>,
    pub worker_log: Option<PathBuf>,
    pub preview_mesh: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectionReference {
    pub triangle_chunk: u32,
    pub triangle_local_index: u32,
    pub position: [f32; 3],
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ValidationSummary {
    pub boundary_loop_count: Option<u64>,
    pub non_manifold_edge_count: Option<u64>,
    pub component_count: Option<u64>,
    pub self_intersection_count: Option<u64>,
    pub internal_cavity_count: Option<u64>,
    pub triangle_count: Option<u64>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExportKind {
    Stl,
    ReportJson,
    ReportMarkdown,
    Screenshot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportRecord {
    pub id: Uuid,
    pub kind: ExportKind,
    pub path: PathBuf,
    pub revision: u32,
    pub validation: ValidationSummary,
    pub created_at_unix_ms: u128,
}

pub fn project_directory_from_selection(path: &Path) -> PathBuf {
    if path.is_dir() {
        path.to_path_buf()
    } else if path
        .file_name()
        .is_some_and(|file_name| file_name == PROJECT_FILE_NAME)
    {
        path.parent().unwrap_or(path).to_path_buf()
    } else {
        path.to_path_buf()
    }
}

pub fn now_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("unsupported project version {version}")]
    UnsupportedVersion { version: u32 },
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use glam::Vec3;
    use meshmend_core::MeshBounds;

    use super::*;

    #[test]
    fn saves_and_loads_directory_project() {
        let root = std::env::temp_dir().join(format!("meshmend-test-{}", Uuid::new_v4()));
        let mut project = MeshMendProject::new("raw", PathBuf::from("raw.stl"), "hash", stats());
        project.record_operation(
            OperationKind::DefectRecord,
            OperationStatus::Applied,
            serde_json::json!({"kind": "open_boundary"}),
            Vec::new(),
        );

        let project_file = project.save_to_dir(&root).unwrap();
        let loaded = MeshMendProject::load_from_dir(&root).unwrap();

        assert_eq!(project_file.file_name().unwrap(), PROJECT_FILE_NAME);
        assert_eq!(loaded.version, MeshMendProject::VERSION);
        assert_eq!(loaded.operations.len(), 1);
        assert!(root.join("meshes").is_dir());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn undo_and_redo_revision_pointer() {
        let mut project = MeshMendProject::new("raw", PathBuf::from("raw.stl"), "hash", stats());
        let operation = project.record_operation(
            OperationKind::CleanMesh,
            OperationStatus::Previewed,
            serde_json::json!({}),
            Vec::new(),
        );

        let revision = project.apply_mesh_revision(
            operation,
            "cleaned",
            PathBuf::from("rev-0001.stl"),
            10,
            ValidationSummary::default(),
        );

        assert_eq!(revision, 1);
        assert_eq!(project.undo(), Some(0));
        assert_eq!(project.redo(), Some(1));
    }

    fn stats() -> MeshStats {
        MeshStats {
            triangle_count: 12,
            vertex_position_count: 36,
            bounds: MeshBounds {
                min: Vec3::splat(-1.0),
                max: Vec3::splat(1.0),
            },
            source_bytes: 100,
        }
    }
}

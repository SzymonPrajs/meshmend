use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const WORKER_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerRequest {
    pub schema_version: u32,
    pub operation_id: Uuid,
    pub operation: WorkerOperation,
    pub input_mesh: PathBuf,
    pub output_mesh: Option<PathBuf>,
    pub response_path: PathBuf,
    pub preview: bool,
    pub scale: Option<ScaleContext>,
    pub target_edge_length: Option<f64>,
    pub roi_bounds: Option<[[f32; 3]; 2]>,
    pub selected_faces: Vec<u64>,
    pub boundary_loops: Vec<Vec<u32>>,
    pub strokes: Vec<WorkerStroke>,
    pub options: serde_json::Value,
}

impl WorkerRequest {
    pub fn new(
        operation: WorkerOperation,
        input_mesh: impl Into<PathBuf>,
        response_path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            schema_version: WORKER_SCHEMA_VERSION,
            operation_id: Uuid::new_v4(),
            operation,
            input_mesh: input_mesh.into(),
            output_mesh: None,
            response_path: response_path.into(),
            preview: true,
            scale: None,
            target_edge_length: None,
            roi_bounds: None,
            selected_faces: Vec::new(),
            boundary_loops: Vec::new(),
            strokes: Vec::new(),
            options: serde_json::json!({}),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerOperation {
    CgalInspect,
    CleanMesh,
    HoleFill,
    Cut,
    Remesh,
    OpenVdbInspect,
    LocalSdfWrap,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ScaleContext {
    pub model_units_per_mm: f64,
    pub target_microns: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerStroke {
    pub kind: String,
    pub radius: f32,
    pub points: Vec<[f32; 3]>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerProgressEvent {
    pub event: WorkerEventKind,
    pub operation_id: Uuid,
    pub phase: Option<String>,
    pub current: Option<u64>,
    pub total: Option<u64>,
    pub message: String,
    pub artifact_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerEventKind {
    Started,
    Phase,
    Progress,
    Warning,
    Artifact,
    Done,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerResponse {
    pub schema_version: u32,
    pub operation_id: Uuid,
    pub success: bool,
    pub output_mesh: Option<PathBuf>,
    pub changed_bounds: Option<[[f32; 3]; 2]>,
    pub metrics: WorkerMetrics,
    pub warnings: Vec<String>,
    pub validation: WorkerValidationSummary,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkerMetrics {
    pub input_triangles: Option<u64>,
    pub output_triangles: Option<u64>,
    pub components: Option<u64>,
    pub boundary_loops: Option<u64>,
    pub non_manifold_edges: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkerValidationSummary {
    pub closed: Option<bool>,
    pub self_intersections: Option<u64>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct WorkerRunResult {
    pub response: WorkerResponse,
    pub progress: Vec<WorkerProgressEvent>,
}

pub struct WorkerRunner {
    binary: PathBuf,
}

impl WorkerRunner {
    pub fn new(binary: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
        }
    }

    pub fn run(
        &self,
        request: &WorkerRequest,
        request_path: &Path,
    ) -> Result<WorkerRunResult, WorkerError> {
        if let Some(parent) = request_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }
        let mut request_file = std::fs::File::create(request_path)?;
        request_file.write_all(serde_json::to_string_pretty(request)?.as_bytes())?;

        let mut child = Command::new(&self.binary)
            .arg("--request")
            .arg(request_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdout = child.stdout.take().ok_or(WorkerError::MissingStdout)?;
        let mut progress = Vec::new();
        for line in BufReader::new(stdout).lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            progress.push(serde_json::from_str::<WorkerProgressEvent>(&line)?);
        }
        let output = child.wait_with_output()?;
        if !output.status.success() {
            return Err(WorkerError::ProcessFailed {
                status: output.status.code(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }
        let response_json = std::fs::read_to_string(&request.response_path)?;
        let response = serde_json::from_str::<WorkerResponse>(&response_json)?;
        Ok(WorkerRunResult { response, progress })
    }
}

pub fn discover_worker_binary(name: &str) -> Option<PathBuf> {
    if let Ok(root) = std::env::var("MESHMEND_WORKER_DIR") {
        let candidate = PathBuf::from(root).join(name);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    for root in [
        PathBuf::from("target/workers/cpp"),
        PathBuf::from("target/workers/cpp/Release"),
        PathBuf::from("workers/cpp/build"),
    ] {
        let candidate = root.join(name);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| {
            exe.parent()
                .map(|parent| parent.join("../Resources/workers").join(name))
        })
        .filter(|candidate| candidate.exists())
}

#[derive(Debug, thiserror::Error)]
pub enum WorkerError {
    #[error("worker process did not expose stdout")]
    MissingStdout,
    #[error("worker process failed with status {status:?}: {stderr}")]
    ProcessFailed { status: Option<i32>, stderr: String },
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trips_json() {
        let request =
            WorkerRequest::new(WorkerOperation::CgalInspect, "input.stl", "response.json");

        let json = serde_json::to_string(&request).unwrap();
        let loaded: WorkerRequest = serde_json::from_str(&json).unwrap();

        assert_eq!(loaded.schema_version, WORKER_SCHEMA_VERSION);
        assert_eq!(loaded.operation, WorkerOperation::CgalInspect);
        assert!(loaded.preview);
    }
}

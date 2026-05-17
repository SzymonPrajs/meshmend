use std::path::PathBuf;

use glam::{Vec2, Vec3};
use meshmend_core::MeshBounds;
use meshmend_geometry::CapDensity;
use meshmend_render::Camera;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum AppCommand {
    LoadStl {
        path: PathBuf,
    },
    SetViewMode {
        mode: ViewModeName,
    },
    FitCamera,
    ResetCamera,
    SetCamera {
        camera: CameraState,
    },
    OrbitCamera {
        delta: [f32; 2],
    },
    PanCamera {
        delta: [f32; 2],
    },
    ZoomCamera {
        delta: f32,
    },
    SetTool {
        tool: ToolName,
    },
    SetSelectionElement {
        element: SelectionElementName,
    },
    SetSelectionDepth {
        depth: SelectionDepthName,
    },
    SetBrushRadius {
        radius: f32,
    },
    SelectAt {
        position: [f32; 2],
    },
    BrushSelect {
        center: [f32; 2],
        radius: Option<f32>,
    },
    ClearSelection,
    SetCutOptions {
        cap_density: CapDensityName,
        smooth_cap: bool,
    },
    PreviewViewLineCut {
        start: [f32; 2],
        end: [f32; 2],
    },
    ApplyCut,
    CancelCut,
    SelectObject {
        index: usize,
    },
    SelectObjectAt {
        position: [f32; 2],
    },
    HideSelectedObject,
    DeleteSelectedObject,
    KeepOnlySelectedObject,
    ShowAllObjects,
    ExportVisible {
        path: PathBuf,
    },
    ExportObject {
        index: usize,
        path: PathBuf,
    },
    ExportAllObjects {
        directory: PathBuf,
    },
    Screenshot {
        path: PathBuf,
    },
    StateReport {
        path: Option<PathBuf>,
    },
    WaitForSelectionAcceleration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum ViewModeName {
    Rendered,
    Wireframe,
    SurfaceWire,
    XrayWire,
    Transparent,
    Normals,
    Studio,
    Headlight,
}

impl ViewModeName {
    pub fn label(self) -> &'static str {
        match self {
            Self::Rendered => "Rendered",
            Self::Wireframe => "Wireframe",
            Self::SurfaceWire => "Surface Wire",
            Self::XrayWire => "X-Ray Wire",
            Self::Transparent => "Transparent",
            Self::Normals => "Normals",
            Self::Studio => "Studio",
            Self::Headlight => "Headlight",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum ToolName {
    Point,
    Brush,
    Cut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum SelectionElementName {
    Vertex,
    Edge,
    Face,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum SelectionDepthName {
    Front,
    Through,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum CapDensityName {
    Coarse,
    Automatic,
    Fine,
}

impl From<CapDensityName> for CapDensity {
    fn from(value: CapDensityName) -> Self {
        match value {
            CapDensityName::Coarse => CapDensity::Coarse,
            CapDensityName::Automatic => CapDensity::Automatic,
            CapDensityName::Fine => CapDensity::Fine,
        }
    }
}

impl From<CapDensity> for CapDensityName {
    fn from(value: CapDensity) -> Self {
        match value {
            CapDensity::Coarse => Self::Coarse,
            CapDensity::Automatic => Self::Automatic,
            CapDensity::Fine => Self::Fine,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraState {
    pub target: [f32; 3],
    pub distance: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub fov_y: f32,
    pub near: f32,
    pub far: f32,
    pub eye: [f32; 3],
    pub up: [f32; 3],
}

impl From<Camera> for CameraState {
    fn from(camera: Camera) -> Self {
        Self {
            target: vec3_to_array(camera.target),
            distance: camera.distance,
            yaw: camera.yaw,
            pitch: camera.pitch,
            fov_y: camera.fov_y,
            near: camera.near,
            far: camera.far,
            eye: vec3_to_array(camera.eye()),
            up: vec3_to_array(Vec3::Y),
        }
    }
}

impl From<CameraState> for Camera {
    fn from(value: CameraState) -> Self {
        Self {
            target: Vec3::from_array(value.target),
            distance: value.distance,
            yaw: value.yaw,
            pitch: value.pitch,
            fov_y: value.fov_y,
            near: value.near,
            far: value.far,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateSnapshot {
    pub file: Option<String>,
    pub triangles: usize,
    pub bounds: Option<BoundsSnapshot>,
    pub view_mode: ViewModeName,
    pub camera: CameraState,
    pub tool: ToolName,
    pub selection_element: SelectionElementName,
    pub selection_depth: SelectionDepthName,
    pub brush_radius: f32,
    pub selection: SelectionSnapshot,
    pub cut_preview: CutPreviewSnapshot,
    pub object_count: usize,
    pub visible_object_count: usize,
    pub selected_object: Option<usize>,
    pub objects: Vec<ObjectSnapshot>,
    pub dirty: bool,
    pub status: String,
    pub latest_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoundsSnapshot {
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub center: [f32; 3],
    pub radius: f32,
}

impl From<MeshBounds> for BoundsSnapshot {
    fn from(bounds: MeshBounds) -> Self {
        Self {
            min: vec3_to_array(bounds.min),
            max: vec3_to_array(bounds.max),
            center: vec3_to_array(bounds.center()),
            radius: bounds.radius(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectionSnapshot {
    pub vertices: usize,
    pub edges: usize,
    pub faces: usize,
}

impl SelectionSnapshot {
    pub fn total(&self) -> usize {
        self.vertices + self.edges + self.faces
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CutPreviewSnapshot {
    pub active: bool,
    pub segments: usize,
    pub affected_triangles: usize,
    pub start: Option<[f32; 2]>,
    pub end: Option<[f32; 2]>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectSnapshot {
    pub index: usize,
    pub name: String,
    pub side: String,
    pub visible: bool,
    pub selected: bool,
    pub triangles: usize,
    pub cap_triangles: usize,
    pub bounds: BoundsSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandStepReport {
    pub index: usize,
    pub command: AppCommand,
    pub ok: bool,
    pub status: String,
    pub state: StateSnapshot,
}

pub fn vec2_from_array(value: [f32; 2]) -> Vec2 {
    Vec2::new(value[0], value[1])
}

pub fn vec2_to_array(value: Vec2) -> [f32; 2] {
    [value.x, value.y]
}

pub fn vec3_to_array(value: Vec3) -> [f32; 3] {
    [value.x, value.y, value.z]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_kebab_case_commands_with_camel_case_fields() {
        let command: AppCommand = serde_json::from_str(
            r#"{"type":"set-cut-options","capDensity":"fine","smoothCap":true}"#,
        )
        .expect("command should parse");

        assert!(matches!(
            command,
            AppCommand::SetCutOptions {
                cap_density: CapDensityName::Fine,
                smooth_cap: true,
            }
        ));
    }

    #[test]
    fn parses_view_line_cut_command() {
        let command: AppCommand = serde_json::from_str(
            r#"{"type":"preview-view-line-cut","start":[10.0,20.0],"end":[30.0,40.0]}"#,
        )
        .expect("command should parse");

        assert!(matches!(
            command,
            AppCommand::PreviewViewLineCut { start, end }
                if start == [10.0, 20.0] && end == [30.0, 40.0]
        ));
    }

    #[test]
    fn camera_state_round_trips_renderer_camera() {
        let camera = Camera {
            target: Vec3::new(1.0, 2.0, 3.0),
            distance: 9.0,
            yaw: 0.5,
            ..Camera::default()
        };

        let state = CameraState::from(camera);
        let restored = Camera::from(state);

        assert_eq!(restored.target, camera.target);
        assert_eq!(restored.distance, camera.distance);
        assert_eq!(restored.yaw, camera.yaw);
    }
}

use std::{collections::BTreeMap, path::PathBuf};

use meshmend_stl::ParsedStl;
use serde::{Deserialize, Serialize};

use crate::commands::{AppCommand, CommandStepReport, StateSnapshot};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScenarioFile {
    pub name: String,
    #[serde(default)]
    pub input: Option<PathBuf>,
    #[serde(default)]
    pub viewport: ScenarioViewport,
    #[serde(default)]
    pub steps: Vec<AppCommand>,
    #[serde(default)]
    pub assertions: Vec<ScenarioAssertion>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScenarioViewport {
    pub width: u32,
    pub height: u32,
}

impl Default for ScenarioViewport {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 800,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum ScenarioAssertion {
    ObjectCountAtLeast {
        count: usize,
    },
    VisibleObjectCountAtLeast {
        count: usize,
    },
    SelectedObjectExists,
    ScreenshotNonblank {
        path: PathBuf,
        #[serde(default)]
        min_coverage: Option<f64>,
    },
    ExportReloads {
        path: PathBuf,
    },
    TriangleCountChanged,
    SelectionCountAtLeast {
        count: usize,
    },
    FaceSelectionCountAtLeast {
        count: usize,
    },
    CameraChanged,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScenarioRunReport {
    pub name: String,
    pub input: Option<PathBuf>,
    pub output_dir: PathBuf,
    pub steps: Vec<CommandStepReport>,
    pub assertions: Vec<AssertionReport>,
    pub image_stats: BTreeMap<String, ImageStats>,
    pub final_state: StateSnapshot,
    pub metrics: ScenarioMetrics,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssertionReport {
    pub assertion: ScenarioAssertion,
    pub ok: bool,
    pub message: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageStats {
    pub width: u32,
    pub height: u32,
    pub non_background_pixels: u64,
    pub coverage: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScenarioMetrics {
    pub initial_triangle_count: Option<usize>,
    pub final_triangle_count: usize,
    pub min_triangle_count: usize,
    pub max_triangle_count: usize,
    pub max_object_count: usize,
    pub max_visible_object_count: usize,
    pub saw_selected_object: bool,
    pub max_selection_count: usize,
    pub max_face_selection_count: usize,
    pub camera_changed: bool,
    pub exported_paths: Vec<PathBuf>,
}

pub fn parsed_stl_triangle_count(parsed: &ParsedStl) -> usize {
    parsed
        .chunks
        .iter()
        .map(|chunk| chunk.triangles.len())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_scenario_with_command_steps_and_assertions() {
        let scenario: ScenarioFile = serde_json::from_str(
            r#"{
              "name": "cube cut",
              "input": "fixtures/stl/cube_binary.stl",
              "viewport": {"width": 640, "height": 480},
              "steps": [
                {"type": "fit-camera"},
                {"type": "set-view-mode", "mode": "surface-wire"},
                {"type": "preview-view-line-cut", "start": [200, 120], "end": [420, 360]}
              ],
              "assertions": [
                {"type": "object-count-at-least", "count": 2}
              ]
            }"#,
        )
        .expect("scenario should parse");

        assert_eq!(scenario.name, "cube cut");
        assert_eq!(scenario.viewport.width, 640);
        assert_eq!(scenario.steps.len(), 3);
        assert!(matches!(
            scenario.assertions[0],
            ScenarioAssertion::ObjectCountAtLeast { count: 2 }
        ));
    }
}

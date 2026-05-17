use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use anyhow::{anyhow, Context, Result};
use glam::{Vec2, Vec3};
use meshmend_core::TriangleId;
use meshmend_render::{DisplaySettings, LightingMode, SelectionOverlay, WgpuRenderer};
use meshmend_stl::{load_binary_stl_with_options, LoadOptions, DEFAULT_CHUNK_TRIANGLES};
use winit::{
    dpi::PhysicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
};

use crate::{
    commands::{
        vec2_from_array, AppCommand, CameraState, CapDensityName, CommandStepReport,
        SelectionDepthName, SelectionElementName, SelectionSnapshot, StateSnapshot, ToolName,
        ViewModeName,
    },
    scenario::{
        parsed_stl_triangle_count, AssertionReport, ImageStats, ScenarioAssertion, ScenarioFile,
        ScenarioMetrics, ScenarioRunReport,
    },
    session::{bounds_for_triangles, MeshSession},
};

const DEFAULT_BRUSH_RADIUS: f32 = 44.0;
const SELECTION_WAIT_TIMEOUT: Duration = Duration::from_secs(5);

pub fn run_render_command(
    input: PathBuf,
    output: PathBuf,
    width: u32,
    height: u32,
    view: ViewModeName,
    camera_path: Option<PathBuf>,
    state_path: Option<PathBuf>,
) -> Result<()> {
    let commands = vec![
        AppCommand::LoadStl { path: input },
        AppCommand::SetViewMode { mode: view },
        if let Some(camera_path) = camera_path {
            let camera: CameraState = serde_json::from_str(
                &std::fs::read_to_string(&camera_path)
                    .with_context(|| format!("read camera file {}", camera_path.display()))?,
            )?;
            AppCommand::SetCamera { camera }
        } else {
            AppCommand::FitCamera
        },
        AppCommand::Screenshot {
            path: output.clone(),
        },
    ];
    let mut result = run_commands_hidden(
        "MeshMend scripted render",
        width,
        height,
        Path::new("."),
        Path::new("."),
        commands,
        Vec::new(),
    )?;
    if let Some(state_path) = state_path {
        write_json(&state_path, &result.final_state)?;
    }
    if let Some(stats) = result.image_stats.remove(&normalize_key(&output)) {
        println!(
            "render {}x{} non_background={} coverage={:.4}",
            stats.width, stats.height, stats.non_background_pixels, stats.coverage
        );
        if stats.coverage <= 0.001 {
            anyhow::bail!("render verification failed: image is blank");
        }
    }
    Ok(())
}

pub fn run_scenario(path: PathBuf, output_dir: PathBuf) -> Result<()> {
    let scenario_text =
        std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let scenario: ScenarioFile = serde_json::from_str(&scenario_text)
        .with_context(|| format!("parse scenario {}", path.display()))?;
    std::fs::create_dir_all(&output_dir)?;
    std::fs::write(output_dir.join("scenario-input.json"), scenario_text)?;
    let scenario_dir = path.parent().unwrap_or_else(|| Path::new("."));

    let mut commands = Vec::new();
    if let Some(input) = scenario.input.clone() {
        commands.push(AppCommand::LoadStl { path: input });
    }
    commands.extend(scenario.steps.clone());

    let report = run_commands_hidden(
        &format!("MeshMend scenario: {}", scenario.name),
        scenario.viewport.width,
        scenario.viewport.height,
        scenario_dir,
        &output_dir,
        commands,
        scenario.assertions.clone(),
    );

    match report {
        Ok(report) => {
            write_json(&output_dir.join("run-report.json"), &report)?;
            let failed = report
                .assertions
                .iter()
                .filter(|assertion| !assertion.ok)
                .count();
            if failed > 0 {
                anyhow::bail!("{failed} scenario assertion(s) failed; see run-report.json");
            }
            println!(
                "scenario '{}' passed: {} steps, {} assertions",
                report.name,
                report.steps.len(),
                report.assertions.len()
            );
            Ok(())
        }
        Err(err) => {
            let failure_path = output_dir.join("run-error.txt");
            std::fs::write(&failure_path, format!("{err:?}\n"))?;
            Err(err)
        }
    }
}

fn run_commands_hidden(
    title: &str,
    width: u32,
    height: u32,
    scenario_dir: &Path,
    output_dir: &Path,
    commands: Vec<AppCommand>,
    assertions: Vec<ScenarioAssertion>,
) -> Result<ScenarioRunReport> {
    let event_loop = EventLoop::new()?;
    let window = WindowBuilder::new()
        .with_title(title)
        .with_inner_size(PhysicalSize::new(width.max(1), height.max(1)))
        .with_visible(false)
        .build(&event_loop)?;
    let window: &'static winit::window::Window = Box::leak(Box::new(window));
    let window_id = window.id();
    let mut renderer = pollster::block_on(WgpuRenderer::new(window))?;
    let result: Arc<Mutex<Option<Result<ScenarioRunReport>>>> = Arc::new(Mutex::new(None));
    let result_writer = Arc::clone(&result);
    let scenario_dir = scenario_dir.to_path_buf();
    let output_dir = output_dir.to_path_buf();
    let mut commands = Some(commands);
    let mut assertions = Some(assertions);
    let mut ran = false;

    event_loop.run(move |event, target| {
        target.set_control_flow(ControlFlow::Wait);
        match event {
            Event::WindowEvent {
                window_id: event_window_id,
                event: WindowEvent::RedrawRequested,
            } if event_window_id == window_id && !ran => {
                ran = true;
                let run_result = {
                    let mut executor =
                        ScriptExecutor::new(&mut renderer, &scenario_dir, &output_dir);
                    executor.run(
                        commands.take().expect("commands should run once"),
                        assertions.take().expect("assertions should run once"),
                    )
                };
                *result_writer.lock().expect("scenario result lock poisoned") = Some(run_result);
                target.exit();
            }
            Event::AboutToWait if !ran => {
                window.request_redraw();
            }
            _ => {}
        }
    })?;

    let run_result = result
        .lock()
        .expect("scenario result lock poisoned")
        .take()
        .unwrap_or_else(|| Err(anyhow!("scripted render did not run")));
    run_result
}

struct ScriptExecutor<'a, 'window> {
    renderer: &'a mut WgpuRenderer<'window>,
    scenario_dir: PathBuf,
    output_dir: PathBuf,
    session: MeshSession,
    view_mode: ViewModeName,
    tool: ToolName,
    selection_element: SelectionElementName,
    selection_depth: SelectionDepthName,
    brush_radius: f32,
    cap_density: CapDensityName,
    smooth_cap: bool,
    selection: ScriptSelection,
    image_stats: BTreeMap<String, ImageStats>,
    exported_paths: Vec<PathBuf>,
    step_reports: Vec<CommandStepReport>,
    status: String,
    latest_error: Option<String>,
    initial_triangle_count: Option<usize>,
    min_triangle_count: usize,
    max_triangle_count: usize,
    max_object_count: usize,
    max_visible_object_count: usize,
    saw_selected_object: bool,
    max_selection_count: usize,
    max_face_selection_count: usize,
    initial_camera: Option<CameraState>,
    camera_changed: bool,
}

impl<'a, 'window> ScriptExecutor<'a, 'window> {
    fn new(
        renderer: &'a mut WgpuRenderer<'window>,
        scenario_dir: &Path,
        output_dir: &Path,
    ) -> Self {
        let initial_camera = CameraState::from(renderer.camera());
        Self {
            renderer,
            scenario_dir: scenario_dir.to_path_buf(),
            output_dir: output_dir.to_path_buf(),
            session: MeshSession::default(),
            view_mode: ViewModeName::Rendered,
            tool: ToolName::Point,
            selection_element: SelectionElementName::Face,
            selection_depth: SelectionDepthName::Front,
            brush_radius: DEFAULT_BRUSH_RADIUS,
            cap_density: CapDensityName::Automatic,
            smooth_cap: false,
            selection: ScriptSelection::default(),
            image_stats: BTreeMap::new(),
            exported_paths: Vec::new(),
            step_reports: Vec::new(),
            status: "Ready".to_string(),
            latest_error: None,
            initial_triangle_count: None,
            min_triangle_count: usize::MAX,
            max_triangle_count: 0,
            max_object_count: 0,
            max_visible_object_count: 0,
            saw_selected_object: false,
            max_selection_count: 0,
            max_face_selection_count: 0,
            initial_camera: Some(initial_camera),
            camera_changed: false,
        }
    }

    fn run(
        &mut self,
        commands: Vec<AppCommand>,
        assertions: Vec<ScenarioAssertion>,
    ) -> Result<ScenarioRunReport> {
        let mut failed_step = None;
        for (index, command) in commands.into_iter().enumerate() {
            let command_result = self.execute_command(&command);
            match command_result {
                Ok(status) => {
                    self.status = status;
                    self.latest_error = None;
                    self.update_metrics();
                    self.step_reports.push(CommandStepReport {
                        index,
                        command,
                        ok: true,
                        status: self.status.clone(),
                        state: self.snapshot(),
                    });
                }
                Err(err) => {
                    self.latest_error = Some(err.to_string());
                    self.status = format!("Step {index} failed: {err}");
                    self.update_metrics();
                    self.step_reports.push(CommandStepReport {
                        index,
                        command,
                        ok: false,
                        status: self.status.clone(),
                        state: self.snapshot(),
                    });
                    failed_step = Some(err);
                    break;
                }
            }
        }

        let assertion_reports = self.evaluate_assertions(&assertions);
        let report = self.report(assertion_reports);
        if let Some(err) = failed_step {
            write_json(&self.output_dir.join("run-report.json"), &report)?;
            return Err(err);
        }
        Ok(report)
    }

    fn execute_command(&mut self, command: &AppCommand) -> Result<String> {
        match command {
            AppCommand::LoadStl { path } => {
                let path = self.resolve_input_path(path);
                let report = self.session.load_stl(&path)?;
                self.upload_session_mesh()?;
                self.selection.clear();
                self.renderer
                    .set_selection_overlay(&SelectionOverlay::default());
                self.initial_triangle_count
                    .get_or_insert(report.stats.triangle_count as usize);
                Ok(format!(
                    "Loaded {} ({} triangles, {} chunks, parse {:.1} ms)",
                    report.file_name,
                    report.stats.triangle_count,
                    report.chunk_count,
                    report.parse_ms
                ))
            }
            AppCommand::SetViewMode { mode } => {
                self.view_mode = *mode;
                let mut settings = self.renderer.display_settings();
                apply_view_mode(*mode, &mut settings);
                self.renderer.set_display_settings(settings);
                Ok(format!("View: {}", mode.label()))
            }
            AppCommand::FitCamera => {
                self.renderer.fit_camera_to_mesh();
                self.mark_camera_changed();
                Ok("Framed mesh".to_string())
            }
            AppCommand::ResetCamera => {
                reset_camera(self.renderer);
                self.mark_camera_changed();
                Ok("Reset camera".to_string())
            }
            AppCommand::SetCamera { camera } => {
                self.renderer.set_camera((*camera).into());
                self.mark_camera_changed();
                Ok("Set camera".to_string())
            }
            AppCommand::OrbitCamera { delta } => {
                let mut camera = self.renderer.camera();
                camera.orbit(Vec2::from_array(*delta));
                self.renderer.set_camera(camera);
                self.mark_camera_changed();
                Ok(format!(
                    "Orbit camera by [{:.1}, {:.1}]",
                    delta[0], delta[1]
                ))
            }
            AppCommand::PanCamera { delta } => {
                let mut camera = self.renderer.camera();
                camera.pan(Vec2::from_array(*delta), self.renderer.size().height as f32);
                self.renderer.set_camera(camera);
                self.mark_camera_changed();
                Ok(format!("Pan camera by [{:.1}, {:.1}]", delta[0], delta[1]))
            }
            AppCommand::ZoomCamera { delta } => {
                let mut camera = self.renderer.camera();
                camera.zoom(*delta, self.renderer.mesh_bounds());
                self.renderer.set_camera(camera);
                self.mark_camera_changed();
                Ok(format!("Zoom camera by {delta:.3}"))
            }
            AppCommand::SetTool { tool } => {
                self.tool = *tool;
                Ok(format!("Tool: {tool:?}"))
            }
            AppCommand::SetSelectionElement { element } => {
                self.selection_element = *element;
                self.sync_selection_overlay();
                Ok(format!("Selection element: {element:?}"))
            }
            AppCommand::SetSelectionDepth { depth } => {
                self.selection_depth = *depth;
                Ok(format!("Selection depth: {depth:?}"))
            }
            AppCommand::SetBrushRadius { radius } => {
                self.brush_radius = radius.clamp(8.0, 240.0);
                Ok(format!("Brush radius: {:.1}px", self.brush_radius))
            }
            AppCommand::SelectAt { position } => {
                let position = vec2_from_array(*position);
                self.select_at(position)?;
                Ok(format!("Selected at {:.1},{:.1}", position.x, position.y))
            }
            AppCommand::BrushSelect { center, radius } => {
                let center = vec2_from_array(*center);
                let radius = radius.unwrap_or(self.brush_radius).clamp(8.0, 240.0);
                let changed = self.brush_select(center, radius)?;
                Ok(format!(
                    "Brush selected at {:.1},{:.1} radius {:.1} ({})",
                    center.x,
                    center.y,
                    radius,
                    if changed { "changed" } else { "unchanged" }
                ))
            }
            AppCommand::ClearSelection => {
                self.selection.clear();
                self.renderer
                    .set_selection_overlay(&SelectionOverlay::default());
                Ok("Selection cleared".to_string())
            }
            AppCommand::SetCutOptions {
                cap_density,
                smooth_cap,
            } => {
                self.cap_density = *cap_density;
                self.smooth_cap = *smooth_cap;
                Ok(format!(
                    "Cut options: {:?}, smooth={}",
                    self.cap_density, self.smooth_cap
                ))
            }
            AppCommand::PreviewViewLineCut { start, end } => {
                let start = vec2_from_array(*start);
                let end = vec2_from_array(*end);
                let plane = self
                    .renderer
                    .view_line_cut_plane(start, end)
                    .ok_or_else(|| anyhow!("draw a longer cut line"))?;
                let preview = self.renderer.cut_preview(plane);
                self.session.set_pending_cut(
                    plane,
                    start,
                    end,
                    preview.segments.len(),
                    preview.affected_triangle_count,
                );
                self.renderer.set_cut_preview_segments(&preview.segments);
                Ok(format!(
                    "Cut preview: {} segments across {} triangles",
                    preview.segments.len(),
                    preview.affected_triangle_count
                ))
            }
            AppCommand::ApplyCut => {
                let report = self
                    .session
                    .apply_pending_cut(self.cap_density, self.smooth_cap)?;
                self.upload_session_mesh()?;
                self.selection.clear();
                if let Some(index) = report.selected_object {
                    self.renderer
                        .set_selection_overlay(&self.session.object_selection_overlay(index));
                } else {
                    self.renderer
                        .set_selection_overlay(&self.session.cap_overlay());
                }
                self.renderer.clear_cut_preview();
                Ok(format!(
                    "Cut applied: {} objects, {} loops, {} cap triangles, target edge {:.5}{}",
                    report.object_count,
                    report.loop_count,
                    report.cap_triangles,
                    report.target_edge_length,
                    if report.warnings.is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", report.warnings.join("; "))
                    }
                ))
            }
            AppCommand::CancelCut => {
                self.session.clear_pending_cut();
                self.renderer.clear_cut_preview();
                Ok("Cut canceled".to_string())
            }
            AppCommand::SelectObject { index } => {
                self.session.select_object(*index)?;
                self.renderer
                    .set_selection_overlay(&self.session.object_selection_overlay(*index));
                Ok(format!("Selected Object {}", index + 1))
            }
            AppCommand::SelectObjectAt { position } => {
                let position = vec2_from_array(*position);
                let pick = self
                    .renderer
                    .pick(position)?
                    .ok_or_else(|| anyhow!("no object under position"))?;
                let global_index = self
                    .renderer
                    .triangle_global_index(pick.triangle_id)
                    .ok_or_else(|| anyhow!("picked triangle has no global index"))?;
                let object_index = self
                    .session
                    .select_object_for_render_triangle(global_index)?;
                self.renderer
                    .set_selection_overlay(&self.session.object_selection_overlay(object_index));
                Ok(format!("Selected Object {}", object_index + 1))
            }
            AppCommand::HideSelectedObject => {
                let label = self.session.hide_selected_object()?;
                self.upload_session_mesh()?;
                self.renderer
                    .set_selection_overlay(&SelectionOverlay::default());
                Ok(format!("Hidden {label}"))
            }
            AppCommand::DeleteSelectedObject => {
                self.session.delete_selected_object()?;
                self.upload_session_mesh()?;
                self.renderer
                    .set_selection_overlay(&SelectionOverlay::default());
                Ok("Deleted selected object".to_string())
            }
            AppCommand::KeepOnlySelectedObject => {
                self.session.keep_only_selected_object()?;
                self.upload_session_mesh()?;
                self.renderer
                    .set_selection_overlay(&SelectionOverlay::default());
                Ok("Kept selected object".to_string())
            }
            AppCommand::ShowAllObjects => {
                let restored = self.session.show_all_objects()?;
                self.upload_session_mesh()?;
                self.renderer
                    .set_selection_overlay(&SelectionOverlay::default());
                Ok(format!("Showing all objects ({restored} restored)"))
            }
            AppCommand::ExportVisible { path } => {
                let path = self.resolve_output_path(path);
                self.session.export_visible(&path)?;
                self.exported_paths.push(path.clone());
                Ok(format!("Exported visible mesh to {}", path.display()))
            }
            AppCommand::ExportObject { index, path } => {
                let path = self.resolve_output_path(path);
                self.session.export_object(*index, &path)?;
                self.exported_paths.push(path.clone());
                Ok(format!(
                    "Exported Object {} to {}",
                    index + 1,
                    path.display()
                ))
            }
            AppCommand::ExportAllObjects { directory } => {
                let directory = self.resolve_output_path(directory);
                let exported = self.session.export_all_objects(&directory)?;
                Ok(format!(
                    "Exported {exported} objects to {}",
                    directory.display()
                ))
            }
            AppCommand::Screenshot { path } => {
                let path = self.resolve_output_path(path);
                let stats = self.renderer.screenshot(Some(&path))?;
                self.image_stats.insert(
                    normalize_key(&path),
                    ImageStats {
                        width: stats.width,
                        height: stats.height,
                        non_background_pixels: stats.non_background_pixels,
                        coverage: stats.coverage,
                    },
                );
                Ok(format!(
                    "Screenshot {} coverage {:.4}",
                    path.display(),
                    stats.coverage
                ))
            }
            AppCommand::StateReport { path } => {
                let snapshot = self.snapshot();
                if let Some(path) = path {
                    let path = self.resolve_output_path(path);
                    write_json(&path, &snapshot)?;
                    Ok(format!("Wrote state report {}", path.display()))
                } else {
                    println!("{}", serde_json::to_string_pretty(&snapshot)?);
                    Ok("Printed state report".to_string())
                }
            }
            AppCommand::WaitForSelectionAcceleration => {
                self.wait_for_selection_acceleration()?;
                Ok("Selection acceleration ready".to_string())
            }
        }
    }

    fn upload_session_mesh(&mut self) -> Result<()> {
        let visible = self.session.visible_triangles();
        if visible.is_empty() {
            return Err(anyhow!("there are no visible triangles"));
        }
        let bounds = bounds_for_triangles(&visible);
        self.renderer.upload_mesh(
            visible
                .chunks(DEFAULT_CHUNK_TRIANGLES)
                .enumerate()
                .map(|(chunk_index, triangles)| {
                    let start_triangle = chunk_index * DEFAULT_CHUNK_TRIANGLES;
                    meshmend_render::MeshChunkUpload {
                        chunk_index: chunk_index as u32,
                        start_triangle: start_triangle as u64,
                        bounds: bounds_for_triangles(triangles),
                        triangles,
                    }
                }),
            bounds,
        );
        Ok(())
    }

    fn select_at(&mut self, position: Vec2) -> Result<bool> {
        let hits = self.pick_results_for_depth(position)?;
        let mut changed = false;
        for hit in hits {
            changed |= self.selection.add_hit(
                self.renderer,
                hit.triangle_id,
                position,
                self.selection_element,
                SelectionStrokeKind::Point,
            );
        }
        if changed {
            self.sync_selection_overlay();
        }
        Ok(changed)
    }

    fn brush_select(&mut self, center: Vec2, radius: f32) -> Result<bool> {
        let mut changed = false;
        for point in brush_sample_points(center, radius) {
            let hits = self.pick_results_for_depth(point)?;
            for hit in hits {
                changed |= self.selection.add_hit(
                    self.renderer,
                    hit.triangle_id,
                    point,
                    self.selection_element,
                    SelectionStrokeKind::Brush,
                );
            }
        }
        if changed {
            self.sync_selection_overlay();
        }
        Ok(changed)
    }

    fn pick_results_for_depth(
        &mut self,
        position: Vec2,
    ) -> Result<Vec<meshmend_render::PickResult>> {
        self.wait_for_selection_acceleration()?;
        self.renderer
            .pick_selection_hits(
                position,
                self.selection_depth == SelectionDepthName::Through,
            )
            .map_err(Into::into)
    }

    fn wait_for_selection_acceleration(&mut self) -> Result<()> {
        let started = Instant::now();
        while !self.renderer.selection_acceleration_ready() {
            if started.elapsed() > SELECTION_WAIT_TIMEOUT {
                return Err(anyhow!("selection acceleration did not finish in time"));
            }
            thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }

    fn sync_selection_overlay(&mut self) {
        self.renderer
            .set_selection_overlay(&self.selection.overlay_for(self.selection_element));
    }

    fn update_metrics(&mut self) {
        let triangle_count = self.session.triangle_count();
        if triangle_count > 0 {
            self.min_triangle_count = self.min_triangle_count.min(triangle_count);
            self.max_triangle_count = self.max_triangle_count.max(triangle_count);
        }
        self.max_object_count = self.max_object_count.max(self.session.object_count());
        self.max_visible_object_count = self
            .max_visible_object_count
            .max(self.session.visible_object_count());
        self.saw_selected_object |= self.session.selected_object_index().is_some();
        let selection = self.selection.snapshot();
        self.max_selection_count = self.max_selection_count.max(selection.total());
        self.max_face_selection_count = self.max_face_selection_count.max(selection.faces);
        self.mark_camera_changed();
    }

    fn evaluate_assertions(&self, assertions: &[ScenarioAssertion]) -> Vec<AssertionReport> {
        assertions
            .iter()
            .cloned()
            .map(|assertion| self.evaluate_assertion(assertion))
            .collect()
    }

    fn evaluate_assertion(&self, assertion: ScenarioAssertion) -> AssertionReport {
        let (ok, message) = match &assertion {
            ScenarioAssertion::ObjectCountAtLeast { count } => {
                let actual = self.max_object_count;
                (
                    actual >= *count,
                    format!("max object count {actual}, expected at least {count}"),
                )
            }
            ScenarioAssertion::VisibleObjectCountAtLeast { count } => {
                let actual = self.max_visible_object_count;
                (
                    actual >= *count,
                    format!("max visible object count {actual}, expected at least {count}"),
                )
            }
            ScenarioAssertion::SelectedObjectExists => (
                self.saw_selected_object,
                format!("selected object observed: {}", self.saw_selected_object),
            ),
            ScenarioAssertion::ScreenshotNonblank { path, min_coverage } => {
                let path = self.resolve_output_path(path);
                let key = normalize_key(&path);
                match self.image_stats.get(&key) {
                    Some(stats) => {
                        let min_coverage = min_coverage.unwrap_or(0.001);
                        (
                            stats.coverage >= min_coverage,
                            format!(
                                "{} coverage {:.4}, expected at least {:.4}",
                                path.display(),
                                stats.coverage,
                                min_coverage
                            ),
                        )
                    }
                    None => (false, format!("no screenshot stats for {}", path.display())),
                }
            }
            ScenarioAssertion::ExportReloads { path } => {
                let path = self.resolve_output_path(path);
                let ok = load_binary_stl_with_options(
                    &path,
                    &LoadOptions {
                        parallel: true,
                        ..LoadOptions::default()
                    },
                )
                .map(|parsed| parsed_stl_triangle_count(&parsed) > 0)
                .unwrap_or(false);
                (ok, format!("export reloads: {}", path.display()))
            }
            ScenarioAssertion::TriangleCountChanged => {
                let changed = self
                    .initial_triangle_count
                    .map(|initial| self.session.triangle_count() != initial)
                    .unwrap_or(false);
                (
                    changed,
                    format!(
                        "initial {:?}, final {}",
                        self.initial_triangle_count,
                        self.session.triangle_count()
                    ),
                )
            }
            ScenarioAssertion::SelectionCountAtLeast { count } => {
                let actual = self.max_selection_count;
                (
                    actual >= *count,
                    format!("max selection count {actual}, expected at least {count}"),
                )
            }
            ScenarioAssertion::FaceSelectionCountAtLeast { count } => {
                let actual = self.max_face_selection_count;
                (
                    actual >= *count,
                    format!("max face selection count {actual}, expected at least {count}"),
                )
            }
            ScenarioAssertion::CameraChanged => (
                self.camera_changed,
                format!("camera changed: {}", self.camera_changed),
            ),
        };
        AssertionReport {
            assertion,
            ok,
            message,
        }
    }

    fn report(&self, assertions: Vec<AssertionReport>) -> ScenarioRunReport {
        ScenarioRunReport {
            name: self
                .session
                .display_name()
                .unwrap_or("scripted run")
                .to_string(),
            input: self.session.source_path().map(Path::to_path_buf),
            output_dir: self.output_dir.clone(),
            steps: self.step_reports.clone(),
            assertions,
            image_stats: self.image_stats.clone(),
            final_state: self.snapshot(),
            metrics: ScenarioMetrics {
                initial_triangle_count: self.initial_triangle_count,
                final_triangle_count: self.session.triangle_count(),
                min_triangle_count: if self.min_triangle_count == usize::MAX {
                    0
                } else {
                    self.min_triangle_count
                },
                max_triangle_count: self.max_triangle_count,
                max_object_count: self.max_object_count,
                max_visible_object_count: self.max_visible_object_count,
                saw_selected_object: self.saw_selected_object,
                max_selection_count: self.max_selection_count,
                max_face_selection_count: self.max_face_selection_count,
                camera_changed: self.camera_changed,
                exported_paths: self.exported_paths.clone(),
            },
        }
    }

    fn snapshot(&self) -> StateSnapshot {
        StateSnapshot {
            file: self
                .session
                .source_path()
                .map(|path| path.display().to_string()),
            triangles: self.session.triangle_count(),
            bounds: self.session.bounds_snapshot(),
            view_mode: self.view_mode,
            camera: self.renderer.camera().into(),
            tool: self.tool,
            selection_element: self.selection_element,
            selection_depth: self.selection_depth,
            brush_radius: self.brush_radius,
            selection: self.selection.snapshot(),
            cut_preview: self.session.cut_preview_snapshot(),
            object_count: self.session.object_count(),
            visible_object_count: self.session.visible_object_count(),
            selected_object: self.session.selected_object_index(),
            objects: self.session.object_snapshots(),
            dirty: self.session.dirty(),
            status: self.status.clone(),
            latest_error: self.latest_error.clone(),
        }
    }

    fn mark_camera_changed(&mut self) {
        let current = CameraState::from(self.renderer.camera());
        self.camera_changed |= self
            .initial_camera
            .map(|initial| !camera_states_close(initial, current))
            .unwrap_or(false);
    }

    fn resolve_input_path(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            let scenario_relative = self.scenario_dir.join(path);
            if scenario_relative.exists() {
                scenario_relative
            } else {
                path.to_path_buf()
            }
        }
    }

    fn resolve_output_path(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.output_dir.join(path)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct VertexKey(i64, i64, i64);

impl VertexKey {
    fn from_position(position: Vec3) -> Self {
        const SCALE: f32 = 1_000_000.0;
        Self(
            (position.x * SCALE).round() as i64,
            (position.y * SCALE).round() as i64,
            (position.z * SCALE).round() as i64,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct EdgeKey(VertexKey, VertexKey);

impl EdgeKey {
    fn new(start: Vec3, end: Vec3) -> Self {
        let a = VertexKey::from_position(start);
        let b = VertexKey::from_position(end);
        if a <= b {
            Self(a, b)
        } else {
            Self(b, a)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectionStrokeKind {
    Point,
    Brush,
}

#[derive(Debug, Clone, Default)]
struct ScriptSelection {
    vertices: HashMap<VertexKey, Vec3>,
    edges: HashMap<EdgeKey, [Vec3; 2]>,
    faces: HashMap<TriangleId, [Vec3; 3]>,
}

impl ScriptSelection {
    fn clear(&mut self) {
        self.vertices.clear();
        self.edges.clear();
        self.faces.clear();
    }

    fn snapshot(&self) -> SelectionSnapshot {
        SelectionSnapshot {
            vertices: self.vertices.len(),
            edges: self.edges.len(),
            faces: self.faces.len(),
        }
    }

    fn overlay_for(&self, element: SelectionElementName) -> SelectionOverlay {
        match element {
            SelectionElementName::Vertex => SelectionOverlay {
                vertices: self.vertices.values().copied().collect(),
                edges: Vec::new(),
                faces: Vec::new(),
            },
            SelectionElementName::Edge => SelectionOverlay {
                vertices: Vec::new(),
                edges: self.edges.values().copied().collect(),
                faces: Vec::new(),
            },
            SelectionElementName::Face => SelectionOverlay {
                vertices: Vec::new(),
                edges: Vec::new(),
                faces: self.faces.values().copied().collect(),
            },
        }
    }

    fn add_hit(
        &mut self,
        renderer: &WgpuRenderer<'_>,
        triangle_id: TriangleId,
        screen_position: Vec2,
        element: SelectionElementName,
        stroke_kind: SelectionStrokeKind,
    ) -> bool {
        let Some(vertices) = renderer.triangle_vertices(triangle_id) else {
            return false;
        };
        match element {
            SelectionElementName::Face => self.faces.insert(triangle_id, vertices).is_none(),
            SelectionElementName::Edge if stroke_kind == SelectionStrokeKind::Brush => {
                self.add_edge(vertices[0], vertices[1])
                    | self.add_edge(vertices[1], vertices[2])
                    | self.add_edge(vertices[2], vertices[0])
            }
            SelectionElementName::Edge => {
                let [start, end] = nearest_screen_edge(renderer, vertices, screen_position);
                self.add_edge(start, end)
            }
            SelectionElementName::Vertex if stroke_kind == SelectionStrokeKind::Brush => {
                self.add_vertex(vertices[0])
                    | self.add_vertex(vertices[1])
                    | self.add_vertex(vertices[2])
            }
            SelectionElementName::Vertex => {
                let vertex = nearest_screen_vertex(renderer, vertices, screen_position);
                self.add_vertex(vertex)
            }
        }
    }

    fn add_vertex(&mut self, position: Vec3) -> bool {
        self.vertices
            .insert(VertexKey::from_position(position), position)
            .is_none()
    }

    fn add_edge(&mut self, start: Vec3, end: Vec3) -> bool {
        self.edges
            .insert(EdgeKey::new(start, end), [start, end])
            .is_none()
    }
}

fn nearest_screen_vertex(renderer: &WgpuRenderer<'_>, vertices: [Vec3; 3], position: Vec2) -> Vec3 {
    vertices
        .into_iter()
        .min_by(|left, right| {
            let left_distance = renderer
                .world_to_screen(*left)
                .map(|screen| screen.distance_squared(position))
                .unwrap_or(f32::MAX);
            let right_distance = renderer
                .world_to_screen(*right)
                .map(|screen| screen.distance_squared(position))
                .unwrap_or(f32::MAX);
            left_distance.total_cmp(&right_distance)
        })
        .unwrap_or(vertices[0])
}

fn nearest_screen_edge(
    renderer: &WgpuRenderer<'_>,
    vertices: [Vec3; 3],
    position: Vec2,
) -> [Vec3; 2] {
    let edges = [
        [vertices[0], vertices[1]],
        [vertices[1], vertices[2]],
        [vertices[2], vertices[0]],
    ];
    edges
        .into_iter()
        .min_by(|left, right| {
            let left_distance = screen_edge_distance_squared(renderer, *left, position);
            let right_distance = screen_edge_distance_squared(renderer, *right, position);
            left_distance.total_cmp(&right_distance)
        })
        .unwrap_or(edges[0])
}

fn screen_edge_distance_squared(
    renderer: &WgpuRenderer<'_>,
    edge: [Vec3; 2],
    position: Vec2,
) -> f32 {
    let Some(a) = renderer.world_to_screen(edge[0]) else {
        return f32::MAX;
    };
    let Some(b) = renderer.world_to_screen(edge[1]) else {
        return f32::MAX;
    };
    distance_to_segment_squared(position, a, b)
}

fn distance_to_segment_squared(point: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let length = ab.length_squared();
    if length <= f32::EPSILON {
        return point.distance_squared(a);
    }
    let t = ((point - a).dot(ab) / length).clamp(0.0, 1.0);
    point.distance_squared(a + ab * t)
}

fn brush_sample_points(center: Vec2, radius: f32) -> Vec<Vec2> {
    let radius = radius.clamp(8.0, 240.0);
    let step = (radius / 4.0).clamp(6.0, 16.0);
    let rings = (radius / step).ceil() as i32;
    let mut points = Vec::new();
    points.push(center);
    for y in -rings..=rings {
        for x in -rings..=rings {
            if x == 0 && y == 0 {
                continue;
            }
            let offset = Vec2::new(x as f32 * step, y as f32 * step);
            if offset.length_squared() <= radius * radius {
                points.push(center + offset);
            }
        }
    }
    points
}

fn apply_view_mode(mode: ViewModeName, display_settings: &mut DisplaySettings) {
    *display_settings = DisplaySettings::default();
    match mode {
        ViewModeName::Rendered => {
            display_settings.lighting_mode = LightingMode::Fixed;
        }
        ViewModeName::Wireframe => {
            display_settings.wireframe = true;
            display_settings.transparent = true;
            display_settings.show_backfaces = false;
        }
        ViewModeName::SurfaceWire => {
            display_settings.wireframe = true;
            display_settings.show_backfaces = true;
        }
        ViewModeName::XrayWire => {
            display_settings.wireframe = true;
            display_settings.transparent = true;
            display_settings.xray_wire = true;
            display_settings.show_backfaces = true;
        }
        ViewModeName::Transparent => {
            display_settings.transparent = true;
            display_settings.show_backfaces = true;
        }
        ViewModeName::Normals => {
            display_settings.normal_debug = true;
        }
        ViewModeName::Studio => {
            display_settings.lighting_mode = LightingMode::Studio;
        }
        ViewModeName::Headlight => {
            display_settings.lighting_mode = LightingMode::Headlight;
        }
    }
}

fn reset_camera(renderer: &mut WgpuRenderer<'_>) {
    if let Some(bounds) = renderer.mesh_bounds() {
        let mut camera = renderer.camera();
        camera.reset_to_bounds(
            bounds,
            renderer.size().width as f32 / renderer.size().height.max(1) as f32,
        );
        renderer.set_camera(camera);
    }
}

fn camera_states_close(left: CameraState, right: CameraState) -> bool {
    arrays_close(left.target, right.target)
        && (left.distance - right.distance).abs() < 1.0e-5
        && (left.yaw - right.yaw).abs() < 1.0e-5
        && (left.pitch - right.pitch).abs() < 1.0e-5
}

fn arrays_close(left: [f32; 3], right: [f32; 3]) -> bool {
    left.iter()
        .zip(right)
        .all(|(left, right)| (*left - right).abs() < 1.0e-5)
}

fn normalize_key(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn write_json(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(value)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brush_samples_include_center() {
        let center = Vec2::new(10.0, 20.0);
        let samples = brush_sample_points(center, 32.0);

        assert!(samples.contains(&center));
        assert!(samples.len() > 8);
        assert!(samples
            .iter()
            .all(|sample| sample.distance(center) <= 32.0 + f32::EPSILON));
    }

    #[test]
    fn distance_to_segment_clamps_to_endpoints() {
        let distance = distance_to_segment_squared(
            Vec2::new(3.0, 4.0),
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
        );

        assert!((distance - 20.0).abs() < f32::EPSILON);
    }
}

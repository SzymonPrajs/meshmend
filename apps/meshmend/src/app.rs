use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{anyhow, Result};
use egui_wgpu::ScreenDescriptor;
use egui_winit::State as EguiWinitState;
use meshmend_core::{CrossSectionAxis, CrossSectionState, MeshStats};
use meshmend_inspection::{BrushLabelKind, IssueKind, IssueSession};
use meshmend_project::{
    project_directory_from_selection, MeshMendProject, OperationKind, OperationStatus,
    SelectionReference,
};
use meshmend_render::{
    DisplaySettings, LabelStrokeOverlay, LightingMode, MeshChunkUpload, PickResult, RendererInfo,
    SelectionSummary, WgpuRenderer,
};
use meshmend_stl::{load_binary_stl, ParsedStl};
use winit::{
    dpi::LogicalSize,
    event::{ElementState, Event, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    keyboard::{KeyCode, ModifiersState, PhysicalKey},
    window::{Window, WindowBuilder},
};

use crate::input::CameraInput;

#[derive(Debug, Clone)]
struct ModelInfo {
    path: PathBuf,
    file_name: String,
    source_hash: String,
    stats: MeshStats,
    chunk_count: usize,
    parse_ms: f64,
    brush_unit: f32,
    selection_summary: Option<SelectionSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum UiAction {
    None,
    LoadStl,
    OpenProject,
    SaveProject,
    ExportReport,
    Undo,
    Redo,
    Fit,
    Reset,
    AddIssue,
    SaveIssues,
    LoadIssues,
    FrameIssue(usize),
    DeleteIssue(usize),
    ResetCrossSection,
    ClearLabelStrokes,
    DeleteLabelStroke(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolMode {
    Select,
    Navigate,
    Analyze,
    CrossSection,
    XrayInspect,
    RepairBrush,
    HoleFill,
    Cut,
    Measure,
    Remesh,
    Export,
}

impl ToolMode {
    const ALL: [Self; 11] = [
        Self::Select,
        Self::Navigate,
        Self::Analyze,
        Self::CrossSection,
        Self::XrayInspect,
        Self::RepairBrush,
        Self::HoleFill,
        Self::Cut,
        Self::Measure,
        Self::Remesh,
        Self::Export,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Select => "Select",
            Self::Navigate => "Navigate",
            Self::Analyze => "Analyze",
            Self::CrossSection => "Cross Section",
            Self::XrayInspect => "X-Ray Inspect",
            Self::RepairBrush => "Repair Brush",
            Self::HoleFill => "Hole Fill",
            Self::Cut => "Cut",
            Self::Measure => "Measure",
            Self::Remesh => "Remesh",
            Self::Export => "Export",
        }
    }

    fn shortcut(self) -> &'static str {
        match self {
            Self::Select => "1",
            Self::Navigate => "2",
            Self::Analyze => "3",
            Self::CrossSection => "4",
            Self::XrayInspect => "5",
            Self::RepairBrush => "6",
            Self::HoleFill => "7",
            Self::Cut => "8",
            Self::Measure => "9",
            Self::Remesh => "0",
            Self::Export => "E",
        }
    }

    fn tooltip(self) -> String {
        format!("{} ({})", self.label(), self.shortcut())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    Rendered,
    Headlight,
    Studio,
    Normals,
    SurfaceWire,
    XrayWire,
    Transparent,
    CrossSection,
    DefectOverlay,
    ThicknessOverlay,
}

impl ViewMode {
    const ALL: [Self; 10] = [
        Self::Rendered,
        Self::Headlight,
        Self::Studio,
        Self::Normals,
        Self::SurfaceWire,
        Self::XrayWire,
        Self::Transparent,
        Self::CrossSection,
        Self::DefectOverlay,
        Self::ThicknessOverlay,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Rendered => "Rendered",
            Self::Headlight => "Headlight",
            Self::Studio => "Studio",
            Self::Normals => "Normals",
            Self::SurfaceWire => "Surface Wire",
            Self::XrayWire => "X-Ray Wire",
            Self::Transparent => "Transparent",
            Self::CrossSection => "Section",
            Self::DefectOverlay => "Defects",
            Self::ThicknessOverlay => "Thickness",
        }
    }
}

#[derive(Debug, Clone)]
struct BrushToolState {
    enabled: bool,
    kind: BrushLabelKind,
    size_units: f32,
    min_screen_spacing: f32,
    active_stroke_index: Option<usize>,
    last_sample_screen: Option<glam::Vec2>,
}

#[derive(Debug, Clone, Default)]
struct HitStackState {
    screen_position: Option<glam::Vec2>,
    hits: Vec<PickResult>,
    index: usize,
}

impl HitStackState {
    fn clear(&mut self) {
        self.screen_position = None;
        self.hits.clear();
        self.index = 0;
    }
}

impl Default for BrushToolState {
    fn default() -> Self {
        Self {
            enabled: false,
            kind: BrushLabelKind::default(),
            size_units: 40.0,
            min_screen_spacing: 10.0,
            active_stroke_index: None,
            last_sample_screen: None,
        }
    }
}

impl BrushToolState {
    fn world_radius(&self, model: &ModelInfo) -> f32 {
        (self.size_units.max(1.0) * model.brush_unit).max(model.brush_unit)
    }

    fn finish_stroke(&mut self) {
        self.active_stroke_index = None;
        self.last_sample_screen = None;
    }
}

pub fn run_native(
    initial_file: Option<PathBuf>,
    smoke_window: bool,
    smoke_pick_center: bool,
) -> Result<()> {
    let event_loop = EventLoop::new()?;
    let title = initial_file
        .as_ref()
        .and_then(|path| path.file_name())
        .map(|name| format!("MeshMend - {}", name.to_string_lossy()))
        .unwrap_or_else(|| "MeshMend".to_string());
    let window = WindowBuilder::new()
        .with_title(title)
        .with_inner_size(LogicalSize::new(1280.0, 800.0))
        .with_min_inner_size(LogicalSize::new(720.0, 480.0))
        .build(&event_loop)?;
    let window: &'static Window = Box::leak(Box::new(window));
    let window_id = window.id();
    let redraw_window = window;
    let mut renderer = pollster::block_on(WgpuRenderer::new(window))?;
    tracing::info!(
        adapter = %renderer.info().adapter_name,
        backend = ?renderer.info().backend,
        "native renderer ready"
    );

    let mut model_info = if let Some(path) = initial_file.as_ref() {
        Some(load_model(path, &mut renderer, window)?)
    } else {
        None
    };
    let mut status = model_info
        .as_ref()
        .map(|model| format!("Loaded {}", model.file_name))
        .unwrap_or_else(|| "Ready".to_string());
    let mut issue_session = model_info
        .as_ref()
        .map(|model| IssueSession::new(model.file_name.clone(), model.stats.source_bytes));
    let mut project = model_info.as_ref().map(project_from_model);
    let mut cross_section = model_info
        .as_ref()
        .map(|model| CrossSectionState::centered(model.stats.bounds))
        .unwrap_or_default();
    let mut selected_issue_kind = IssueKind::default();
    let mut brush_tool = BrushToolState::default();
    let mut tool_mode = ToolMode::Select;
    let mut view_mode = ViewMode::Headlight;

    let egui_ctx = egui::Context::default();
    egui_ctx.set_visuals(egui::Visuals::dark());
    let mut egui_state = EguiWinitState::new(
        egui_ctx.clone(),
        egui::ViewportId::ROOT,
        window,
        Some(window.scale_factor() as f32),
        None,
    );
    let mut camera_input = CameraInput::default();
    let mut active_modifiers = ModifiersState::default();
    let mut needs_redraw = true;
    let mut selected_pick: Option<PickResult> = None;
    let mut hit_stack_state = HitStackState::default();

    event_loop.run(move |event, target| {
        target.set_control_flow(ControlFlow::Wait);

        match event {
            Event::WindowEvent {
                window_id: event_window_id,
                event,
            } if event_window_id == window_id => {
                let egui_response = egui_state.on_window_event(redraw_window, &event);
                if egui_response.repaint {
                    needs_redraw = true;
                }

                match event {
                    WindowEvent::CloseRequested => target.exit(),
                    WindowEvent::Resized(size) => {
                        renderer.resize(size);
                        needs_redraw = true;
                    }
                    WindowEvent::ModifiersChanged(modifiers) => {
                        active_modifiers = modifiers.state();
                    }
                    WindowEvent::RedrawRequested => {
                        let raw_input = egui_state.take_egui_input(redraw_window);
                        let mut action = UiAction::None;
                        let mut display_settings = renderer.display_settings();
                        let full_output = egui_ctx.run(raw_input, |ctx| {
                            draw_ui(
                                ctx,
                                renderer.info(),
                                model_info.as_ref(),
                                selected_pick,
                                &mut issue_session,
                                project.as_ref(),
                                &mut cross_section,
                                &mut selected_issue_kind,
                                &mut brush_tool,
                                &mut tool_mode,
                                &mut view_mode,
                                renderer.gpu_buffer_bytes(),
                                &status,
                                &mut display_settings,
                                &mut action,
                            );
                        });
                        egui_state
                            .handle_platform_output(redraw_window, full_output.platform_output);

                        if display_settings != renderer.display_settings() {
                            renderer.set_display_settings(display_settings);
                            needs_redraw = true;
                        }
                        handle_ui_action(
                            action,
                            &mut renderer,
                            redraw_window,
                            &mut model_info,
                            &mut issue_session,
                            &mut project,
                            &mut cross_section,
                            selected_issue_kind,
                            &mut brush_tool,
                            &mut selected_pick,
                            &mut hit_stack_state,
                            &mut status,
                            &mut needs_redraw,
                        );
                        if cross_section != renderer.cross_section() {
                            renderer.set_cross_section(cross_section);
                            needs_redraw = true;
                        }

                        let pixels_per_point = full_output.pixels_per_point;
                        let paint_jobs = egui_ctx.tessellate(full_output.shapes, pixels_per_point);
                        let screen_descriptor = ScreenDescriptor {
                            size_in_pixels: [renderer.size().width, renderer.size().height],
                            pixels_per_point,
                        };

                        if let Err(err) = renderer.render_with_egui(
                            &paint_jobs,
                            &full_output.textures_delta,
                            &screen_descriptor,
                        ) {
                            tracing::error!(error = %err, "render failed");
                            target.exit();
                        }
                        if smoke_pick_center {
                            let center = glam::Vec2::new(
                                renderer.size().width as f32 * 0.5,
                                renderer.size().height as f32 * 0.5,
                            );
                            match renderer.pick(center) {
                                Ok(Some(pick)) => {
                                    println!(
                                        "picked triangle {}:{} at {:.6},{:.6},{:.6}",
                                        pick.triangle_id.chunk,
                                        pick.triangle_id.local_index,
                                        pick.position.x,
                                        pick.position.y,
                                        pick.position.z
                                    );
                                }
                                Ok(None) => {
                                    println!("picked none");
                                }
                                Err(err) => {
                                    eprintln!("pick failed: {err}");
                                    target.exit();
                                }
                            }
                            target.exit();
                        }
                        if smoke_window {
                            target.exit();
                        }
                    }
                    WindowEvent::DroppedFile(path) => {
                        match load_model(&path, &mut renderer, redraw_window) {
                            Ok(info) => {
                                status = format!("Loaded {}", info.file_name);
                                issue_session = Some(IssueSession::new(
                                    info.file_name.clone(),
                                    info.stats.source_bytes,
                                ));
                                project = Some(project_from_model(&info));
                                cross_section = CrossSectionState::centered(info.stats.bounds);
                                model_info = Some(info);
                                selected_pick = None;
                                hit_stack_state.clear();
                                brush_tool.finish_stroke();
                                renderer.set_issue_markers(&[]);
                                renderer.set_label_strokes(&[]);
                            }
                            Err(err) => {
                                status = format!("Load failed: {err}");
                                tracing::error!(error = %err, "failed to load dropped STL");
                            }
                        }
                        needs_redraw = true;
                    }
                    WindowEvent::MouseInput { state, button, .. } => match state {
                        ElementState::Pressed
                            if is_camera_button(button) && !egui_response.consumed =>
                        {
                            camera_input.press(button);
                            if button == MouseButton::Left
                                && brush_tool.enabled
                                && !active_modifiers.shift_key()
                            {
                                if let Some(position) = camera_input.cursor_position() {
                                    sample_label_brush(
                                        &mut renderer,
                                        issue_session.as_mut(),
                                        model_info.as_ref(),
                                        &mut brush_tool,
                                        position,
                                        &mut selected_pick,
                                        &mut status,
                                        &mut needs_redraw,
                                    );
                                }
                            }
                        }
                        ElementState::Released => {
                            if let Some(position) = camera_input.release(button) {
                                if button == MouseButton::Left
                                    && brush_tool.active_stroke_index.is_some()
                                {
                                    if let Some(session) = issue_session.as_mut() {
                                        session.discard_empty_label_strokes();
                                        update_label_strokes(&mut renderer, session);
                                    }
                                    brush_tool.finish_stroke();
                                    needs_redraw = true;
                                } else if button == MouseButton::Left
                                    && !brush_tool.enabled
                                    && !active_modifiers.shift_key()
                                    && !egui_response.consumed
                                {
                                    select_at_cursor(
                                        &mut renderer,
                                        view_mode,
                                        position,
                                        &mut selected_pick,
                                        &mut hit_stack_state,
                                        &mut status,
                                        &mut needs_redraw,
                                    );
                                }
                            }
                        }
                        _ => {}
                    },
                    WindowEvent::CursorMoved { position, .. } if !egui_response.consumed => {
                        if let Some((button, delta)) =
                            camera_input.cursor_delta(position.x, position.y)
                        {
                            let mut camera = renderer.camera();
                            match button {
                                MouseButton::Left
                                    if brush_tool.enabled && !active_modifiers.shift_key() =>
                                {
                                    sample_label_brush(
                                        &mut renderer,
                                        issue_session.as_mut(),
                                        model_info.as_ref(),
                                        &mut brush_tool,
                                        glam::Vec2::new(position.x as f32, position.y as f32),
                                        &mut selected_pick,
                                        &mut status,
                                        &mut needs_redraw,
                                    );
                                }
                                MouseButton::Left if active_modifiers.shift_key() => {
                                    camera.pan(delta, renderer.size().height as f32);
                                }
                                MouseButton::Left => camera.orbit(delta),
                                MouseButton::Right | MouseButton::Middle => {
                                    camera.pan(delta, renderer.size().height as f32);
                                }
                                _ => {}
                            }
                            renderer.set_camera(camera);
                            needs_redraw = true;
                        }
                    }
                    WindowEvent::MouseWheel { delta, .. } if !egui_response.consumed => {
                        let wheel_delta = match delta {
                            MouseScrollDelta::LineDelta(_, y) => y,
                            MouseScrollDelta::PixelDelta(position) => position.y as f32 * 0.02,
                        };
                        let mut camera = renderer.camera();
                        camera.zoom(wheel_delta, renderer.mesh_bounds());
                        renderer.set_camera(camera);
                        needs_redraw = true;
                    }
                    WindowEvent::KeyboardInput { event, .. }
                        if event.state == ElementState::Pressed && !egui_response.consumed =>
                    {
                        match event.physical_key {
                            PhysicalKey::Code(KeyCode::KeyF) => {
                                renderer.fit_camera_to_mesh();
                                needs_redraw = true;
                            }
                            PhysicalKey::Code(KeyCode::KeyR) => {
                                reset_camera(&mut renderer);
                                needs_redraw = true;
                            }
                            PhysicalKey::Code(KeyCode::Digit1) => {
                                set_tool_mode(&mut tool_mode, &mut brush_tool, ToolMode::Select);
                                needs_redraw = true;
                            }
                            PhysicalKey::Code(KeyCode::Digit2) => {
                                set_tool_mode(&mut tool_mode, &mut brush_tool, ToolMode::Navigate);
                                needs_redraw = true;
                            }
                            PhysicalKey::Code(KeyCode::Digit3) => {
                                set_tool_mode(&mut tool_mode, &mut brush_tool, ToolMode::Analyze);
                                needs_redraw = true;
                            }
                            PhysicalKey::Code(KeyCode::Digit4) => {
                                set_tool_mode(
                                    &mut tool_mode,
                                    &mut brush_tool,
                                    ToolMode::CrossSection,
                                );
                                cross_section.enabled = true;
                                view_mode = ViewMode::CrossSection;
                                needs_redraw = true;
                            }
                            PhysicalKey::Code(KeyCode::Digit5) => {
                                set_tool_mode(
                                    &mut tool_mode,
                                    &mut brush_tool,
                                    ToolMode::XrayInspect,
                                );
                                view_mode = ViewMode::XrayWire;
                                let mut settings = renderer.display_settings();
                                apply_view_mode(view_mode, &mut settings, &mut cross_section);
                                renderer.set_display_settings(settings);
                                needs_redraw = true;
                            }
                            PhysicalKey::Code(KeyCode::Digit6) => {
                                set_tool_mode(
                                    &mut tool_mode,
                                    &mut brush_tool,
                                    ToolMode::RepairBrush,
                                );
                                needs_redraw = true;
                            }
                            PhysicalKey::Code(KeyCode::Digit7) => {
                                set_tool_mode(&mut tool_mode, &mut brush_tool, ToolMode::HoleFill);
                                needs_redraw = true;
                            }
                            PhysicalKey::Code(KeyCode::Digit8) => {
                                set_tool_mode(&mut tool_mode, &mut brush_tool, ToolMode::Cut);
                                needs_redraw = true;
                            }
                            PhysicalKey::Code(KeyCode::Digit9) => {
                                set_tool_mode(&mut tool_mode, &mut brush_tool, ToolMode::Measure);
                                needs_redraw = true;
                            }
                            PhysicalKey::Code(KeyCode::Digit0) => {
                                set_tool_mode(&mut tool_mode, &mut brush_tool, ToolMode::Remesh);
                                needs_redraw = true;
                            }
                            PhysicalKey::Code(KeyCode::KeyE) => {
                                set_tool_mode(&mut tool_mode, &mut brush_tool, ToolMode::Export);
                                needs_redraw = true;
                            }
                            PhysicalKey::Code(KeyCode::KeyN) => {
                                view_mode = ViewMode::Normals;
                                let mut settings = renderer.display_settings();
                                apply_view_mode(view_mode, &mut settings, &mut cross_section);
                                renderer.set_display_settings(settings);
                                needs_redraw = true;
                            }
                            PhysicalKey::Code(KeyCode::KeyW) => {
                                view_mode = ViewMode::SurfaceWire;
                                let mut settings = renderer.display_settings();
                                apply_view_mode(view_mode, &mut settings, &mut cross_section);
                                renderer.set_display_settings(settings);
                                needs_redraw = true;
                            }
                            PhysicalKey::Code(KeyCode::KeyX) => {
                                view_mode = ViewMode::XrayWire;
                                let mut settings = renderer.display_settings();
                                apply_view_mode(view_mode, &mut settings, &mut cross_section);
                                renderer.set_display_settings(settings);
                                needs_redraw = true;
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
            Event::AboutToWait if needs_redraw => {
                redraw_window.request_redraw();
                needs_redraw = false;
            }
            _ => {}
        }
    })?;

    Ok(())
}

pub fn run_capture(input: PathBuf, output: Option<PathBuf>) -> Result<()> {
    run_capture_with_options(input, output, None, false)
}

pub fn run_cross_section_capture(input: PathBuf, output: Option<PathBuf>) -> Result<()> {
    let cross_section = CrossSectionState {
        enabled: true,
        axis: CrossSectionAxis::X,
        offset: 0.0,
        flip_side: false,
        show_plane_guide: true,
    };
    run_capture_with_options(input, output, Some(cross_section), true)
}

pub fn run_view_mode_verification(input: PathBuf) -> Result<()> {
    let event_loop = EventLoop::new()?;
    let window = WindowBuilder::new()
        .with_title("MeshMend view mode verification")
        .with_inner_size(LogicalSize::new(1280.0, 800.0))
        .with_visible(false)
        .build(&event_loop)?;
    let window: &'static Window = Box::leak(Box::new(window));
    let window_id = window.id();
    let redraw_window = window;
    let mut renderer = pollster::block_on(WgpuRenderer::new(window))?;
    let model = load_model(&input, &mut renderer, window)?;
    let bounds = model.stats.bounds;
    let result: Arc<Mutex<Option<Result<()>>>> = Arc::new(Mutex::new(None));
    let result_writer = Arc::clone(&result);
    let mut needs_redraw = true;
    let mut captured = false;

    event_loop.run(move |event, target| {
        target.set_control_flow(ControlFlow::Wait);

        match event {
            Event::WindowEvent {
                window_id: event_window_id,
                event: WindowEvent::RedrawRequested,
            } if event_window_id == window_id && !captured => {
                captured = true;
                let capture = ViewMode::ALL.iter().copied().try_for_each(|mode| {
                    let mut settings = DisplaySettings::default();
                    let mut cross_section = CrossSectionState::centered(bounds);
                    apply_view_mode(mode, &mut settings, &mut cross_section);
                    renderer.set_display_settings(settings);
                    renderer.set_cross_section(cross_section);
                    let stats = renderer.screenshot(None).map_err(anyhow::Error::from)?;
                    println!(
                        "view-mode {} {}x{} non_background={} coverage={:.4}",
                        mode.label(),
                        stats.width,
                        stats.height,
                        stats.non_background_pixels,
                        stats.coverage
                    );
                    if stats.coverage <= 0.001 {
                        Err(anyhow!(
                            "{} view verification failed: image is blank",
                            mode.label()
                        ))
                    } else {
                        Ok(())
                    }
                });
                *result_writer
                    .lock()
                    .expect("view mode result lock poisoned") = Some(capture);
                target.exit();
            }
            Event::AboutToWait if needs_redraw => {
                redraw_window.request_redraw();
                needs_redraw = false;
            }
            _ => {}
        }
    })?;

    let mut guard = result.lock().expect("view mode result lock poisoned");
    guard
        .take()
        .unwrap_or_else(|| Err(anyhow!("view mode verification did not run")))
}

pub fn run_hit_stack_verification(input: PathBuf) -> Result<()> {
    let event_loop = EventLoop::new()?;
    let window = WindowBuilder::new()
        .with_title("MeshMend hit stack verification")
        .with_inner_size(LogicalSize::new(1280.0, 800.0))
        .with_visible(false)
        .build(&event_loop)?;
    let window: &'static Window = Box::leak(Box::new(window));
    let window_id = window.id();
    let redraw_window = window;
    let mut renderer = pollster::block_on(WgpuRenderer::new(window))?;
    let _model = load_model(&input, &mut renderer, window)?;
    let result: Arc<Mutex<Option<Result<()>>>> = Arc::new(Mutex::new(None));
    let result_writer = Arc::clone(&result);
    let mut needs_redraw = true;
    let mut captured = false;

    event_loop.run(move |event, target| {
        target.set_control_flow(ControlFlow::Wait);

        match event {
            Event::WindowEvent {
                window_id: event_window_id,
                event: WindowEvent::RedrawRequested,
            } if event_window_id == window_id && !captured => {
                captured = true;
                let center = glam::Vec2::new(
                    renderer.size().width as f32 * 0.5,
                    renderer.size().height as f32 * 0.5,
                );
                let capture = renderer
                    .pick_hit_stack(center)
                    .map_err(anyhow::Error::from)
                    .and_then(|hits| {
                        println!("hit-stack center count={}", hits.len());
                        for (index, hit) in hits.iter().take(6).enumerate() {
                            println!(
                                "hit {} triangle {}:{} at {:.6},{:.6},{:.6}",
                                index + 1,
                                hit.triangle_id.chunk,
                                hit.triangle_id.local_index,
                                hit.position.x,
                                hit.position.y,
                                hit.position.z
                            );
                        }
                        if hits.len() < 2 {
                            Err(anyhow!("hit stack verification expected at least two hits"))
                        } else {
                            Ok(())
                        }
                    });
                *result_writer
                    .lock()
                    .expect("hit stack result lock poisoned") = Some(capture);
                target.exit();
            }
            Event::AboutToWait if needs_redraw => {
                redraw_window.request_redraw();
                needs_redraw = false;
            }
            _ => {}
        }
    })?;

    let mut guard = result.lock().expect("hit stack result lock poisoned");
    guard
        .take()
        .unwrap_or_else(|| Err(anyhow!("hit stack verification did not run")))
}

fn run_capture_with_options(
    input: PathBuf,
    output: Option<PathBuf>,
    cross_section: Option<CrossSectionState>,
    verify_pick: bool,
) -> Result<()> {
    let event_loop = EventLoop::new()?;
    let window = WindowBuilder::new()
        .with_title("MeshMend render verification")
        .with_inner_size(LogicalSize::new(1280.0, 800.0))
        .with_visible(false)
        .build(&event_loop)?;
    let window: &'static Window = Box::leak(Box::new(window));
    let window_id = window.id();
    let redraw_window = window;
    let mut renderer = pollster::block_on(WgpuRenderer::new(window))?;
    let _info = load_model(&input, &mut renderer, window)?;
    if let Some(cross_section) = cross_section {
        renderer.set_cross_section(cross_section);
    }
    let result: Arc<Mutex<Option<Result<()>>>> = Arc::new(Mutex::new(None));
    let result_writer = Arc::clone(&result);
    let mut needs_redraw = true;
    let mut captured = false;

    event_loop.run(move |event, target| {
        target.set_control_flow(ControlFlow::Wait);

        match event {
            Event::WindowEvent {
                window_id: event_window_id,
                event: WindowEvent::RedrawRequested,
            } if event_window_id == window_id && !captured => {
                captured = true;
                let pick_result = if verify_pick {
                    let center = glam::Vec2::new(
                        renderer.size().width as f32 * 0.5,
                        renderer.size().height as f32 * 0.5,
                    );
                    let plane = renderer.cross_section().plane();
                    renderer
                        .pick(center)
                        .map_err(anyhow::Error::from)
                        .and_then(|pick| match pick {
                            Some(pick) if plane.keeps_point(pick.position) => {
                                println!(
                                    "cross-section pick {}:{} at {:.6},{:.6},{:.6}",
                                    pick.triangle_id.chunk,
                                    pick.triangle_id.local_index,
                                    pick.position.x,
                                    pick.position.y,
                                    pick.position.z
                                );
                                Ok(())
                            }
                            Some(pick) => Err(anyhow!(
                                "cross-section pick returned hidden-side point {:.6},{:.6},{:.6}",
                                pick.position.x,
                                pick.position.y,
                                pick.position.z
                            )),
                            None => Err(anyhow!("cross-section pick did not hit the visible mesh")),
                        })
                } else {
                    Ok(())
                };

                let capture = pick_result.and_then(|()| {
                    renderer
                        .screenshot(output.as_deref())
                        .map_err(anyhow::Error::from)
                        .and_then(|stats| {
                            println!(
                                "render {}x{} non_background={} coverage={:.4}",
                                stats.width,
                                stats.height,
                                stats.non_background_pixels,
                                stats.coverage
                            );
                            if stats.coverage <= 0.001 {
                                Err(anyhow!("render verification failed: image is blank"))
                            } else {
                                Ok(())
                            }
                        })
                });
                *result_writer.lock().expect("capture result lock poisoned") = Some(capture);
                target.exit();
            }
            Event::AboutToWait if needs_redraw => {
                redraw_window.request_redraw();
                needs_redraw = false;
            }
            _ => {}
        }
    })?;

    let mut guard = result.lock().expect("capture result lock poisoned");
    guard
        .take()
        .unwrap_or_else(|| Err(anyhow!("render verification did not run")))
}

pub fn run_perf(input: PathBuf, output: PathBuf) -> Result<()> {
    let event_loop = EventLoop::new()?;
    let window = WindowBuilder::new()
        .with_title("MeshMend performance capture")
        .with_inner_size(LogicalSize::new(1280.0, 800.0))
        .with_visible(false)
        .build(&event_loop)?;
    let window: &'static Window = Box::leak(Box::new(window));
    let window_id = window.id();
    let redraw_window = window;
    let mut renderer = pollster::block_on(WgpuRenderer::new(window))?;

    let load_started = Instant::now();
    let parsed = load_binary_stl(&input)?;
    let upload_started = Instant::now();
    upload_parsed_mesh(&mut renderer, &parsed);
    let upload_ms = upload_started.elapsed().as_secs_f64() * 1000.0;
    let time_to_interactive_ms = load_started.elapsed().as_secs_f64() * 1000.0;
    let frame_stats = measure_frame_stats(&mut renderer)?;
    let cpu_rss_mb = current_rss_mb().unwrap_or(0.0);
    let result: Arc<Mutex<Option<Result<()>>>> = Arc::new(Mutex::new(None));
    let result_writer = Arc::clone(&result);
    let mut needs_redraw = true;
    let mut captured = false;

    event_loop.run(move |event, target| {
        target.set_control_flow(ControlFlow::Wait);

        match event {
            Event::WindowEvent {
                window_id: event_window_id,
                event: WindowEvent::RedrawRequested,
            } if event_window_id == window_id && !captured => {
                captured = true;
                let screenshot_started = Instant::now();
                let capture = renderer
                    .screenshot(None)
                    .map_err(anyhow::Error::from)
                    .and_then(|stats| {
                        let screenshot_ms = screenshot_started.elapsed().as_secs_f64() * 1000.0;
                        let report = serde_json::json!({
                            "version": 1,
                            "app_version": env!("CARGO_PKG_VERSION"),
                            "platform": std::env::consts::OS,
                            "gpu_backend": format!("{:?}", renderer.info().backend),
                            "adapter": renderer.info().adapter_name,
                            "file": {
                                "name": parsed.file_name.as_str(),
                                "bytes": parsed.stats.source_bytes,
                                "triangles": parsed.stats.triangle_count,
                            },
                            "timings_ms": {
                                "file_map": parsed.timings.map_file.as_secs_f64() * 1000.0,
                                "validate": parsed.timings.validate.as_secs_f64() * 1000.0,
                                "parse_total": parsed.timings.parse.as_secs_f64() * 1000.0,
                                "gpu_upload_total": upload_ms,
                                "first_frame": screenshot_ms,
                                "time_to_interactive": time_to_interactive_ms,
                                "screenshot": screenshot_ms,
                            },
                            "frame_stats": {
                                "idle_fps_avg": frame_stats.idle_fps_avg,
                                "orbit_fps_avg": frame_stats.orbit_fps_avg,
                                "pan_fps_avg": frame_stats.pan_fps_avg,
                                "zoom_fps_avg": frame_stats.zoom_fps_avg,
                                "p95_frame_ms": frame_stats.p95_frame_ms,
                                "p99_frame_ms": frame_stats.p99_frame_ms,
                            },
                            "memory": {
                                "cpu_rss_mb": cpu_rss_mb,
                                "gpu_buffer_mb": renderer.gpu_buffer_bytes() as f64 / (1024.0 * 1024.0),
                                "chunk_count": parsed.chunks.len(),
                            },
                            "render_check": {
                                "width": stats.width,
                                "height": stats.height,
                                "non_background_pixels": stats.non_background_pixels,
                                "coverage": stats.coverage,
                            }
                        });
                        if let Some(parent) = output.parent().filter(|parent| !parent.as_os_str().is_empty()) {
                            fs::create_dir_all(parent)?;
                        }
                        fs::write(&output, serde_json::to_string_pretty(&report)?)?;
                        println!(
                            "perf wrote {} parse_ms={:.3} upload_ms={:.3} screenshot_ms={:.3} orbit_fps={:.1} coverage={:.4}",
                            output.display(),
                            parsed.timings.parse.as_secs_f64() * 1000.0,
                            upload_ms,
                            screenshot_ms,
                            frame_stats.orbit_fps_avg,
                            stats.coverage
                        );
                        Ok(())
                    });
                *result_writer.lock().expect("perf result lock poisoned") = Some(capture);
                target.exit();
            }
            Event::AboutToWait if needs_redraw => {
                redraw_window.request_redraw();
                needs_redraw = false;
            }
            _ => {}
        }
    })?;

    let mut guard = result.lock().expect("perf result lock poisoned");
    guard
        .take()
        .unwrap_or_else(|| Err(anyhow!("performance capture did not run")))
}

#[derive(Debug, Clone, Copy)]
struct FrameStats {
    idle_fps_avg: f64,
    orbit_fps_avg: f64,
    pan_fps_avg: f64,
    zoom_fps_avg: f64,
    p95_frame_ms: f64,
    p99_frame_ms: f64,
}

const PERF_FRAMES_PER_MODE: usize = 24;

fn measure_frame_stats(renderer: &mut WgpuRenderer<'_>) -> Result<FrameStats> {
    let original_camera = renderer.camera();
    let mut all_frame_ms = Vec::with_capacity(PERF_FRAMES_PER_MODE * 4);

    let idle = measure_mode(renderer, PERF_FRAMES_PER_MODE, |_| {}, &mut all_frame_ms)?;
    let orbit = measure_mode(
        renderer,
        PERF_FRAMES_PER_MODE,
        |renderer| {
            let mut camera = renderer.camera();
            camera.orbit(glam::Vec2::new(8.0, 2.5));
            renderer.set_camera(camera);
        },
        &mut all_frame_ms,
    )?;
    let pan = measure_mode(
        renderer,
        PERF_FRAMES_PER_MODE,
        |renderer| {
            let mut camera = renderer.camera();
            camera.pan(glam::Vec2::new(4.0, -2.0), renderer.size().height as f32);
            renderer.set_camera(camera);
        },
        &mut all_frame_ms,
    )?;
    let zoom = measure_mode(
        renderer,
        PERF_FRAMES_PER_MODE,
        |renderer| {
            let mut camera = renderer.camera();
            camera.zoom(0.15, renderer.mesh_bounds());
            renderer.set_camera(camera);
        },
        &mut all_frame_ms,
    )?;

    renderer.set_camera(original_camera);
    all_frame_ms.sort_by(f64::total_cmp);

    Ok(FrameStats {
        idle_fps_avg: fps_from_duration(idle),
        orbit_fps_avg: fps_from_duration(orbit),
        pan_fps_avg: fps_from_duration(pan),
        zoom_fps_avg: fps_from_duration(zoom),
        p95_frame_ms: percentile(&all_frame_ms, 0.95),
        p99_frame_ms: percentile(&all_frame_ms, 0.99),
    })
}

fn measure_mode(
    renderer: &mut WgpuRenderer<'_>,
    frames: usize,
    mut apply: impl FnMut(&mut WgpuRenderer<'_>),
    all_frame_ms: &mut Vec<f64>,
) -> Result<Duration> {
    let started = Instant::now();
    for _ in 0..frames {
        apply(renderer);
        let frame_started = Instant::now();
        renderer.render()?;
        all_frame_ms.push(frame_started.elapsed().as_secs_f64() * 1000.0);
    }
    Ok(started.elapsed())
}

fn fps_from_duration(duration: Duration) -> f64 {
    if duration.is_zero() {
        0.0
    } else {
        PERF_FRAMES_PER_MODE as f64 / duration.as_secs_f64()
    }
}

fn percentile(sorted_values: &[f64], percentile: f64) -> f64 {
    if sorted_values.is_empty() {
        return 0.0;
    }
    let index = ((sorted_values.len() - 1) as f64 * percentile).round() as usize;
    sorted_values[index]
}

fn current_rss_mb() -> Option<f64> {
    #[cfg(unix)]
    {
        let output = std::process::Command::new("ps")
            .args(["-o", "rss=", "-p", &std::process::id().to_string()])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let rss_kb = String::from_utf8(output.stdout)
            .ok()?
            .trim()
            .parse::<f64>()
            .ok()?;
        Some(rss_kb / 1024.0)
    }
    #[cfg(not(unix))]
    {
        None
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_ui_action(
    action: UiAction,
    renderer: &mut WgpuRenderer<'_>,
    window: &Window,
    model_info: &mut Option<ModelInfo>,
    issue_session: &mut Option<IssueSession>,
    project: &mut Option<MeshMendProject>,
    cross_section: &mut CrossSectionState,
    selected_issue_kind: IssueKind,
    brush_tool: &mut BrushToolState,
    selected_pick: &mut Option<PickResult>,
    hit_stack_state: &mut HitStackState,
    status: &mut String,
    needs_redraw: &mut bool,
) {
    match action {
        UiAction::None => {}
        UiAction::LoadStl => {
            if let Some(path) = meshmend_io::pick_stl_file() {
                match load_model(&path, renderer, window) {
                    Ok(info) => {
                        *status = format!("Loaded {}", info.file_name);
                        *issue_session = Some(IssueSession::new(
                            info.file_name.clone(),
                            info.stats.source_bytes,
                        ));
                        *project = Some(project_from_model(&info));
                        *cross_section = CrossSectionState::centered(info.stats.bounds);
                        *model_info = Some(info);
                        *selected_pick = None;
                        hit_stack_state.clear();
                        brush_tool.finish_stroke();
                        renderer.set_selection_marker(None);
                        renderer.set_issue_markers(&[]);
                        renderer.set_label_strokes(&[]);
                    }
                    Err(err) => {
                        *status = format!("Load failed: {err}");
                        tracing::error!(error = %err, "failed to load STL");
                    }
                }
                *needs_redraw = true;
            }
        }
        UiAction::OpenProject => {
            if let Some(path) = meshmend_io::pick_project_to_open() {
                let directory = project_directory_from_selection(&path);
                match MeshMendProject::load_from_dir(&directory) {
                    Ok(loaded_project) => {
                        let source_path = loaded_project.source.path.clone();
                        if source_path.exists() {
                            match load_model(&source_path, renderer, window) {
                                Ok(info) => {
                                    *issue_session = Some(IssueSession::new(
                                        info.file_name.clone(),
                                        info.stats.source_bytes,
                                    ));
                                    *cross_section = CrossSectionState::centered(info.stats.bounds);
                                    *model_info = Some(info);
                                    *selected_pick = None;
                                    hit_stack_state.clear();
                                    brush_tool.finish_stroke();
                                    renderer.set_selection_marker(None);
                                    renderer.set_issue_markers(&[]);
                                    renderer.set_label_strokes(&[]);
                                    *status = format!("Opened project {}", directory.display());
                                }
                                Err(err) => {
                                    *status = format!("Project opened, source load failed: {err}");
                                }
                            }
                        } else {
                            *status = format!(
                                "Opened project; source STL missing at {}",
                                source_path.display()
                            );
                        }
                        *project = Some(loaded_project);
                    }
                    Err(err) => {
                        *status = format!("Open project failed: {err}");
                        tracing::error!(error = %err, "failed to open project");
                    }
                }
                *needs_redraw = true;
            }
        }
        UiAction::SaveProject => {
            if project.is_none() {
                if let Some(model) = model_info.as_ref() {
                    *project = Some(project_from_model(model));
                }
            }
            if let Some(project) = project.as_mut() {
                let default_name = format!("{}.meshmend", project.metadata.name);
                if let Some(path) = meshmend_io::pick_project_to_save(&default_name) {
                    let directory = project_directory_from_selection(&path);
                    match project.save_to_dir(&directory) {
                        Ok(project_file) => {
                            *status = format!("Saved project {}", project_file.display());
                        }
                        Err(err) => {
                            *status = format!("Save project failed: {err}");
                            tracing::error!(error = %err, "failed to save project");
                        }
                    }
                    *needs_redraw = true;
                }
            } else {
                *status = "Load an STL before saving a project".to_string();
                *needs_redraw = true;
            }
        }
        UiAction::ExportReport => {
            if let Some(project) = project.as_mut() {
                let default_name = format!("{}-repair-report.md", project.metadata.name);
                if let Some(path) = meshmend_io::pick_report_to_save(&default_name) {
                    match project.write_markdown_report(&path) {
                        Ok(()) => {
                            *status = format!("Exported report {}", path.display());
                        }
                        Err(err) => {
                            *status = format!("Export report failed: {err}");
                            tracing::error!(error = %err, "failed to export report");
                        }
                    }
                    *needs_redraw = true;
                }
            } else {
                *status = "No project state to report".to_string();
                *needs_redraw = true;
            }
        }
        UiAction::Undo => {
            if let Some(project) = project.as_mut() {
                match project.undo() {
                    Some(revision) => *status = format!("Undo to revision {revision}"),
                    None => *status = "Nothing to undo".to_string(),
                }
                *needs_redraw = true;
            }
        }
        UiAction::Redo => {
            if let Some(project) = project.as_mut() {
                match project.redo() {
                    Some(revision) => *status = format!("Redo to revision {revision}"),
                    None => *status = "Nothing to redo".to_string(),
                }
                *needs_redraw = true;
            }
        }
        UiAction::Fit => {
            renderer.fit_camera_to_mesh();
            *needs_redraw = true;
        }
        UiAction::Reset => {
            reset_camera(renderer);
            *needs_redraw = true;
        }
        UiAction::AddIssue => {
            if let (Some(session), Some(pick)) = (issue_session.as_mut(), *selected_pick) {
                session.add_issue(
                    selected_issue_kind,
                    pick.triangle_id,
                    pick.position.to_array(),
                    *cross_section,
                );
                if let Some(project) = project.as_mut() {
                    project.record_operation(
                        OperationKind::DefectRecord,
                        OperationStatus::Applied,
                        serde_json::json!({
                            "defect_kind": selected_issue_kind.label(),
                            "cross_section": {
                                "axis": cross_section.axis.label(),
                                "offset": cross_section.offset,
                                "flip_side": cross_section.flip_side,
                            }
                        }),
                        vec![selection_reference_from_pick(pick)],
                    );
                }
                update_issue_markers(renderer, session);
                *status = format!("Added {}", selected_issue_kind.label());
                *needs_redraw = true;
            }
        }
        UiAction::SaveIssues => {
            if let Some(session) = issue_session.as_ref() {
                let default_name = format!("{}.issues.json", session.model_file_name);
                if let Some(path) = meshmend_io::pick_issue_session_to_save(&default_name) {
                    match session.save_to_path(&path) {
                        Ok(()) => {
                            *status = format!("Saved issues to {}", path.display());
                        }
                        Err(err) => {
                            *status = format!("Save failed: {err}");
                            tracing::error!(error = %err, "failed to save issues");
                        }
                    }
                    *needs_redraw = true;
                }
            }
        }
        UiAction::LoadIssues => {
            if let Some(path) = meshmend_io::pick_issue_session_to_load() {
                match IssueSession::load_from_path(&path) {
                    Ok(session) => {
                        update_issue_markers(renderer, &session);
                        update_label_strokes(renderer, &session);
                        *status = format!("Loaded issues from {}", path.display());
                        *issue_session = Some(session);
                    }
                    Err(err) => {
                        *status = format!("Load issues failed: {err}");
                        tracing::error!(error = %err, "failed to load issues");
                    }
                }
                *needs_redraw = true;
            }
        }
        UiAction::FrameIssue(index) => {
            if let Some(issue) = issue_session
                .as_ref()
                .and_then(|session| session.issues.get(index))
            {
                let position = glam::Vec3::from_array(issue.position);
                *selected_pick = Some(PickResult {
                    triangle_id: issue.triangle,
                    position,
                });
                cross_section.enabled = true;
                cross_section.axis = issue.cross_section_axis;
                cross_section.offset = issue.cross_section_offset;
                cross_section.flip_side = issue.cross_section_flipped;
                cross_section.show_plane_guide = true;
                renderer.set_selection_marker(Some(position));
                *status = format!(
                    "Framed issue {}:{}",
                    issue.triangle.chunk, issue.triangle.local_index
                );
                *needs_redraw = true;
            }
        }
        UiAction::DeleteIssue(index) => {
            if let Some(session) = issue_session.as_mut() {
                if index < session.issues.len() {
                    session.issues.remove(index);
                    update_issue_markers(renderer, session);
                    *status = "Deleted issue".to_string();
                    *needs_redraw = true;
                }
            }
        }
        UiAction::ResetCrossSection => {
            if let Some(model) = model_info.as_ref() {
                cross_section.reset_to_center(model.stats.bounds);
                *status = "Centered cross section".to_string();
                *needs_redraw = true;
            }
        }
        UiAction::ClearLabelStrokes => {
            if let Some(session) = issue_session.as_mut() {
                session.clear_label_strokes();
                renderer.set_label_strokes(&[]);
                brush_tool.finish_stroke();
                *status = "Cleared brush labels".to_string();
                *needs_redraw = true;
            }
        }
        UiAction::DeleteLabelStroke(index) => {
            if let Some(session) = issue_session.as_mut() {
                session.remove_label_stroke(index);
                update_label_strokes(renderer, session);
                brush_tool.finish_stroke();
                *status = "Deleted brush label".to_string();
                *needs_redraw = true;
            }
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

fn select_at_cursor(
    renderer: &mut WgpuRenderer<'_>,
    view_mode: ViewMode,
    position: glam::Vec2,
    selected_pick: &mut Option<PickResult>,
    hit_stack_state: &mut HitStackState,
    status: &mut String,
    needs_redraw: &mut bool,
) {
    if view_mode == ViewMode::XrayWire {
        match renderer.pick_hit_stack(position) {
            Ok(hits) if hits.is_empty() => {
                *selected_pick = None;
                hit_stack_state.clear();
                renderer.set_selection_marker(None);
                *status = "X-ray pick found no mesh hits".to_string();
                *needs_redraw = true;
            }
            Ok(hits) => {
                let same_cursor = hit_stack_state
                    .screen_position
                    .is_some_and(|previous| previous.distance(position) < 4.0);
                let index = if same_cursor && hit_stack_state.hits.len() == hits.len() {
                    (hit_stack_state.index + 1) % hits.len()
                } else {
                    0
                };
                hit_stack_state.screen_position = Some(position);
                hit_stack_state.hits = hits;
                hit_stack_state.index = index;

                let pick = hit_stack_state.hits[index];
                *selected_pick = Some(pick);
                renderer.set_selection_marker(Some(pick.position));
                *status = format!(
                    "X-ray hit {}/{} triangle {}:{}",
                    index + 1,
                    hit_stack_state.hits.len(),
                    pick.triangle_id.chunk,
                    pick.triangle_id.local_index
                );
                *needs_redraw = true;
            }
            Err(err) => {
                *status = format!("X-ray pick failed: {err}");
                tracing::error!(error = %err, "failed to build x-ray hit stack");
                *needs_redraw = true;
            }
        }
        return;
    }

    hit_stack_state.clear();
    match renderer.pick(position) {
        Ok(pick) => {
            *selected_pick = pick;
            if let Some(pick) = *selected_pick {
                *status = format!(
                    "Selected triangle {}:{}",
                    pick.triangle_id.chunk, pick.triangle_id.local_index
                );
            }
            *needs_redraw = true;
        }
        Err(err) => {
            *status = format!("Pick failed: {err}");
            tracing::error!(error = %err, "failed to pick triangle");
            *needs_redraw = true;
        }
    }
}

fn is_camera_button(button: MouseButton) -> bool {
    matches!(
        button,
        MouseButton::Left | MouseButton::Right | MouseButton::Middle
    )
}

fn load_model(
    path: &Path,
    renderer: &mut WgpuRenderer<'_>,
    window: &winit::window::Window,
) -> Result<ModelInfo> {
    let parsed = load_binary_stl(path)?;
    let source_hash = hash_file_fnv1a64(path)?;
    let brush_unit = estimate_mesh_detail_unit(&parsed);
    upload_parsed_mesh(renderer, &parsed);
    let info = ModelInfo {
        path: parsed.source_path.clone(),
        file_name: parsed.file_name.clone(),
        source_hash,
        stats: parsed.stats.clone(),
        chunk_count: parsed.chunks.len(),
        parse_ms: parsed.timings.parse.as_secs_f64() * 1000.0,
        brush_unit,
        selection_summary: renderer.selection_summary(),
    };
    window.set_title(&format!("MeshMend - {}", info.file_name));
    tracing::info!(
        file = %parsed.source_path.display(),
        triangles = parsed.stats.triangle_count,
        chunks = parsed.chunks.len(),
        gpu_buffer_mb = renderer.gpu_buffer_bytes() as f64 / (1024.0 * 1024.0),
        "loaded STL mesh"
    );
    Ok(info)
}

fn project_from_model(model: &ModelInfo) -> MeshMendProject {
    let name = model
        .path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| model.file_name.clone());
    MeshMendProject::new(
        name,
        model.path.clone(),
        model.source_hash.clone(),
        model.stats.clone(),
    )
}

fn selection_reference_from_pick(pick: PickResult) -> SelectionReference {
    SelectionReference {
        triangle_chunk: pick.triangle_id.chunk,
        triangle_local_index: pick.triangle_id.local_index,
        position: pick.position.to_array(),
    }
}

fn hash_file_fnv1a64(path: &Path) -> Result<String> {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    let bytes = fs::read(path)?;
    let hash = bytes.iter().fold(FNV_OFFSET, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME)
    });
    Ok(format!("fnv1a64:{hash:016x}"))
}

fn estimate_mesh_detail_unit(parsed: &ParsedStl) -> f32 {
    const MAX_SAMPLED_TRIANGLES: usize = 50_000;
    const MIN_BOUND_FRACTION: f32 = 0.0001;
    const FALLBACK_BOUND_FRACTION: f32 = 0.01;

    let triangle_count = parsed.stats.triangle_count as usize;
    let step = (triangle_count / MAX_SAMPLED_TRIANGLES).max(1);
    let mut edge_total = 0.0_f64;
    let mut edge_count = 0_u64;
    let mut sampled = 0_usize;
    let mut triangle_index = 0_usize;

    for chunk in &parsed.chunks {
        for triangle in &chunk.triangles {
            if triangle_index % step == 0 {
                let [a, b, c] = triangle.vertices;
                edge_total += f64::from(a.distance(b));
                edge_total += f64::from(b.distance(c));
                edge_total += f64::from(c.distance(a));
                edge_count += 3;
                sampled += 1;
                if sampled >= MAX_SAMPLED_TRIANGLES {
                    break;
                }
            }
            triangle_index += 1;
        }
        if sampled >= MAX_SAMPLED_TRIANGLES {
            break;
        }
    }

    let bounds_radius = parsed.stats.bounds.radius();
    let bounds_radius = if bounds_radius.is_finite() && bounds_radius > 0.0 {
        bounds_radius
    } else {
        1.0
    };
    let fallback = bounds_radius * FALLBACK_BOUND_FRACTION;
    let minimum = bounds_radius * MIN_BOUND_FRACTION;
    let average_edge = if edge_count == 0 {
        fallback
    } else {
        (edge_total / edge_count as f64) as f32
    };

    average_edge.max(minimum).max(f32::EPSILON)
}

fn upload_parsed_mesh(renderer: &mut WgpuRenderer<'_>, parsed: &ParsedStl) {
    renderer.upload_mesh(
        parsed.chunks.iter().map(|chunk| MeshChunkUpload {
            chunk_index: chunk.chunk_index,
            start_triangle: chunk.start_triangle,
            bounds: chunk.bounds,
            triangles: &chunk.triangles,
        }),
        parsed.stats.bounds,
    );
}

fn update_issue_markers(renderer: &mut WgpuRenderer<'_>, session: &IssueSession) {
    let positions = session
        .issues
        .iter()
        .map(|issue| glam::Vec3::from_array(issue.position))
        .collect::<Vec<_>>();
    renderer.set_issue_markers(&positions);
}

fn update_label_strokes(renderer: &mut WgpuRenderer<'_>, session: &IssueSession) {
    let strokes = session
        .label_strokes
        .iter()
        .filter(|stroke| !stroke.samples.is_empty())
        .map(|stroke| LabelStrokeOverlay {
            points: stroke
                .samples
                .iter()
                .map(|sample| glam::Vec3::from_array(sample.position))
                .collect(),
            radius: stroke.radius,
            color: stroke.kind.color(),
        })
        .collect::<Vec<_>>();
    renderer.set_label_strokes(&strokes);
}

#[allow(clippy::too_many_arguments)]
fn sample_label_brush(
    renderer: &mut WgpuRenderer<'_>,
    session: Option<&mut IssueSession>,
    model_info: Option<&ModelInfo>,
    brush: &mut BrushToolState,
    screen_position: glam::Vec2,
    selected_pick: &mut Option<PickResult>,
    status: &mut String,
    needs_redraw: &mut bool,
) {
    let Some(session) = session else {
        return;
    };
    if let Some(last) = brush.last_sample_screen {
        if screen_position.distance(last) < brush.min_screen_spacing {
            return;
        }
    }

    let brush_radius = model_info
        .map(|model| brush.world_radius(model))
        .unwrap_or_else(|| brush.size_units.max(1.0));
    let stroke_index = brush
        .active_stroke_index
        .unwrap_or_else(|| session.start_label_stroke(brush.kind, brush_radius));
    brush.active_stroke_index = Some(stroke_index);

    match renderer.pick(screen_position) {
        Ok(Some(pick)) => {
            session.add_label_sample(stroke_index, pick.triangle_id, pick.position.to_array());
            brush.last_sample_screen = Some(screen_position);
            *selected_pick = Some(pick);
            update_label_strokes(renderer, session);
            *status = format!("Painted {}", brush.kind.label());
            *needs_redraw = true;
        }
        Ok(None) => {}
        Err(err) => {
            *status = format!("Brush pick failed: {err}");
            tracing::error!(error = %err, "failed to paint brush label");
            *needs_redraw = true;
        }
    }
}

fn set_tool_mode(tool_mode: &mut ToolMode, brush: &mut BrushToolState, mode: ToolMode) {
    *tool_mode = mode;
    brush.enabled = mode == ToolMode::RepairBrush;
    if !brush.enabled {
        brush.finish_stroke();
    }
}

fn apply_view_mode(
    view_mode: ViewMode,
    display_settings: &mut DisplaySettings,
    cross_section: &mut CrossSectionState,
) {
    display_settings.wireframe = false;
    display_settings.normal_debug = false;
    display_settings.transparent = false;
    display_settings.xray_wire = false;
    display_settings.lighting_mode = LightingMode::Headlight;

    match view_mode {
        ViewMode::Rendered => {
            display_settings.lighting_mode = LightingMode::Fixed;
        }
        ViewMode::Headlight => {}
        ViewMode::Studio => {
            display_settings.lighting_mode = LightingMode::Studio;
        }
        ViewMode::Normals => {
            display_settings.normal_debug = true;
        }
        ViewMode::SurfaceWire => {
            display_settings.wireframe = true;
        }
        ViewMode::XrayWire => {
            display_settings.wireframe = true;
            display_settings.transparent = true;
            display_settings.xray_wire = true;
            display_settings.show_backfaces = true;
        }
        ViewMode::Transparent => {
            display_settings.transparent = true;
            display_settings.show_backfaces = true;
        }
        ViewMode::CrossSection => {
            cross_section.enabled = true;
        }
        ViewMode::DefectOverlay | ViewMode::ThicknessOverlay => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_ui(
    ctx: &egui::Context,
    renderer_info: &RendererInfo,
    model_info: Option<&ModelInfo>,
    selected_pick: Option<PickResult>,
    issue_session: &mut Option<IssueSession>,
    project: Option<&MeshMendProject>,
    cross_section: &mut CrossSectionState,
    selected_issue_kind: &mut IssueKind,
    brush_tool: &mut BrushToolState,
    tool_mode: &mut ToolMode,
    view_mode: &mut ViewMode,
    gpu_buffer_bytes: u64,
    status: &str,
    display_settings: &mut DisplaySettings,
    action: &mut UiAction,
) {
    egui::SidePanel::left("tool_palette")
        .resizable(false)
        .exact_width(56.0)
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(4.0);
                for mode in ToolMode::ALL {
                    if tool_button(ui, mode, *tool_mode == mode).clicked() {
                        set_tool_mode(tool_mode, brush_tool, mode);
                    }
                    ui.add_space(4.0);
                }
            });
        });

    if *tool_mode == ToolMode::CrossSection {
        cross_section.enabled = true;
        if *view_mode != ViewMode::CrossSection {
            *view_mode = ViewMode::CrossSection;
            apply_view_mode(*view_mode, display_settings, cross_section);
        }
    } else if *tool_mode == ToolMode::XrayInspect && *view_mode != ViewMode::XrayWire {
        *view_mode = ViewMode::XrayWire;
        apply_view_mode(*view_mode, display_settings, cross_section);
    }

    egui::TopBottomPanel::top("tool_options").show(ctx, |ui| {
        ui.horizontal_wrapped(|ui| {
            if ui.button("Load STL").clicked() {
                *action = UiAction::LoadStl;
            }
            if ui.button("Open Project").clicked() {
                *action = UiAction::OpenProject;
            }
            if ui.button("Save Project").clicked() {
                *action = UiAction::SaveProject;
            }
            if ui.button("Fit").clicked() {
                *action = UiAction::Fit;
            }
            if ui.button("Reset").clicked() {
                *action = UiAction::Reset;
            }
            ui.separator();
            ui.label("View");
            let previous_view = *view_mode;
            for mode in ViewMode::ALL {
                if ui
                    .selectable_label(*view_mode == mode, mode.label())
                    .on_hover_text(mode.label())
                    .clicked()
                {
                    *view_mode = mode;
                }
            }
            if *view_mode != previous_view {
                apply_view_mode(*view_mode, display_settings, cross_section);
                match *view_mode {
                    ViewMode::XrayWire => {
                        set_tool_mode(tool_mode, brush_tool, ToolMode::XrayInspect)
                    }
                    ViewMode::CrossSection => {
                        set_tool_mode(tool_mode, brush_tool, ToolMode::CrossSection)
                    }
                    _ => {}
                }
            }
            ui.separator();
            ui.checkbox(&mut display_settings.show_grid, "Grid");
            ui.checkbox(&mut display_settings.show_axes, "Axes");
            ui.checkbox(&mut display_settings.show_backfaces, "Backfaces");
        });
    });

    egui::SidePanel::right("repair_panel")
        .resizable(false)
        .default_width(340.0)
        .show(ctx, |ui| {
            draw_repair_panel(
                ui,
                renderer_info,
                model_info,
                selected_pick,
                issue_session,
                project,
                cross_section,
                selected_issue_kind,
                brush_tool,
                *tool_mode,
                *view_mode,
                gpu_buffer_bytes,
                action,
            );
        });

    egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(format!("Tool: {}", tool_mode.label()));
            ui.separator();
            ui.label(format!("View: {}", view_mode.label()));
            ui.separator();
            ui.label(status);
        });
    });
}

fn tool_button(ui: &mut egui::Ui, mode: ToolMode, selected: bool) -> egui::Response {
    let size = egui::vec2(40.0, 40.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let hovered = response.hovered();
    let visuals = ui.visuals();
    let fill = if selected {
        visuals.selection.bg_fill
    } else if hovered {
        visuals.widgets.hovered.bg_fill
    } else {
        visuals.widgets.inactive.bg_fill
    };
    let stroke_color = if selected {
        visuals.selection.stroke.color
    } else {
        visuals.widgets.inactive.fg_stroke.color
    };
    let painter = ui.painter();
    painter.rect_filled(rect, egui::Rounding::same(6.0), fill);
    painter.rect_stroke(
        rect,
        egui::Rounding::same(6.0),
        egui::Stroke::new(1.0, stroke_color),
    );
    draw_tool_icon(painter, rect.shrink(8.0), mode, stroke_color);
    response.on_hover_text(mode.tooltip())
}

fn draw_tool_icon(painter: &egui::Painter, rect: egui::Rect, mode: ToolMode, color: egui::Color32) {
    let stroke = egui::Stroke::new(1.8, color);
    let p = |x: f32, y: f32| {
        egui::pos2(
            rect.left() + rect.width() * x,
            rect.top() + rect.height() * y,
        )
    };
    let center = rect.center();
    match mode {
        ToolMode::Select => {
            painter.line_segment([p(0.22, 0.12), p(0.78, 0.48)], stroke);
            painter.line_segment([p(0.22, 0.12), p(0.42, 0.82)], stroke);
            painter.line_segment([p(0.42, 0.82), p(0.53, 0.55)], stroke);
        }
        ToolMode::Navigate => {
            painter.circle_stroke(center, rect.width() * 0.28, stroke);
            painter.line_segment([p(0.18, 0.50), p(0.82, 0.50)], stroke);
            painter.line_segment([p(0.50, 0.18), p(0.50, 0.82)], stroke);
        }
        ToolMode::Analyze => {
            painter.circle_stroke(p(0.42, 0.42), rect.width() * 0.20, stroke);
            painter.line_segment([p(0.58, 0.58), p(0.82, 0.82)], stroke);
            painter.line_segment([p(0.42, 0.30), p(0.42, 0.54)], stroke);
            painter.line_segment([p(0.30, 0.42), p(0.54, 0.42)], stroke);
        }
        ToolMode::CrossSection => {
            painter.rect_stroke(rect.shrink(2.0), egui::Rounding::same(2.0), stroke);
            painter.line_segment([p(0.50, 0.10), p(0.50, 0.90)], stroke);
        }
        ToolMode::XrayInspect => {
            painter.line_segment([p(0.10, 0.50), p(0.32, 0.28)], stroke);
            painter.line_segment([p(0.32, 0.28), p(0.68, 0.28)], stroke);
            painter.line_segment([p(0.68, 0.28), p(0.90, 0.50)], stroke);
            painter.line_segment([p(0.90, 0.50), p(0.68, 0.72)], stroke);
            painter.line_segment([p(0.68, 0.72), p(0.32, 0.72)], stroke);
            painter.line_segment([p(0.32, 0.72), p(0.10, 0.50)], stroke);
            painter.circle_stroke(center, rect.width() * 0.12, stroke);
        }
        ToolMode::RepairBrush => {
            painter.circle_stroke(p(0.62, 0.34), rect.width() * 0.18, stroke);
            painter.line_segment([p(0.50, 0.50), p(0.20, 0.82)], stroke);
            painter.line_segment([p(0.25, 0.76), p(0.36, 0.88)], stroke);
        }
        ToolMode::HoleFill => {
            painter.circle_stroke(center, rect.width() * 0.28, stroke);
            painter.line_segment([p(0.50, 0.34), p(0.50, 0.66)], stroke);
            painter.line_segment([p(0.34, 0.50), p(0.66, 0.50)], stroke);
        }
        ToolMode::Cut => {
            painter.line_segment([p(0.18, 0.82), p(0.82, 0.18)], stroke);
            painter.line_segment([p(0.18, 0.18), p(0.34, 0.34)], stroke);
            painter.line_segment([p(0.66, 0.66), p(0.82, 0.82)], stroke);
        }
        ToolMode::Measure => {
            painter.line_segment([p(0.16, 0.50), p(0.84, 0.50)], stroke);
            painter.line_segment([p(0.16, 0.34), p(0.16, 0.66)], stroke);
            painter.line_segment([p(0.84, 0.34), p(0.84, 0.66)], stroke);
        }
        ToolMode::Remesh => {
            for offset in [0.25, 0.50, 0.75] {
                painter.line_segment([p(offset, 0.15), p(offset, 0.85)], stroke);
                painter.line_segment([p(0.15, offset), p(0.85, offset)], stroke);
            }
        }
        ToolMode::Export => {
            painter.rect_stroke(
                egui::Rect::from_min_max(p(0.22, 0.48), p(0.78, 0.84)),
                egui::Rounding::same(2.0),
                stroke,
            );
            painter.line_segment([p(0.50, 0.14), p(0.50, 0.62)], stroke);
            painter.line_segment([p(0.32, 0.32), p(0.50, 0.14)], stroke);
            painter.line_segment([p(0.68, 0.32), p(0.50, 0.14)], stroke);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_repair_panel(
    ui: &mut egui::Ui,
    renderer_info: &RendererInfo,
    model_info: Option<&ModelInfo>,
    selected_pick: Option<PickResult>,
    issue_session: &mut Option<IssueSession>,
    project: Option<&MeshMendProject>,
    cross_section: &mut CrossSectionState,
    selected_issue_kind: &mut IssueKind,
    brush_tool: &mut BrushToolState,
    tool_mode: ToolMode,
    view_mode: ViewMode,
    gpu_buffer_bytes: u64,
    action: &mut UiAction,
) {
    ui.heading("Repair");
    ui.label(format!("Tool: {}", tool_mode.label()));
    ui.label(format!("View: {}", view_mode.label()));
    draw_project_controls(ui, project, action);

    let Some(model) = model_info else {
        ui.separator();
        ui.label("Load an STL to begin repair.");
        if ui.button("Load STL").clicked() {
            *action = UiAction::LoadStl;
        }
        return;
    };

    cross_section.clamp_to_bounds(model.stats.bounds);
    draw_model_summary(ui, renderer_info, model, selected_pick, gpu_buffer_bytes);
    ui.separator();

    match tool_mode {
        ToolMode::Analyze => draw_defect_tools(
            ui,
            selected_pick,
            issue_session,
            selected_issue_kind,
            action,
        ),
        ToolMode::CrossSection => draw_cross_section_tools(ui, model, cross_section, action),
        ToolMode::RepairBrush => {
            draw_repair_brush_tools(ui, model, issue_session, brush_tool, action)
        }
        ToolMode::HoleFill => draw_operation_stub(
            ui,
            "Hole Fill",
            "Select an open boundary loop, preview a refined cap, then apply the repair.",
        ),
        ToolMode::Cut => draw_operation_stub(
            ui,
            "Cut",
            "Draw a cut line, preview both sides, then apply a capped printable split.",
        ),
        ToolMode::Measure => draw_operation_stub(
            ui,
            "Measure",
            "Pick two points and assign a physical distance for printer-aware remeshing.",
        ),
        ToolMode::Remesh => draw_operation_stub(
            ui,
            "Remesh",
            "Choose a physical target resolution and preview mesh density changes.",
        ),
        ToolMode::Export => draw_operation_stub(
            ui,
            "Export",
            "Validate the current mesh and export repaired STL plus a repair report.",
        ),
        ToolMode::Select | ToolMode::Navigate | ToolMode::XrayInspect => {
            draw_cross_section_tools(ui, model, cross_section, action);
            ui.separator();
            draw_defect_tools(
                ui,
                selected_pick,
                issue_session,
                selected_issue_kind,
                action,
            );
        }
    }

    ui.separator();
    draw_operation_history(ui, issue_session, project, action);
}

fn draw_model_summary(
    ui: &mut egui::Ui,
    renderer_info: &RendererInfo,
    model: &ModelInfo,
    selected_pick: Option<PickResult>,
    gpu_buffer_bytes: u64,
) {
    egui::CollapsingHeader::new("Model")
        .default_open(false)
        .show(ui, |ui| {
            ui.label(model.file_name.as_str());
            ui.label(format!("Path: {}", model.path.display()));
            ui.label(format!("Source hash: {}", model.source_hash));
            ui.label(format!("Triangles: {}", model.stats.triangle_count));
            ui.label(format!("Chunks: {}", model.chunk_count));
            if let Some(summary) = model.selection_summary {
                ui.label(format!("Indexed vertices: {}", summary.vertex_count));
                ui.label(format!("Indexed faces: {}", summary.face_count));
                ui.label(format!("Components: {}", summary.component_count));
                ui.label(format!("Boundary loops: {}", summary.boundary_loop_count));
                ui.label(format!(
                    "Non-manifold edges: {}",
                    summary.non_manifold_edge_count
                ));
            }
            ui.label(format!("Bytes: {}", model.stats.source_bytes));
            ui.label(format!("Parse: {:.2} ms", model.parse_ms));
            ui.label(format!("Brush unit: {:.5}", model.brush_unit));
            ui.label(format!(
                "GPU buffers: {:.2} MB",
                gpu_buffer_bytes as f64 / (1024.0 * 1024.0)
            ));
            ui.label(format!(
                "Min: {:.4}, {:.4}, {:.4}",
                model.stats.bounds.min.x, model.stats.bounds.min.y, model.stats.bounds.min.z
            ));
            ui.label(format!(
                "Max: {:.4}, {:.4}, {:.4}",
                model.stats.bounds.max.x, model.stats.bounds.max.y, model.stats.bounds.max.z
            ));
            if let Some(pick) = selected_pick {
                ui.separator();
                ui.label(format!(
                    "Selected: {}:{}",
                    pick.triangle_id.chunk, pick.triangle_id.local_index
                ));
                ui.label(format!(
                    "Point: {:.4}, {:.4}, {:.4}",
                    pick.position.x, pick.position.y, pick.position.z
                ));
            }
            ui.separator();
            ui.label(format!("GPU: {}", renderer_info.adapter_name));
            ui.label(format!("Backend: {:?}", renderer_info.backend));
        });
}

fn draw_project_controls(
    ui: &mut egui::Ui,
    project: Option<&MeshMendProject>,
    action: &mut UiAction,
) {
    ui.horizontal_wrapped(|ui| {
        if ui.button("Save Project").clicked() {
            *action = UiAction::SaveProject;
        }
        if ui.button("Open Project").clicked() {
            *action = UiAction::OpenProject;
        }
        if ui.button("Export Report").clicked() {
            *action = UiAction::ExportReport;
        }
    });
    ui.horizontal_wrapped(|ui| {
        let can_undo = project.is_some_and(|project| !project.undo_stack.is_empty());
        let can_redo = project.is_some_and(|project| !project.redo_stack.is_empty());
        if ui
            .add_enabled(can_undo, egui::Button::new("Undo"))
            .clicked()
        {
            *action = UiAction::Undo;
        }
        if ui
            .add_enabled(can_redo, egui::Button::new("Redo"))
            .clicked()
        {
            *action = UiAction::Redo;
        }
    });
    if let Some(project) = project {
        ui.small(format!(
            "Project rev {} | {} operations | {} exports",
            project.current_revision,
            project.operations.len(),
            project.exports.len()
        ));
    }
}

fn draw_cross_section_tools(
    ui: &mut egui::Ui,
    model: &ModelInfo,
    cross_section: &mut CrossSectionState,
    action: &mut UiAction,
) {
    ui.heading("Section");
    ui.checkbox(&mut cross_section.enabled, "Cross Section");
    ui.horizontal(|ui| {
        ui.label("Axis");
        let previous_axis = cross_section.axis;
        for axis in CrossSectionAxis::ALL {
            ui.selectable_value(&mut cross_section.axis, axis, axis.label());
        }
        if cross_section.axis != previous_axis {
            cross_section.set_axis(cross_section.axis, model.stats.bounds);
        }
    });

    let range = cross_section.range(model.stats.bounds);
    ui.add(
        egui::Slider::new(&mut cross_section.offset, range)
            .text("Offset")
            .show_value(false),
    );
    cross_section.clamp_to_bounds(model.stats.bounds);
    ui.label(format!(
        "{} = {:.4}",
        cross_section.axis.label(),
        cross_section.offset
    ));
    ui.checkbox(&mut cross_section.flip_side, "Flip side");
    ui.checkbox(&mut cross_section.show_plane_guide, "Show plane guide");
    if ui.button("Center Plane").clicked() {
        *action = UiAction::ResetCrossSection;
    }
}

fn draw_repair_brush_tools(
    ui: &mut egui::Ui,
    model: &ModelInfo,
    issue_session: &mut Option<IssueSession>,
    brush_tool: &mut BrushToolState,
    action: &mut UiAction,
) {
    ui.heading("Repair Brush");
    ui.label("Paint regions that feed repair operations.");
    egui::ComboBox::from_label("Region")
        .selected_text(brush_tool.kind.label())
        .show_ui(ui, |ui| {
            for kind in BrushLabelKind::ALL {
                ui.selectable_value(&mut brush_tool.kind, kind, kind.label());
            }
        });

    ui.add(egui::Slider::new(&mut brush_tool.size_units, 1.0..=200.0).text("Brush radius"));
    ui.label(format!(
        "World radius: {:.5}",
        brush_tool.world_radius(model)
    ));
    ui.add(egui::Slider::new(&mut brush_tool.min_screen_spacing, 2.0..=32.0).text("Spacing"));

    ui.horizontal(|ui| {
        ui.add_enabled(false, egui::Button::new("Preview"));
        ui.add_enabled(false, egui::Button::new("Apply"));
        if ui.button("Clear Regions").clicked() {
            *action = UiAction::ClearLabelStrokes;
        }
    });

    if let Some(session) = issue_session.as_mut() {
        if !session.label_strokes.is_empty() {
            ui.separator();
            ui.label("Repair regions");
        }
        for (index, stroke) in session.label_strokes.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!(
                    "{}: {} samples",
                    stroke.kind.label(),
                    stroke.samples.len()
                ));
                if ui.button("Delete").clicked() {
                    *action = UiAction::DeleteLabelStroke(index);
                }
            });
        }
    }
}

fn draw_defect_tools(
    ui: &mut egui::Ui,
    selected_pick: Option<PickResult>,
    issue_session: &mut Option<IssueSession>,
    selected_issue_kind: &mut IssueKind,
    action: &mut UiAction,
) {
    ui.heading("Defects");
    egui::ComboBox::from_label("Defect type")
        .selected_text(selected_issue_kind.label())
        .show_ui(ui, |ui| {
            for kind in IssueKind::ALL {
                ui.selectable_value(selected_issue_kind, kind, kind.label());
            }
        });

    let can_add = selected_pick.is_some() && issue_session.is_some();
    if ui
        .add_enabled(can_add, egui::Button::new("Record Selected Defect"))
        .clicked()
    {
        *action = UiAction::AddIssue;
    }

    ui.horizontal(|ui| {
        if ui.button("Save Repair Data").clicked() {
            *action = UiAction::SaveIssues;
        }
        if ui.button("Load Repair Data").clicked() {
            *action = UiAction::LoadIssues;
        }
    });

    if let Some(pick) = selected_pick {
        ui.small(format!(
            "Selected {}:{} at {:.3}, {:.3}, {:.3}",
            pick.triangle_id.chunk,
            pick.triangle_id.local_index,
            pick.position.x,
            pick.position.y,
            pick.position.z
        ));
    }
}

fn draw_operation_history(
    ui: &mut egui::Ui,
    issue_session: &mut Option<IssueSession>,
    project: Option<&MeshMendProject>,
    action: &mut UiAction,
) {
    ui.heading("Operations");
    if let Some(project) = project {
        if project.operations.is_empty() {
            ui.label("No project operations recorded");
        } else {
            for operation in project.operations.iter().rev().take(6) {
                ui.small(format!(
                    "{:?} {:?} rev {}",
                    operation.kind, operation.status, operation.input_revision
                ));
            }
        }
        ui.separator();
    }
    if let Some(session) = issue_session.as_mut() {
        if session.issues.is_empty() {
            ui.label("No defect operations recorded");
        }
        for (index, issue) in session.issues.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                if ui.button("Frame").clicked() {
                    *action = UiAction::FrameIssue(index);
                }
                if ui.button("Delete").clicked() {
                    *action = UiAction::DeleteIssue(index);
                }
            });
            ui.label(issue.kind.label());
            ui.text_edit_singleline(&mut issue.label);
            ui.small(format!(
                "{}:{}  {:.3}, {:.3}, {:.3}",
                issue.triangle.chunk,
                issue.triangle.local_index,
                issue.position[0],
                issue.position[1],
                issue.position[2]
            ));
            ui.small(format!(
                "Section {} {:.3}{}",
                issue.cross_section_axis.label(),
                issue.cross_section_offset,
                if issue.cross_section_flipped {
                    " flipped"
                } else {
                    ""
                }
            ));
            ui.separator();
        }
    }
}

fn draw_operation_stub(ui: &mut egui::Ui, title: &str, description: &str) {
    ui.heading(title);
    ui.label(description);
    ui.horizontal(|ui| {
        ui.add_enabled(false, egui::Button::new("Preview"));
        ui.add_enabled(false, egui::Button::new("Apply"));
        ui.add_enabled(false, egui::Button::new("Cancel"));
    });
}

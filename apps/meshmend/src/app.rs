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
use meshmend_notes::{IssueKind, IssueSession};
use meshmend_render::{DisplaySettings, MeshChunkUpload, PickResult, RendererInfo, WgpuRenderer};
use meshmend_stl::{load_binary_stl, ParsedStl};
use winit::{
    dpi::LogicalSize,
    event::{ElementState, Event, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowBuilder},
};

use crate::input::CameraInput;

#[derive(Debug, Clone)]
struct ModelInfo {
    path: PathBuf,
    file_name: String,
    stats: MeshStats,
    chunk_count: usize,
    parse_ms: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum UiAction {
    None,
    LoadStl,
    Fit,
    Reset,
    AddIssue,
    SaveIssues,
    LoadIssues,
    FrameIssue(usize),
    DeleteIssue(usize),
    ResetCrossSection,
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
    let mut cross_section = model_info
        .as_ref()
        .map(|model| CrossSectionState::centered(model.stats.bounds))
        .unwrap_or_default();
    let mut selected_issue_kind = IssueKind::default();

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
    let mut needs_redraw = true;
    let mut selected_pick: Option<PickResult> = None;

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
                                &mut cross_section,
                                &mut selected_issue_kind,
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
                            &mut cross_section,
                            selected_issue_kind,
                            &mut selected_pick,
                            &mut status,
                            &mut needs_redraw,
                        );

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
                                    cross_section = CrossSectionState::centered(info.stats.bounds);
                                    model_info = Some(info);
                                    selected_pick = None;
                                    renderer.set_note_markers(&[]);
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
                        }
                        ElementState::Released => {
                            if let Some(position) = camera_input.release(button) {
                                if button == MouseButton::Left && !egui_response.consumed {
                                    match renderer.pick(position) {
                                        Ok(pick) => {
                                            selected_pick = pick;
                                            if let Some(pick) = selected_pick {
                                                status = format!(
                                                    "Selected triangle {}:{}",
                                                    pick.triangle_id.chunk,
                                                    pick.triangle_id.local_index
                                                );
                                            }
                                            needs_redraw = true;
                                        }
                                        Err(err) => {
                                            status = format!("Pick failed: {err}");
                                            tracing::error!(error = %err, "failed to pick triangle");
                                            needs_redraw = true;
                                        }
                                    }
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
                let capture = renderer
                    .screenshot(output.as_deref())
                    .map_err(anyhow::Error::from)
                    .and_then(|stats| {
                        println!(
                            "render {}x{} non_background={} coverage={:.4}",
                            stats.width, stats.height, stats.non_background_pixels, stats.coverage
                        );
                        if stats.coverage <= 0.001 {
                            Err(anyhow!("render verification failed: image is blank"))
                        } else {
                            Ok(())
                        }
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
    cross_section: &mut CrossSectionState,
    selected_issue_kind: IssueKind,
    selected_pick: &mut Option<PickResult>,
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
                        *cross_section = CrossSectionState::centered(info.stats.bounds);
                        *model_info = Some(info);
                        *selected_pick = None;
                        renderer.set_selection_marker(None);
                        renderer.set_note_markers(&[]);
                    }
                    Err(err) => {
                        *status = format!("Load failed: {err}");
                        tracing::error!(error = %err, "failed to load STL");
                    }
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
    upload_parsed_mesh(renderer, &parsed);
    let info = ModelInfo {
        path: parsed.source_path.clone(),
        file_name: parsed.file_name.clone(),
        stats: parsed.stats.clone(),
        chunk_count: parsed.chunks.len(),
        parse_ms: parsed.timings.parse.as_secs_f64() * 1000.0,
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
    renderer.set_note_markers(&positions);
}

#[allow(clippy::too_many_arguments)]
fn draw_ui(
    ctx: &egui::Context,
    renderer_info: &RendererInfo,
    model_info: Option<&ModelInfo>,
    selected_pick: Option<PickResult>,
    issue_session: &mut Option<IssueSession>,
    cross_section: &mut CrossSectionState,
    selected_issue_kind: &mut IssueKind,
    gpu_buffer_bytes: u64,
    status: &str,
    display_settings: &mut DisplaySettings,
    action: &mut UiAction,
) {
    egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
        ui.horizontal(|ui| {
            if ui.button("Load STL").clicked() {
                *action = UiAction::LoadStl;
            }
            if ui.button("Fit").clicked() {
                *action = UiAction::Fit;
            }
            if ui.button("Reset").clicked() {
                *action = UiAction::Reset;
            }
            ui.separator();
            ui.checkbox(&mut display_settings.wireframe, "Wire");
            ui.checkbox(&mut display_settings.show_backfaces, "Backfaces");
            ui.checkbox(&mut display_settings.show_grid, "Grid");
            ui.checkbox(&mut display_settings.show_axes, "Axes");
            ui.checkbox(&mut display_settings.normal_debug, "Normals");
        });
    });

    egui::SidePanel::left("model_panel")
        .resizable(false)
        .default_width(270.0)
        .show(ctx, |ui| {
            ui.heading("Model");
            if let Some(model) = model_info {
                ui.label(model.file_name.as_str());
                ui.separator();
                ui.label(format!("Path: {}", model.path.display()));
                ui.label(format!("Triangles: {}", model.stats.triangle_count));
                ui.label(format!("Chunks: {}", model.chunk_count));
                ui.label(format!("Bytes: {}", model.stats.source_bytes));
                ui.label(format!("Parse: {:.2} ms", model.parse_ms));
                ui.label(format!(
                    "GPU buffers: {:.2} MB",
                    gpu_buffer_bytes as f64 / (1024.0 * 1024.0)
                ));
                ui.separator();
                ui.label(format!(
                    "Min: {:.4}, {:.4}, {:.4}",
                    model.stats.bounds.min.x, model.stats.bounds.min.y, model.stats.bounds.min.z
                ));
                ui.label(format!(
                    "Max: {:.4}, {:.4}, {:.4}",
                    model.stats.bounds.max.x, model.stats.bounds.max.y, model.stats.bounds.max.z
                ));
            } else {
                ui.label("No model loaded");
            }
            ui.separator();
            if let Some(pick) = selected_pick {
                ui.label(format!(
                    "Selected: {}:{}",
                    pick.triangle_id.chunk, pick.triangle_id.local_index
                ));
                ui.label(format!(
                    "Point: {:.4}, {:.4}, {:.4}",
                    pick.position.x, pick.position.y, pick.position.z
                ));
                ui.separator();
            }
            ui.label(format!("GPU: {}", renderer_info.adapter_name));
            ui.label(format!("Backend: {:?}", renderer_info.backend));
        });

    egui::SidePanel::right("inspection_panel")
        .resizable(false)
        .default_width(330.0)
        .show(ctx, |ui| {
            ui.heading("Inspection");

            let Some(model) = model_info else {
                ui.label("Load an STL to inspect internal geometry.");
                return;
            };

            cross_section.clamp_to_bounds(model.stats.bounds);

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

            ui.separator();
            ui.heading("Issues");

            egui::ComboBox::from_label("Issue kind")
                .selected_text(selected_issue_kind.label())
                .show_ui(ui, |ui| {
                    for kind in IssueKind::ALL {
                        ui.selectable_value(selected_issue_kind, kind, kind.label());
                    }
                });

            let can_add_issue = selected_pick.is_some() && issue_session.is_some();
            if ui
                .add_enabled(can_add_issue, egui::Button::new("Add Issue"))
                .clicked()
            {
                *action = UiAction::AddIssue;
            }

            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    *action = UiAction::SaveIssues;
                }
                if ui.button("Load").clicked() {
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

            ui.separator();
            if let Some(session) = issue_session.as_mut() {
                if session.issues.is_empty() {
                    ui.label("No issues recorded");
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
                        "Cross Section {} {:.3}{}",
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
        });

    egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label(status);
        });
    });
}

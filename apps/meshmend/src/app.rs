use std::path::{Path, PathBuf};

use anyhow::Result;
use egui_wgpu::ScreenDescriptor;
use egui_winit::State as EguiWinitState;
use meshmend_core::MeshStats;
use meshmend_render::{MeshChunkUpload, RendererInfo, WgpuRenderer};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UiAction {
    None,
    LoadStl,
    Fit,
    Reset,
}

pub fn run_native(initial_file: Option<PathBuf>, smoke_window: bool) -> Result<()> {
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
                        let full_output = egui_ctx.run(raw_input, |ctx| {
                            draw_ui(
                                ctx,
                                renderer.info(),
                                model_info.as_ref(),
                                renderer.gpu_buffer_bytes(),
                                &status,
                                &mut action,
                            );
                        });
                        egui_state
                            .handle_platform_output(redraw_window, full_output.platform_output);

                        handle_ui_action(
                            action,
                            &mut renderer,
                            redraw_window,
                            &mut model_info,
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
                        if smoke_window {
                            target.exit();
                        }
                    }
                    WindowEvent::DroppedFile(path) => {
                        match load_model(&path, &mut renderer, redraw_window) {
                            Ok(info) => {
                                status = format!("Loaded {}", info.file_name);
                                model_info = Some(info);
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
                            camera_input.release(button);
                        }
                        _ => {}
                    },
                    WindowEvent::CursorMoved { position, .. } => {
                        if !egui_response.consumed {
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
                    }
                    WindowEvent::MouseWheel { delta, .. } => {
                        if !egui_response.consumed {
                            let wheel_delta = match delta {
                                MouseScrollDelta::LineDelta(_, y) => y,
                                MouseScrollDelta::PixelDelta(position) => position.y as f32 * 0.02,
                            };
                            let mut camera = renderer.camera();
                            camera.zoom(wheel_delta, renderer.mesh_bounds());
                            renderer.set_camera(camera);
                            needs_redraw = true;
                        }
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
            Event::AboutToWait => {
                if needs_redraw {
                    redraw_window.request_redraw();
                    needs_redraw = false;
                }
            }
            _ => {}
        }
    })?;

    Ok(())
}

fn handle_ui_action(
    action: UiAction,
    renderer: &mut WgpuRenderer<'_>,
    window: &Window,
    model_info: &mut Option<ModelInfo>,
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
                        *model_info = Some(info);
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

fn draw_ui(
    ctx: &egui::Context,
    renderer_info: &RendererInfo,
    model_info: Option<&ModelInfo>,
    gpu_buffer_bytes: u64,
    status: &str,
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
            ui.label(format!("GPU: {}", renderer_info.adapter_name));
            ui.label(format!("Backend: {:?}", renderer_info.backend));
        });

    egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label(status);
        });
    });
}

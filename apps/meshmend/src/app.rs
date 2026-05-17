use std::path::PathBuf;

use anyhow::Result;
use meshmend_render::{MeshChunkUpload, WgpuRenderer};
use meshmend_stl::load_binary_stl;
use winit::{
    dpi::LogicalSize,
    event::{ElementState, Event, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::WindowBuilder,
};

use crate::input::CameraInput;

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
    let window: &'static winit::window::Window = Box::leak(Box::new(window));
    let window_id = window.id();
    let redraw_window = window;
    let mut renderer = pollster::block_on(WgpuRenderer::new(window))?;
    tracing::info!(
        adapter = %renderer.info().adapter_name,
        backend = ?renderer.info().backend,
        "native renderer ready"
    );
    if let Some(path) = initial_file.as_ref() {
        let parsed = load_binary_stl(path)?;
        renderer.upload_mesh(
            parsed.chunks.iter().map(|chunk| MeshChunkUpload {
                chunk_index: chunk.chunk_index,
                start_triangle: chunk.start_triangle,
                bounds: chunk.bounds,
                triangles: &chunk.triangles,
            }),
            parsed.stats.bounds,
        );
        tracing::info!(
            file = %parsed.source_path.display(),
            triangles = parsed.stats.triangle_count,
            chunks = parsed.chunks.len(),
            gpu_buffer_mb = renderer.gpu_buffer_bytes() as f64 / (1024.0 * 1024.0),
            "loaded STL mesh"
        );
    }
    let mut camera_input = CameraInput::default();
    let mut needs_redraw = true;

    event_loop.run(move |event, target| {
        target.set_control_flow(ControlFlow::Wait);

        match event {
            Event::WindowEvent {
                window_id: event_window_id,
                event,
            } if event_window_id == window_id => match event {
                WindowEvent::CloseRequested => target.exit(),
                WindowEvent::Resized(size) => {
                    renderer.resize(size);
                    needs_redraw = true;
                }
                WindowEvent::RedrawRequested => {
                    if let Err(err) = renderer.render() {
                        tracing::error!(error = %err, "render failed");
                        target.exit();
                    }
                    if smoke_window {
                        target.exit();
                    }
                }
                WindowEvent::MouseInput { state, button, .. } => match state {
                    ElementState::Pressed if is_camera_button(button) => {
                        camera_input.press(button);
                    }
                    ElementState::Released => {
                        camera_input.release(button);
                    }
                    _ => {}
                },
                WindowEvent::CursorMoved { position, .. } => {
                    if let Some((button, delta)) = camera_input.cursor_delta(position.x, position.y)
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
                WindowEvent::MouseWheel { delta, .. } => {
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
                    if event.state == ElementState::Pressed =>
                {
                    match event.physical_key {
                        PhysicalKey::Code(KeyCode::KeyF) => {
                            renderer.fit_camera_to_mesh();
                            needs_redraw = true;
                        }
                        PhysicalKey::Code(KeyCode::KeyR) => {
                            if let Some(bounds) = renderer.mesh_bounds() {
                                let mut camera = renderer.camera();
                                camera.reset_to_bounds(
                                    bounds,
                                    renderer.size().width as f32
                                        / renderer.size().height.max(1) as f32,
                                );
                                renderer.set_camera(camera);
                                needs_redraw = true;
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            },
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

fn is_camera_button(button: MouseButton) -> bool {
    matches!(
        button,
        MouseButton::Left | MouseButton::Right | MouseButton::Middle
    )
}

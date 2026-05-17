use std::path::PathBuf;

use anyhow::Result;
use meshmend_render::{MeshChunkUpload, WgpuRenderer};
use meshmend_stl::load_binary_stl;
use winit::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
};

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

    event_loop.run(move |event, target| {
        target.set_control_flow(ControlFlow::Wait);

        match event {
            Event::WindowEvent {
                window_id: event_window_id,
                event,
            } if event_window_id == window_id => match event {
                WindowEvent::CloseRequested => target.exit(),
                WindowEvent::Resized(size) => renderer.resize(size),
                WindowEvent::RedrawRequested => {
                    if let Err(err) = renderer.render() {
                        tracing::error!(error = %err, "render failed");
                        target.exit();
                    }
                    if smoke_window {
                        target.exit();
                    }
                }
                _ => {}
            },
            Event::AboutToWait => {
                redraw_window.request_redraw();
            }
            _ => {}
        }
    })?;

    Ok(())
}

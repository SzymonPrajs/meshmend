use std::{fs, path::Path};

use glam::{Vec2, Vec3, Vec4};
use meshmend_core::{CrossSectionAxis, CrossSectionState, MeshBounds, Triangle, TriangleId};
use wgpu::util::DeviceExt;
use winit::{dpi::PhysicalSize, window::Window};

use crate::{
    buffers::{GpuTriangle, MeshChunkUpload},
    Camera,
};

#[derive(Debug, Clone)]
pub struct RendererInfo {
    pub adapter_name: String,
    pub backend: wgpu::Backend,
    pub surface_format: wgpu::TextureFormat,
    pub present_mode: wgpu::PresentMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LightingMode {
    Fixed,
    Headlight,
    Studio,
}

impl LightingMode {
    fn uniform_value(self) -> u32 {
        match self {
            Self::Fixed => 0,
            Self::Headlight => 1,
            Self::Studio => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplaySettings {
    pub wireframe: bool,
    pub show_backfaces: bool,
    pub show_grid: bool,
    pub show_axes: bool,
    pub normal_debug: bool,
    pub transparent: bool,
    pub xray_wire: bool,
    pub lighting_mode: LightingMode,
}

impl Default for DisplaySettings {
    fn default() -> Self {
        Self {
            wireframe: false,
            show_backfaces: true,
            show_grid: true,
            show_axes: true,
            normal_debug: false,
            transparent: false,
            xray_wire: false,
            lighting_mode: LightingMode::Headlight,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PickResult {
    pub triangle_id: TriangleId,
    pub position: Vec3,
}

#[derive(Debug, Clone, Copy)]
pub struct ScreenshotStats {
    pub width: u32,
    pub height: u32,
    pub non_background_pixels: u64,
    pub coverage: f64,
}

#[derive(Debug, Clone)]
pub struct LabelStrokeOverlay {
    pub points: Vec<Vec3>,
    pub radius: f32,
    pub color: [f32; 4],
}

pub struct WgpuRenderer<'window> {
    surface: wgpu::Surface<'window>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    size: PhysicalSize<u32>,
    depth: DepthTexture,
    camera: Camera,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    chunk_bind_group_layout: wgpu::BindGroupLayout,
    mesh_pipeline: wgpu::RenderPipeline,
    mesh_culled_pipeline: wgpu::RenderPipeline,
    mesh_xray_pipeline: wgpu::RenderPipeline,
    grid_pipeline: wgpu::RenderPipeline,
    picking_pipeline: wgpu::RenderPipeline,
    picking_target: PickingTarget,
    egui_renderer: egui_wgpu::Renderer,
    mesh_chunks: Vec<GpuMeshChunk>,
    scene_lines: Option<SceneLines>,
    mesh_bounds: Option<MeshBounds>,
    cross_section: CrossSectionState,
    cross_section_guide: Option<SceneLines>,
    label_strokes: Option<SceneLines>,
    selection_marker: Option<SceneLines>,
    issue_markers: Option<SceneLines>,
    display_settings: DisplaySettings,
    info: RendererInfo,
}

impl<'window> WgpuRenderer<'window> {
    pub async fn new(window: &'window Window) -> Result<Self, RenderError> {
        let size = window.inner_size();
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: preferred_backends(),
            ..Default::default()
        });
        let surface = instance.create_surface(window)?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .ok_or(RenderError::NoAdapter)?;
        let adapter_info = adapter.get_info();
        let limits = wgpu::Limits::default();
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("MeshMend device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: limits,
                },
                None,
            )
            .await?;

        let caps = surface.get_capabilities(&adapter);
        let surface_format = caps
            .formats
            .iter()
            .copied()
            .find(|format| {
                matches!(
                    format,
                    wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Bgra8Unorm
                )
            })
            .unwrap_or(caps.formats[0]);
        let present_mode = caps
            .present_modes
            .iter()
            .copied()
            .find(|mode| *mode == wgpu::PresentMode::Mailbox)
            .unwrap_or(wgpu::PresentMode::Fifo);
        let alpha_mode = caps.alpha_modes[0];
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            format: surface_format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode,
            alpha_mode,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);
        let depth = DepthTexture::new(&device, config.width, config.height);
        let camera = Camera::default();
        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("MeshMend camera uniform"),
            contents: bytemuck::bytes_of(&CameraUniform::from_camera(
                camera,
                aspect_from_size(size),
                DisplaySettings::default(),
                CrossSectionState::default(),
            )),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let camera_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("MeshMend camera bind group layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("MeshMend camera bind group"),
            layout: &camera_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });
        let chunk_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("MeshMend triangle chunk bind group layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });
        let mesh_pipeline = create_mesh_pipeline(
            &device,
            surface_format,
            &camera_bind_group_layout,
            &chunk_bind_group_layout,
            MeshPipelineConfig {
                label: "MeshMend mesh pipeline",
                cull_mode: None,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                blend: wgpu::BlendState::REPLACE,
            },
        );
        let mesh_culled_pipeline = create_mesh_pipeline(
            &device,
            surface_format,
            &camera_bind_group_layout,
            &chunk_bind_group_layout,
            MeshPipelineConfig {
                label: "MeshMend culled mesh pipeline",
                cull_mode: Some(wgpu::Face::Back),
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                blend: wgpu::BlendState::REPLACE,
            },
        );
        let mesh_xray_pipeline = create_mesh_pipeline(
            &device,
            surface_format,
            &camera_bind_group_layout,
            &chunk_bind_group_layout,
            MeshPipelineConfig {
                label: "MeshMend x-ray mesh pipeline",
                cull_mode: None,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,
                blend: wgpu::BlendState::ALPHA_BLENDING,
            },
        );
        let grid_pipeline =
            create_grid_pipeline(&device, surface_format, &camera_bind_group_layout);
        let picking_pipeline =
            create_picking_pipeline(&device, &camera_bind_group_layout, &chunk_bind_group_layout);
        let picking_target = PickingTarget::new(&device, config.width, config.height);
        let egui_renderer = egui_wgpu::Renderer::new(&device, surface_format, None, 1);
        let info = RendererInfo {
            adapter_name: adapter_info.name,
            backend: adapter_info.backend,
            surface_format,
            present_mode,
        };

        tracing::info!(
            adapter = %info.adapter_name,
            backend = ?info.backend,
            surface_format = ?info.surface_format,
            present_mode = ?info.present_mode,
            limits = ?device.limits(),
            "initialized native WGPU renderer"
        );

        Ok(Self {
            surface,
            device,
            queue,
            config,
            size,
            depth,
            camera,
            camera_buffer,
            camera_bind_group,
            chunk_bind_group_layout,
            mesh_pipeline,
            mesh_culled_pipeline,
            mesh_xray_pipeline,
            grid_pipeline,
            picking_pipeline,
            picking_target,
            egui_renderer,
            mesh_chunks: Vec::new(),
            scene_lines: None,
            mesh_bounds: None,
            cross_section: CrossSectionState::default(),
            cross_section_guide: None,
            label_strokes: None,
            selection_marker: None,
            issue_markers: None,
            display_settings: DisplaySettings::default(),
            info,
        })
    }

    pub fn info(&self) -> &RendererInfo {
        &self.info
    }

    pub fn size(&self) -> PhysicalSize<u32> {
        self.size
    }

    pub fn resize(&mut self, size: PhysicalSize<u32>) {
        if size.width == 0 || size.height == 0 {
            return;
        }
        self.size = size;
        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(&self.device, &self.config);
        self.depth = DepthTexture::new(&self.device, self.config.width, self.config.height);
        self.picking_target =
            PickingTarget::new(&self.device, self.config.width, self.config.height);
        self.write_camera();
    }

    pub fn camera(&self) -> Camera {
        self.camera
    }

    pub fn set_camera(&mut self, camera: Camera) {
        self.camera = camera;
        self.write_camera();
    }

    pub fn display_settings(&self) -> DisplaySettings {
        self.display_settings
    }

    pub fn set_display_settings(&mut self, settings: DisplaySettings) {
        self.display_settings = settings;
        self.write_camera();
    }

    pub fn cross_section(&self) -> CrossSectionState {
        self.cross_section
    }

    pub fn set_cross_section(&mut self, mut cross_section: CrossSectionState) {
        if let Some(bounds) = self.mesh_bounds {
            cross_section.clamp_to_bounds(bounds);
        }
        self.cross_section = cross_section;
        self.update_cross_section_guide();
        self.write_camera();
    }

    pub fn mesh_bounds(&self) -> Option<MeshBounds> {
        self.mesh_bounds
    }

    pub fn fit_camera_to_mesh(&mut self) {
        if let Some(bounds) = self.mesh_bounds {
            self.camera.fit_to_bounds(bounds, self.aspect());
            self.write_camera();
        }
    }

    pub fn upload_mesh<'a>(
        &mut self,
        chunks: impl IntoIterator<Item = MeshChunkUpload<'a>>,
        bounds: MeshBounds,
    ) {
        self.mesh_chunks.clear();
        self.mesh_bounds = Some(bounds);
        self.scene_lines = Some(SceneLines::new(&self.device, bounds));
        self.cross_section = CrossSectionState::centered(bounds);
        self.update_cross_section_guide();
        self.label_strokes = None;
        self.selection_marker = None;
        self.issue_markers = None;

        for chunk in chunks {
            if chunk.triangles.is_empty() {
                continue;
            }
            let triangles = chunk
                .triangles
                .iter()
                .copied()
                .map(GpuTriangle::from)
                .collect::<Vec<_>>();
            let triangle_buffer =
                self.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("MeshMend triangle chunk"),
                        contents: bytemuck::cast_slice(&triangles),
                        usage: wgpu::BufferUsages::STORAGE,
                    });
            let chunk_uniform = ChunkUniform {
                chunk_index: chunk.chunk_index,
                _pad: [0; 3],
            };
            let chunk_buffer = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("MeshMend chunk uniform"),
                    contents: bytemuck::bytes_of(&chunk_uniform),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
            let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("MeshMend triangle chunk bind group"),
                layout: &self.chunk_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: triangle_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: chunk_buffer.as_entire_binding(),
                    },
                ],
            });

            self.mesh_chunks.push(GpuMeshChunk {
                chunk_index: chunk.chunk_index,
                start_triangle: chunk.start_triangle,
                bounds: chunk.bounds,
                triangle_count: triangles.len() as u32,
                cpu_triangles: chunk.triangles.to_vec(),
                triangle_buffer,
                chunk_buffer,
                bind_group,
            });
        }

        self.fit_camera_to_mesh();
    }

    pub fn render(&mut self) -> Result<(), RenderError> {
        self.render_internal(None)
    }

    pub fn render_with_egui(
        &mut self,
        paint_jobs: &[egui::ClippedPrimitive],
        textures_delta: &egui::TexturesDelta,
        screen_descriptor: &egui_wgpu::ScreenDescriptor,
    ) -> Result<(), RenderError> {
        self.render_internal(Some((paint_jobs, textures_delta, screen_descriptor)))
    }

    fn render_internal(
        &mut self,
        egui_data: Option<(
            &[egui::ClippedPrimitive],
            &egui::TexturesDelta,
            &egui_wgpu::ScreenDescriptor,
        )>,
    ) -> Result<(), RenderError> {
        let frame = match self.surface.get_current_texture() {
            Ok(frame) => frame,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                self.resize(self.size);
                return Ok(());
            }
            Err(wgpu::SurfaceError::Timeout) => return Ok(()),
            Err(wgpu::SurfaceError::OutOfMemory) => return Err(RenderError::SurfaceOutOfMemory),
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("MeshMend render encoder"),
            });
        let mut command_buffers = Vec::new();

        if let Some((paint_jobs, textures_delta, screen_descriptor)) = egui_data {
            for (texture_id, image_delta) in &textures_delta.set {
                self.egui_renderer.update_texture(
                    &self.device,
                    &self.queue,
                    *texture_id,
                    image_delta,
                );
            }
            command_buffers.extend(self.egui_renderer.update_buffers(
                &self.device,
                &self.queue,
                &mut encoder,
                paint_jobs,
                screen_descriptor,
            ));
        }

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("MeshMend clear pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.045,
                            g: 0.052,
                            b: 0.06,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth.view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: None,
            });
            if let Some(lines) = &self.scene_lines {
                pass.set_pipeline(&self.grid_pipeline);
                pass.set_bind_group(0, &self.camera_bind_group, &[]);
                pass.set_vertex_buffer(0, lines.buffer.slice(..));
                if self.display_settings.show_grid && lines.grid_vertex_count > 0 {
                    pass.draw(0..lines.grid_vertex_count, 0..1);
                }
                if self.display_settings.show_axes && lines.axes_vertex_count > 0 {
                    pass.draw(
                        lines.grid_vertex_count..lines.grid_vertex_count + lines.axes_vertex_count,
                        0..1,
                    );
                }
            }
            let mesh_pipeline =
                if self.display_settings.xray_wire || self.display_settings.transparent {
                    &self.mesh_xray_pipeline
                } else if self.display_settings.show_backfaces {
                    &self.mesh_pipeline
                } else {
                    &self.mesh_culled_pipeline
                };
            pass.set_pipeline(mesh_pipeline);
            pass.set_bind_group(0, &self.camera_bind_group, &[]);
            for chunk in &self.mesh_chunks {
                pass.set_bind_group(1, &chunk.bind_group, &[]);
                pass.draw(0..3, 0..chunk.triangle_count);
            }
            if let Some(guide) = &self.cross_section_guide {
                pass.set_pipeline(&self.grid_pipeline);
                pass.set_bind_group(0, &self.camera_bind_group, &[]);
                pass.set_vertex_buffer(0, guide.buffer.slice(..));
                pass.draw(0..guide.grid_vertex_count, 0..1);
            }
            if let Some(strokes) = &self.label_strokes {
                pass.set_pipeline(&self.grid_pipeline);
                pass.set_bind_group(0, &self.camera_bind_group, &[]);
                pass.set_vertex_buffer(0, strokes.buffer.slice(..));
                pass.draw(0..strokes.grid_vertex_count, 0..1);
            }
            if let Some(marker) = &self.selection_marker {
                pass.set_pipeline(&self.grid_pipeline);
                pass.set_bind_group(0, &self.camera_bind_group, &[]);
                pass.set_vertex_buffer(0, marker.buffer.slice(..));
                pass.draw(0..marker.grid_vertex_count, 0..1);
            }
            if let Some(markers) = &self.issue_markers {
                pass.set_pipeline(&self.grid_pipeline);
                pass.set_bind_group(0, &self.camera_bind_group, &[]);
                pass.set_vertex_buffer(0, markers.buffer.slice(..));
                pass.draw(0..markers.grid_vertex_count, 0..1);
            }
        }

        if let Some((paint_jobs, textures_delta, screen_descriptor)) = egui_data {
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("MeshMend egui pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    occlusion_query_set: None,
                    timestamp_writes: None,
                });
                self.egui_renderer
                    .render(&mut pass, paint_jobs, screen_descriptor);
            }
            for texture_id in &textures_delta.free {
                self.egui_renderer.free_texture(texture_id);
            }
        }

        command_buffers.push(encoder.finish());
        self.queue.submit(command_buffers);
        frame.present();
        Ok(())
    }

    pub fn gpu_buffer_bytes(&self) -> u64 {
        self.mesh_chunks
            .iter()
            .map(|chunk| {
                u64::from(chunk.triangle_count) * std::mem::size_of::<GpuTriangle>() as u64
                    + std::mem::size_of::<ChunkUniform>() as u64
            })
            .sum()
    }

    pub fn screenshot(&mut self, path: Option<&Path>) -> Result<ScreenshotStats, RenderError> {
        let frame = match self.surface.get_current_texture() {
            Ok(frame) => frame,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                self.resize(self.size);
                return Err(RenderError::SurfaceUnavailable);
            }
            Err(wgpu::SurfaceError::Timeout) => return Err(RenderError::SurfaceUnavailable),
            Err(wgpu::SurfaceError::OutOfMemory) => return Err(RenderError::SurfaceOutOfMemory),
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let width = self.config.width.max(1);
        let height = self.config.height.max(1);
        let unpadded_bytes_per_row = width * 4;
        let padded_bytes_per_row = align_to(unpadded_bytes_per_row, 256);
        let output_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("MeshMend screenshot readback"),
            size: u64::from(padded_bytes_per_row) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("MeshMend screenshot encoder"),
            });
        self.encode_scene(&mut encoder, &view);
        encoder.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: &frame.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &output_buffer,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(encoder.finish()));
        frame.present();

        let slice = output_buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        self.device.poll(wgpu::Maintain::Wait);
        receiver
            .recv()
            .map_err(|_| RenderError::PickReadbackClosed)??;
        let data = slice.get_mapped_range();
        let mut rgba = vec![0; (width * height * 4) as usize];
        for y in 0..height as usize {
            let src_start = y * padded_bytes_per_row as usize;
            let dst_start = y * unpadded_bytes_per_row as usize;
            let src = &data[src_start..src_start + unpadded_bytes_per_row as usize];
            let dst = &mut rgba[dst_start..dst_start + unpadded_bytes_per_row as usize];
            match self.config.format {
                wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb => {
                    for (src, dst) in src.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
                        dst[0] = src[2];
                        dst[1] = src[1];
                        dst[2] = src[0];
                        dst[3] = src[3];
                    }
                }
                _ => dst.copy_from_slice(src),
            }
        }
        drop(data);
        output_buffer.unmap();

        if let Some(path) = path {
            if let Some(parent) = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            image::save_buffer(path, &rgba, width, height, image::ColorType::Rgba8)?;
        }

        let non_background_pixels = count_non_background_pixels(&rgba);
        let total = u64::from(width) * u64::from(height);
        Ok(ScreenshotStats {
            width,
            height,
            non_background_pixels,
            coverage: non_background_pixels as f64 / total.max(1) as f64,
        })
    }

    pub fn pick(&mut self, screen_position: Vec2) -> Result<Option<PickResult>, RenderError> {
        if self.mesh_chunks.is_empty() || self.size.width == 0 || self.size.height == 0 {
            return Ok(None);
        }

        let x = screen_position.x.clamp(0.0, self.size.width as f32 - 1.0) as u32;
        let y = screen_position.y.clamp(0.0, self.size.height as f32 - 1.0) as u32;
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("MeshMend picking encoder"),
            });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("MeshMend picking pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.picking_target.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.picking_target.depth.view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.picking_pipeline);
            pass.set_bind_group(0, &self.camera_bind_group, &[]);
            for chunk in &self.mesh_chunks {
                pass.set_bind_group(1, &chunk.bind_group, &[]);
                pass.draw(0..3, 0..chunk.triangle_count);
            }
        }

        encoder.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: &self.picking_target.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &self.picking_target.readback,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(PickingTarget::READBACK_BYTES_PER_ROW),
                    rows_per_image: Some(1),
                },
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(encoder.finish()));

        let slice = self.picking_target.readback.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        self.device.poll(wgpu::Maintain::Wait);
        receiver
            .recv()
            .map_err(|_| RenderError::PickReadbackClosed)??;
        let data = slice.get_mapped_range();
        let pick_id = u32::from_le_bytes(data[0..4].try_into().unwrap());
        drop(data);
        self.picking_target.readback.unmap();

        let Some(triangle_id) = TriangleId::decode_picking_id(pick_id) else {
            self.set_selection_marker(None);
            return Ok(None);
        };
        let Some(triangle) = self.triangle(triangle_id) else {
            self.set_selection_marker(None);
            return Ok(None);
        };
        let ray = self.pick_ray(screen_position);
        let position = intersect_triangle(ray, triangle).unwrap_or_else(|| {
            (triangle.vertices[0] + triangle.vertices[1] + triangle.vertices[2]) / 3.0
        });
        self.set_selection_marker(Some(position));

        Ok(Some(PickResult {
            triangle_id,
            position,
        }))
    }

    pub fn set_selection_marker(&mut self, position: Option<Vec3>) {
        self.selection_marker = position.map(|position| {
            SceneLines::marker(
                &self.device,
                position,
                self.marker_radius(),
                [1.0, 0.72, 0.16, 1.0],
            )
        });
    }

    pub fn set_issue_markers(&mut self, positions: &[Vec3]) {
        self.issue_markers = (!positions.is_empty()).then(|| {
            SceneLines::markers(
                &self.device,
                positions,
                self.marker_radius() * 0.8,
                [0.45, 0.82, 1.0, 0.95],
            )
        });
    }

    pub fn set_label_strokes(&mut self, strokes: &[LabelStrokeOverlay]) {
        self.label_strokes =
            (!strokes.is_empty()).then(|| SceneLines::label_strokes(&self.device, strokes));
    }

    fn update_cross_section_guide(&mut self) {
        self.cross_section_guide = self
            .mesh_bounds
            .filter(|_| self.cross_section.enabled && self.cross_section.show_plane_guide)
            .map(|bounds| SceneLines::cross_section(&self.device, bounds, self.cross_section));
    }

    fn triangle(&self, triangle_id: TriangleId) -> Option<Triangle> {
        let chunk = self
            .mesh_chunks
            .iter()
            .find(|chunk| chunk.chunk_index == triangle_id.chunk)?;
        chunk
            .cpu_triangles
            .get(triangle_id.local_index as usize)
            .copied()
    }

    fn pick_ray(&self, screen_position: Vec2) -> Ray {
        let width = self.size.width.max(1) as f32;
        let height = self.size.height.max(1) as f32;
        let ndc_x = screen_position.x / width * 2.0 - 1.0;
        let ndc_y = 1.0 - screen_position.y / height * 2.0;
        let inverse = self.camera.view_projection(self.aspect()).inverse();
        let near = inverse.project_point3(Vec3::new(ndc_x, ndc_y, -1.0));
        let far = inverse.project_point3(Vec3::new(ndc_x, ndc_y, 1.0));
        Ray {
            origin: near,
            direction: (far - near).normalize_or_zero(),
        }
    }

    fn marker_radius(&self) -> f32 {
        self.mesh_bounds
            .map(|bounds| bounds.radius() * 0.018)
            .unwrap_or(0.05)
            .max(0.002)
    }

    fn encode_scene(&self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("MeshMend screenshot scene pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.045,
                        g: 0.052,
                        b: 0.06,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.depth.view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            occlusion_query_set: None,
            timestamp_writes: None,
        });
        if let Some(lines) = &self.scene_lines {
            pass.set_pipeline(&self.grid_pipeline);
            pass.set_bind_group(0, &self.camera_bind_group, &[]);
            pass.set_vertex_buffer(0, lines.buffer.slice(..));
            if self.display_settings.show_grid && lines.grid_vertex_count > 0 {
                pass.draw(0..lines.grid_vertex_count, 0..1);
            }
            if self.display_settings.show_axes && lines.axes_vertex_count > 0 {
                pass.draw(
                    lines.grid_vertex_count..lines.grid_vertex_count + lines.axes_vertex_count,
                    0..1,
                );
            }
        }
        let mesh_pipeline = if self.display_settings.xray_wire || self.display_settings.transparent
        {
            &self.mesh_xray_pipeline
        } else if self.display_settings.show_backfaces {
            &self.mesh_pipeline
        } else {
            &self.mesh_culled_pipeline
        };
        pass.set_pipeline(mesh_pipeline);
        pass.set_bind_group(0, &self.camera_bind_group, &[]);
        for chunk in &self.mesh_chunks {
            pass.set_bind_group(1, &chunk.bind_group, &[]);
            pass.draw(0..3, 0..chunk.triangle_count);
        }
        if let Some(guide) = &self.cross_section_guide {
            pass.set_pipeline(&self.grid_pipeline);
            pass.set_bind_group(0, &self.camera_bind_group, &[]);
            pass.set_vertex_buffer(0, guide.buffer.slice(..));
            pass.draw(0..guide.grid_vertex_count, 0..1);
        }
        if let Some(strokes) = &self.label_strokes {
            pass.set_pipeline(&self.grid_pipeline);
            pass.set_bind_group(0, &self.camera_bind_group, &[]);
            pass.set_vertex_buffer(0, strokes.buffer.slice(..));
            pass.draw(0..strokes.grid_vertex_count, 0..1);
        }
    }

    fn write_camera(&self) {
        self.queue.write_buffer(
            &self.camera_buffer,
            0,
            bytemuck::bytes_of(&CameraUniform::from_camera(
                self.camera,
                self.aspect(),
                self.display_settings,
                self.cross_section,
            )),
        );
    }

    fn aspect(&self) -> f32 {
        self.config.width as f32 / self.config.height.max(1) as f32
    }
}

#[allow(dead_code)]
struct GpuMeshChunk {
    chunk_index: u32,
    start_triangle: u64,
    bounds: MeshBounds,
    triangle_count: u32,
    cpu_triangles: Vec<Triangle>,
    triangle_buffer: wgpu::Buffer,
    chunk_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct CameraUniform {
    view_proj: [[f32; 4]; 4],
    eye: [f32; 4],
    light_dir: [f32; 4],
    material: [f32; 4],
    clip_plane: [f32; 4],
    settings: [u32; 4],
    view: [u32; 4],
}

impl CameraUniform {
    fn from_camera(
        camera: Camera,
        aspect: f32,
        settings: DisplaySettings,
        cross_section: CrossSectionState,
    ) -> Self {
        let plane = cross_section.plane();
        let (_, _, camera_forward) = camera.basis();
        let light_dir = match settings.lighting_mode {
            LightingMode::Fixed | LightingMode::Studio => Vec3::new(-0.35, -0.65, -0.62),
            LightingMode::Headlight => camera_forward,
        };
        Self {
            view_proj: camera.view_projection(aspect).to_cols_array_2d(),
            eye: camera.eye().extend(1.0).to_array(),
            light_dir: light_dir.extend(0.0).to_array(),
            material: Vec4::new(0.66, 0.70, 0.70, 1.0).to_array(),
            clip_plane: plane.normal.extend(plane.offset).to_array(),
            settings: [
                settings.wireframe as u32,
                settings.normal_debug as u32,
                cross_section.enabled as u32,
                0,
            ],
            view: [
                settings.transparent as u32,
                settings.xray_wire as u32,
                settings.lighting_mode.uniform_value(),
                0,
            ],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct ChunkUniform {
    chunk_index: u32,
    _pad: [u32; 3],
}

struct MeshPipelineConfig {
    label: &'static str,
    cull_mode: Option<wgpu::Face>,
    depth_write_enabled: bool,
    depth_compare: wgpu::CompareFunction,
    blend: wgpu::BlendState,
}

fn create_mesh_pipeline(
    device: &wgpu::Device,
    surface_format: wgpu::TextureFormat,
    camera_bind_group_layout: &wgpu::BindGroupLayout,
    chunk_bind_group_layout: &wgpu::BindGroupLayout,
    config: MeshPipelineConfig,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("MeshMend mesh shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shaders/mesh.wgsl").into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("MeshMend mesh pipeline layout"),
        bind_group_layouts: &[camera_bind_group_layout, chunk_bind_group_layout],
        push_constant_ranges: &[],
    });

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(config.label),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: "vs_main",
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: "fs_main",
            targets: &[Some(wgpu::ColorTargetState {
                format: surface_format,
                blend: Some(config.blend),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: config.cull_mode,
            polygon_mode: wgpu::PolygonMode::Fill,
            unclipped_depth: false,
            conservative: false,
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DepthTexture::FORMAT,
            depth_write_enabled: config.depth_write_enabled,
            depth_compare: config.depth_compare,
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
    })
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct LineVertex {
    position: [f32; 3],
    color: [f32; 4],
}

impl LineVertex {
    const ATTRIBUTES: [wgpu::VertexAttribute; 2] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4];

    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<LineVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

struct SceneLines {
    buffer: wgpu::Buffer,
    grid_vertex_count: u32,
    axes_vertex_count: u32,
}

impl SceneLines {
    fn new(device: &wgpu::Device, bounds: MeshBounds) -> Self {
        let mut vertices = Vec::new();
        let center = bounds.center();
        let extent = bounds.extent();
        let radius = bounds.radius().max(1.0);
        let half = extent.x.max(extent.y).max(radius) * 0.75;
        let z = bounds.min.z;
        let step = (half * 2.0 / 12.0).max(0.001);
        let grid_color = [0.26, 0.30, 0.32, 0.42];

        for i in -12..=12 {
            let offset = i as f32 * step;
            vertices.push(LineVertex {
                position: [center.x - half, center.y + offset, z],
                color: grid_color,
            });
            vertices.push(LineVertex {
                position: [center.x + half, center.y + offset, z],
                color: grid_color,
            });
            vertices.push(LineVertex {
                position: [center.x + offset, center.y - half, z],
                color: grid_color,
            });
            vertices.push(LineVertex {
                position: [center.x + offset, center.y + half, z],
                color: grid_color,
            });
        }
        let grid_vertex_count = vertices.len() as u32;

        let axis = radius.max(half) * 1.15;
        let axes = [
            (
                [center.x - axis, center.y, center.z],
                [center.x + axis, center.y, center.z],
                [0.95, 0.25, 0.22, 0.95],
            ),
            (
                [center.x, center.y - axis, center.z],
                [center.x, center.y + axis, center.z],
                [0.35, 0.82, 0.36, 0.95],
            ),
            (
                [center.x, center.y, center.z - axis],
                [center.x, center.y, center.z + axis],
                [0.32, 0.55, 1.0, 0.95],
            ),
        ];
        for (start, end, color) in axes {
            vertices.push(LineVertex {
                position: start,
                color,
            });
            vertices.push(LineVertex {
                position: end,
                color,
            });
        }
        let axes_vertex_count = vertices.len() as u32 - grid_vertex_count;

        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("MeshMend grid and axes buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Self {
            buffer,
            grid_vertex_count,
            axes_vertex_count,
        }
    }

    fn marker(device: &wgpu::Device, position: Vec3, radius: f32, color: [f32; 4]) -> Self {
        Self::markers(device, &[position], radius, color)
    }

    fn markers(device: &wgpu::Device, positions: &[Vec3], radius: f32, color: [f32; 4]) -> Self {
        let mut vertices = Vec::with_capacity(positions.len() * 6);
        for position in positions {
            vertices.extend_from_slice(&[
                LineVertex {
                    position: (*position + Vec3::new(-radius, 0.0, 0.0)).to_array(),
                    color,
                },
                LineVertex {
                    position: (*position + Vec3::new(radius, 0.0, 0.0)).to_array(),
                    color,
                },
                LineVertex {
                    position: (*position + Vec3::new(0.0, -radius, 0.0)).to_array(),
                    color,
                },
                LineVertex {
                    position: (*position + Vec3::new(0.0, radius, 0.0)).to_array(),
                    color,
                },
                LineVertex {
                    position: (*position + Vec3::new(0.0, 0.0, -radius)).to_array(),
                    color,
                },
                LineVertex {
                    position: (*position + Vec3::new(0.0, 0.0, radius)).to_array(),
                    color,
                },
            ]);
        }
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("MeshMend marker buffer"),
            contents: bytemuck::cast_slice(vertices.as_slice()),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Self {
            buffer,
            grid_vertex_count: vertices.len() as u32,
            axes_vertex_count: 0,
        }
    }

    fn label_strokes(device: &wgpu::Device, strokes: &[LabelStrokeOverlay]) -> Self {
        let mut vertices = Vec::new();
        for stroke in strokes {
            for segment in stroke.points.windows(2) {
                vertices.push(LineVertex {
                    position: segment[0].to_array(),
                    color: stroke.color,
                });
                vertices.push(LineVertex {
                    position: segment[1].to_array(),
                    color: stroke.color,
                });
            }

            let radius = stroke.radius.max(0.001);
            for (index, point) in stroke.points.iter().enumerate() {
                if index % 2 == 0 || index + 1 == stroke.points.len() {
                    Self::push_ring(
                        &mut vertices,
                        *point,
                        radius,
                        Vec3::X,
                        Vec3::Y,
                        stroke.color,
                    );
                    Self::push_ring(
                        &mut vertices,
                        *point,
                        radius,
                        Vec3::X,
                        Vec3::Z,
                        stroke.color,
                    );
                    Self::push_ring(
                        &mut vertices,
                        *point,
                        radius,
                        Vec3::Y,
                        Vec3::Z,
                        stroke.color,
                    );
                }
            }
        }

        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("MeshMend label stroke buffer"),
            contents: bytemuck::cast_slice(vertices.as_slice()),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Self {
            buffer,
            grid_vertex_count: vertices.len() as u32,
            axes_vertex_count: 0,
        }
    }

    fn push_ring(
        vertices: &mut Vec<LineVertex>,
        center: Vec3,
        radius: f32,
        axis_a: Vec3,
        axis_b: Vec3,
        color: [f32; 4],
    ) {
        const SEGMENTS: usize = 32;

        for segment in 0..SEGMENTS {
            let start_angle = segment as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
            let end_angle = (segment + 1) as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
            let start = center + (axis_a * start_angle.cos() + axis_b * start_angle.sin()) * radius;
            let end = center + (axis_a * end_angle.cos() + axis_b * end_angle.sin()) * radius;
            vertices.push(LineVertex {
                position: start.to_array(),
                color,
            });
            vertices.push(LineVertex {
                position: end.to_array(),
                color,
            });
        }
    }

    fn cross_section(
        device: &wgpu::Device,
        bounds: MeshBounds,
        cross_section: CrossSectionState,
    ) -> Self {
        let color = cross_section.axis.color();
        let min = bounds.min;
        let max = bounds.max;
        let offset = cross_section.offset;
        let corners = match cross_section.axis {
            CrossSectionAxis::X => [
                Vec3::new(offset, min.y, min.z),
                Vec3::new(offset, max.y, min.z),
                Vec3::new(offset, max.y, max.z),
                Vec3::new(offset, min.y, max.z),
            ],
            CrossSectionAxis::Y => [
                Vec3::new(min.x, offset, min.z),
                Vec3::new(max.x, offset, min.z),
                Vec3::new(max.x, offset, max.z),
                Vec3::new(min.x, offset, max.z),
            ],
            CrossSectionAxis::Z => [
                Vec3::new(min.x, min.y, offset),
                Vec3::new(max.x, min.y, offset),
                Vec3::new(max.x, max.y, offset),
                Vec3::new(min.x, max.y, offset),
            ],
        };
        let center = (corners[0] + corners[2]) * 0.5;
        let mut vertices = Vec::with_capacity(12);
        for index in 0..4 {
            vertices.push(LineVertex {
                position: corners[index].to_array(),
                color,
            });
            vertices.push(LineVertex {
                position: corners[(index + 1) % 4].to_array(),
                color,
            });
        }
        vertices.push(LineVertex {
            position: corners[0].to_array(),
            color,
        });
        vertices.push(LineVertex {
            position: corners[2].to_array(),
            color,
        });
        vertices.push(LineVertex {
            position: corners[1].to_array(),
            color,
        });
        vertices.push(LineVertex {
            position: corners[3].to_array(),
            color,
        });
        vertices.push(LineVertex {
            position: center.to_array(),
            color: [color[0], color[1], color[2], 1.0],
        });
        vertices.push(LineVertex {
            position: (center + cross_section.axis.normal() * bounds.radius() * 0.08).to_array(),
            color: [color[0], color[1], color[2], 1.0],
        });

        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("MeshMend cross-section guide buffer"),
            contents: bytemuck::cast_slice(vertices.as_slice()),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Self {
            buffer,
            grid_vertex_count: vertices.len() as u32,
            axes_vertex_count: 0,
        }
    }
}

fn create_grid_pipeline(
    device: &wgpu::Device,
    surface_format: wgpu::TextureFormat,
    camera_bind_group_layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("MeshMend grid shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shaders/grid.wgsl").into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("MeshMend grid pipeline layout"),
        bind_group_layouts: &[camera_bind_group_layout],
        push_constant_ranges: &[],
    });

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("MeshMend grid pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: "vs_main",
            buffers: &[LineVertex::layout()],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: "fs_main",
            targets: &[Some(wgpu::ColorTargetState {
                format: surface_format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::LineList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            polygon_mode: wgpu::PolygonMode::Fill,
            unclipped_depth: false,
            conservative: false,
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DepthTexture::FORMAT,
            depth_write_enabled: false,
            depth_compare: wgpu::CompareFunction::LessEqual,
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
    })
}

fn create_picking_pipeline(
    device: &wgpu::Device,
    camera_bind_group_layout: &wgpu::BindGroupLayout,
    chunk_bind_group_layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("MeshMend picking shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shaders/picking.wgsl").into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("MeshMend picking pipeline layout"),
        bind_group_layouts: &[camera_bind_group_layout, chunk_bind_group_layout],
        push_constant_ranges: &[],
    });

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("MeshMend picking pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: "vs_main",
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: "fs_main",
            targets: &[Some(wgpu::ColorTargetState {
                format: PickingTarget::FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            polygon_mode: wgpu::PolygonMode::Fill,
            unclipped_depth: false,
            conservative: false,
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DepthTexture::FORMAT,
            depth_write_enabled: true,
            depth_compare: wgpu::CompareFunction::Less,
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
    })
}

struct PickingTarget {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    depth: DepthTexture,
    readback: wgpu::Buffer,
}

impl PickingTarget {
    const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;
    const READBACK_BYTES_PER_ROW: u32 = 256;

    fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("MeshMend picking texture"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: Self::FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("MeshMend picking readback"),
            size: Self::READBACK_BYTES_PER_ROW as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        Self {
            texture,
            view,
            depth: DepthTexture::new(device, width, height),
            readback,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Ray {
    origin: Vec3,
    direction: Vec3,
}

fn intersect_triangle(ray: Ray, triangle: Triangle) -> Option<Vec3> {
    let epsilon = 1.0e-7;
    let edge1 = triangle.vertices[1] - triangle.vertices[0];
    let edge2 = triangle.vertices[2] - triangle.vertices[0];
    let h = ray.direction.cross(edge2);
    let a = edge1.dot(h);
    if a.abs() < epsilon {
        return None;
    }

    let f = 1.0 / a;
    let s = ray.origin - triangle.vertices[0];
    let u = f * s.dot(h);
    if !(0.0..=1.0).contains(&u) {
        return None;
    }

    let q = s.cross(edge1);
    let v = f * ray.direction.dot(q);
    if v < 0.0 || u + v > 1.0 {
        return None;
    }

    let t = f * edge2.dot(q);
    (t > epsilon).then(|| ray.origin + ray.direction * t)
}

fn align_to(value: u32, alignment: u32) -> u32 {
    value.div_ceil(alignment) * alignment
}

fn count_non_background_pixels(rgba: &[u8]) -> u64 {
    let background = [12_u8, 13_u8, 15_u8, 255_u8];
    rgba.chunks_exact(4)
        .filter(|pixel| {
            pixel[3] > 0
                && pixel
                    .iter()
                    .zip(background)
                    .map(|(actual, expected)| actual.abs_diff(expected) as u16)
                    .sum::<u16>()
                    > 18
        })
        .count() as u64
}

fn aspect_from_size(size: PhysicalSize<u32>) -> f32 {
    size.width.max(1) as f32 / size.height.max(1) as f32
}

struct DepthTexture {
    view: wgpu::TextureView,
}

impl DepthTexture {
    const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

    fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("MeshMend depth texture"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: Self::FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        Self {
            view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
        }
    }
}

fn preferred_backends() -> wgpu::Backends {
    #[cfg(target_os = "macos")]
    {
        wgpu::Backends::METAL
    }
    #[cfg(target_os = "windows")]
    {
        wgpu::Backends::DX12
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        wgpu::Backends::PRIMARY
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("no compatible GPU adapter was found")]
    NoAdapter,
    #[error("GPU surface was unavailable for screenshot capture")]
    SurfaceUnavailable,
    #[error("GPU surface is out of memory")]
    SurfaceOutOfMemory,
    #[error("GPU picking readback channel closed")]
    PickReadbackClosed,
    #[error(transparent)]
    PickReadback(#[from] wgpu::BufferAsyncError),
    #[error(transparent)]
    CreateSurface(#[from] wgpu::CreateSurfaceError),
    #[error(transparent)]
    RequestDevice(#[from] wgpu::RequestDeviceError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Image(#[from] image::ImageError),
}

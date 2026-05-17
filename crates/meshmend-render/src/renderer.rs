use glam::Vec4;
use meshmend_core::MeshBounds;
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
    egui_renderer: egui_wgpu::Renderer,
    mesh_chunks: Vec<GpuMeshChunk>,
    mesh_bounds: Option<MeshBounds>,
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
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
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
        );
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
            egui_renderer,
            mesh_chunks: Vec::new(),
            mesh_bounds: None,
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
        self.write_camera();
    }

    pub fn camera(&self) -> Camera {
        self.camera
    }

    pub fn set_camera(&mut self, camera: Camera) {
        self.camera = camera;
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
            pass.set_pipeline(&self.mesh_pipeline);
            pass.set_bind_group(0, &self.camera_bind_group, &[]);
            for chunk in &self.mesh_chunks {
                pass.set_bind_group(1, &chunk.bind_group, &[]);
                pass.draw(0..3, 0..chunk.triangle_count);
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

    fn write_camera(&self) {
        self.queue.write_buffer(
            &self.camera_buffer,
            0,
            bytemuck::bytes_of(&CameraUniform::from_camera(self.camera, self.aspect())),
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
}

impl CameraUniform {
    fn from_camera(camera: Camera, aspect: f32) -> Self {
        Self {
            view_proj: camera.view_projection(aspect).to_cols_array_2d(),
            eye: camera.eye().extend(1.0).to_array(),
            light_dir: Vec4::new(-0.35, -0.65, -0.62, 0.0).to_array(),
            material: Vec4::new(0.66, 0.70, 0.70, 1.0).to_array(),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct ChunkUniform {
    chunk_index: u32,
    _pad: [u32; 3],
}

fn create_mesh_pipeline(
    device: &wgpu::Device,
    surface_format: wgpu::TextureFormat,
    camera_bind_group_layout: &wgpu::BindGroupLayout,
    chunk_bind_group_layout: &wgpu::BindGroupLayout,
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
        label: Some("MeshMend mesh pipeline"),
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
                blend: Some(wgpu::BlendState::REPLACE),
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
    #[error("GPU surface is out of memory")]
    SurfaceOutOfMemory,
    #[error(transparent)]
    CreateSurface(#[from] wgpu::CreateSurfaceError),
    #[error(transparent)]
    RequestDevice(#[from] wgpu::RequestDeviceError),
}

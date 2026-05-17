use std::{
    fs,
    path::Path,
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
    time::Instant,
};

use glam::{Vec2, Vec3, Vec4};
use meshmend_core::{
    CrossSectionAxis, CrossSectionPlane, CrossSectionState, MeshBounds, Triangle, TriangleId,
};
use meshmend_geometry::{
    cut_plane_from_view_rays, preview_cut, CutPlane, CutPreview, Ray, SelectionMesh,
};
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

#[derive(Debug, Clone, Copy)]
pub struct SelectionSummary {
    pub vertex_count: usize,
    pub face_count: usize,
    pub component_count: u32,
    pub boundary_loop_count: usize,
    pub non_manifold_edge_count: usize,
}

#[derive(Debug, Clone)]
pub struct LabelStrokeOverlay {
    pub points: Vec<Vec3>,
    pub radius: f32,
    pub color: [f32; 4],
}

#[derive(Debug, Clone, Default)]
pub struct SelectionOverlay {
    pub vertices: Vec<Vec3>,
    pub edges: Vec<[Vec3; 2]>,
    pub faces: Vec<[Vec3; 3]>,
}

impl SelectionOverlay {
    pub fn is_empty(&self) -> bool {
        self.vertices.is_empty() && self.edges.is_empty() && self.faces.is_empty()
    }
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
    selection_face_pipeline: wgpu::RenderPipeline,
    picking_pipeline: wgpu::RenderPipeline,
    picking_target: PickingTarget,
    egui_renderer: egui_wgpu::Renderer,
    mesh_chunks: Vec<GpuMeshChunk>,
    scene_lines: Option<SceneLines>,
    mesh_bounds: Option<MeshBounds>,
    cross_section: CrossSectionState,
    cross_section_guide: Option<SceneLines>,
    label_strokes: Option<SceneLines>,
    selection_overlay: Option<SelectionSceneOverlay>,
    cut_preview: Option<SceneLines>,
    selection_marker: Option<SceneLines>,
    issue_markers: Option<SceneLines>,
    pick_mesh: Option<PickMesh>,
    pick_mesh_build: Option<Receiver<PickMesh>>,
    selection_mesh: Option<SelectionMesh>,
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
        let selection_face_pipeline =
            create_selection_face_pipeline(&device, surface_format, &camera_bind_group_layout);
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
            selection_face_pipeline,
            picking_pipeline,
            picking_target,
            egui_renderer,
            mesh_chunks: Vec::new(),
            scene_lines: None,
            mesh_bounds: None,
            cross_section: CrossSectionState::default(),
            cross_section_guide: None,
            label_strokes: None,
            selection_overlay: None,
            cut_preview: None,
            selection_marker: None,
            issue_markers: None,
            pick_mesh: None,
            pick_mesh_build: None,
            selection_mesh: None,
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
        self.selection_overlay = None;
        self.cut_preview = None;
        self.selection_marker = None;
        self.issue_markers = None;
        self.pick_mesh = None;
        self.pick_mesh_build = None;
        self.selection_mesh = None;

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

        self.start_pick_mesh_build();
        self.fit_camera_to_mesh();
    }

    fn ensure_selection_mesh(&mut self) {
        if self.selection_mesh.is_some() {
            return;
        }
        let Some(bounds) = self.mesh_bounds else {
            return;
        };
        let triangle_count = self
            .mesh_chunks
            .iter()
            .map(|chunk| chunk.cpu_triangles.len())
            .sum();
        if triangle_count == 0 {
            return;
        }

        let started = Instant::now();
        let mut selection_triangles = Vec::with_capacity(triangle_count);
        for chunk in &self.mesh_chunks {
            selection_triangles.extend(chunk.cpu_triangles.iter().copied().enumerate().map(
                |(local_index, triangle)| {
                    (
                        TriangleId {
                            chunk: chunk.chunk_index,
                            local_index: local_index as u32,
                        },
                        triangle,
                    )
                },
            ));
        }

        let tolerance = selection_weld_tolerance(bounds);
        let selection_mesh = SelectionMesh::from_triangles(selection_triangles, tolerance);
        tracing::info!(
            vertices = selection_mesh.mesh.vertices.len(),
            faces = selection_mesh.mesh.faces.len(),
            components = selection_mesh.mesh.connectivity.component_count,
            boundary_loops = selection_mesh.mesh.connectivity.boundary_loops.len(),
            non_manifold_edges = selection_mesh.mesh.connectivity.non_manifold_edges.len(),
            build_ms = started.elapsed().as_secs_f64() * 1000.0,
            "built CPU selection geometry on demand"
        );
        self.selection_mesh = Some(selection_mesh);
    }

    fn start_pick_mesh_build(&mut self) {
        let chunk_triangles = self
            .mesh_chunks
            .iter()
            .map(|chunk| (chunk.chunk_index, chunk.cpu_triangles.clone()))
            .collect::<Vec<_>>();
        let triangle_count = chunk_triangles
            .iter()
            .map(|(_, triangles)| triangles.len())
            .sum();
        if triangle_count == 0 {
            return;
        }

        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let started = Instant::now();
            let pick_mesh = PickMesh::from_chunk_triangles(chunk_triangles, triangle_count);
            tracing::info!(
                faces = pick_mesh.triangles.len(),
                build_ms = started.elapsed().as_secs_f64() * 1000.0,
                "built fast CPU pick mesh in background"
            );
            let _ = sender.send(pick_mesh);
        });
        self.pick_mesh_build = Some(receiver);
    }

    fn poll_pick_mesh_build(&mut self) {
        let Some(result) = self.pick_mesh_build.as_ref().map(Receiver::try_recv) else {
            return;
        };
        match result {
            Ok(pick_mesh) => {
                self.pick_mesh = Some(pick_mesh);
                self.pick_mesh_build = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.pick_mesh_build = None;
            }
        }
    }

    pub fn selection_acceleration_ready(&mut self) -> bool {
        self.poll_pick_mesh_build();
        self.pick_mesh.is_some() || self.mesh_chunks.is_empty()
    }

    fn ensure_pick_mesh(&mut self) {
        self.poll_pick_mesh_build();
        if self.pick_mesh.is_some() {
            return;
        }
        let started = Instant::now();
        let pick_mesh = if let Some(receiver) = self.pick_mesh_build.take() {
            match receiver.recv() {
                Ok(pick_mesh) => pick_mesh,
                Err(_) => {
                    let chunk_triangles = self
                        .mesh_chunks
                        .iter()
                        .map(|chunk| (chunk.chunk_index, chunk.cpu_triangles.clone()))
                        .collect::<Vec<_>>();
                    let triangle_count = chunk_triangles
                        .iter()
                        .map(|(_, triangles)| triangles.len())
                        .sum();
                    PickMesh::from_chunk_triangles(chunk_triangles, triangle_count)
                }
            }
        } else {
            let chunk_triangles = self
                .mesh_chunks
                .iter()
                .map(|chunk| (chunk.chunk_index, chunk.cpu_triangles.clone()))
                .collect::<Vec<_>>();
            let triangle_count = chunk_triangles
                .iter()
                .map(|(_, triangles)| triangles.len())
                .sum();
            PickMesh::from_chunk_triangles(chunk_triangles, triangle_count)
        };
        tracing::info!(
            faces = pick_mesh.triangles.len(),
            build_ms = started.elapsed().as_secs_f64() * 1000.0,
            "waited for fast CPU pick mesh"
        );
        self.pick_mesh = Some(pick_mesh);
    }

    pub fn selection_summary(&self) -> Option<SelectionSummary> {
        let mesh = self.selection_mesh.as_ref()?;
        Some(SelectionSummary {
            vertex_count: mesh.mesh.vertices.len(),
            face_count: mesh.mesh.faces.len(),
            component_count: mesh.mesh.connectivity.component_count,
            boundary_loop_count: mesh.mesh.connectivity.boundary_loops.len(),
            non_manifold_edge_count: mesh.mesh.connectivity.non_manifold_edges.len(),
        })
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
            if let Some(selection) = &self.selection_overlay {
                pass.set_bind_group(0, &self.camera_bind_group, &[]);
                pass.set_vertex_buffer(0, selection.buffer.slice(..));
                if selection.face_vertex_count > 0 {
                    pass.set_pipeline(&self.selection_face_pipeline);
                    pass.draw(0..selection.face_vertex_count, 0..1);
                }
                if selection.line_vertex_count > 0 {
                    pass.set_pipeline(&self.grid_pipeline);
                    pass.draw(
                        selection.face_vertex_count
                            ..selection.face_vertex_count + selection.line_vertex_count,
                        0..1,
                    );
                }
            }
            if let Some(preview) = &self.cut_preview {
                pass.set_pipeline(&self.grid_pipeline);
                pass.set_bind_group(0, &self.camera_bind_group, &[]);
                pass.set_vertex_buffer(0, preview.buffer.slice(..));
                pass.draw(0..preview.grid_vertex_count, 0..1);
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
            return Ok(None);
        };
        let Some(triangle) = self.triangle(triangle_id) else {
            return Ok(None);
        };
        let ray = self.pick_ray(screen_position);
        let position = intersect_triangle(ray, triangle).unwrap_or_else(|| {
            (triangle.vertices[0] + triangle.vertices[1] + triangle.vertices[2]) / 3.0
        });

        Ok(Some(PickResult {
            triangle_id,
            position,
        }))
    }

    pub fn pick_hit_stack(
        &mut self,
        screen_position: Vec2,
    ) -> Result<Vec<PickResult>, RenderError> {
        self.pick_selection_hits(screen_position, true)
    }

    pub fn pick_selection_hits(
        &mut self,
        screen_position: Vec2,
        through: bool,
    ) -> Result<Vec<PickResult>, RenderError> {
        self.ensure_pick_mesh();
        let ray = self.pick_ray(screen_position);
        let clip_plane = self
            .cross_section
            .enabled
            .then(|| self.cross_section.plane());
        let Some(pick_mesh) = &self.pick_mesh else {
            return Ok(Vec::new());
        };
        Ok(pick_mesh
            .hits(ray, clip_plane, through)
            .into_iter()
            .map(|hit| PickResult {
                triangle_id: hit.triangle_id,
                position: hit.position,
            })
            .collect())
    }

    pub fn surface_brush_region(
        &mut self,
        seed: PickResult,
        radius: f32,
        max_faces: usize,
    ) -> Vec<PickResult> {
        self.ensure_selection_mesh();
        self.selection_mesh
            .as_ref()
            .map(|selection_mesh| {
                selection_mesh
                    .surface_faces_within_radius(seed.triangle_id, seed.position, radius, max_faces)
                    .into_iter()
                    .map(|face| PickResult {
                        triangle_id: face.source_id,
                        position: face.center,
                    })
                    .collect()
            })
            .unwrap_or_default()
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

    pub fn set_selection_overlay(&mut self, overlay: &SelectionOverlay) {
        self.selection_overlay = (!overlay.is_empty()).then(|| {
            SelectionSceneOverlay::new(&self.device, overlay, self.marker_radius() * 0.55)
        });
    }

    pub fn clear_cut_preview(&mut self) {
        self.cut_preview = None;
    }

    pub fn set_cut_preview_segments(&mut self, segments: &[[Vec3; 2]]) {
        self.cut_preview = (!segments.is_empty()).then(|| {
            SceneLines::emphasized_segments(
                &self.device,
                segments,
                self.marker_radius() * 0.045,
                [1.0, 0.56, 0.08, 1.0],
            )
        });
    }

    pub fn view_line_cut_plane(&self, start: Vec2, end: Vec2) -> Option<CutPlane> {
        if start.distance_squared(end) < 16.0 {
            return None;
        }
        let start_ray = self.pick_ray(start);
        let end_ray = self.pick_ray(end);
        cut_plane_from_view_rays(self.camera.eye(), start_ray.direction, end_ray.direction)
    }

    pub fn cut_preview(&self, plane: CutPlane) -> CutPreview {
        let epsilon = self
            .mesh_bounds
            .map(|bounds| bounds.radius().max(1.0) * 1.0e-6)
            .unwrap_or(1.0e-6);
        let mut segments = Vec::new();
        let mut affected_triangle_count = 0;
        for chunk in &self.mesh_chunks {
            let preview = preview_cut(&chunk.cpu_triangles, plane, epsilon);
            affected_triangle_count += preview.affected_triangle_count;
            segments.extend(preview.segments);
        }
        CutPreview {
            segments,
            affected_triangle_count,
        }
    }

    pub fn all_triangles(&self) -> Vec<Triangle> {
        self.mesh_chunks
            .iter()
            .flat_map(|chunk| chunk.cpu_triangles.iter().copied())
            .collect()
    }

    pub fn triangle_global_index(&self, triangle_id: TriangleId) -> Option<usize> {
        let chunk = self
            .mesh_chunks
            .iter()
            .find(|chunk| chunk.chunk_index == triangle_id.chunk)?;
        let local = triangle_id.local_index as usize;
        (local < chunk.cpu_triangles.len()).then_some(chunk.start_triangle as usize + local)
    }

    pub fn triangle_vertices(&self, triangle_id: TriangleId) -> Option<[Vec3; 3]> {
        self.triangle(triangle_id).map(|triangle| triangle.vertices)
    }

    pub fn world_to_screen(&self, point: Vec3) -> Option<Vec2> {
        if self.size.width == 0 || self.size.height == 0 {
            return None;
        }
        let clip = self.camera.view_projection(self.aspect()) * point.extend(1.0);
        if clip.w.abs() <= f32::EPSILON {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        Some(Vec2::new(
            (ndc.x + 1.0) * 0.5 * self.size.width as f32,
            (1.0 - ndc.y) * 0.5 * self.size.height as f32,
        ))
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
        if let Some(selection) = &self.selection_overlay {
            pass.set_bind_group(0, &self.camera_bind_group, &[]);
            pass.set_vertex_buffer(0, selection.buffer.slice(..));
            if selection.face_vertex_count > 0 {
                pass.set_pipeline(&self.selection_face_pipeline);
                pass.draw(0..selection.face_vertex_count, 0..1);
            }
            if selection.line_vertex_count > 0 {
                pass.set_pipeline(&self.grid_pipeline);
                pass.draw(
                    selection.face_vertex_count
                        ..selection.face_vertex_count + selection.line_vertex_count,
                    0..1,
                );
            }
        }
        if let Some(preview) = &self.cut_preview {
            pass.set_pipeline(&self.grid_pipeline);
            pass.set_bind_group(0, &self.camera_bind_group, &[]);
            pass.set_vertex_buffer(0, preview.buffer.slice(..));
            pass.draw(0..preview.grid_vertex_count, 0..1);
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
            if stroke.points.len() <= 64 {
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
            }

            let radius = stroke.radius.max(0.001);
            let ring_step = (stroke.points.len() / 96).max(1);
            for (index, point) in stroke.points.iter().enumerate() {
                if index % ring_step == 0 || index + 1 == stroke.points.len() {
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

    fn emphasized_segments(
        device: &wgpu::Device,
        segments: &[[Vec3; 2]],
        radius: f32,
        color: [f32; 4],
    ) -> Self {
        let mut vertices = Vec::with_capacity(segments.len() * 10);
        for [start, end] in segments {
            Self::push_emphasized_segment(&mut vertices, *start, *end, radius, color);
        }
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("MeshMend emphasized segment overlay buffer"),
            contents: bytemuck::cast_slice(vertices.as_slice()),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Self {
            buffer,
            grid_vertex_count: vertices.len() as u32,
            axes_vertex_count: 0,
        }
    }

    fn push_segment(vertices: &mut Vec<LineVertex>, start: Vec3, end: Vec3, color: [f32; 4]) {
        vertices.push(LineVertex {
            position: start.to_array(),
            color,
        });
        vertices.push(LineVertex {
            position: end.to_array(),
            color,
        });
    }

    fn push_emphasized_segment(
        vertices: &mut Vec<LineVertex>,
        start: Vec3,
        end: Vec3,
        radius: f32,
        color: [f32; 4],
    ) {
        Self::push_segment(vertices, start, end, color);
        let direction = (end - start).normalize_or_zero();
        if direction.length_squared() <= f32::EPSILON || radius <= 0.0 {
            return;
        }
        let reference = if direction.y.abs() < 0.9 {
            Vec3::Y
        } else {
            Vec3::X
        };
        let side = direction.cross(reference).normalize_or_zero() * radius;
        let up = direction.cross(side).normalize_or_zero() * radius;
        for offset in [side, -side, up, -up] {
            Self::push_segment(vertices, start + offset, end + offset, color);
        }
    }

    fn push_triangle(vertices: &mut Vec<LineVertex>, face: [Vec3; 3], color: [f32; 4]) {
        for position in face {
            vertices.push(LineVertex {
                position: position.to_array(),
                color,
            });
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

struct SelectionSceneOverlay {
    buffer: wgpu::Buffer,
    face_vertex_count: u32,
    line_vertex_count: u32,
}

impl SelectionSceneOverlay {
    fn new(device: &wgpu::Device, overlay: &SelectionOverlay, marker_radius: f32) -> Self {
        let face_fill = [1.0, 0.48, 0.02, 0.46];
        let edge_color = [1.0, 0.68, 0.18, 1.0];
        let vertex_color = [1.0, 0.72, 0.20, 1.0];
        let mut vertices = Vec::with_capacity(
            overlay.faces.len() * 3 + overlay.edges.len() * 10 + overlay.vertices.len() * 96,
        );

        for face in &overlay.faces {
            SceneLines::push_triangle(&mut vertices, *face, face_fill);
        }
        let face_vertex_count = vertices.len() as u32;

        for [start, end] in &overlay.edges {
            SceneLines::push_emphasized_segment(
                &mut vertices,
                *start,
                *end,
                marker_radius * 0.055,
                edge_color,
            );
        }
        for position in &overlay.vertices {
            let radius = marker_radius * 1.35;
            SceneLines::push_ring(
                &mut vertices,
                *position,
                radius,
                Vec3::X,
                Vec3::Y,
                vertex_color,
            );
            SceneLines::push_ring(
                &mut vertices,
                *position,
                radius,
                Vec3::X,
                Vec3::Z,
                vertex_color,
            );
            SceneLines::push_ring(
                &mut vertices,
                *position,
                radius,
                Vec3::Y,
                Vec3::Z,
                vertex_color,
            );
        }

        let line_vertex_count = vertices.len() as u32 - face_vertex_count;
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("MeshMend selection overlay buffer"),
            contents: bytemuck::cast_slice(vertices.as_slice()),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Self {
            buffer,
            face_vertex_count,
            line_vertex_count,
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

fn create_selection_face_pipeline(
    device: &wgpu::Device,
    surface_format: wgpu::TextureFormat,
    camera_bind_group_layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("MeshMend selection face shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shaders/grid.wgsl").into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("MeshMend selection face pipeline layout"),
        bind_group_layouts: &[camera_bind_group_layout],
        push_constant_ranges: &[],
    });

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("MeshMend selection face pipeline"),
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

#[derive(Debug, Clone)]
struct PickMesh {
    triangles: Vec<PickTriangle>,
    bvh: PickBvh,
}

impl PickMesh {
    fn from_chunk_triangles(
        chunk_triangles: Vec<(u32, Vec<Triangle>)>,
        triangle_count: usize,
    ) -> Self {
        let mut triangles = Vec::with_capacity(triangle_count);
        for (chunk_index, chunk_triangles) in chunk_triangles {
            triangles.extend(chunk_triangles.into_iter().enumerate().map(
                |(local_index, triangle)| {
                    PickTriangle::new(
                        TriangleId {
                            chunk: chunk_index,
                            local_index: local_index as u32,
                        },
                        triangle,
                    )
                },
            ));
        }
        let bvh = PickBvh::build(&triangles);
        Self { triangles, bvh }
    }

    fn hits(&self, ray: Ray, clip_plane: Option<CrossSectionPlane>, through: bool) -> Vec<PickHit> {
        if through {
            let mut hits = Vec::new();
            self.bvh.traverse(ray, |triangle_index| {
                if let Some(hit) = self.intersect(triangle_index, ray, clip_plane) {
                    hits.push(hit);
                }
            });
            hits.sort_by(|left, right| {
                left.distance
                    .partial_cmp(&right.distance)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            hits
        } else {
            self.nearest_hit(ray, clip_plane).into_iter().collect()
        }
    }

    fn nearest_hit(&self, ray: Ray, clip_plane: Option<CrossSectionPlane>) -> Option<PickHit> {
        let mut nearest = None::<PickHit>;
        if self.bvh.nodes.is_empty() {
            return None;
        }

        let mut stack = vec![0_u32];
        while let Some(node_index) = stack.pop() {
            let node = self.bvh.nodes[node_index as usize];
            let Some(node_distance) = node.bounds.intersect_ray(ray) else {
                continue;
            };
            if nearest
                .as_ref()
                .map(|hit| node_distance > hit.distance)
                .unwrap_or(false)
            {
                continue;
            }

            if node.is_leaf() {
                let start = node.first as usize;
                let end = start + node.count as usize;
                for &triangle_index in &self.bvh.triangle_indices[start..end] {
                    let Some(hit) = self.intersect(triangle_index, ray, clip_plane) else {
                        continue;
                    };
                    if nearest
                        .as_ref()
                        .map(|current| hit.distance < current.distance)
                        .unwrap_or(true)
                    {
                        nearest = Some(hit);
                    }
                }
            } else {
                let left_distance = self.bvh.nodes[node.left as usize].bounds.intersect_ray(ray);
                let right_distance = self.bvh.nodes[node.right as usize]
                    .bounds
                    .intersect_ray(ray);
                match (left_distance, right_distance) {
                    (Some(left), Some(right)) if left < right => {
                        stack.push(node.right);
                        stack.push(node.left);
                    }
                    (Some(_), Some(_)) => {
                        stack.push(node.left);
                        stack.push(node.right);
                    }
                    (Some(_), None) => stack.push(node.left),
                    (None, Some(_)) => stack.push(node.right),
                    (None, None) => {}
                }
            }
        }
        nearest
    }

    fn intersect(
        &self,
        triangle_index: u32,
        ray: Ray,
        clip_plane: Option<CrossSectionPlane>,
    ) -> Option<PickHit> {
        let triangle = self.triangles[triangle_index as usize];
        let position = intersect_triangle(ray, triangle.triangle)?;
        if let Some(plane) = clip_plane {
            if !plane.keeps_point(position) {
                return None;
            }
        }
        Some(PickHit {
            triangle_id: triangle.id,
            distance: position.distance(ray.origin),
            position,
        })
    }
}

#[derive(Debug, Clone, Copy)]
struct PickTriangle {
    id: TriangleId,
    triangle: Triangle,
    bounds: PickAabb,
    centroid: Vec3,
}

impl PickTriangle {
    fn new(id: TriangleId, triangle: Triangle) -> Self {
        let bounds = PickAabb::from_points(triangle.vertices);
        Self {
            id,
            triangle,
            bounds,
            centroid: bounds.center(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct PickHit {
    triangle_id: TriangleId,
    distance: f32,
    position: Vec3,
}

#[derive(Debug, Clone)]
struct PickBvh {
    nodes: Vec<PickBvhNode>,
    triangle_indices: Vec<u32>,
}

impl PickBvh {
    const LEAF_SIZE: usize = 8;

    fn build(triangles: &[PickTriangle]) -> Self {
        let mut bvh = Self {
            nodes: Vec::new(),
            triangle_indices: (0..triangles.len() as u32).collect(),
        };
        if !bvh.triangle_indices.is_empty() {
            bvh.build_node(triangles, 0, bvh.triangle_indices.len());
        }
        bvh
    }

    fn build_node(&mut self, triangles: &[PickTriangle], start: usize, end: usize) -> u32 {
        let node_index = self.nodes.len() as u32;
        self.nodes.push(PickBvhNode::empty());
        let bounds = self.bounds_for_range(triangles, start, end);
        let count = end - start;
        if count <= Self::LEAF_SIZE {
            self.nodes[node_index as usize] = PickBvhNode {
                bounds,
                first: start as u32,
                count: count as u32,
                left: u32::MAX,
                right: u32::MAX,
            };
            return node_index;
        }

        let centroid_bounds = self.centroid_bounds_for_range(triangles, start, end);
        let axis = centroid_bounds.longest_axis();
        let mid = start + count / 2;
        self.triangle_indices[start..end].select_nth_unstable_by(mid - start, |left, right| {
            triangles[*left as usize].centroid[axis]
                .partial_cmp(&triangles[*right as usize].centroid[axis])
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let left = self.build_node(triangles, start, mid);
        let right = self.build_node(triangles, mid, end);
        self.nodes[node_index as usize] = PickBvhNode {
            bounds,
            first: 0,
            count: 0,
            left,
            right,
        };
        node_index
    }

    fn bounds_for_range(&self, triangles: &[PickTriangle], start: usize, end: usize) -> PickAabb {
        let mut bounds = PickAabb::empty();
        for &triangle_index in &self.triangle_indices[start..end] {
            bounds = bounds.union(triangles[triangle_index as usize].bounds);
        }
        bounds
    }

    fn centroid_bounds_for_range(
        &self,
        triangles: &[PickTriangle],
        start: usize,
        end: usize,
    ) -> PickAabb {
        let mut bounds = PickAabb::empty();
        for &triangle_index in &self.triangle_indices[start..end] {
            bounds.include_point(triangles[triangle_index as usize].centroid);
        }
        bounds
    }

    fn traverse(&self, ray: Ray, mut visit: impl FnMut(u32)) {
        if self.nodes.is_empty() {
            return;
        }
        let mut stack = vec![0_u32];
        while let Some(node_index) = stack.pop() {
            let node = self.nodes[node_index as usize];
            if node.bounds.intersect_ray(ray).is_none() {
                continue;
            }
            if node.is_leaf() {
                let start = node.first as usize;
                let end = start + node.count as usize;
                for &triangle_index in &self.triangle_indices[start..end] {
                    visit(triangle_index);
                }
            } else {
                stack.push(node.left);
                stack.push(node.right);
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct PickBvhNode {
    bounds: PickAabb,
    first: u32,
    count: u32,
    left: u32,
    right: u32,
}

impl PickBvhNode {
    fn empty() -> Self {
        Self {
            bounds: PickAabb::empty(),
            first: 0,
            count: 0,
            left: u32::MAX,
            right: u32::MAX,
        }
    }

    fn is_leaf(self) -> bool {
        self.count > 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct PickAabb {
    min: Vec3,
    max: Vec3,
}

impl PickAabb {
    const RAY_EPSILON: f32 = 1.0e-7;

    fn empty() -> Self {
        Self {
            min: Vec3::splat(f32::INFINITY),
            max: Vec3::splat(f32::NEG_INFINITY),
        }
    }

    fn from_points(points: [Vec3; 3]) -> Self {
        let mut bounds = Self::empty();
        for point in points {
            bounds.include_point(point);
        }
        bounds
    }

    fn include_point(&mut self, point: Vec3) {
        self.min = self.min.min(point);
        self.max = self.max.max(point);
    }

    fn union(mut self, other: Self) -> Self {
        self.include_point(other.min);
        self.include_point(other.max);
        self
    }

    fn center(self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    fn longest_axis(self) -> usize {
        let extent = self.max - self.min;
        if extent.x >= extent.y && extent.x >= extent.z {
            0
        } else if extent.y >= extent.z {
            1
        } else {
            2
        }
    }

    fn intersect_ray(self, ray: Ray) -> Option<f32> {
        let mut t_min = 0.0_f32;
        let mut t_max = f32::INFINITY;
        for axis in 0..3 {
            let origin = ray.origin[axis];
            let direction = ray.direction[axis];
            if direction.abs() < Self::RAY_EPSILON {
                if origin < self.min[axis] || origin > self.max[axis] {
                    return None;
                }
                continue;
            }
            let inverse = 1.0 / direction;
            let mut near = (self.min[axis] - origin) * inverse;
            let mut far = (self.max[axis] - origin) * inverse;
            if near > far {
                std::mem::swap(&mut near, &mut far);
            }
            t_min = t_min.max(near);
            t_max = t_max.min(far);
            if t_min > t_max {
                return None;
            }
        }
        Some(t_min)
    }
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

fn selection_weld_tolerance(bounds: MeshBounds) -> f32 {
    bounds.radius().max(1.0) * 1.0e-6
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

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{anyhow, Result};
use egui_wgpu::ScreenDescriptor;
use egui_winit::State as EguiWinitState;
use meshmend_analysis::AnalysisReport;
use meshmend_core::{CrossSectionAxis, CrossSectionState, MeshStats};
use meshmend_render::{DisplaySettings, LightingMode, MeshChunkUpload, RendererInfo, WgpuRenderer};
use meshmend_stl::{load_binary_stl, ParsedStl};
use winit::{
    dpi::LogicalSize,
    event::{ElementState, Event, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ControlFlow, EventLoop, EventLoopBuilder},
    keyboard::{KeyCode, ModifiersState, PhysicalKey},
    window::{Window, WindowBuilder},
};

use crate::{
    icons::{draw_icon, Icon},
    input::CameraInput,
};

const FPS_DISPLAY_INTERVAL: Duration = Duration::from_secs(1);

#[cfg(target_os = "macos")]
#[derive(Debug, Clone)]
enum AppEvent {
    Menu(muda::MenuEvent),
}

#[cfg(not(target_os = "macos"))]
#[derive(Debug, Clone)]
enum AppEvent {}

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
    OpenStl,
    Save,
    SaveAs,
    Fit,
    Reset,
    Quit,
    ShowShortcuts,
    SetView(ViewMode),
}

#[cfg(target_os = "macos")]
struct NativeAppMenu {
    _menu: muda::Menu,
    save: muda::MenuItem,
    save_as: muda::MenuItem,
}

#[cfg(target_os = "macos")]
impl NativeAppMenu {
    fn install(event_loop: &EventLoop<AppEvent>) -> Result<Self> {
        use muda::{Menu, MenuItem, PredefinedMenuItem, Submenu};

        let proxy = Arc::new(Mutex::new(event_loop.create_proxy()));
        let menu_proxy = Arc::clone(&proxy);
        muda::MenuEvent::set_event_handler(Some(move |event| {
            if let Ok(proxy) = menu_proxy.lock() {
                let _ = proxy.send_event(AppEvent::Menu(event));
            }
        }));

        let app_about = PredefinedMenuItem::about(None, None);
        let app_services = PredefinedMenuItem::services(None);
        let app_hide = PredefinedMenuItem::hide(None);
        let app_hide_others = PredefinedMenuItem::hide_others(None);
        let app_show_all = PredefinedMenuItem::show_all(None);
        let app_quit = MenuItem::with_id(
            NativeMenuCommand::Quit.id(),
            "Quit MeshMend",
            true,
            Some(accelerator("cmd+q")?),
        );
        let app_menu = Submenu::with_items(
            "MeshMend",
            true,
            &[
                &app_about,
                &PredefinedMenuItem::separator(),
                &app_services,
                &PredefinedMenuItem::separator(),
                &app_hide,
                &app_hide_others,
                &app_show_all,
                &PredefinedMenuItem::separator(),
                &app_quit,
            ],
        )?;

        let open = MenuItem::with_id(
            NativeMenuCommand::OpenStl.id(),
            "Open STL...",
            true,
            Some(accelerator("cmd+o")?),
        );
        let save = MenuItem::with_id(
            NativeMenuCommand::Save.id(),
            "Save",
            false,
            Some(accelerator("cmd+s")?),
        );
        let save_as = MenuItem::with_id(
            NativeMenuCommand::SaveAs.id(),
            "Save As / Export STL...",
            false,
            Some(accelerator("cmd+shift+s")?),
        );
        let file_menu = Submenu::with_items(
            "File",
            true,
            &[
                &open,
                &PredefinedMenuItem::separator(),
                &save,
                &save_as,
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::close_window(None),
            ],
        )?;

        let rendered = view_menu_item(NativeMenuCommand::ViewRendered, "Rendered", "1")?;
        let wireframe = view_menu_item(NativeMenuCommand::ViewWireframe, "Wireframe", "2")?;
        let surface_wire = view_menu_item(NativeMenuCommand::ViewSurfaceWire, "Surface Wire", "3")?;
        let xray_wire = view_menu_item(NativeMenuCommand::ViewXrayWire, "X-Ray Wire", "4")?;
        let transparent = view_menu_item(NativeMenuCommand::ViewTransparent, "Transparent", "5")?;
        let normals = view_menu_item(NativeMenuCommand::ViewNormals, "Normals", "n")?;
        let studio = view_menu_item(NativeMenuCommand::ViewStudio, "Studio", "6")?;
        let headlight = view_menu_item(NativeMenuCommand::ViewHeadlight, "Headlight", "7")?;
        let frame = MenuItem::with_id(
            NativeMenuCommand::Fit.id(),
            "Frame Mesh",
            true,
            Some(accelerator("f")?),
        );
        let reset = MenuItem::with_id(
            NativeMenuCommand::Reset.id(),
            "Reset View",
            true,
            Some(accelerator("home")?),
        );
        let view_menu = Submenu::with_items(
            "View",
            true,
            &[
                &rendered,
                &wireframe,
                &surface_wire,
                &xray_wire,
                &transparent,
                &normals,
                &studio,
                &headlight,
                &PredefinedMenuItem::separator(),
                &frame,
                &reset,
            ],
        )?;

        let show_shortcuts = MenuItem::with_id(
            NativeMenuCommand::ShowShortcuts.id(),
            "Show Shortcuts",
            true,
            Some(accelerator("z")?),
        );
        let shortcuts_menu = Submenu::with_items("Shortcuts", true, &[&show_shortcuts])?;

        let minimize = PredefinedMenuItem::minimize(None);
        let zoom = PredefinedMenuItem::maximize(Some("Zoom"));
        let bring_all_to_front = PredefinedMenuItem::bring_all_to_front(None);
        let window_menu = Submenu::with_items(
            "Window",
            true,
            &[
                &minimize,
                &zoom,
                &PredefinedMenuItem::separator(),
                &bring_all_to_front,
            ],
        )?;

        let menu = Menu::with_items(&[
            &app_menu,
            &file_menu,
            &view_menu,
            &shortcuts_menu,
            &window_menu,
        ])?;
        menu.init_for_nsapp();
        window_menu.set_as_windows_menu_for_nsapp();

        Ok(Self {
            _menu: menu,
            save,
            save_as,
        })
    }

    fn set_model_loaded(&self, loaded: bool) {
        self.save.set_enabled(loaded);
        self.save_as.set_enabled(loaded);
    }

    fn action_for_event(&self, event: &muda::MenuEvent) -> Option<UiAction> {
        NativeMenuCommand::from_id(event.id().as_ref()).map(NativeMenuCommand::action)
    }

    fn next_action(&self) -> Option<UiAction> {
        muda::MenuEvent::receiver()
            .try_iter()
            .find_map(|event| self.action_for_event(&event))
    }
}

#[cfg(not(target_os = "macos"))]
#[derive(Debug)]
struct NativeAppMenu;

#[cfg(not(target_os = "macos"))]
impl NativeAppMenu {
    fn install(_event_loop: &EventLoop<AppEvent>) -> Result<Self> {
        Ok(Self)
    }

    fn set_model_loaded(&self, _loaded: bool) {}

    fn next_action(&self) -> Option<UiAction> {
        None
    }
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NativeMenuCommand {
    OpenStl,
    Save,
    SaveAs,
    Fit,
    Reset,
    Quit,
    ShowShortcuts,
    ViewRendered,
    ViewWireframe,
    ViewSurfaceWire,
    ViewXrayWire,
    ViewTransparent,
    ViewNormals,
    ViewStudio,
    ViewHeadlight,
}

#[cfg(target_os = "macos")]
impl NativeMenuCommand {
    fn id(self) -> &'static str {
        match self {
            Self::OpenStl => "meshmend.open-stl",
            Self::Save => "meshmend.save",
            Self::SaveAs => "meshmend.save-as",
            Self::Fit => "meshmend.fit",
            Self::Reset => "meshmend.reset",
            Self::Quit => "meshmend.quit",
            Self::ShowShortcuts => "meshmend.show-shortcuts",
            Self::ViewRendered => "meshmend.view.rendered",
            Self::ViewWireframe => "meshmend.view.wireframe",
            Self::ViewSurfaceWire => "meshmend.view.surface-wire",
            Self::ViewXrayWire => "meshmend.view.xray-wire",
            Self::ViewTransparent => "meshmend.view.transparent",
            Self::ViewNormals => "meshmend.view.normals",
            Self::ViewStudio => "meshmend.view.studio",
            Self::ViewHeadlight => "meshmend.view.headlight",
        }
    }

    fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "meshmend.open-stl" => Self::OpenStl,
            "meshmend.save" => Self::Save,
            "meshmend.save-as" => Self::SaveAs,
            "meshmend.fit" => Self::Fit,
            "meshmend.reset" => Self::Reset,
            "meshmend.quit" => Self::Quit,
            "meshmend.show-shortcuts" => Self::ShowShortcuts,
            "meshmend.view.rendered" => Self::ViewRendered,
            "meshmend.view.wireframe" => Self::ViewWireframe,
            "meshmend.view.surface-wire" => Self::ViewSurfaceWire,
            "meshmend.view.xray-wire" => Self::ViewXrayWire,
            "meshmend.view.transparent" => Self::ViewTransparent,
            "meshmend.view.normals" => Self::ViewNormals,
            "meshmend.view.studio" => Self::ViewStudio,
            "meshmend.view.headlight" => Self::ViewHeadlight,
            _ => return None,
        })
    }

    fn action(self) -> UiAction {
        match self {
            Self::OpenStl => UiAction::OpenStl,
            Self::Save => UiAction::Save,
            Self::SaveAs => UiAction::SaveAs,
            Self::Fit => UiAction::Fit,
            Self::Reset => UiAction::Reset,
            Self::Quit => UiAction::Quit,
            Self::ShowShortcuts => UiAction::ShowShortcuts,
            Self::ViewRendered => UiAction::SetView(ViewMode::Rendered),
            Self::ViewWireframe => UiAction::SetView(ViewMode::Wireframe),
            Self::ViewSurfaceWire => UiAction::SetView(ViewMode::SurfaceWire),
            Self::ViewXrayWire => UiAction::SetView(ViewMode::XrayWire),
            Self::ViewTransparent => UiAction::SetView(ViewMode::Transparent),
            Self::ViewNormals => UiAction::SetView(ViewMode::Normals),
            Self::ViewStudio => UiAction::SetView(ViewMode::Studio),
            Self::ViewHeadlight => UiAction::SetView(ViewMode::Headlight),
        }
    }
}

#[cfg(target_os = "macos")]
fn accelerator(input: &str) -> Result<muda::accelerator::Accelerator> {
    input
        .parse()
        .map_err(|err| anyhow!("invalid menu accelerator {input}: {err}"))
}

#[cfg(target_os = "macos")]
fn view_menu_item(
    command: NativeMenuCommand,
    label: &str,
    accelerator_text: &str,
) -> Result<muda::MenuItem> {
    Ok(muda::MenuItem::with_id(
        command.id(),
        label,
        true,
        Some(accelerator(accelerator_text)?),
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    Rendered,
    Wireframe,
    SurfaceWire,
    XrayWire,
    Transparent,
    Normals,
    Studio,
    Headlight,
}

impl ViewMode {
    const ALL: [Self; 8] = [
        Self::Rendered,
        Self::Wireframe,
        Self::SurfaceWire,
        Self::XrayWire,
        Self::Transparent,
        Self::Normals,
        Self::Studio,
        Self::Headlight,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Rendered => "Rendered",
            Self::Wireframe => "Wireframe",
            Self::SurfaceWire => "Surface Wire",
            Self::XrayWire => "X-Ray Wire",
            Self::Transparent => "Transparent",
            Self::Normals => "Normals",
            Self::Studio => "Studio",
            Self::Headlight => "Headlight",
        }
    }

    fn shortcut(self) -> &'static str {
        match self {
            Self::Rendered => "1",
            Self::Wireframe => "2",
            Self::SurfaceWire => "3",
            Self::XrayWire => "4",
            Self::Transparent => "5",
            Self::Normals => "N",
            Self::Studio => "6",
            Self::Headlight => "7",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Self::Rendered => Icon::Rendered,
            Self::Wireframe => Icon::Wireframe,
            Self::SurfaceWire => Icon::SurfaceWire,
            Self::XrayWire => Icon::XrayWire,
            Self::Transparent => Icon::Transparent,
            Self::Normals => Icon::Normals,
            Self::Studio => Icon::Studio,
            Self::Headlight => Icon::Headlight,
        }
    }

    fn tooltip(self) -> String {
        format!("{} ({})", self.label(), self.shortcut())
    }
}

#[derive(Debug, Clone)]
struct FrameMeter {
    last_frame: Instant,
    last_display_update: Instant,
    smoothed_frame_ms: f64,
    smoothed_fps: f64,
    display_frame_ms: f64,
    display_fps: f64,
}

impl Default for FrameMeter {
    fn default() -> Self {
        let now = Instant::now();
        Self {
            last_frame: now,
            last_display_update: now - FPS_DISPLAY_INTERVAL,
            smoothed_frame_ms: 0.0,
            smoothed_fps: 0.0,
            display_frame_ms: 0.0,
            display_fps: 0.0,
        }
    }
}

impl FrameMeter {
    fn tick(&mut self) {
        self.tick_at(Instant::now());
    }

    fn tick_at(&mut self, now: Instant) {
        let frame_ms = (now - self.last_frame).as_secs_f64() * 1000.0;
        self.last_frame = now;
        if frame_ms <= 0.0 {
            return;
        }
        let fps = 1000.0 / frame_ms;
        if self.smoothed_frame_ms == 0.0 {
            self.smoothed_frame_ms = frame_ms;
            self.smoothed_fps = fps;
        } else {
            self.smoothed_frame_ms = self.smoothed_frame_ms * 0.9 + frame_ms * 0.1;
            self.smoothed_fps = self.smoothed_fps * 0.9 + fps * 0.1;
        }

        if now.duration_since(self.last_display_update) >= FPS_DISPLAY_INTERVAL {
            self.display_frame_ms = self.smoothed_frame_ms;
            self.display_fps = self.smoothed_fps;
            self.last_display_update = now;
        }
    }

    fn display_fps(&self) -> f64 {
        self.display_fps
    }

    fn display_frame_ms(&self) -> f64 {
        self.display_frame_ms
    }
}

#[derive(Debug, Default, Clone)]
struct UiInputRegions {
    blocked: Vec<egui::Rect>,
}

impl UiInputRegions {
    fn add(&mut self, rect: egui::Rect) {
        self.blocked.push(rect.expand(2.0));
    }

    fn blocks_physical_position(&self, position: glam::Vec2, scale_factor: f32) -> bool {
        let scale_factor = scale_factor.max(0.1);
        let point = egui::pos2(position.x / scale_factor, position.y / scale_factor);
        self.blocked.iter().any(|rect| rect.contains(point))
    }

    fn allows_physical_position(&self, position: Option<glam::Vec2>, scale_factor: f32) -> bool {
        position
            .map(|position| !self.blocks_physical_position(position, scale_factor))
            .unwrap_or(true)
    }
}

#[derive(Debug, Default)]
struct UiInputState {
    regions: UiInputRegions,
    cursor_position: Option<glam::Vec2>,
}

impl UiInputState {
    fn allows_pointer_action(&self, scale_factor: f32) -> bool {
        self.regions
            .allows_physical_position(self.cursor_position, scale_factor)
    }

    fn set_cursor_position(&mut self, x: f64, y: f64) {
        self.cursor_position = Some(glam::Vec2::new(x as f32, y as f32));
    }

    fn set_regions(&mut self, regions: UiInputRegions) {
        self.regions = regions;
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum CameraDrag {
    Orbit(glam::Vec2),
    Pan(glam::Vec2),
}

fn camera_drag_for_button(
    button: MouseButton,
    delta: glam::Vec2,
    modifiers: ModifiersState,
) -> Option<CameraDrag> {
    match button {
        MouseButton::Left if modifiers.shift_key() => Some(CameraDrag::Pan(delta)),
        MouseButton::Left => Some(CameraDrag::Orbit(delta)),
        MouseButton::Right | MouseButton::Middle => Some(CameraDrag::Pan(delta)),
        _ => None,
    }
}

fn wheel_delta_from_event(delta: MouseScrollDelta) -> f32 {
    match delta {
        MouseScrollDelta::LineDelta(_, y) => y,
        MouseScrollDelta::PixelDelta(position) => position.y as f32 * 0.02,
    }
}

fn apply_camera_drag(renderer: &mut WgpuRenderer<'_>, drag: CameraDrag, needs_redraw: &mut bool) {
    let mut camera = renderer.camera();
    match drag {
        CameraDrag::Orbit(delta) => camera.orbit(delta),
        CameraDrag::Pan(delta) => camera.pan(delta, renderer.size().height as f32),
    }
    renderer.set_camera(camera);
    *needs_redraw = true;
}

fn apply_camera_zoom(renderer: &mut WgpuRenderer<'_>, wheel_delta: f32, needs_redraw: &mut bool) {
    let mut camera = renderer.camera();
    camera.zoom(wheel_delta, renderer.mesh_bounds());
    renderer.set_camera(camera);
    *needs_redraw = true;
}

pub fn run_native(
    initial_file: Option<PathBuf>,
    smoke_window: bool,
    smoke_pick_center: bool,
) -> Result<()> {
    let event_loop = EventLoopBuilder::<AppEvent>::with_user_event().build()?;
    let native_menu = NativeAppMenu::install(&event_loop)?;
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
    native_menu.set_model_loaded(model_info.is_some());
    let mut status = model_info
        .as_ref()
        .map(|model| format!("Loaded {}", model.file_name))
        .unwrap_or_else(|| "Ready".to_string());
    let mut view_mode = ViewMode::Rendered;
    let mut show_shortcuts = false;
    let mut show_view_switcher = false;
    let mut frame_meter = FrameMeter::default();

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
    let mut ui_input = UiInputState::default();
    let mut active_modifiers = ModifiersState::default();
    let mut needs_redraw = true;

    let mut display_settings = renderer.display_settings();
    apply_view_mode(view_mode, &mut display_settings);
    renderer.set_display_settings(display_settings);

    event_loop.run(move |event, target| {
        target.set_control_flow(ControlFlow::Wait);

        match event {
            #[cfg(target_os = "macos")]
            Event::UserEvent(AppEvent::Menu(menu_event)) => {
                if let Some(action) = native_menu.action_for_event(&menu_event) {
                    match action {
                        UiAction::Quit => target.exit(),
                        UiAction::None => {}
                        _ => {
                            handle_ui_action(
                                action,
                                &mut renderer,
                                redraw_window,
                                &mut model_info,
                                &mut view_mode,
                                &mut show_shortcuts,
                                &mut status,
                                &mut needs_redraw,
                            );
                            native_menu.set_model_loaded(model_info.is_some());
                        }
                    }
                }
            }
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
                        frame_meter.tick();
                        let raw_input = egui_state.take_egui_input(redraw_window);
                        let mut action = UiAction::None;
                        let mut next_display_settings = renderer.display_settings();
                        let mut next_ui_regions = UiInputRegions::default();
                        let full_output = egui_ctx.run(raw_input, |ctx| {
                            draw_ui(
                                ctx,
                                renderer.info(),
                                model_info.as_ref(),
                                view_mode,
                                &frame_meter,
                                renderer.gpu_buffer_bytes(),
                                &status,
                                &mut next_display_settings,
                                &mut show_shortcuts,
                                &mut show_view_switcher,
                                &mut action,
                                &mut next_ui_regions,
                            );
                        });
                        ui_input.set_regions(next_ui_regions);
                        egui_state
                            .handle_platform_output(redraw_window, full_output.platform_output);

                        if next_display_settings != renderer.display_settings() {
                            renderer.set_display_settings(next_display_settings);
                            needs_redraw = true;
                        }

                        match action {
                            UiAction::Quit => {
                                target.exit();
                            }
                            UiAction::None => {}
                            _ => {
                                handle_ui_action(
                                    action,
                                    &mut renderer,
                                    redraw_window,
                                    &mut model_info,
                                    &mut view_mode,
                                    &mut show_shortcuts,
                                    &mut status,
                                    &mut needs_redraw,
                                );
                                native_menu.set_model_loaded(model_info.is_some());
                            }
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
                        load_model_into_app(
                            &path,
                            &mut renderer,
                            redraw_window,
                            &mut model_info,
                            &mut status,
                        );
                        needs_redraw = true;
                        native_menu.set_model_loaded(model_info.is_some());
                    }
                    WindowEvent::MouseInput { state, button, .. } => match state {
                        ElementState::Pressed
                            if is_camera_button(button)
                                && ui_input.allows_pointer_action(window.scale_factor() as f32) =>
                        {
                            camera_input.press(button);
                        }
                        ElementState::Released => {
                            camera_input.release(button);
                        }
                        _ => {}
                    },
                    WindowEvent::CursorMoved { position, .. } => {
                        ui_input.set_cursor_position(position.x, position.y);
                        if let Some((button, delta)) =
                            camera_input.cursor_delta(position.x, position.y)
                        {
                            if let Some(drag) =
                                camera_drag_for_button(button, delta, active_modifiers)
                            {
                                apply_camera_drag(&mut renderer, drag, &mut needs_redraw);
                            }
                        }
                    }
                    WindowEvent::MouseWheel { delta, .. }
                        if ui_input.allows_pointer_action(window.scale_factor() as f32) =>
                    {
                        apply_camera_zoom(
                            &mut renderer,
                            wheel_delta_from_event(delta),
                            &mut needs_redraw,
                        );
                    }
                    WindowEvent::KeyboardInput { event, .. }
                        if event.state == ElementState::Pressed && !egui_response.consumed =>
                    {
                        if let Some(action) = shortcut_action(event.physical_key, active_modifiers)
                        {
                            match action {
                                UiAction::Quit => target.exit(),
                                UiAction::SetView(mode) => {
                                    handle_ui_action(
                                        UiAction::SetView(mode),
                                        &mut renderer,
                                        redraw_window,
                                        &mut model_info,
                                        &mut view_mode,
                                        &mut show_shortcuts,
                                        &mut status,
                                        &mut needs_redraw,
                                    );
                                    native_menu.set_model_loaded(model_info.is_some());
                                }
                                _ => {
                                    handle_ui_action(
                                        action,
                                        &mut renderer,
                                        redraw_window,
                                        &mut model_info,
                                        &mut view_mode,
                                        &mut show_shortcuts,
                                        &mut status,
                                        &mut needs_redraw,
                                    );
                                    native_menu.set_model_loaded(model_info.is_some());
                                }
                            }
                        } else if matches!(event.physical_key, PhysicalKey::Code(KeyCode::KeyZ)) {
                            show_view_switcher = true;
                            needs_redraw = true;
                        }
                    }
                    _ => {}
                }
            }
            Event::AboutToWait => {
                while let Some(action) = native_menu.next_action() {
                    match action {
                        UiAction::Quit => target.exit(),
                        UiAction::None => {}
                        _ => {
                            handle_ui_action(
                                action,
                                &mut renderer,
                                redraw_window,
                                &mut model_info,
                                &mut view_mode,
                                &mut show_shortcuts,
                                &mut status,
                                &mut needs_redraw,
                            );
                            native_menu.set_model_loaded(model_info.is_some());
                        }
                    }
                }
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
    load_model(&input, &mut renderer, window)?;
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
                    apply_view_mode(mode, &mut settings);
                    renderer.set_display_settings(settings);
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
    load_model(&input, &mut renderer, window)?;
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
                        if let Some(parent) =
                            output.parent().filter(|parent| !parent.as_os_str().is_empty())
                        {
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
    load_model(&input, &mut renderer, window)?;
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

#[allow(clippy::too_many_arguments)]
fn draw_ui(
    ctx: &egui::Context,
    renderer_info: &RendererInfo,
    model_info: Option<&ModelInfo>,
    view_mode: ViewMode,
    frame_meter: &FrameMeter,
    gpu_buffer_bytes: u64,
    status: &str,
    display_settings: &mut DisplaySettings,
    show_shortcuts: &mut bool,
    show_view_switcher: &mut bool,
    action: &mut UiAction,
    ui_regions: &mut UiInputRegions,
) {
    let top_response = egui::TopBottomPanel::top("view_toolbar").show(ctx, |ui| {
        ui.horizontal_wrapped(|ui| {
            draw_view_toolbar(ui, view_mode, display_settings, action);
            ui.separator();
            if toolbar_command_button(ui, Icon::Fit, "Frame", "F").clicked() {
                *action = UiAction::Fit;
            }
            if toolbar_command_button(ui, Icon::Reset, "Reset", "Home").clicked() {
                *action = UiAction::Reset;
            }
        });
    });
    ui_regions.add(top_response.response.rect);

    egui::CentralPanel::default()
        .frame(egui::Frame::none())
        .show(ctx, |_ui| {});

    let bottom_response = egui::TopBottomPanel::bottom("status_bar")
        .resizable(false)
        .show(ctx, |ui| {
            draw_status_bar(
                ui,
                renderer_info,
                model_info,
                view_mode,
                frame_meter,
                gpu_buffer_bytes,
                status,
            );
        });
    ui_regions.add(bottom_response.response.rect);

    if *show_shortcuts {
        if let Some(rect) = draw_shortcuts_window(ctx, show_shortcuts) {
            ui_regions.add(rect);
        }
    }
    if *show_view_switcher {
        if let Some(rect) =
            draw_view_switcher(ctx, view_mode, display_settings, show_view_switcher, action)
        {
            ui_regions.add(rect);
        }
    }
}

fn draw_view_toolbar(
    ui: &mut egui::Ui,
    view_mode: ViewMode,
    display_settings: &mut DisplaySettings,
    action: &mut UiAction,
) {
    for mode in ViewMode::ALL {
        if view_button(ui, mode, view_mode == mode).clicked() {
            *action = UiAction::SetView(mode);
            apply_view_mode(mode, display_settings);
        }
    }
}

fn view_button(ui: &mut egui::Ui, mode: ViewMode, selected: bool) -> egui::Response {
    toolbar_button(ui, mode.icon(), mode.label(), mode.shortcut(), selected)
        .on_hover_text(mode.tooltip())
}

fn toolbar_command_button(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    shortcut: &str,
) -> egui::Response {
    toolbar_button(ui, icon, label, shortcut, false)
}

fn toolbar_button(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    shortcut: &str,
    selected: bool,
) -> egui::Response {
    let size = egui::vec2(112.0, 44.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let visuals = ui.visuals();
    let fill = if selected {
        visuals.selection.bg_fill
    } else if response.hovered() {
        visuals.widgets.hovered.bg_fill
    } else {
        visuals.widgets.inactive.bg_fill
    };
    let stroke_color = if selected {
        visuals.selection.stroke.color
    } else {
        visuals.widgets.inactive.fg_stroke.color
    };
    ui.painter()
        .rect_filled(rect, egui::Rounding::same(7.0), fill);
    ui.painter().rect_stroke(
        rect,
        egui::Rounding::same(7.0),
        egui::Stroke::new(1.0, stroke_color.gamma_multiply(0.78)),
    );
    let icon_rect = egui::Rect::from_min_size(
        rect.left_center() + egui::vec2(10.0, -11.0),
        egui::vec2(22.0, 22.0),
    );
    draw_icon(ui.painter(), icon_rect, icon, stroke_color);
    ui.painter().text(
        rect.left_center() + egui::vec2(38.0, -5.0),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(12.5),
        stroke_color,
    );
    ui.painter().text(
        rect.left_center() + egui::vec2(38.0, 11.0),
        egui::Align2::LEFT_CENTER,
        shortcut,
        egui::FontId::monospace(10.5),
        stroke_color.gamma_multiply(0.68),
    );
    response
}

fn draw_status_bar(
    ui: &mut egui::Ui,
    renderer_info: &RendererInfo,
    model_info: Option<&ModelInfo>,
    view_mode: ViewMode,
    frame_meter: &FrameMeter,
    gpu_buffer_bytes: u64,
    status: &str,
) {
    ui.horizontal_wrapped(|ui| {
        if let Some(model) = model_info {
            ui.label(model.file_name.as_str());
            ui.separator();
            ui.label(format!("{} triangles", model.stats.triangle_count));
            ui.separator();
            ui.label(format!("{} chunks", model.chunk_count));
            ui.separator();
            ui.label(format!("parse {:.1} ms", model.parse_ms));
        } else {
            ui.label("No STL loaded");
        }
        ui.separator();
        ui.label(format!("View: {}", view_mode.label()));
        ui.separator();
        ui.label(format!("{:?}", renderer_info.backend));
        ui.separator();
        ui.label(format!(
            "GPU {:.1} MB",
            gpu_buffer_bytes as f64 / (1024.0 * 1024.0)
        ));
        ui.separator();
        ui.label(format!(
            "{:.1} fps / {:.1} ms",
            frame_meter.display_fps(),
            frame_meter.display_frame_ms()
        ));
        ui.separator();
        ui.label(status);
    });
}

fn draw_shortcuts_window(ctx: &egui::Context, show_shortcuts: &mut bool) -> Option<egui::Rect> {
    egui::Window::new("Shortcuts")
        .open(show_shortcuts)
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            ui.label("File");
            ui.monospace("O            Open STL");
            ui.monospace("Cmd+S        Save");
            ui.monospace("Cmd+Shift+S  Save As / Export STL");
            ui.separator();
            ui.label("View");
            ui.monospace("F            Frame mesh");
            ui.monospace("Home         Reset view");
            ui.monospace("Z            View switcher");
            ui.monospace("1-5, N, 6-7 View modes");
            ui.separator();
            ui.label("Camera");
            ui.monospace("Left drag        Orbit");
            ui.monospace("Shift+Left drag  Pan");
            ui.monospace("Middle/Right     Pan");
            ui.monospace("Wheel/trackpad   Zoom");
        })
        .map(|response| response.response.rect)
}

fn draw_view_switcher(
    ctx: &egui::Context,
    view_mode: ViewMode,
    display_settings: &mut DisplaySettings,
    show_view_switcher: &mut bool,
    action: &mut UiAction,
) -> Option<egui::Rect> {
    egui::Window::new("View")
        .open(show_view_switcher)
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                for mode in ViewMode::ALL {
                    if view_button(ui, mode, view_mode == mode).clicked() {
                        *action = UiAction::SetView(mode);
                        apply_view_mode(mode, display_settings);
                    }
                }
            });
        })
        .map(|response| response.response.rect)
}

#[allow(clippy::too_many_arguments)]
fn handle_ui_action(
    action: UiAction,
    renderer: &mut WgpuRenderer<'_>,
    window: &Window,
    model_info: &mut Option<ModelInfo>,
    view_mode: &mut ViewMode,
    show_shortcuts: &mut bool,
    status: &mut String,
    needs_redraw: &mut bool,
) {
    match action {
        UiAction::None | UiAction::Quit => {}
        UiAction::ShowShortcuts => {
            *show_shortcuts = true;
            *status = "Showing shortcuts".to_string();
            *needs_redraw = true;
        }
        UiAction::OpenStl => {
            if let Some(path) = meshmend_io::pick_stl_file() {
                load_model_into_app(&path, renderer, window, model_info, status);
                *needs_redraw = true;
            }
        }
        UiAction::Save => {
            let Some(model) = model_info.as_ref() else {
                *status = "Load an STL before saving".to_string();
                *needs_redraw = true;
                return;
            };
            match load_binary_stl(&model.path) {
                Ok(_) => *status = format!("Current STL already saved: {}", model.path.display()),
                Err(err) => *status = format!("Save validation failed: {err}"),
            }
            *needs_redraw = true;
        }
        UiAction::SaveAs => {
            let Some(model) = model_info.as_ref() else {
                *status = "Load an STL before exporting".to_string();
                *needs_redraw = true;
                return;
            };
            let default_name = model.file_name.clone();
            if let Some(path) = meshmend_io::pick_stl_to_save(&default_name) {
                match export_current_stl(&model.path, &path) {
                    Ok(()) => {
                        load_model_into_app(&path, renderer, window, model_info, status);
                        *status = format!("Exported {}", path.display());
                    }
                    Err(err) => *status = format!("Export failed: {err}"),
                }
                *needs_redraw = true;
            }
        }
        UiAction::Fit => {
            renderer.fit_camera_to_mesh();
            *status = "Framed mesh".to_string();
            *needs_redraw = true;
        }
        UiAction::Reset => {
            reset_camera(renderer);
            *status = "Reset view".to_string();
            *needs_redraw = true;
        }
        UiAction::SetView(mode) => {
            *view_mode = mode;
            let mut settings = renderer.display_settings();
            apply_view_mode(mode, &mut settings);
            renderer.set_display_settings(settings);
            *status = format!("View: {}", mode.label());
            *needs_redraw = true;
        }
    }
}

fn shortcut_action(key: PhysicalKey, modifiers: ModifiersState) -> Option<UiAction> {
    match key {
        PhysicalKey::Code(KeyCode::KeyO) => Some(UiAction::OpenStl),
        PhysicalKey::Code(KeyCode::KeyS) if modifiers.super_key() && modifiers.shift_key() => {
            Some(UiAction::SaveAs)
        }
        PhysicalKey::Code(KeyCode::KeyS) if modifiers.super_key() => Some(UiAction::Save),
        PhysicalKey::Code(KeyCode::KeyF) => Some(UiAction::Fit),
        PhysicalKey::Code(KeyCode::Home) => Some(UiAction::Reset),
        PhysicalKey::Code(KeyCode::Digit1) => Some(UiAction::SetView(ViewMode::Rendered)),
        PhysicalKey::Code(KeyCode::Digit2) => Some(UiAction::SetView(ViewMode::Wireframe)),
        PhysicalKey::Code(KeyCode::Digit3) => Some(UiAction::SetView(ViewMode::SurfaceWire)),
        PhysicalKey::Code(KeyCode::Digit4) => Some(UiAction::SetView(ViewMode::XrayWire)),
        PhysicalKey::Code(KeyCode::Digit5) => Some(UiAction::SetView(ViewMode::Transparent)),
        PhysicalKey::Code(KeyCode::Digit6) => Some(UiAction::SetView(ViewMode::Studio)),
        PhysicalKey::Code(KeyCode::Digit7) => Some(UiAction::SetView(ViewMode::Headlight)),
        PhysicalKey::Code(KeyCode::KeyN) => Some(UiAction::SetView(ViewMode::Normals)),
        _ => None,
    }
}

fn load_model_into_app(
    path: &Path,
    renderer: &mut WgpuRenderer<'_>,
    window: &Window,
    model_info: &mut Option<ModelInfo>,
    status: &mut String,
) {
    match load_model(path, renderer, window) {
        Ok(info) => {
            *status = format!("Loaded {}", info.file_name);
            *model_info = Some(info);
        }
        Err(err) => {
            *status = format!("Load failed: {err}");
            tracing::error!(error = %err, "failed to load STL");
        }
    }
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

pub fn analyze_parsed_stl(parsed: &ParsedStl) -> AnalysisReport {
    meshmend_analysis::analyze_triangles(
        parsed.chunks.iter().flat_map(|chunk| {
            chunk
                .triangles
                .iter()
                .copied()
                .enumerate()
                .map(move |(local_index, triangle)| {
                    (
                        meshmend_core::TriangleId {
                            chunk: chunk.chunk_index,
                            local_index: local_index as u32,
                        },
                        triangle,
                    )
                })
        }),
        parsed.stats.clone(),
        parsed.stats.bounds.radius().max(1.0) * 1.0e-6,
    )
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

fn export_current_stl(source: &Path, output: &Path) -> Result<()> {
    if same_file_path(source, output) {
        load_binary_stl(source)?;
        return Ok(());
    }
    if let Some(parent) = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    fs::copy(source, output)?;
    load_binary_stl(output)?;
    Ok(())
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

fn apply_view_mode(mode: ViewMode, display_settings: &mut DisplaySettings) {
    *display_settings = DisplaySettings::default();
    match mode {
        ViewMode::Rendered => {
            display_settings.lighting_mode = LightingMode::Fixed;
        }
        ViewMode::Wireframe => {
            display_settings.wireframe = true;
            display_settings.transparent = true;
            display_settings.show_backfaces = false;
        }
        ViewMode::SurfaceWire => {
            display_settings.wireframe = true;
            display_settings.show_backfaces = true;
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
        ViewMode::Normals => {
            display_settings.normal_debug = true;
        }
        ViewMode::Studio => {
            display_settings.lighting_mode = LightingMode::Studio;
        }
        ViewMode::Headlight => {
            display_settings.lighting_mode = LightingMode::Headlight;
        }
    }
}

fn is_camera_button(button: MouseButton) -> bool {
    matches!(
        button,
        MouseButton::Left | MouseButton::Right | MouseButton::Middle
    )
}

fn same_file_path(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

#[derive(Debug, Clone, Copy)]
struct PerfFrameStats {
    idle_fps_avg: f64,
    orbit_fps_avg: f64,
    pan_fps_avg: f64,
    zoom_fps_avg: f64,
    p95_frame_ms: f64,
    p99_frame_ms: f64,
}

const PERF_FRAMES_PER_MODE: usize = 24;

fn measure_frame_stats(renderer: &mut WgpuRenderer<'_>) -> Result<PerfFrameStats> {
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

    Ok(PerfFrameStats {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn export_current_stl_copies_and_reloads_fixture() {
        let source = fixture_path("cube_binary.stl");
        let output = temp_output_path("cube-export.stl");

        export_current_stl(&source, &output).expect("export should copy and validate STL");
        let source_mesh = load_binary_stl(&source).expect("source fixture should load");
        let exported_mesh = load_binary_stl(&output).expect("exported STL should reload");
        assert_eq!(
            source_mesh.stats.triangle_count,
            exported_mesh.stats.triangle_count
        );

        let _ = fs::remove_file(output);
    }

    #[test]
    fn export_current_stl_accepts_same_path_as_already_saved() {
        let source = fixture_path("cube_binary.stl");
        export_current_stl(&source, &source).expect("same-path save should validate source");
    }

    #[test]
    fn frame_meter_throttles_display_updates() {
        let start = Instant::now();
        let mut meter = FrameMeter {
            last_frame: start,
            last_display_update: start - FPS_DISPLAY_INTERVAL,
            smoothed_frame_ms: 0.0,
            smoothed_fps: 0.0,
            display_frame_ms: 0.0,
            display_fps: 0.0,
        };

        meter.tick_at(start + Duration::from_millis(2));
        let first_display_fps = meter.display_fps();
        assert!(first_display_fps > 0.0);

        meter.tick_at(start + Duration::from_millis(4));
        assert_eq!(meter.display_fps(), first_display_fps);

        meter.tick_at(start + FPS_DISPLAY_INTERVAL + Duration::from_millis(4));
        assert_ne!(meter.display_fps(), first_display_fps);
    }

    #[test]
    fn ui_regions_block_controls_but_allow_viewport() {
        let mut regions = UiInputRegions::default();
        regions.add(egui::Rect::from_min_max(
            egui::pos2(0.0, 0.0),
            egui::pos2(100.0, 50.0),
        ));

        assert!(regions.blocks_physical_position(glam::Vec2::new(50.0, 25.0), 1.0));
        assert!(!regions.blocks_physical_position(glam::Vec2::new(50.0, 80.0), 1.0));
        assert!(regions.allows_physical_position(Some(glam::Vec2::new(50.0, 160.0)), 2.0));
    }

    #[test]
    fn camera_drag_mapping_keeps_shift_left_as_pan() {
        let delta = glam::Vec2::new(4.0, -3.0);
        let mut shift = ModifiersState::empty();
        shift.set(ModifiersState::SHIFT, true);

        assert_eq!(
            camera_drag_for_button(MouseButton::Left, delta, ModifiersState::empty()),
            Some(CameraDrag::Orbit(delta))
        );
        assert_eq!(
            camera_drag_for_button(MouseButton::Left, delta, shift),
            Some(CameraDrag::Pan(delta))
        );
        assert_eq!(
            camera_drag_for_button(MouseButton::Right, delta, ModifiersState::empty()),
            Some(CameraDrag::Pan(delta))
        );
    }

    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/stl")
            .join(name)
    }

    fn temp_output_path(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("meshmend-{stamp}-{name}"))
    }
}

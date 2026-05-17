use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{anyhow, Result};
use egui_wgpu::ScreenDescriptor;
use egui_winit::State as EguiWinitState;
use glam::{Vec2, Vec3};
use meshmend_analysis::AnalysisReport;
use meshmend_core::{CrossSectionAxis, CrossSectionState, MeshStats, TriangleId};
use meshmend_render::{
    DisplaySettings, LightingMode, MeshChunkUpload, PickResult, RendererInfo, SelectionOverlay,
    WgpuRenderer,
};
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
const SELECTION_UNDO_LIMIT: usize = 64;

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
    ClearSelection,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectionElement {
    Vertex,
    Edge,
    Face,
}

impl SelectionElement {
    const ALL: [Self; 3] = [Self::Vertex, Self::Edge, Self::Face];

    fn label(self) -> &'static str {
        match self {
            Self::Vertex => "Vertex",
            Self::Edge => "Edge",
            Self::Face => "Face",
        }
    }

    fn shortcut(self) -> &'static str {
        match self {
            Self::Vertex => "A",
            Self::Edge => "S",
            Self::Face => "D",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Self::Vertex => Icon::VertexSelect,
            Self::Edge => Icon::EdgeSelect,
            Self::Face => Icon::FaceSelect,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectionTool {
    Point,
    Brush,
    Line,
}

impl SelectionTool {
    const ALL: [Self; 3] = [Self::Point, Self::Brush, Self::Line];

    fn label(self) -> &'static str {
        match self {
            Self::Point => "Point",
            Self::Brush => "Brush",
            Self::Line => "Line",
        }
    }

    fn shortcut(self) -> &'static str {
        match self {
            Self::Point => "Q",
            Self::Brush => "W",
            Self::Line => "E",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Self::Point => Icon::PointSelect,
            Self::Brush => Icon::BrushSelect,
            Self::Line => Icon::LineSelect,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectionDepth {
    Front,
    Through,
}

impl SelectionDepth {
    fn label(self) -> &'static str {
        match self {
            Self::Front => "Front",
            Self::Through => "Through",
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Front => Self::Through,
            Self::Through => Self::Front,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct FaceKey {
    chunk: u32,
    local_index: u32,
}

impl From<TriangleId> for FaceKey {
    fn from(value: TriangleId) -> Self {
        Self {
            chunk: value.chunk,
            local_index: value.local_index,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
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

#[derive(Debug, Clone, Default, PartialEq)]
struct SelectedElements {
    vertices: BTreeMap<VertexKey, Vec3>,
    edges: BTreeMap<EdgeKey, [Vec3; 2]>,
    faces: BTreeMap<FaceKey, [Vec3; 3]>,
}

impl SelectedElements {
    fn clear(&mut self) {
        self.vertices.clear();
        self.edges.clear();
        self.faces.clear();
    }

    fn is_empty(&self) -> bool {
        self.vertices.is_empty() && self.edges.is_empty() && self.faces.is_empty()
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

    fn add_face(&mut self, triangle_id: TriangleId, vertices: [Vec3; 3]) -> bool {
        self.faces
            .insert(FaceKey::from(triangle_id), vertices)
            .is_none()
    }

    fn overlay_for(&self, element: SelectionElement) -> SelectionOverlay {
        match element {
            SelectionElement::Vertex => SelectionOverlay {
                vertices: self.vertices.values().copied().collect(),
                edges: Vec::new(),
                faces: Vec::new(),
            },
            SelectionElement::Edge => SelectionOverlay {
                vertices: Vec::new(),
                edges: self.edges.values().copied().collect(),
                faces: Vec::new(),
            },
            SelectionElement::Face => SelectionOverlay {
                vertices: Vec::new(),
                edges: Vec::new(),
                faces: self.faces.values().copied().collect(),
            },
        }
    }

    fn summary_for(&self, element: SelectionElement) -> String {
        match element {
            SelectionElement::Vertex => format!("{} vertices selected", self.vertices.len()),
            SelectionElement::Edge => format!("{} edges selected", self.edges.len()),
            SelectionElement::Face => format!("{} faces selected", self.faces.len()),
        }
    }
}

#[derive(Debug)]
struct SelectionState {
    element: SelectionElement,
    tool: SelectionTool,
    depth: SelectionDepth,
    brush_radius_px: f32,
    brush_active: bool,
    last_brush_position: Option<Vec2>,
    cursor_position: Option<Vec2>,
    line_start: Option<Vec2>,
    selected: SelectedElements,
    undo_stack: VecDeque<SelectedElements>,
    brush_undo_snapshot: Option<SelectedElements>,
}

impl Default for SelectionState {
    fn default() -> Self {
        Self {
            element: SelectionElement::Face,
            tool: SelectionTool::Point,
            depth: SelectionDepth::Front,
            brush_radius_px: 44.0,
            brush_active: false,
            last_brush_position: None,
            cursor_position: None,
            line_start: None,
            selected: SelectedElements::default(),
            undo_stack: VecDeque::new(),
            brush_undo_snapshot: None,
        }
    }
}

impl SelectionState {
    fn summary(&self) -> String {
        self.selected.summary_for(self.element)
    }

    fn push_undo_snapshot(&mut self, snapshot: SelectedElements) {
        self.undo_stack.push_back(snapshot);
        while self.undo_stack.len() > SELECTION_UNDO_LIMIT {
            self.undo_stack.pop_front();
        }
    }

    fn undo_last_selection(&mut self) -> bool {
        let Some(snapshot) = self.undo_stack.pop_back() else {
            return false;
        };
        self.selected = snapshot;
        self.cancel_active_gesture();
        true
    }

    fn cancel_active_gesture(&mut self) {
        self.line_start = None;
        self.last_brush_position = None;
        self.brush_active = false;
        self.brush_undo_snapshot = None;
    }

    fn reset_for_model_change(&mut self) {
        self.selected.clear();
        self.undo_stack.clear();
        self.cancel_active_gesture();
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectionStrokeKind {
    Point,
    Brush,
    Line,
}

fn apply_selection_at_screen(
    renderer: &mut WgpuRenderer<'_>,
    selection: &mut SelectionState,
    screen_position: Vec2,
    stroke_kind: SelectionStrokeKind,
    seen_faces: Option<&mut HashSet<TriangleId>>,
) -> Result<bool> {
    let hits = pick_results_for_depth(renderer, screen_position, selection.depth)?;
    let mut changed = false;
    let mut seen_faces = seen_faces;
    for hit in hits {
        if let Some(seen_faces) = seen_faces.as_mut() {
            if !seen_faces.insert(hit.triangle_id) {
                continue;
            }
        }
        changed |= add_hit_to_selection(renderer, selection, hit, screen_position, stroke_kind);
    }
    Ok(changed)
}

fn apply_brush_selection(
    renderer: &mut WgpuRenderer<'_>,
    selection: &mut SelectionState,
    center: Vec2,
) -> Result<bool> {
    let mut changed = false;
    let mut seen_faces = HashSet::new();
    for point in brush_sample_points(center, selection.brush_radius_px) {
        changed |= apply_selection_at_screen(
            renderer,
            selection,
            point,
            SelectionStrokeKind::Brush,
            Some(&mut seen_faces),
        )?;
    }
    if changed {
        sync_selection_overlay(renderer, selection);
    }
    Ok(changed)
}

fn apply_line_selection(
    renderer: &mut WgpuRenderer<'_>,
    selection: &mut SelectionState,
    start: Vec2,
    end: Vec2,
) -> Result<bool> {
    let mut changed = false;
    let mut seen_faces = HashSet::new();
    for point in line_sample_points(start, end) {
        changed |= apply_selection_at_screen(
            renderer,
            selection,
            point,
            SelectionStrokeKind::Line,
            Some(&mut seen_faces),
        )?;
    }
    if changed {
        sync_selection_overlay(renderer, selection);
    }
    Ok(changed)
}

fn pick_results_for_depth(
    renderer: &mut WgpuRenderer<'_>,
    screen_position: Vec2,
    depth: SelectionDepth,
) -> Result<Vec<PickResult>> {
    if !renderer.selection_acceleration_ready() {
        return Err(anyhow!("Selection acceleration is still preparing"));
    }
    renderer
        .pick_selection_hits(screen_position, depth == SelectionDepth::Through)
        .map_err(Into::into)
}

fn add_hit_to_selection(
    renderer: &WgpuRenderer<'_>,
    selection: &mut SelectionState,
    hit: PickResult,
    screen_position: Vec2,
    stroke_kind: SelectionStrokeKind,
) -> bool {
    let Some(vertices) = renderer.triangle_vertices(hit.triangle_id) else {
        return false;
    };

    match selection.element {
        SelectionElement::Face => selection.selected.add_face(hit.triangle_id, vertices),
        SelectionElement::Edge if stroke_kind == SelectionStrokeKind::Brush => {
            selection.selected.add_edge(vertices[0], vertices[1])
                | selection.selected.add_edge(vertices[1], vertices[2])
                | selection.selected.add_edge(vertices[2], vertices[0])
        }
        SelectionElement::Edge => {
            let [start, end] = nearest_screen_edge(renderer, vertices, screen_position);
            selection.selected.add_edge(start, end)
        }
        SelectionElement::Vertex if stroke_kind == SelectionStrokeKind::Brush => {
            selection.selected.add_vertex(vertices[0])
                | selection.selected.add_vertex(vertices[1])
                | selection.selected.add_vertex(vertices[2])
        }
        SelectionElement::Vertex if stroke_kind == SelectionStrokeKind::Line => {
            let [start, end] = nearest_screen_edge(renderer, vertices, screen_position);
            selection.selected.add_vertex(start) | selection.selected.add_vertex(end)
        }
        SelectionElement::Vertex => {
            let vertex = nearest_screen_vertex(renderer, vertices, screen_position);
            selection.selected.add_vertex(vertex)
        }
    }
}

fn sync_selection_overlay(renderer: &mut WgpuRenderer<'_>, selection: &SelectionState) {
    renderer.set_selection_overlay(&selection.selected.overlay_for(selection.element));
}

fn clear_selection(
    renderer: &mut WgpuRenderer<'_>,
    selection: &mut SelectionState,
    record_history: bool,
) -> bool {
    let had_selection = !selection.selected.is_empty();
    let had_gesture = selection.line_start.is_some() || selection.brush_active;

    if record_history && had_selection {
        selection.push_undo_snapshot(selection.selected.clone());
    }
    selection.selected.clear();
    selection.cancel_active_gesture();
    renderer.set_selection_overlay(&SelectionOverlay::default());
    had_selection || had_gesture
}

fn undo_selection(renderer: &mut WgpuRenderer<'_>, selection: &mut SelectionState) -> bool {
    if !selection.undo_last_selection() {
        return false;
    }
    sync_selection_overlay(renderer, selection);
    true
}

fn brush_sample_points(center: Vec2, radius: f32) -> Vec<Vec2> {
    let radius = radius.clamp(8.0, 180.0);
    let step = (radius / 4.0).clamp(6.0, 14.0);
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

fn line_sample_points(start: Vec2, end: Vec2) -> Vec<Vec2> {
    let length = (end - start).length();
    if length <= 0.5 {
        return vec![start];
    }
    let steps = (length / 6.0).ceil().clamp(1.0, 1024.0) as usize;
    (0..=steps)
        .map(|index| {
            let t = index as f32 / steps as f32;
            start.lerp(end, t)
        })
        .collect()
}

fn nearest_screen_vertex(
    renderer: &WgpuRenderer<'_>,
    vertices: [Vec3; 3],
    screen_position: Vec2,
) -> Vec3 {
    vertices
        .into_iter()
        .min_by(|a, b| {
            let da = renderer
                .world_to_screen(*a)
                .map(|point| point.distance_squared(screen_position))
                .unwrap_or(f32::MAX);
            let db = renderer
                .world_to_screen(*b)
                .map(|point| point.distance_squared(screen_position))
                .unwrap_or(f32::MAX);
            da.total_cmp(&db)
        })
        .unwrap_or(vertices[0])
}

fn nearest_screen_edge(
    renderer: &WgpuRenderer<'_>,
    vertices: [Vec3; 3],
    screen_position: Vec2,
) -> [Vec3; 2] {
    let edges = [
        [vertices[0], vertices[1]],
        [vertices[1], vertices[2]],
        [vertices[2], vertices[0]],
    ];
    edges
        .into_iter()
        .min_by(|a, b| {
            let da = screen_edge_distance_squared(renderer, *a, screen_position);
            let db = screen_edge_distance_squared(renderer, *b, screen_position);
            da.total_cmp(&db)
        })
        .unwrap_or(edges[0])
}

fn screen_edge_distance_squared(
    renderer: &WgpuRenderer<'_>,
    edge: [Vec3; 2],
    screen_position: Vec2,
) -> f32 {
    let Some(start) = renderer.world_to_screen(edge[0]) else {
        return f32::MAX;
    };
    let Some(end) = renderer.world_to_screen(edge[1]) else {
        return f32::MAX;
    };
    point_segment_distance_squared(screen_position, start, end)
}

fn point_segment_distance_squared(point: Vec2, start: Vec2, end: Vec2) -> f32 {
    let segment = end - start;
    let length_squared = segment.length_squared();
    if length_squared <= f32::EPSILON {
        return point.distance_squared(start);
    }
    let t = ((point - start).dot(segment) / length_squared).clamp(0.0, 1.0);
    point.distance_squared(start + segment * t)
}

fn handle_selection_click(
    renderer: &mut WgpuRenderer<'_>,
    selection: &mut SelectionState,
    screen_position: Vec2,
    status: &mut String,
    needs_redraw: &mut bool,
) {
    let snapshot = selection.selected.clone();
    let result = match selection.tool {
        SelectionTool::Point => apply_selection_at_screen(
            renderer,
            selection,
            screen_position,
            SelectionStrokeKind::Point,
            None,
        ),
        SelectionTool::Brush => apply_brush_selection(renderer, selection, screen_position),
        SelectionTool::Line => {
            if let Some(start) = selection.line_start.take() {
                apply_line_selection(renderer, selection, start, screen_position)
            } else {
                selection.line_start = Some(screen_position);
                *status = "Line selector start point set".to_string();
                *needs_redraw = true;
                return;
            }
        }
    };

    match result {
        Ok(true) => {
            selection.push_undo_snapshot(snapshot);
            sync_selection_overlay(renderer, selection);
            *status = selection.summary();
            *needs_redraw = true;
        }
        Ok(false) => {
            *status = "Selection hit nothing".to_string();
            *needs_redraw = true;
        }
        Err(err) => {
            *status = format!("Selection failed: {err}");
            *needs_redraw = true;
        }
    }
}

fn handle_brush_selection(
    renderer: &mut WgpuRenderer<'_>,
    selection: &mut SelectionState,
    screen_position: Vec2,
    status: &mut String,
    needs_redraw: &mut bool,
) {
    if selection
        .last_brush_position
        .map(|last| last.distance(screen_position) < 3.0)
        .unwrap_or(false)
    {
        return;
    }
    selection.last_brush_position = Some(screen_position);

    match apply_brush_selection(renderer, selection, screen_position) {
        Ok(true) => {
            if let Some(snapshot) = selection.brush_undo_snapshot.take() {
                selection.push_undo_snapshot(snapshot);
            }
            sync_selection_overlay(renderer, selection);
            *status = selection.summary();
            *needs_redraw = true;
        }
        Ok(false) => {}
        Err(err) => {
            *status = format!("Brush selection failed: {err}");
            *needs_redraw = true;
        }
    }
}

fn handle_selection_shortcut(
    key: PhysicalKey,
    modifiers: ModifiersState,
    renderer: &mut WgpuRenderer<'_>,
    selection: &mut SelectionState,
    status: &mut String,
    needs_redraw: &mut bool,
) -> bool {
    if matches!(key, PhysicalKey::Code(KeyCode::KeyZ))
        && (modifiers.control_key() || modifiers.super_key())
        && !modifiers.alt_key()
        && !modifiers.shift_key()
    {
        if undo_selection(renderer, selection) {
            *status = format!("Undid selection: {}", selection.summary());
        } else {
            *status = "No selection undo available".to_string();
        }
        *needs_redraw = true;
        return true;
    }

    if modifiers.super_key() || modifiers.control_key() || modifiers.alt_key() {
        return false;
    }

    match key {
        PhysicalKey::Code(KeyCode::KeyA) => selection.element = SelectionElement::Vertex,
        PhysicalKey::Code(KeyCode::KeyS) => selection.element = SelectionElement::Edge,
        PhysicalKey::Code(KeyCode::KeyD) => selection.element = SelectionElement::Face,
        PhysicalKey::Code(KeyCode::KeyQ) => {
            selection.tool = SelectionTool::Point;
            selection.cancel_active_gesture();
        }
        PhysicalKey::Code(KeyCode::KeyW) => {
            selection.tool = SelectionTool::Brush;
            selection.cancel_active_gesture();
        }
        PhysicalKey::Code(KeyCode::KeyE) => {
            selection.tool = SelectionTool::Line;
            selection.cancel_active_gesture();
        }
        PhysicalKey::Code(KeyCode::KeyX) => selection.depth = selection.depth.next(),
        PhysicalKey::Code(KeyCode::Escape) => {
            if clear_selection(renderer, selection, true) {
                *status = "Selection cleared".to_string();
            } else {
                *status = "Selection already clear".to_string();
            }
            *needs_redraw = true;
            return true;
        }
        PhysicalKey::Code(KeyCode::Backspace) | PhysicalKey::Code(KeyCode::Delete) => {
            if clear_selection(renderer, selection, true) {
                *status = "Selection cleared".to_string();
            } else {
                *status = "Selection already clear".to_string();
            }
            *needs_redraw = true;
            return true;
        }
        _ => return false,
    }

    *status = format!(
        "{} {} selection ({})",
        selection.depth.label(),
        selection.element.label(),
        selection.tool.label()
    );
    selection.last_brush_position = None;
    sync_selection_overlay(renderer, selection);
    *needs_redraw = true;
    true
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
    let mut selection_state = SelectionState::default();
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
                                &mut selection_state,
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
                        let previous_selection_element = selection_state.element;
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
                                &mut selection_state,
                                window.scale_factor() as f32,
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
                        if previous_selection_element != selection_state.element {
                            sync_selection_overlay(&mut renderer, &selection_state);
                            status = selection_state.summary();
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
                                    &mut selection_state,
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
                        selection_state.reset_for_model_change();
                        needs_redraw = true;
                        native_menu.set_model_loaded(model_info.is_some());
                    }
                    WindowEvent::MouseInput { state, button, .. } => match state {
                        ElementState::Pressed
                            if button == MouseButton::Left
                                && selection_state.tool == SelectionTool::Brush
                                && !active_modifiers.shift_key()
                                && ui_input.allows_pointer_action(window.scale_factor() as f32) =>
                        {
                            selection_state.brush_active = true;
                            selection_state.last_brush_position = None;
                            selection_state.brush_undo_snapshot =
                                Some(selection_state.selected.clone());
                            if let Some(position) = selection_state.cursor_position {
                                handle_brush_selection(
                                    &mut renderer,
                                    &mut selection_state,
                                    position,
                                    &mut status,
                                    &mut needs_redraw,
                                );
                            }
                        }
                        ElementState::Pressed
                            if is_camera_button(button)
                                && ui_input.allows_pointer_action(window.scale_factor() as f32) =>
                        {
                            camera_input.press(button);
                        }
                        ElementState::Released
                            if button == MouseButton::Left && selection_state.brush_active =>
                        {
                            selection_state.brush_active = false;
                            selection_state.last_brush_position = None;
                            selection_state.brush_undo_snapshot = None;
                            needs_redraw = true;
                        }
                        ElementState::Released => {
                            if let Some(click_position) = camera_input.release(button) {
                                if button == MouseButton::Left
                                    && ui_input.regions.allows_physical_position(
                                        Some(click_position),
                                        window.scale_factor() as f32,
                                    )
                                {
                                    handle_selection_click(
                                        &mut renderer,
                                        &mut selection_state,
                                        click_position,
                                        &mut status,
                                        &mut needs_redraw,
                                    );
                                }
                            }
                        }
                        _ => {}
                    },
                    WindowEvent::CursorMoved { position, .. } => {
                        ui_input.set_cursor_position(position.x, position.y);
                        let cursor = Vec2::new(position.x as f32, position.y as f32);
                        selection_state.cursor_position = Some(cursor);
                        if selection_state.brush_active {
                            let _ = camera_input.cursor_delta(position.x, position.y);
                            if ui_input.allows_pointer_action(window.scale_factor() as f32) {
                                handle_brush_selection(
                                    &mut renderer,
                                    &mut selection_state,
                                    cursor,
                                    &mut status,
                                    &mut needs_redraw,
                                );
                            }
                        } else if let Some((button, delta)) =
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
                                        &mut selection_state,
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
                                        &mut selection_state,
                                        &mut needs_redraw,
                                    );
                                    native_menu.set_model_loaded(model_info.is_some());
                                }
                            }
                        } else if handle_selection_shortcut(
                            event.physical_key,
                            active_modifiers,
                            &mut renderer,
                            &mut selection_state,
                            &mut status,
                            &mut needs_redraw,
                        ) {
                        } else if matches!(event.physical_key, PhysicalKey::Code(KeyCode::KeyZ))
                            && !active_modifiers.control_key()
                            && !active_modifiers.super_key()
                            && !active_modifiers.alt_key()
                        {
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
                                &mut selection_state,
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
    selection_state: &mut SelectionState,
    scale_factor: f32,
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

    if let Some(rect) = draw_selection_palette(ctx, selection_state, action) {
        ui_regions.add(rect);
    }
    draw_selection_cursor(ctx, selection_state, scale_factor);

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

fn draw_selection_palette(
    ctx: &egui::Context,
    selection_state: &mut SelectionState,
    action: &mut UiAction,
) -> Option<egui::Rect> {
    let response = egui::Area::new(egui::Id::new("selection_palette"))
        .anchor(egui::Align2::LEFT_TOP, egui::vec2(8.0, 50.0))
        .show(ctx, |ui| {
            egui::Frame::dark_canvas(ui.style())
                .rounding(egui::Rounding::same(7.0))
                .inner_margin(egui::Margin::symmetric(5.0, 6.0))
                .show(ui, |ui| {
                    ui.set_width(50.0);
                    palette_caption(ui, "Element");
                    for element in SelectionElement::ALL {
                        if palette_icon_button(
                            ui,
                            element.icon(),
                            &format!("{} select ({})", element.label(), element.shortcut()),
                            selection_state.element == element,
                        )
                        .clicked()
                        {
                            selection_state.element = element;
                        }
                    }

                    ui.add_space(5.0);
                    ui.separator();
                    ui.add_space(5.0);
                    palette_caption(ui, "Tool");
                    for tool in SelectionTool::ALL {
                        if palette_icon_button(
                            ui,
                            tool.icon(),
                            &format!("{} selection ({})", tool.label(), tool.shortcut()),
                            selection_state.tool == tool,
                        )
                        .clicked()
                        {
                            selection_state.tool = tool;
                            selection_state.cancel_active_gesture();
                        }
                    }

                    ui.add_space(5.0);
                    ui.separator();
                    ui.add_space(5.0);
                    let depth_label = match selection_state.depth {
                        SelectionDepth::Front => "1st",
                        SelectionDepth::Through => "All",
                    };
                    if palette_button(
                        ui,
                        depth_label,
                        "Toggle front-surface vs through selection (X)",
                        selection_state.depth == SelectionDepth::Through,
                    )
                    .clicked()
                    {
                        selection_state.depth = selection_state.depth.next();
                    }

                    if selection_state.tool == SelectionTool::Brush {
                        ui.add_space(5.0);
                        ui.horizontal(|ui| {
                            if mini_button(ui, "-").clicked() {
                                selection_state.brush_radius_px =
                                    (selection_state.brush_radius_px - 4.0).max(8.0);
                            }
                            ui.label(format!("{:.0}", selection_state.brush_radius_px));
                            if mini_button(ui, "+").clicked() {
                                selection_state.brush_radius_px =
                                    (selection_state.brush_radius_px + 4.0).min(180.0);
                            }
                        });
                    }

                    ui.add_space(5.0);
                    ui.separator();
                    ui.add_space(5.0);
                    if palette_icon_button(ui, Icon::ClearSelection, "Clear selection (Esc)", false)
                        .clicked()
                    {
                        *action = UiAction::ClearSelection;
                    }
                });
        });
    Some(response.response.rect)
}

fn palette_caption(ui: &mut egui::Ui, label: &str) {
    ui.label(
        egui::RichText::new(label)
            .size(9.5)
            .color(ui.visuals().weak_text_color()),
    );
}

fn palette_icon_button(
    ui: &mut egui::Ui,
    icon: Icon,
    tooltip: &str,
    selected: bool,
) -> egui::Response {
    let size = egui::vec2(42.0, 31.0);
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
        .rect_filled(rect, egui::Rounding::same(6.0), fill);
    ui.painter().rect_stroke(
        rect,
        egui::Rounding::same(6.0),
        egui::Stroke::new(1.0, stroke_color.gamma_multiply(0.75)),
    );
    let icon_rect = egui::Rect::from_center_size(rect.center(), egui::vec2(19.0, 19.0));
    draw_icon(ui.painter(), icon_rect, icon, stroke_color);
    response.on_hover_text(tooltip)
}

fn palette_button(ui: &mut egui::Ui, label: &str, tooltip: &str, selected: bool) -> egui::Response {
    let size = egui::vec2(42.0, 31.0);
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
        .rect_filled(rect, egui::Rounding::same(6.0), fill);
    ui.painter().rect_stroke(
        rect,
        egui::Rounding::same(6.0),
        egui::Stroke::new(1.0, stroke_color.gamma_multiply(0.75)),
    );
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(12.5),
        stroke_color,
    );
    response.on_hover_text(tooltip)
}

fn mini_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add_sized([16.0, 20.0], egui::Button::new(label))
}

fn draw_selection_cursor(ctx: &egui::Context, selection_state: &SelectionState, scale_factor: f32) {
    let scale_factor = scale_factor.max(0.1);
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("selection_cursor"),
    ));
    let cursor = selection_state
        .cursor_position
        .map(|position| egui::pos2(position.x / scale_factor, position.y / scale_factor));

    if selection_state.tool == SelectionTool::Brush {
        if let Some(cursor) = cursor {
            let radius = selection_state.brush_radius_px / scale_factor;
            painter.circle_stroke(
                cursor,
                radius,
                egui::Stroke::new(1.5, egui::Color32::from_rgb(155, 220, 255)),
            );
            painter.circle_filled(cursor, 2.0, egui::Color32::from_rgb(210, 245, 255));
        }
    }

    if selection_state.tool == SelectionTool::Line {
        if let Some(start) = selection_state.line_start {
            let start = egui::pos2(start.x / scale_factor, start.y / scale_factor);
            painter.circle_filled(start, 3.5, egui::Color32::from_rgb(210, 245, 255));
            if let Some(cursor) = cursor {
                painter.line_segment(
                    [start, cursor],
                    egui::Stroke::new(1.5, egui::Color32::from_rgb(155, 220, 255)),
                );
            }
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
    let estimated_label_width = label.chars().count() as f32 * 6.4;
    let size = egui::vec2((44.0 + estimated_label_width).clamp(72.0, 108.0), 34.0);
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
        rect.left_center() + egui::vec2(9.0, -9.0),
        egui::vec2(18.0, 18.0),
    );
    draw_icon(ui.painter(), icon_rect, icon, stroke_color);
    ui.painter().text(
        rect.left_center() + egui::vec2(33.0, 0.0),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(12.0),
        stroke_color,
    );
    response.on_hover_text(format!("{label} ({shortcut})"))
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
            ui.separator();
            ui.label("Selection");
            ui.monospace("A/S/D        Vertex / Edge / Face");
            ui.monospace("Q/W/E        Point / Brush / Line");
            ui.monospace("X            Front / Through");
            ui.monospace("Esc/Delete   Clear selection");
            ui.monospace("Cmd/Ctrl+Z   Undo selection");
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
    selection_state: &mut SelectionState,
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
                selection_state.reset_for_model_change();
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
                        selection_state.reset_for_model_change();
                        *status = format!("Exported {}", path.display());
                    }
                    Err(err) => *status = format!("Export failed: {err}"),
                }
                *needs_redraw = true;
            }
        }
        UiAction::ClearSelection => {
            if clear_selection(renderer, selection_state, true) {
                *status = "Selection cleared".to_string();
            } else {
                *status = "Selection already clear".to_string();
            }
            *needs_redraw = true;
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

    #[test]
    fn selection_edges_deduplicate_when_direction_changes() {
        let mut selected = SelectedElements::default();
        let a = Vec3::new(1.0, 2.0, 3.0);
        let b = Vec3::new(4.0, 5.0, 6.0);

        assert!(selected.add_edge(a, b));
        assert!(!selected.add_edge(b, a));
        assert_eq!(selected.edges.len(), 1);
    }

    #[test]
    fn selection_overlay_only_contains_active_element_type() {
        let mut selected = SelectedElements::default();
        let a = Vec3::new(0.0, 0.0, 0.0);
        let b = Vec3::new(1.0, 0.0, 0.0);
        let c = Vec3::new(0.0, 1.0, 0.0);

        selected.add_vertex(a);
        selected.add_edge(a, b);
        selected.add_face(
            TriangleId {
                chunk: 0,
                local_index: 0,
            },
            [a, b, c],
        );

        let face_overlay = selected.overlay_for(SelectionElement::Face);
        assert_eq!(face_overlay.faces.len(), 1);
        assert!(face_overlay.edges.is_empty());
        assert!(face_overlay.vertices.is_empty());

        let edge_overlay = selected.overlay_for(SelectionElement::Edge);
        assert_eq!(edge_overlay.edges.len(), 1);
        assert!(edge_overlay.faces.is_empty());
        assert!(edge_overlay.vertices.is_empty());

        let vertex_overlay = selected.overlay_for(SelectionElement::Vertex);
        assert_eq!(vertex_overlay.vertices.len(), 1);
        assert!(vertex_overlay.faces.is_empty());
        assert!(vertex_overlay.edges.is_empty());
    }

    #[test]
    fn selection_undo_restores_previous_snapshot() {
        let mut selection = SelectionState::default();
        let before = selection.selected.clone();
        let a = Vec3::new(0.0, 0.0, 0.0);
        let b = Vec3::new(1.0, 0.0, 0.0);

        selection.selected.add_edge(a, b);
        selection.push_undo_snapshot(before);

        assert_eq!(selection.selected.edges.len(), 1);
        assert!(selection.undo_last_selection());
        assert!(selection.selected.is_empty());
        assert!(!selection.undo_last_selection());
    }

    #[test]
    fn selection_reset_for_model_change_clears_selection_and_undo() {
        let mut selection = SelectionState::default();
        let a = Vec3::new(0.0, 0.0, 0.0);
        let b = Vec3::new(1.0, 0.0, 0.0);

        selection.selected.add_edge(a, b);
        selection.push_undo_snapshot(SelectedElements::default());
        selection.line_start = Some(Vec2::new(4.0, 8.0));
        selection.brush_active = true;
        selection.reset_for_model_change();

        assert!(selection.selected.is_empty());
        assert!(selection.undo_stack.is_empty());
        assert!(selection.line_start.is_none());
        assert!(!selection.brush_active);
    }

    #[test]
    fn brush_samples_include_center_and_stay_inside_circle() {
        let center = Vec2::new(120.0, 80.0);
        let radius = 44.0;
        let points = brush_sample_points(center, radius);

        assert!(points.contains(&center));
        assert!(points.len() > 8);
        assert!(points
            .iter()
            .all(|point| point.distance(center) <= radius + f32::EPSILON));
    }

    #[test]
    fn line_samples_include_endpoints() {
        let start = Vec2::new(10.0, 20.0);
        let end = Vec2::new(40.0, 20.0);
        let points = line_sample_points(start, end);

        assert_eq!(points.first().copied(), Some(start));
        assert_eq!(points.last().copied(), Some(end));
        assert!(points.len() > 2);
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

use glam::Vec2;
use winit::event::MouseButton;

#[derive(Debug, Default)]
pub struct CameraInput {
    active_button: Option<MouseButton>,
    last_cursor: Option<Vec2>,
    drag_distance: f32,
}

impl CameraInput {
    pub fn press(&mut self, button: MouseButton) {
        self.active_button = Some(button);
        self.drag_distance = 0.0;
    }

    pub fn release(&mut self, button: MouseButton) -> Option<Vec2> {
        if self.active_button == Some(button) {
            self.active_button = None;
            if self.drag_distance < 4.0 {
                return self.last_cursor;
            }
        }
        None
    }

    pub fn cursor_delta(&mut self, x: f64, y: f64) -> Option<(MouseButton, Vec2)> {
        let cursor = Vec2::new(x as f32, y as f32);
        let delta = self.last_cursor.map(|last| cursor - last);
        self.last_cursor = Some(cursor);
        if let Some(delta) = delta {
            if self.active_button.is_some() {
                self.drag_distance += delta.length();
            }
        }
        self.active_button
            .zip(delta)
            .filter(|(_, delta)| delta.length_squared() > 0.0)
    }

    pub fn cursor_position(&self) -> Option<Vec2> {
        self.last_cursor
    }
}

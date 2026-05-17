use glam::Vec2;
use winit::event::MouseButton;

#[derive(Debug, Default)]
pub struct CameraInput {
    active_button: Option<MouseButton>,
    last_cursor: Option<Vec2>,
}

impl CameraInput {
    pub fn press(&mut self, button: MouseButton) {
        self.active_button = Some(button);
    }

    pub fn release(&mut self, button: MouseButton) {
        if self.active_button == Some(button) {
            self.active_button = None;
        }
    }

    pub fn cursor_delta(&mut self, x: f64, y: f64) -> Option<(MouseButton, Vec2)> {
        let cursor = Vec2::new(x as f32, y as f32);
        let delta = self.last_cursor.map(|last| cursor - last);
        self.last_cursor = Some(cursor);
        self.active_button
            .zip(delta)
            .filter(|(_, delta)| delta.length_squared() > 0.0)
    }
}

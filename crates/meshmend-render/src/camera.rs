use glam::{Mat4, Vec2, Vec3};
use meshmend_core::MeshBounds;

#[derive(Debug, Clone, Copy)]
pub struct Camera {
    pub target: Vec3,
    pub distance: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub fov_y: f32,
    pub near: f32,
    pub far: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            target: Vec3::ZERO,
            distance: 4.0,
            yaw: -0.6,
            pitch: 0.35,
            fov_y: 45.0_f32.to_radians(),
            near: 0.01,
            far: 1000.0,
        }
    }
}

impl Camera {
    pub fn fit_to_bounds(&mut self, bounds: MeshBounds, aspect: f32) {
        let radius = bounds.radius().max(0.001);
        self.target = bounds.center();
        self.distance = radius / (self.fov_y * 0.5).tan();
        self.distance *= aspect.max(1.0).sqrt().max(1.0);
        self.near = (self.distance - radius * 2.0)
            .max(radius * 0.001)
            .max(0.001);
        self.far = self.distance + radius * 4.0;
    }

    pub fn reset_to_bounds(&mut self, bounds: MeshBounds, aspect: f32) {
        let fov_y = self.fov_y;
        *self = Self {
            fov_y,
            ..Self::default()
        };
        self.fit_to_bounds(bounds, aspect);
    }

    pub fn eye(self) -> Vec3 {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let (sin_pitch, cos_pitch) = self.pitch.sin_cos();
        let dir = Vec3::new(cos_pitch * sin_yaw, sin_pitch, cos_pitch * cos_yaw);
        self.target + dir * self.distance
    }

    pub fn view_projection(self, aspect: f32) -> Mat4 {
        let view = Mat4::look_at_rh(self.eye(), self.target, Vec3::Y);
        let projection = Mat4::perspective_rh(self.fov_y, aspect.max(0.001), self.near, self.far);
        projection * view
    }

    pub fn orbit(&mut self, delta: Vec2) {
        self.yaw -= delta.x * 0.008;
        self.pitch = (self.pitch + delta.y * 0.008).clamp(-1.45, 1.45);
    }

    pub fn pan(&mut self, delta: Vec2, viewport_height: f32) {
        let (right, up, _) = self.basis();
        let world_per_pixel =
            2.0 * self.distance * (self.fov_y * 0.5).tan() / viewport_height.max(1.0);
        self.target += (-right * delta.x + up * delta.y) * world_per_pixel;
    }

    pub fn zoom(&mut self, wheel_delta: f32, bounds: Option<MeshBounds>) {
        let radius = bounds
            .map(|bounds| bounds.radius().max(0.001))
            .unwrap_or(1.0);
        let factor = (-wheel_delta * 0.12).exp();
        self.distance = (self.distance * factor).clamp(radius * 0.02, radius * 200.0);
        self.near = (self.distance - radius * 2.0)
            .max(radius * 0.001)
            .max(0.001);
        self.far = self.distance + radius * 4.0;
    }

    pub fn basis(self) -> (Vec3, Vec3, Vec3) {
        let forward = (self.target - self.eye()).normalize_or_zero();
        let right = forward.cross(Vec3::Y).normalize_or_zero();
        let up = right.cross(forward).normalize_or_zero();
        (right, up, forward)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_sets_target_to_bounds_center() {
        let bounds = MeshBounds {
            min: Vec3::new(-1.0, -2.0, -3.0),
            max: Vec3::new(3.0, 2.0, 5.0),
        };
        let mut camera = Camera::default();

        camera.fit_to_bounds(bounds, 1.0);

        assert_eq!(camera.target, Vec3::new(1.0, 0.0, 1.0));
        assert!(camera.distance > bounds.radius());
        assert!(camera.far > camera.near);
    }

    #[test]
    fn zoom_clamps_against_bounds() {
        let bounds = MeshBounds {
            min: Vec3::splat(-1.0),
            max: Vec3::splat(1.0),
        };
        let mut camera = Camera::default();
        camera.fit_to_bounds(bounds, 1.0);

        camera.zoom(100.0, Some(bounds));

        assert!(camera.distance >= bounds.radius() * 0.02);
        assert!(camera.far > camera.near);
    }
}

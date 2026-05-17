use glam::Vec3;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MeshBounds {
    pub min: Vec3,
    pub max: Vec3,
}

impl MeshBounds {
    pub const EMPTY: Self = Self {
        min: Vec3::splat(f32::INFINITY),
        max: Vec3::splat(f32::NEG_INFINITY),
    };

    pub fn from_point(point: Vec3) -> Self {
        Self {
            min: point,
            max: point,
        }
    }

    pub fn include_point(&mut self, point: Vec3) {
        self.min = self.min.min(point);
        self.max = self.max.max(point);
    }

    pub fn union(self, other: Self) -> Self {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        Self {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }

    pub fn center(self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    pub fn extent(self) -> Vec3 {
        self.max - self.min
    }

    pub fn radius(self) -> f32 {
        self.extent().length() * 0.5
    }

    pub fn is_empty(self) -> bool {
        !self.min.is_finite() || !self.max.is_finite()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unions_bounds() {
        let a = MeshBounds {
            min: Vec3::new(-1.0, 2.0, 0.0),
            max: Vec3::new(1.0, 3.0, 4.0),
        };
        let b = MeshBounds {
            min: Vec3::new(-3.0, -2.0, 1.0),
            max: Vec3::new(0.0, 8.0, 2.0),
        };

        let union = a.union(b);
        assert_eq!(union.min, Vec3::new(-3.0, -2.0, 0.0));
        assert_eq!(union.max, Vec3::new(1.0, 8.0, 4.0));
    }

    #[test]
    fn computes_center_and_radius() {
        let bounds = MeshBounds {
            min: Vec3::new(-1.0, -1.0, -1.0),
            max: Vec3::new(1.0, 1.0, 1.0),
        };

        assert_eq!(bounds.center(), Vec3::ZERO);
        assert!((bounds.radius() - 3.0_f32.sqrt()).abs() < f32::EPSILON);
    }
}

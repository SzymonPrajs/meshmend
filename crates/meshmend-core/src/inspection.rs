use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::MeshBounds;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CrossSectionAxis {
    X,
    Y,
    #[default]
    Z,
}

impl CrossSectionAxis {
    pub const ALL: [Self; 3] = [Self::X, Self::Y, Self::Z];

    pub fn label(self) -> &'static str {
        match self {
            Self::X => "X",
            Self::Y => "Y",
            Self::Z => "Z",
        }
    }

    pub fn normal(self) -> Vec3 {
        match self {
            Self::X => Vec3::X,
            Self::Y => Vec3::Y,
            Self::Z => Vec3::Z,
        }
    }

    pub fn coordinate(self, point: Vec3) -> f32 {
        match self {
            Self::X => point.x,
            Self::Y => point.y,
            Self::Z => point.z,
        }
    }

    pub fn range(self, bounds: MeshBounds) -> std::ops::RangeInclusive<f32> {
        self.coordinate(bounds.min)..=self.coordinate(bounds.max)
    }

    pub fn color(self) -> [f32; 4] {
        match self {
            Self::X => [0.95, 0.25, 0.22, 0.95],
            Self::Y => [0.35, 0.82, 0.36, 0.95],
            Self::Z => [0.32, 0.55, 1.0, 0.95],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CrossSectionState {
    pub enabled: bool,
    pub axis: CrossSectionAxis,
    pub offset: f32,
    pub flip_side: bool,
    pub show_plane_guide: bool,
}

impl Default for CrossSectionState {
    fn default() -> Self {
        Self {
            enabled: false,
            axis: CrossSectionAxis::default(),
            offset: 0.0,
            flip_side: false,
            show_plane_guide: true,
        }
    }
}

impl CrossSectionState {
    pub fn centered(bounds: MeshBounds) -> Self {
        let mut state = Self::default();
        state.reset_to_center(bounds);
        state
    }

    pub fn set_axis(&mut self, axis: CrossSectionAxis, bounds: MeshBounds) {
        self.axis = axis;
        self.reset_to_center(bounds);
    }

    pub fn reset_to_center(&mut self, bounds: MeshBounds) {
        self.offset = self.axis.coordinate(bounds.center());
        self.clamp_to_bounds(bounds);
    }

    pub fn clamp_to_bounds(&mut self, bounds: MeshBounds) {
        if bounds.is_empty() {
            return;
        }
        let start = self.axis.coordinate(bounds.min);
        let end = self.axis.coordinate(bounds.max);
        self.offset = self.offset.clamp(start.min(end), start.max(end));
    }

    pub fn range(self, bounds: MeshBounds) -> std::ops::RangeInclusive<f32> {
        self.axis.range(bounds)
    }

    pub fn plane(self) -> CrossSectionPlane {
        let sign = if self.flip_side { -1.0 } else { 1.0 };
        CrossSectionPlane {
            normal: self.axis.normal() * sign,
            offset: self.offset * sign,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CrossSectionPlane {
    pub normal: Vec3,
    pub offset: f32,
}

impl CrossSectionPlane {
    pub fn signed_distance(self, point: Vec3) -> f32 {
        point.dot(self.normal) - self.offset
    }

    pub fn keeps_point(self, point: Vec3) -> bool {
        self.signed_distance(point) >= 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds() -> MeshBounds {
        MeshBounds {
            min: Vec3::new(-2.0, 10.0, -5.0),
            max: Vec3::new(4.0, 20.0, 7.0),
        }
    }

    #[test]
    fn centers_and_clamps_offsets_to_axis_bounds() {
        let mut state = CrossSectionState::centered(bounds());
        assert_eq!(state.axis, CrossSectionAxis::Z);
        assert_eq!(state.offset, 1.0);

        state.offset = 99.0;
        state.clamp_to_bounds(bounds());
        assert_eq!(state.offset, 7.0);

        state.offset = -99.0;
        state.clamp_to_bounds(bounds());
        assert_eq!(state.offset, -5.0);
    }

    #[test]
    fn changing_axis_recenters_to_that_axis() {
        let mut state = CrossSectionState::centered(bounds());
        state.set_axis(CrossSectionAxis::Y, bounds());

        assert_eq!(state.axis, CrossSectionAxis::Y);
        assert_eq!(state.offset, 15.0);
    }

    #[test]
    fn plane_keeps_expected_side_and_flips() {
        let mut state = CrossSectionState {
            axis: CrossSectionAxis::X,
            offset: 1.5,
            ..CrossSectionState::default()
        };

        let plane = state.plane();
        assert!(plane.keeps_point(Vec3::new(2.0, 0.0, 0.0)));
        assert!(!plane.keeps_point(Vec3::new(1.0, 0.0, 0.0)));

        state.flip_side = true;
        let flipped = state.plane();
        assert!(flipped.keeps_point(Vec3::new(1.0, 0.0, 0.0)));
        assert!(!flipped.keeps_point(Vec3::new(2.0, 0.0, 0.0)));
    }
}

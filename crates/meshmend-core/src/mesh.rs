use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::MeshBounds;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Triangle {
    pub normal: Vec3,
    pub vertices: [Vec3; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TriangleId {
    pub chunk: u32,
    pub local_index: u32,
}

impl TriangleId {
    pub const LOCAL_BITS: u32 = 20;
    pub const LOCAL_MASK: u32 = (1 << Self::LOCAL_BITS) - 1;

    pub fn encode_picking_id(self) -> Option<u32> {
        if self.local_index > Self::LOCAL_MASK || self.chunk >= (1 << 12) {
            return None;
        }
        Some((self.chunk << Self::LOCAL_BITS) | self.local_index | 1)
    }

    pub fn decode_picking_id(value: u32) -> Option<Self> {
        if value == 0 {
            return None;
        }
        let encoded = value - 1;
        Some(Self {
            chunk: encoded >> Self::LOCAL_BITS,
            local_index: encoded & Self::LOCAL_MASK,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeshStats {
    pub triangle_count: u64,
    pub vertex_position_count: u64,
    pub bounds: MeshBounds,
    pub source_bytes: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_picking_id() {
        let id = TriangleId {
            chunk: 12,
            local_index: 3456,
        };

        let encoded = id.encode_picking_id().expect("id should fit");
        assert_eq!(TriangleId::decode_picking_id(encoded), Some(id));
        assert_eq!(TriangleId::decode_picking_id(0), None);
    }
}

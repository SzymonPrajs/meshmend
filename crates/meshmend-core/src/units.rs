use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LengthUnit {
    Unknown,
    Millimeter,
    Inch,
}

impl Default for LengthUnit {
    fn default() -> Self {
        Self::Unknown
    }
}

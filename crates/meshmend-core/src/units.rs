use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LengthUnit {
    #[default]
    Unknown,
    Millimeter,
    Inch,
}

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Decision {
    Qualified,
    Unsupported,
    Inconclusive,
    Regressed,
    NotProven,
}

impl Decision {
    pub fn exit_code(self) -> u8 {
        match self {
            Self::Qualified => 0,
            Self::NotProven => 2,
            Self::Unsupported => 3,
            Self::Inconclusive => 4,
            Self::Regressed => 5,
        }
    }
}

//! Optional, device-specific output controls, separate from the mixer Master.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MacaronSettings {
    pub volume: Option<u8>,
    /// -10 = left, 0 = centre, 10 = right.
    pub balance: Option<i8>,
    pub gain: Option<u8>,
    pub filter: Option<u8>,
    pub output_mode: Option<u8>,
}

impl MacaronSettings {
    pub fn validate(&self) -> bool {
        self.volume.is_none_or(|v| v <= 100)
            && self.balance.is_none_or(|v| (-10..=10).contains(&v))
            && self.gain.is_none_or(|v| v <= 2)
            && self.filter.is_none_or(|v| v <= 4)
            && self.output_mode.is_none_or(|v| v <= 1)
    }

    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct OutputControl {
    /// Physical device instance, not a friendly name or USB VID/PID alone.
    pub id: String,
    pub name: String,
    pub output: Option<String>,
    /// Volume/balance are read from hardware; remaining controls are last saved writes.
    pub settings: MacaronSettings,
    /// Volume/balance were read from hardware during this connection.
    pub live_volume: bool,
    pub error: Option<String>,
}

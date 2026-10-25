//! Versioned declarations for replacement providers and their options.
//!
//! These are planning contracts, not runtime registration or trust grants.
//! Existing native service names, signed package policies and C ABI tables
//! remain authoritative when a provider is actually bound.

pub mod capture;
pub mod configuration;

use serde::{Deserialize, Serialize};

pub const EXTENSION_CONTRACT_MAJOR: u16 = 1;
pub const EXTENSION_CONTRACT_MINOR: u16 = 0;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionVersion {
    pub major: u16,
    pub minor: u16,
}

impl ExtensionVersion {
    pub const CURRENT: Self = Self {
        major: EXTENSION_CONTRACT_MAJOR,
        minor: EXTENSION_CONTRACT_MINOR,
    };

    pub fn is_supported(self) -> bool {
        self.major == EXTENSION_CONTRACT_MAJOR && self.minor == EXTENSION_CONTRACT_MINOR
    }
}

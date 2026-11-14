//! What plugins declare about the extension points they contribute.

use serde::{Deserialize, Serialize};

use crate::ids::ProviderId;

/// Session-behaviour facts reported by the provider's own code.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    /// A session can be resumed from its provider session handle.
    pub resume: bool,
    pub fork: bool,
    /// Input sent while a turn is running is fed into that turn.
    pub steer: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderInfo {
    pub id: ProviderId,
    pub name: String,
    pub capabilities: ProviderCapabilities,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentInfo {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
}

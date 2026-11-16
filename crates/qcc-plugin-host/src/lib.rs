//! Runs plugin components.
//!
//! Each loaded plugin is an actor: one OS thread owns the wasmtime store and delivers
//! events to the component one at a time. Host imports never block on the outside
//! world — process output and timers are queued as events — so the component model's
//! lack of async is not felt by plugin authors.
//!
//! The host gives a plugin nothing it did not ask for: every import checks the
//! permissions requested in the manifest and granted by the user.

mod actor;
mod api;
mod bindings;
mod convert;
mod host;
mod manifest;
mod permissions;
pub mod services;

pub use actor::{PluginError, PluginHandle};
pub use api::{
    Contribution, EnvironmentRequest, Outcome, PluginRequest, ProviderRequest, SessionStart,
    SettledThread, SpawnThread, ThreadSummary, ThreadsAnswer, ThreadsCall, UiRequest,
};
pub use host::{LoadError, PluginHost};
pub use manifest::{API_VERSION, MANIFEST_FILE, Manifest, ManifestError, valid_name};
pub use permissions::{Grant, Permissions};
pub use services::{ProgramPaths, Services, resolve_program};

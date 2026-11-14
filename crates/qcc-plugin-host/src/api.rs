//! What the daemon asks of a plugin, in domain terms. The WIT types stay inside this
//! crate.

use qcc_domain::{
    EnvironmentInfo, InteractionResolution, ModelInfo, ProviderId, ProviderInfo, ThreadId,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Contribution {
    Provider(ProviderInfo),
    Environment(EnvironmentInfo),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionStart {
    pub thread: ThreadId,
    pub cwd: String,
    pub model: Option<String>,
    /// Provider session handle from an earlier start of this thread.
    pub resume: Option<String>,
    pub prompt: Option<String>,
    /// Variables the agent process must see.
    pub env: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderRequest {
    Start(SessionStart),
    Input {
        thread: ThreadId,
        text: String,
    },
    Interrupt {
        thread: ThreadId,
    },
    Stop {
        thread: ThreadId,
    },
    ResolveInteraction {
        thread: ThreadId,
        request_id: String,
        resolution: InteractionResolution,
    },
    ListModels,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvironmentRequest {
    Prepare {
        thread: ThreadId,
        source_dir: String,
    },
    Remove {
        thread: ThreadId,
        cwd: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginRequest {
    Provider {
        provider: ProviderId,
        request: ProviderRequest,
    },
    Environment {
        environment: String,
        request: EnvironmentRequest,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Done,
    SessionStarted { provider_session: Option<String> },
    Models(Vec<ModelInfo>),
    EnvironmentReady { cwd: String },
}

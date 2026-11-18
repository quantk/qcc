//! qccd: the daemon.
//!
//! One [`core`] task owns all state — the supervisor, the journal and the plugin
//! handles — and processes commands one at a time, so there are no locks and no
//! ordering questions. Connections and plugin calls run as separate tasks that talk
//! to it through a channel.

mod core;
mod handover;
mod plugins;
mod projects;
mod schedule;
mod server;
mod settings;

use std::future::Future;
use std::path::PathBuf;

use anyhow::Context;

/// Where a daemon instance lives.
#[derive(Debug, Clone)]
pub struct Config {
    /// Journal, installed plugins and plugin data.
    pub data_dir: PathBuf,
    /// The unix socket clients connect to.
    pub socket: PathBuf,
    /// Compiled plugins and other things that can be thrown away.
    pub cache_dir: PathBuf,
}

impl Config {
    pub fn plugins_dir(&self) -> PathBuf {
        self.data_dir.join("plugins")
    }

    /// Per-plugin permission grants of plugins that are not bundled.
    pub fn grants_dir(&self) -> PathBuf {
        self.data_dir.join("grants")
    }

    pub fn plugin_data_dir(&self, plugin: &str) -> PathBuf {
        self.data_dir.join("plugin-data").join(plugin)
    }

    /// Where an environment plugin makes the working directories of its threads.
    pub fn environment_root(&self, environment: &str) -> PathBuf {
        self.data_dir.join("environments").join(environment)
    }

    /// Skills plugins offer to agents, written out as `<plugin>/<name>/SKILL.md`.
    pub fn skills_dir(&self) -> PathBuf {
        self.data_dir.join("skills")
    }

    pub fn settings(&self) -> PathBuf {
        self.data_dir.join("settings.json")
    }

    /// Copies of the files attached to messages, a folder per thread.
    pub fn attachments_dir(&self) -> PathBuf {
        self.data_dir.join("attachments")
    }

    pub fn database(&self) -> PathBuf {
        self.data_dir.join("qcc.db")
    }
}

/// Run a daemon until `shutdown` resolves. `ready` fires once the socket accepts
/// connections.
pub async fn serve(
    config: Config,
    ready: impl FnOnce() + Send,
    shutdown: impl Future<Output = ()> + Send,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(&config.data_dir)
        .with_context(|| format!("cannot create {}", config.data_dir.display()))?;
    let listener = server::bind(&config.socket).await?;

    let (commands, queue) = tokio::sync::mpsc::unbounded_channel();
    let core = core::Core::start(&config, commands.clone())?;
    let mut core_task = tokio::spawn(core.run(queue));
    tracing::info!(socket = %config.socket.display(), "qccd is listening");
    ready();

    // The core ending on its own (a journal it cannot write, a panic) stops the
    // daemon too: without it every connection would be refused anyway.
    let finished = tokio::select! {
        () = server::accept(listener, commands.clone()) => None,
        () = shutdown => {
            tracing::info!("shutting down");
            None
        }
        finished = &mut core_task => Some(finished),
    };

    let finished = match finished {
        Some(finished) => finished,
        None => {
            let _ = commands.send(core::Command::Shutdown);
            core_task.await
        }
    };
    let _ = std::fs::remove_file(&config.socket);
    finished.context("core task panicked")?
}

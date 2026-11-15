use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use qcc_domain::{EnvironmentInfo, ProviderId, ProviderInfo};
use wasmtime::component::Component;
use wasmtime::{Cache, CacheConfig, Config, Engine};

use crate::actor::{self, PluginHandle};
use crate::api::Contribution;
use crate::manifest::{MANIFEST_FILE, Manifest, ManifestError};
use crate::permissions::Grant;
use crate::services::Services;

/// Resolution of the watchdog that stops a plugin stuck inside `handle`.
pub(crate) const EPOCH_TICK: Duration = Duration::from_millis(50);

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("{0}")]
    Manifest(#[from] ManifestError),
    #[error("cannot read {path}: {error}")]
    Io { path: String, error: std::io::Error },
    #[error("plugin {0:?} is already loaded")]
    DuplicatePlugin(String),
    #[error(
        "plugin {plugin:?} contributes provider {provider:?}, which {owner:?} already provides"
    )]
    DuplicateProvider { plugin: String, provider: ProviderId, owner: String },
    #[error("plugin {plugin:?} is not a valid component: {error}")]
    Component { plugin: String, error: String },
    #[error("plugin {plugin:?} failed to start: {error}")]
    Start { plugin: String, error: String },
    #[error("wasm engine: {0}")]
    Engine(String),
}

/// The set of loaded plugins and the engine they run on.
pub struct PluginHost {
    engine: Engine,
    plugins: Vec<PluginHandle>,
    ticker_stop: Arc<AtomicBool>,
}

impl PluginHost {
    pub fn new() -> Result<Self, LoadError> {
        Self::with_cache(None)
    }

    /// Like [`PluginHost::new`], keeping compiled components in `cache_dir` so that a
    /// plugin is compiled once per version instead of on every start.
    pub fn with_cache(cache_dir: Option<&Path>) -> Result<Self, LoadError> {
        let mut config = Config::new();
        config.wasm_component_model(true);
        config.epoch_interruption(true);
        if let Some(dir) = cache_dir {
            let mut cache = CacheConfig::new();
            cache.with_directory(dir);
            // A cache that cannot be set up costs time, not correctness.
            match Cache::new(cache) {
                Ok(cache) => {
                    config.cache(Some(cache));
                }
                Err(error) => tracing::warn!(%error, "compiling plugins without a cache"),
            }
        }
        let engine = Engine::new(&config).map_err(|error| LoadError::Engine(error.to_string()))?;

        let ticker_stop = Arc::new(AtomicBool::new(false));
        let (ticker_engine, stop) = (engine.clone(), ticker_stop.clone());
        std::thread::Builder::new()
            .name("plugin-watchdog".into())
            .spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    std::thread::sleep(EPOCH_TICK);
                    ticker_engine.increment_epoch();
                }
            })
            .map_err(|error| LoadError::Engine(error.to_string()))?;

        Ok(Self { engine, plugins: Vec::new(), ticker_stop })
    }

    /// Load a plugin from its manifest and component bytes, and run its `init`.
    pub fn load(
        &mut self,
        manifest: Manifest,
        wasm: &[u8],
        grant: &Grant,
        services: Services,
    ) -> Result<PluginHandle, LoadError> {
        let id = manifest.id.clone();
        if self.plugin(&id).is_some() {
            return Err(LoadError::DuplicatePlugin(id));
        }
        let component = Component::new(&self.engine, wasm).map_err(|error| {
            LoadError::Component { plugin: id.clone(), error: format!("{error:#}") }
        })?;
        let permissions = grant.effective(&manifest.permissions);
        let handle = actor::spawn(&self.engine, component, manifest, permissions, services)
            .map_err(|error| LoadError::Start { plugin: id.clone(), error })?;

        for contribution in handle.contributions() {
            if let Contribution::Provider(info) = contribution
                && let Some((owner, _)) = self.provider(&info.id)
            {
                return Err(LoadError::DuplicateProvider {
                    plugin: id,
                    provider: info.id.clone(),
                    owner: owner.id().to_owned(),
                });
            }
        }
        self.plugins.push(handle.clone());
        Ok(handle)
    }

    /// Load a plugin from a directory holding `plugin.toml` and the component it names.
    pub fn load_dir(
        &mut self,
        dir: &Path,
        grant: &Grant,
        services: impl FnOnce(&Manifest) -> Services,
    ) -> Result<PluginHandle, LoadError> {
        let read = |path: &Path| {
            std::fs::read(path)
                .map_err(|error| LoadError::Io { path: path.display().to_string(), error })
        };
        let manifest_text = String::from_utf8_lossy(&read(&dir.join(MANIFEST_FILE))?).into_owned();
        let manifest = Manifest::parse(&manifest_text)?;
        let wasm = read(&dir.join(&manifest.wasm))?;
        let services = services(&manifest);
        self.load(manifest, &wasm, grant, services)
    }

    pub fn plugins(&self) -> &[PluginHandle] {
        &self.plugins
    }

    pub fn plugin(&self, id: &str) -> Option<&PluginHandle> {
        self.plugins.iter().find(|plugin| plugin.id() == id)
    }

    /// The plugin that contributes a provider.
    pub fn provider(&self, id: &ProviderId) -> Option<(&PluginHandle, &ProviderInfo)> {
        self.plugins.iter().find_map(|plugin| {
            plugin.contributions().iter().find_map(|contribution| match contribution {
                Contribution::Provider(info) if &info.id == id => Some((plugin, info)),
                _ => None,
            })
        })
    }

    /// The plugin that contributes an environment. With two of the same id the one
    /// loaded first wins.
    pub fn environment(&self, id: &str) -> Option<(&PluginHandle, &EnvironmentInfo)> {
        self.plugins.iter().find_map(|plugin| {
            plugin.contributions().iter().find_map(|contribution| match contribution {
                Contribution::Environment(info) if info.id == id => Some((plugin, info)),
                _ => None,
            })
        })
    }

    pub fn providers(&self) -> impl Iterator<Item = (&PluginHandle, &ProviderInfo)> {
        self.plugins.iter().flat_map(|plugin| {
            plugin.contributions().iter().filter_map(move |contribution| match contribution {
                Contribution::Provider(info) => Some((plugin, info)),
                Contribution::Environment(_) => None,
            })
        })
    }
}

impl Drop for PluginHost {
    fn drop(&mut self) {
        self.ticker_stop.store(true, Ordering::Relaxed);
    }
}

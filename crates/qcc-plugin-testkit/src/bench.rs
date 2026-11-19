//! A bench for plugins that are not agent bridges: orchestration, reviewers and the
//! like. The plugin runs in the real wasm host; the test plays the daemon, answering
//! what the plugin asks of threads and comments and posting it events.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use qcc_plugin_host::services::KvStore;
use qcc_plugin_host::{
    Grant, Manifest, PluginError, PluginHandle, PluginHost, PluginRequest, Services, ThreadsAnswer,
    ThreadsCall,
};

pub use qcc_plugin_host::Outcome;

/// What the test answers for the daemon: the plugin's `threads` and `comments` calls.
pub type Daemon = Arc<dyn Fn(&ThreadsCall) -> Result<ThreadsAnswer, String> + Send + Sync>;

/// Configures a [`Bench`].
pub struct BenchBuilder {
    manifest: Manifest,
    wasm: Vec<u8>,
    grant: Grant,
    kv: Option<Box<dyn KvStore>>,
    daemon: Daemon,
}

impl BenchBuilder {
    /// What the user granted. Defaults to everything the manifest requests.
    pub fn grant(mut self, grant: Grant) -> Self {
        self.grant = grant;
        self
    }

    pub fn kv(mut self, kv: impl KvStore + 'static) -> Self {
        self.kv = Some(Box::new(kv));
        self
    }

    /// How the daemon answers. By default every call is refused.
    pub fn daemon(
        mut self,
        daemon: impl Fn(&ThreadsCall) -> Result<ThreadsAnswer, String> + Send + Sync + 'static,
    ) -> Self {
        self.daemon = Arc::new(daemon);
        self
    }

    pub fn build(self) -> Bench {
        let calls: Arc<Mutex<Vec<ThreadsCall>>> = Arc::default();
        let crash = Arc::new(Mutex::new(None));
        let mut services = Services::new(Arc::new(|_, _, _| {}));
        if let Some(kv) = self.kv {
            services.kv = kv;
        }
        let (log, daemon) = (calls.clone(), self.daemon);
        services.threads = Arc::new(move |_plugin, call| {
            let answer = daemon(&call);
            log.lock().unwrap().push(call);
            answer
        });
        let crash_slot = crash.clone();
        services.on_crash = Arc::new(move |_plugin, reason| {
            *crash_slot.lock().unwrap() = Some(reason.to_owned());
        });
        let mut host = PluginHost::new().expect("wasm engine");
        let plugin =
            host.load(self.manifest, &self.wasm, &self.grant, services).expect("plugin loads");
        Bench { _host: host, plugin, calls, crash }
    }
}

/// One plugin and the daemon's side, played by the test.
pub struct Bench {
    // Keeps the engine's watchdog alive for as long as the plugin runs.
    _host: PluginHost,
    plugin: PluginHandle,
    calls: Arc<Mutex<Vec<ThreadsCall>>>,
    crash: Arc<Mutex<Option<String>>>,
}

impl Bench {
    /// A plugin shipped in this repository, by its directory name under `plugins/`.
    pub fn bundled(plugin_id: &str) -> BenchBuilder {
        let plugin = qcc_plugins_bundle::plugin(plugin_id)
            .unwrap_or_else(|| panic!("no bundled plugin {plugin_id:?}"));
        Self::plugin(plugin.manifest, plugin.wasm)
    }

    /// Any plugin, from its manifest text and component bytes.
    pub fn plugin(manifest: &str, wasm: &[u8]) -> BenchBuilder {
        BenchBuilder {
            manifest: Manifest::parse(manifest).expect("valid manifest"),
            wasm: wasm.to_vec(),
            grant: Grant::AllRequested,
            kv: None,
            daemon: Arc::new(|call| Err(format!("the bench does not answer {call:?}"))),
        }
    }

    pub fn handle(&self) -> &PluginHandle {
        &self.plugin
    }

    /// Ask the plugin and wait for its answer.
    pub fn call(&self, request: PluginRequest) -> Result<Outcome, PluginError> {
        self.plugin.call_blocking(request)
    }

    /// Deliver an event nobody waits on, as the daemon does, and wait until the
    /// plugin has handled it.
    pub fn post(&self, request: PluginRequest) {
        self.plugin.post(request).expect("the plugin runs");
        self.plugin.flush_blocking();
    }

    /// Every call the plugin made so far, oldest first.
    pub fn calls(&self) -> Vec<ThreadsCall> {
        self.calls.lock().unwrap().clone()
    }

    /// The calls made since the last time they were taken.
    pub fn take_calls(&self) -> Vec<ThreadsCall> {
        std::mem::take(&mut *self.calls.lock().unwrap())
    }

    /// Wait until the plugin made a call that `matches`, its timers firing
    /// meanwhile, and return the calls so far. Panics after a few seconds.
    pub fn wait_for_call(
        &self,
        what: &str,
        matches: impl Fn(&ThreadsCall) -> bool,
    ) -> Vec<ThreadsCall> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            self.plugin.flush_blocking();
            let calls = self.calls();
            if calls.iter().any(&matches) {
                return calls;
            }
            assert!(self.crash().is_none(), "the plugin crashed: {:?}", self.crash());
            assert!(Instant::now() < deadline, "the plugin never {what}: {calls:#?}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Why the plugin crashed, if it did.
    pub fn crash(&self) -> Option<String> {
        self.crash.lock().unwrap().clone()
    }
}

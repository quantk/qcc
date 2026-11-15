//! Guest side of the qcc plugin contract.
//!
//! A plugin is a WebAssembly component built for `wasm32-wasip2`. Implement [`Plugin`]
//! and the extension-point traits you need, then export it:
//!
//! ```ignore
//! use qcc_plugin_sdk::prelude::*;
//!
//! #[derive(Default)]
//! struct Hello;
//!
//! impl Provider for Hello {
//!     fn info(&self) -> ProviderInfo {
//!         ProviderInfo {
//!             id: "hello".into(),
//!             name: "Hello".into(),
//!             capabilities: ProviderCapabilities {
//!                 resume: false,
//!                 fork: false,
//!                 steer: false,
//!                 permission_modes: vec![],
//!             },
//!         }
//!     }
//!
//!     fn start(&mut self, start: SessionStart) -> Result<SessionStarted, String> {
//!         deltas::emit(&start.thread, &[Delta::TurnOpen /* … */])?;
//!         Ok(SessionStarted { provider_session: None })
//!     }
//!
//!     fn input(&mut self, input: ThreadInput) -> Result<(), String> {
//!         Ok(())
//!     }
//! }
//!
//! impl Plugin for Hello {
//!     fn providers(&mut self) -> Vec<&mut dyn Provider> {
//!         vec![self]
//!     }
//! }
//!
//! qcc_plugin_sdk::export_plugin!(Hello);
//! ```
//!
//! The plugin is a single-threaded actor: the host calls it with one event at a time
//! and no host import blocks on the outside world. Process output and timers come back
//! through [`Plugin::on_process`] and [`Plugin::on_timer`].

// The generated bindings carry the component's raw ABI glue.
#[allow(unsafe_code, clippy::all, missing_docs)]
pub mod bindings {
    wit_bindgen::generate!({
        path: "../../wit",
        world: "plugin",
        pub_export_macro: true,
        export_macro_name: "__export_plugin_world",
        default_bindings_module: "qcc_plugin_sdk::bindings",
        additional_derives: [PartialEq],
    });
}

#[cfg(feature = "jsonrpc")]
pub mod jsonrpc;
pub mod lines;

pub use bindings::qcc::plugin::types;
/// Host imports. Each is gated by a manifest permission; a call without it fails.
pub use bindings::qcc::plugin::{deltas, kv, process, timers};

use types::{
    Contribution, EnvironmentInfo, EnvironmentPrepare, EnvironmentRemove, EnvironmentRequest,
    Event, InteractionAnswer, ModelInfo, Outcome, ProviderInfo, ProviderRequest, SessionStart,
    SessionStarted, ThreadInput,
};

/// Everything a plugin usually needs.
pub mod prelude {
    pub use crate::types::*;
    pub use crate::{
        Environment, Plugin, ProcessEvent, Provider, deltas, kv, log, process, timers,
    };
}

/// Logging through the host. Always available.
pub mod log {
    use crate::bindings::qcc::plugin::log::{Level, log};

    pub fn debug(message: impl AsRef<str>) {
        log(Level::Debug, message.as_ref());
    }

    pub fn info(message: impl AsRef<str>) {
        log(Level::Info, message.as_ref());
    }

    pub fn warn(message: impl AsRef<str>) {
        log(Level::Warn, message.as_ref());
    }

    pub fn error(message: impl AsRef<str>) {
        log(Level::Error, message.as_ref());
    }
}

/// An agent provider: a bridge between one agent's dialect and the delta grammar.
///
/// The bridge reports what happens through [`deltas::emit`] and nothing else. It never
/// mints timeline ids and keeps no timeline state: the core's assembler owns both.
pub trait Provider {
    fn info(&self) -> ProviderInfo;

    /// Start a session, or resume one when `start.resume` is set.
    fn start(&mut self, start: SessionStart) -> Result<SessionStarted, String>;

    /// Deliver a user message to a started session.
    fn input(&mut self, input: ThreadInput) -> Result<(), String>;

    /// Feed a user message into the running turn. Called only if the provider declared
    /// the `steer` capability.
    fn steer(&mut self, input: ThreadInput) -> Result<(), String> {
        let _ = input;
        Err("this provider cannot steer a running turn".into())
    }

    /// Interrupt the running turn. Emit the settling deltas before returning.
    fn interrupt(&mut self, thread: String) -> Result<(), String> {
        let _ = thread;
        Ok(())
    }

    /// Release the session. Afterwards the plugin holds nothing for the thread.
    fn stop(&mut self, thread: String) -> Result<(), String> {
        let _ = thread;
        Ok(())
    }

    /// The user answered an `interaction-request` this provider emitted.
    fn resolve_interaction(&mut self, answer: InteractionAnswer) -> Result<(), String> {
        Err(format!("no interaction {:?} is pending", answer.request_id))
    }

    fn models(&mut self) -> Result<Vec<ModelInfo>, String> {
        Ok(Vec::new())
    }
}

/// Prepares and removes the working directory of a thread.
pub trait Environment {
    fn info(&self) -> EnvironmentInfo;

    /// Returns the directory the thread works in.
    fn prepare(&mut self, request: EnvironmentPrepare) -> Result<String, String>;

    fn remove(&mut self, request: EnvironmentRemove) -> Result<(), String> {
        let _ = request;
        Ok(())
    }
}

/// Something that happened to a process the plugin spawned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessEvent {
    Stdout {
        process: u64,
        data: Vec<u8>,
    },
    Stderr {
        process: u64,
        data: Vec<u8>,
    },
    /// Delivered after all output. `code` is `None` when a signal killed the process.
    Exit {
        process: u64,
        code: Option<i32>,
    },
}

/// The plugin itself. Every method has a default, so implement only what you contribute.
pub trait Plugin: Default + 'static {
    fn providers(&mut self) -> Vec<&mut dyn Provider> {
        Vec::new()
    }

    fn environments(&mut self) -> Vec<&mut dyn Environment> {
        Vec::new()
    }

    fn on_process(&mut self, event: ProcessEvent) {
        let _ = event;
    }

    fn on_timer(&mut self, timer: u64) {
        let _ = timer;
    }
}

/// Export a [`Plugin`] as the component's `init` and `handle`.
#[macro_export]
macro_rules! export_plugin {
    ($plugin:ty) => {
        #[doc(hidden)]
        struct __QccPluginExport;

        impl $crate::bindings::Guest for __QccPluginExport {
            fn init() -> ::core::result::Result<
                ::std::vec::Vec<$crate::types::Contribution>,
                ::std::string::String,
            > {
                $crate::__private::init::<$plugin>()
            }

            fn handle(
                event: $crate::types::Event,
            ) -> ::core::result::Result<$crate::types::Outcome, ::std::string::String> {
                $crate::__private::handle::<$plugin>(event)
            }
        }

        $crate::bindings::__export_plugin_world!(__QccPluginExport with_types_in $crate::bindings);
    };
}

#[doc(hidden)]
pub mod __private {
    use std::any::Any;
    use std::cell::RefCell;

    use super::*;

    thread_local! {
        static INSTANCE: RefCell<Option<Box<dyn Any>>> = const { RefCell::new(None) };
    }

    fn with_plugin<P: Plugin, R>(f: impl FnOnce(&mut P) -> R) -> R {
        INSTANCE.with_borrow_mut(|slot| {
            let instance = slot.get_or_insert_with(|| Box::new(P::default()));
            let plugin = instance.downcast_mut::<P>().expect("one plugin type per component");
            f(plugin)
        })
    }

    pub fn init<P: Plugin>() -> Result<Vec<Contribution>, String> {
        with_plugin::<P, _>(|plugin| {
            let mut contributions = Vec::new();
            for provider in plugin.providers() {
                contributions.push(Contribution::Provider(provider.info()));
            }
            for environment in plugin.environments() {
                contributions.push(Contribution::Environment(environment.info()));
            }
            Ok(contributions)
        })
    }

    pub fn handle<P: Plugin>(event: Event) -> Result<Outcome, String> {
        with_plugin::<P, _>(|plugin| match event {
            Event::Provider(call) => {
                let provider = plugin
                    .providers()
                    .into_iter()
                    .find(|provider| provider.info().id == call.provider)
                    .ok_or_else(|| format!("unknown provider {:?}", call.provider))?;
                match call.request {
                    ProviderRequest::Start(start) => {
                        provider.start(start).map(Outcome::SessionStarted)
                    }
                    ProviderRequest::Input(input) => provider.input(input).map(done),
                    ProviderRequest::Steer(input) => provider.steer(input).map(done),
                    ProviderRequest::Interrupt(thread) => provider.interrupt(thread).map(done),
                    ProviderRequest::Stop(thread) => provider.stop(thread).map(done),
                    ProviderRequest::ResolveInteraction(answer) => {
                        provider.resolve_interaction(answer).map(done)
                    }
                    ProviderRequest::ListModels => provider.models().map(Outcome::Models),
                }
            }
            Event::Environment(call) => {
                let environment = plugin
                    .environments()
                    .into_iter()
                    .find(|environment| environment.info().id == call.environment)
                    .ok_or_else(|| format!("unknown environment {:?}", call.environment))?;
                match call.request {
                    EnvironmentRequest::Prepare(request) => {
                        environment.prepare(request).map(Outcome::EnvironmentReady)
                    }
                    EnvironmentRequest::Remove(request) => environment.remove(request).map(done),
                }
            }
            Event::ProcessStdout(output) => {
                plugin.on_process(ProcessEvent::Stdout {
                    process: output.process,
                    data: output.data,
                });
                Ok(Outcome::Done)
            }
            Event::ProcessStderr(output) => {
                plugin.on_process(ProcessEvent::Stderr {
                    process: output.process,
                    data: output.data,
                });
                Ok(Outcome::Done)
            }
            Event::ProcessExit(exit) => {
                plugin.on_process(ProcessEvent::Exit { process: exit.process, code: exit.code });
                Ok(Outcome::Done)
            }
            Event::Timer(timer) => {
                plugin.on_timer(timer);
                Ok(Outcome::Done)
            }
        })
    }

    fn done(_: ()) -> Outcome {
        Outcome::Done
    }
}

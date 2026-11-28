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
//!                 steer: false,
//!                 permission_modes: vec![],
//!                 mcp_live: false,
//!                 compact: false,
//!                 list_skills: false,
//!                 list_commands: false,
//!                 context_usage: false,
//!                 rate_limits: false,
//!                 fork: ForkSupport::None,
//!             },
//!             // An SVG document of the agent's mark, or `None` for its initials.
//!             icon: None,
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
//! and no host import blocks on the outside world. Process output, timers and watched
//! threads come back through [`Plugin::on_process`], [`Plugin::on_timer`] and
//! [`Plugin::on_thread_settled`]; every thread's changes, for a plugin allowed to hear
//! them, through [`Plugin::on_thread_changed`].

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

#[cfg(feature = "json")]
pub mod json {
    //! Small helpers for reading an agent's JSON.

    use serde_json::Value;

    /// A string field of `value`, or `""` when it is missing or not a string.
    pub fn str_field<'a>(value: &'a Value, name: &str) -> &'a str {
        value.get(name).and_then(Value::as_str).unwrap_or("")
    }
}
#[cfg(feature = "jsonrpc")]
pub mod jsonrpc;
pub mod lines;
pub mod stdio;
#[cfg(feature = "ui")]
pub mod ui;

pub use bindings::qcc::plugin::types;
/// Host imports. Each is gated by a manifest permission; a call without it fails.
pub use bindings::qcc::plugin::{
    comments, completions, deltas, files, kv, notifications, process, threads, timers, views,
};

use types::{
    AgentCommand, AgentSkill, Attachment, CommandCall, CommandInfo, CommandResult, Contribution,
    Delta, EnvironmentChanges, EnvironmentDeliver, EnvironmentDiff, EnvironmentInfo,
    EnvironmentPrepare, EnvironmentQuery, EnvironmentRemove, EnvironmentRequest, Event,
    InteractionAnswer, InteractionOption, McpServersUpdate, ModelInfo, OptionKind, Outcome,
    ProviderInfo, ProviderModels, ProviderRequest, SessionStart, SessionStarted, SettingField,
    SettledThread, SkillInfo, SkillRef, ThreadChange, ThreadChanges, ThreadDiff, ThreadInput,
    TokenUsage, TurnBoundary, TurnEnded, TurnStatus, UiAction, UiItem, UiRender, UiRequest,
    UiViewInfo,
};

/// Everything a plugin usually needs.
pub mod prelude {
    pub use crate::types::*;
    pub use crate::{
        Commands, Environment, Plugin, ProcessEvent, Provider, Reply, Ui, add_usage,
        attachments_note, boundary, boundary_at, comments, completions, deltas, files, kv, log,
        message, notifications, option, process, session_context, skills_prompt, slash_command,
        threads, timers, views, with_notes,
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

    /// Change the qcc MCP servers of the running session to `update.servers`. Called
    /// only if the provider declared the `mcp_live` capability.
    fn set_mcp_servers(&mut self, update: McpServersUpdate) -> Result<(), String> {
        let _ = update;
        Err("this provider cannot change the MCP servers of a running session".into())
    }

    /// Compact the session's context. Called only if the provider declared the
    /// `compact` capability, and only while no turn runs. Open a turn, report
    /// `context-compacted` and settle the turn, as the agent does the work.
    fn compact(&mut self, thread: String) -> Result<(), String> {
        let _ = thread;
        Err("this provider cannot compact a session's context".into())
    }

    /// The models the agent offers. Asking the agent usually means running it, so
    /// the answer may come later: return [`Reply::Later`] and answer with
    /// [`Outcome::Models`] through [`completions::complete`] (see [`stdio::Probes`]).
    fn models(&mut self) -> Result<Reply<Vec<ModelInfo>>, String> {
        Ok(Reply::Now(Vec::new()))
    }

    /// The skills the agent finds by itself (its own folders, its plugins). Like
    /// [`Provider::models`], usually answered later, with [`Outcome::Skills`].
    /// `cwd` is a thread's working directory when the user asks about one thread:
    /// ask the agent there, so the project's own skills are among those it finds.
    fn skills(&mut self, _cwd: Option<String>) -> Result<Reply<Vec<AgentSkill>>, String> {
        Ok(Reply::Now(Vec::new()))
    }

    /// The slash commands the agent runs when a message starts with `/name`, in
    /// `cwd` when given. Called only if the provider declared `list_commands`. Only
    /// commands that work in a qcc session belong here. Like [`Provider::skills`],
    /// usually answered later, with [`Outcome::Commands`].
    fn commands(&mut self, _cwd: Option<String>) -> Result<Reply<Vec<AgentCommand>>, String> {
        Ok(Reply::Now(Vec::new()))
    }

    /// The account's usage limits as they are now, without a session. Called only if
    /// the provider declared the `rate_limits` capability. Like [`Provider::models`],
    /// usually answered later, with [`Outcome::RateLimits`] (a complete report).
    fn rate_limits(&mut self) -> Result<Reply<types::RateLimits>, String> {
        Ok(Reply::Now(types::RateLimits { plan: None, windows: Vec::new(), complete: true }))
    }
}

/// When a request is answered.
///
/// No host import blocks, so work that waits on a process cannot finish inside the
/// call that asked for it. Such a request returns [`Reply::Later`] with a token of the
/// plugin's choosing and is answered by [`completions::complete`] once the result is
/// known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply<T> {
    Now(T),
    Later(u64),
}

/// Makes and removes the working directory of a thread, and reports what the thread
/// changed in it.
pub trait Environment {
    fn info(&self) -> EnvironmentInfo;

    /// Make the directory the thread works in and return its path. Answer a
    /// deferred call with [`Outcome::EnvironmentReady`].
    fn prepare(&mut self, request: EnvironmentPrepare) -> Result<Reply<String>, String>;

    /// Remove what `prepare` made. Refuse when work would be lost and
    /// `request.force` is not set. Answer a deferred call with [`Outcome::Done`].
    fn remove(&mut self, request: EnvironmentRemove) -> Result<Reply<()>, String> {
        let _ = request;
        Ok(Reply::Now(()))
    }

    /// Answer a deferred call with [`Outcome::EnvironmentChanges`].
    fn changes(&mut self, query: EnvironmentQuery) -> Result<Reply<EnvironmentChanges>, String> {
        let _ = query;
        Err("this environment does not track changes".into())
    }

    /// Answer a deferred call with [`Outcome::EnvironmentDiff`].
    fn diff(&mut self, query: EnvironmentQuery) -> Result<Reply<EnvironmentDiff>, String> {
        let _ = query;
        Err("this environment does not track changes".into())
    }

    /// Bring the thread's work back to the directory the environment was made from,
    /// and say what was done. Refuse, changing nothing, when it does not fit. Answer
    /// a deferred call with [`Outcome::EnvironmentDelivered`].
    fn deliver(&mut self, request: EnvironmentDeliver) -> Result<Reply<String>, String> {
        let _ = request;
        Err("this environment cannot bring its work back".into())
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

impl ProcessEvent {
    /// The process the event is about.
    pub fn process(&self) -> u64 {
        match self {
            Self::Stdout { process, .. }
            | Self::Stderr { process, .. }
            | Self::Exit { process, .. } => *process,
        }
    }
}

/// The plugin's own views in the app, and how its timeline items are drawn.
///
/// Every answer is a tree of UI nodes as JSON; the `ui` feature has builders for it.
/// The app draws nodes from a fixed set and nothing else, so a view is data, not code.
pub trait Ui {
    fn views(&self) -> Vec<UiViewInfo> {
        Vec::new()
    }

    /// Extension item kinds this plugin draws: `<plugin-id>/<name>`.
    fn item_kinds(&self) -> Vec<String> {
        Vec::new()
    }

    fn render(&mut self, request: UiRender) -> Result<String, String>;

    /// The user activated a node. The answer replaces the view; by default it is
    /// the view rendered again.
    fn action(&mut self, action: UiAction) -> Result<String, String> {
        self.render(UiRender { view: action.view, thread: action.thread })
    }

    fn render_item(&mut self, item: UiItem) -> Result<String, String> {
        Err(format!("this plugin does not draw {:?} items", item.kind))
    }
}

/// Commands for the CLI (`qcc <plugin-id> <command>`) and the app's command palette.
pub trait Commands {
    fn commands(&self) -> Vec<CommandInfo>;

    /// Run one. The host has checked that the command exists, that a thread is named
    /// if it needs one and that required parameters are there. Answer a deferred
    /// call with [`Outcome::CommandResult`].
    fn run(&mut self, call: CommandCall) -> Result<Reply<CommandResult>, String>;
}

/// The paragraph a provider bridge hands its agent about the skills of a session,
/// or `None` if there are none. Every bridge uses it, so agents hear the same thing
/// whatever runs them.
pub fn skills_prompt(skills: &[SkillRef]) -> Option<String> {
    if skills.is_empty() {
        return None;
    }
    let mut text = String::from(
        "This session runs under qcc, a control center for coding agents. qcc offers \
         these skills: instructions kept in a file. When a task matches a skill's \
         description, read the file first and follow it.\n",
    );
    for skill in skills {
        text.push_str(&format!("\n- {}: {} (file: {})", skill.name, skill.description, skill.path));
    }
    Some(text)
}

/// The slash command a message starts with, and what follows it: `/review src`
/// is `("review", "src")`. The name runs up to whitespace and holds letters,
/// digits and `-_.:/` (`opsx:apply`, `team/review`), but not a second `/` right
/// after the first or a trailing one, so a path such as `/usr/bin/` or `//x` is
/// not a command. `None` when the message does not start with one.
pub fn slash_command(text: &str) -> Option<(&str, &str)> {
    let rest = text.strip_prefix('/')?;
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let name = &rest[..end];
    let valid = !name.is_empty()
        && name.chars().all(|c| c.is_alphanumeric() || "-_.:/".contains(c))
        && !name.starts_with('/')
        && !name.ends_with('/')
        && name.chars().next().is_some_and(char::is_alphanumeric);
    valid.then(|| (name, rest[end..].trim_start()))
}

/// A user message with what qcc tells the agent along with it (`thread-input.notes`)
/// put in front, marked as qcc's so the agent does not take it for the user's words.
/// A message that starts with a slash command ([`slash_command`]) keeps it in front,
/// so the agent still sees the command, and gets the notes after it. The text alone
/// when there are no notes. For bridges whose agent has no other place for such
/// facts.
pub fn with_notes(notes: &[String], text: &str) -> String {
    if notes.is_empty() {
        return text.to_owned();
    }
    let mut block = String::from(
        "<qcc-notes>\nqcc, which runs this session, adds (the user did not write this):\n",
    );
    for note in notes {
        block.push_str(&format!("- {note}\n"));
    }
    block.push_str("</qcc-notes>");
    if slash_command(text).is_some() {
        format!("{text}\n\n{block}")
    } else {
        format!("{block}\n\n{text}")
    }
}

/// A user message as a bridge hands it to its agent: the text, with qcc's notes in
/// front ([`with_notes`]), and the attached images the agent is to look at.
#[derive(Debug, Clone)]
pub struct Message<'a> {
    pub text: String,
    /// Attachments that came with their bytes (`attachment.data`), in order. Empty
    /// unless the bridge asked for pictures.
    pub images: Vec<&'a Attachment>,
}

/// Put a message together the way most bridges hand one over. With `pictures`,
/// images that came with their bytes go to [`Message::images`]; every other
/// attachment becomes a note saying where it is saved ([`attachments_note`]), so
/// the agent can read it.
pub fn message<'a>(
    text: &str,
    notes: &[String],
    attachments: &'a [Attachment],
    pictures: bool,
) -> Message<'a> {
    let (images, files): (Vec<&Attachment>, Vec<&Attachment>) =
        attachments.iter().partition(|attachment| pictures && attachment.data.is_some());
    let mut notes = notes.to_vec();
    notes.extend(attachments_note(&files));
    Message { text: with_notes(&notes, text), images }
}

/// The note that tells an agent where the files attached to a message are saved,
/// or `None` if there are none. For attachments a bridge does not hand over itself.
pub fn attachments_note(attachments: &[&Attachment]) -> Option<String> {
    if attachments.is_empty() {
        return None;
    }
    let mut note = String::from(
        "The user attached files to this message; qcc saved them where you can read them:",
    );
    for attachment in attachments {
        note.push_str(&format!(
            "\n  - {} ({}, {} bytes): {}",
            attachment.name, attachment.mime, attachment.size, attachment.path
        ));
    }
    Some(note)
}

/// Standard base64 with padding, the way agents take image bytes in JSON.
pub fn base64(data: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(data)
}

/// The paragraph that tells an agent about the other folders of its project, or
/// `None` if there are none. For bridges whose agent has no way to take extra
/// directories, or to make sure it knows of them.
fn extra_dirs_prompt(dirs: &[String]) -> Option<String> {
    if dirs.is_empty() {
        return None;
    }
    let mut text = String::from(
        "Besides the working directory, these folders belong to the same project. Read \
         and change them as the task needs, the same way as the working directory:\n",
    );
    for dir in dirs {
        text.push_str(&format!("\n- {dir}"));
    }
    Some(text)
}

/// The end of the running turn, as a bridge reports it.
pub fn boundary(status: TurnStatus, error: Option<String>) -> Delta {
    Delta::TurnBoundary(TurnBoundary { status, error, claim_if_idle: false, point: None })
}

/// The end of the running turn, with where the session stands after it: what a fork
/// from this turn starts from (`session-start.fork-at`).
pub fn boundary_at(status: TurnStatus, error: Option<String>, point: Option<String>) -> Delta {
    Delta::TurnBoundary(TurnBoundary { status, error, claim_if_idle: false, point })
}

/// One choice of an approval prompt.
pub fn option(id: &str, label: &str, kind: OptionKind) -> InteractionOption {
    InteractionOption { id: id.into(), label: label.into(), kind }
}

/// Add one turn's usage to the session's total, for agents that report per turn.
/// Cached tokens count once the agent reports any.
pub fn add_usage(total: &mut TokenUsage, last: &TokenUsage) {
    total.input_tokens += last.input_tokens;
    total.output_tokens += last.output_tokens;
    if let Some(cached) = last.cached_input_tokens {
        total.cached_input_tokens = Some(total.cached_input_tokens.unwrap_or(0) + cached);
    }
}

/// Everything qcc tells an agent at the start of a session that is not the user's
/// message: its skills and the folders of its project.
pub fn session_context(start: &SessionStart) -> Option<String> {
    let parts: Vec<String> = [skills_prompt(&start.skills), extra_dirs_prompt(&start.extra_dirs)]
        .into_iter()
        .flatten()
        .collect();
    (!parts.is_empty()).then(|| parts.join("\n\n"))
}

/// The plugin itself. Every method has a default, so implement only what you contribute.
pub trait Plugin: Default + 'static {
    fn providers(&mut self) -> Vec<&mut dyn Provider> {
        Vec::new()
    }

    fn environments(&mut self) -> Vec<&mut dyn Environment> {
        Vec::new()
    }

    fn ui(&mut self) -> Option<&mut dyn Ui> {
        None
    }

    fn commands(&mut self) -> Option<&mut dyn Commands> {
        None
    }

    /// Instructions for agents. They reach every agent session, so the host takes
    /// them only from a plugin granted the `skills` permission.
    fn skills(&self) -> Vec<SkillInfo> {
        Vec::new()
    }

    /// Settings of the plugin's own, which the user sets in the app or with
    /// `qcc plugin set`. Their values arrive in [`Plugin::on_settings`].
    fn settings(&self) -> Vec<SettingField> {
        Vec::new()
    }

    /// The plugin's settings by id, defaults filled in: once the daemon is up, and
    /// again whenever the user changes one.
    fn on_settings(&mut self, values: Vec<(String, String)>) {
        let _ = values;
    }

    /// A thread passed to [`threads::watch`] settled.
    fn on_thread_settled(&mut self, settled: SettledThread) {
        let _ = settled;
    }

    /// A turn of some thread ended. Needs the `turn-events` permission.
    fn on_turn_ended(&mut self, ended: TurnEnded) {
        let _ = ended;
    }

    /// The diff asked for with [`threads::diff`].
    fn on_thread_diff(&mut self, diff: ThreadDiff) {
        let _ = diff;
    }

    /// The comments on a thread's changes changed. Needs the `comments` permission.
    fn on_comments_changed(&mut self, thread: String) {
        let _ = thread;
    }

    /// The changed files asked for with [`threads::changes`].
    fn on_thread_changes(&mut self, changes: ThreadChanges) {
        let _ = changes;
    }

    /// The models asked for with [`threads::models`].
    fn on_provider_models(&mut self, models: ProviderModels) {
        let _ = models;
    }

    /// A thread changed: it moved to another state, a turn ended, or it waits on
    /// another request. Only for a plugin with the `thread-events` permission, which
    /// hears of every thread. Wider than [`Plugin::on_turn_ended`].
    fn on_thread_changed(&mut self, change: ThreadChange) {
        let _ = change;
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
            if let Some(ui) = plugin.ui() {
                contributions.extend(ui.views().into_iter().map(Contribution::UiView));
                contributions.extend(ui.item_kinds().into_iter().map(Contribution::UiItem));
            }
            if let Some(commands) = plugin.commands() {
                contributions.extend(commands.commands().into_iter().map(Contribution::Command));
            }
            contributions.extend(plugin.skills().into_iter().map(Contribution::Skill));
            contributions.extend(plugin.settings().into_iter().map(Contribution::Setting));
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
                    ProviderRequest::ListModels => {
                        provider.models().map(|r| reply(r, Outcome::Models))
                    }
                    ProviderRequest::ListSkills(cwd) => {
                        provider.skills(cwd).map(|r| reply(r, Outcome::Skills))
                    }
                    ProviderRequest::ListCommands(cwd) => {
                        provider.commands(cwd).map(|r| reply(r, Outcome::Commands))
                    }
                    ProviderRequest::ReadRateLimits => {
                        provider.rate_limits().map(|r| reply(r, Outcome::RateLimits))
                    }
                    ProviderRequest::SetMcpServers(update) => {
                        provider.set_mcp_servers(update).map(done)
                    }
                    ProviderRequest::Compact(thread) => provider.compact(thread).map(done),
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
                        environment.prepare(request).map(|r| reply(r, Outcome::EnvironmentReady))
                    }
                    EnvironmentRequest::Remove(request) => {
                        environment.remove(request).map(|r| reply(r, done))
                    }
                    EnvironmentRequest::Changes(query) => {
                        environment.changes(query).map(|r| reply(r, Outcome::EnvironmentChanges))
                    }
                    EnvironmentRequest::Diff(query) => {
                        environment.diff(query).map(|r| reply(r, Outcome::EnvironmentDiff))
                    }
                    EnvironmentRequest::Deliver(request) => environment
                        .deliver(request)
                        .map(|r| reply(r, Outcome::EnvironmentDelivered)),
                }
            }
            Event::Ui(request) => {
                let ui = plugin.ui().ok_or("this plugin contributes no UI")?;
                match request {
                    UiRequest::Render(request) => ui.render(request),
                    UiRequest::Action(action) => ui.action(action),
                    UiRequest::RenderItem(item) => ui.render_item(item),
                }
                .map(Outcome::UiTree)
            }
            Event::Command(call) => {
                let commands = plugin.commands().ok_or("this plugin contributes no commands")?;
                commands.run(call).map(|r| reply(r, Outcome::CommandResult))
            }
            Event::ThreadSettled(settled) => {
                plugin.on_thread_settled(settled);
                Ok(Outcome::Done)
            }
            Event::TurnEnded(ended) => {
                plugin.on_turn_ended(ended);
                Ok(Outcome::Done)
            }
            Event::ThreadDiff(diff) => {
                plugin.on_thread_diff(diff);
                Ok(Outcome::Done)
            }
            Event::CommentsChanged(thread) => {
                plugin.on_comments_changed(thread);
                Ok(Outcome::Done)
            }
            Event::ThreadChanges(changes) => {
                plugin.on_thread_changes(changes);
                Ok(Outcome::Done)
            }
            Event::ProviderModels(models) => {
                plugin.on_provider_models(models);
                Ok(Outcome::Done)
            }
            Event::ThreadChanged(change) => {
                plugin.on_thread_changed(change);
                Ok(Outcome::Done)
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
            Event::Settings(values) => {
                plugin.on_settings(values);
                Ok(Outcome::Done)
            }
        })
    }

    fn done(_: ()) -> Outcome {
        Outcome::Done
    }

    fn reply<T>(reply: Reply<T>, now: impl FnOnce(T) -> Outcome) -> Outcome {
        match reply {
            Reply::Now(value) => now(value),
            Reply::Later(token) => Outcome::Deferred(token),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_starts_with_a_command_or_not() {
        assert_eq!(slash_command("/review src/a.rs"), Some(("review", "src/a.rs")));
        assert_eq!(slash_command("/opsx:apply"), Some(("opsx:apply", "")));
        assert_eq!(slash_command("/team/review now"), Some(("team/review", "now")));
        assert_eq!(slash_command("/compact\nkeep the plan"), Some(("compact", "keep the plan")));
        for text in ["/usr/bin/", "//x", "/", "/ x", "see /review", "/-x", "/a!"] {
            assert_eq!(slash_command(text), None, "{text}");
        }
    }

    #[test]
    fn notes_go_after_a_command_and_before_anything_else() {
        let notes = ["a file is attached".to_owned()];
        let plain = with_notes(&notes, "look");
        assert!(plain.starts_with("<qcc-notes>") && plain.ends_with("\n\nlook"), "{plain}");
        let command = with_notes(&notes, "/review now");
        assert!(command.starts_with("/review now\n\n<qcc-notes>"), "{command}");
        assert_eq!(with_notes(&[], "/review"), "/review");
    }
}

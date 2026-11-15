//! qcc: the command-line client of qccd.
//!
//! It is the same interface for people and for agents: every command takes `--json`,
//! and a `qcc thread spawn` run from inside an agent session records that session's
//! thread as the parent. Plugin commands run as `qcc <plugin> <command>`.

mod render;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, bail};
use clap::{Args, Parser, Subcommand};
use qcc_client::protocol::methods::{
    CommandList, CommandRun, CommandRunParams, DaemonStatus, Delivery, Empty, InteractionResolve,
    InteractionResolveParams, PluginList, ProjectCreate, ProjectCreateParams, ProjectDelete,
    ProjectDeleteParams, ProjectList, ProjectListResult, ProjectUpdate, ProjectUpdateParams,
    ProviderModels, ProviderModelsParams, ProviderSettings, SendMode, SettingsGet, SettingsResult,
    SettingsSet, SettingsSetParams, StopMode, ThreadArchive, ThreadArchiveParams, ThreadChanges,
    ThreadDiff, ThreadGet, ThreadGetParams, ThreadList, ThreadListParams, ThreadRename,
    ThreadRenameParams, ThreadSend, ThreadSendParams, ThreadSetAgent, ThreadSetAgentParams,
    ThreadSetMode, ThreadSetModeParams, ThreadSetProject, ThreadSetProjectParams, ThreadSpawn,
    ThreadSpawnParams, ThreadStop, ThreadStopParams, ThreadUnqueue, ThreadUnqueueParams,
    ThreadWait, ThreadWaitParams,
};
use qcc_client::protocol::paths;
use qcc_client::{Client, EventStream};
use qcc_domain::{
    EventPayload, InteractionId, InteractionResolution, PermissionMode, Project, ProviderId,
    ThreadEvent, ThreadId, ThreadState, ThreadStatus, TurnStatus,
};
use serde::Serialize;

/// Control center for coding agents.
#[derive(Parser)]
#[command(name = "qcc", version)]
struct Cli {
    /// Socket of the daemon [default: $XDG_RUNTIME_DIR/qcc/qccd.sock].
    #[arg(long, global = true, env = paths::SOCKET_ENV)]
    socket: Option<PathBuf>,

    /// Print machine-readable JSON instead of text.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Inspect the daemon.
    #[command(subcommand)]
    Daemon(DaemonCommand),
    /// Create, drive and inspect threads.
    #[command(subcommand)]
    Thread(ThreadCommand),
    /// Answer approvals and questions agents are waiting on.
    #[command(subcommand)]
    Interaction(InteractionCommand),
    /// Inspect plugins.
    #[command(subcommand)]
    Plugin(PluginCommand),
    /// Group threads by project: named sets of repositories and folders.
    #[command(subcommand)]
    Project(ProjectCommand),
    /// Show and change the daemon's settings: paths of agent programs, defaults and
    /// environment of each agent.
    #[command(subcommand)]
    Settings(SettingsCommand),
    /// Inspect agent providers.
    #[command(subcommand)]
    Provider(ProviderCommand),
    /// List the commands plugins contribute.
    #[command(subcommand, name = "command")]
    PluginCommands(CommandCommand),
    /// Run a plugin's command: `qcc <plugin> <command> [--thread <id>] [--<param> <value>]…`.
    /// The thread defaults to $QCC_THREAD_ID; `qcc command list` shows what there is.
    #[command(external_subcommand)]
    External(Vec<String>),
}

#[derive(Subcommand)]
enum CommandCommand {
    /// List every plugin command and how to run it.
    List,
}

#[derive(Subcommand)]
enum DaemonCommand {
    /// Show whether the daemon is running and where it lives.
    Status,
}

#[derive(Subcommand)]
enum ThreadCommand {
    /// Start a new thread with a provider.
    Spawn {
        /// Provider id, as listed by `qcc plugin list` [default: the parent's].
        #[arg(long)]
        provider: Option<String>,
        /// The first message to the agent.
        #[arg(long)]
        prompt: Option<String>,
        /// Working directory of the thread [default: the current directory].
        #[arg(long)]
        cwd: Option<PathBuf>,
        /// Give the thread a working directory of its own, made from --cwd by this
        /// environment (`git-worktree`: a worktree on a new branch).
        #[arg(long, value_name = "ENVIRONMENT")]
        env: Option<String>,
        /// What the environment starts from: for git-worktree a branch, tag or
        /// commit [default: the current HEAD].
        #[arg(long, value_name = "REF", requires = "env")]
        base: Option<String>,
        #[arg(long)]
        model: Option<String>,
        /// Parent thread [default: $QCC_THREAD_ID, set inside agent sessions].
        #[arg(long, env = paths::THREAD_ID_ENV)]
        parent: Option<String>,
        /// Project id or name [default: the parent's, or the project whose folder
        /// holds --cwd].
        #[arg(long)]
        project: Option<String>,
        /// How much the agent may do unasked: ask, accept-edits, auto or full
        /// [default: the agent's own configuration].
        #[arg(long, value_name = "MODE")]
        permission_mode: Option<PermissionMode>,
        /// Stream the thread's events until the first turn settles.
        #[arg(long)]
        follow: bool,
    },
    /// Send a message to a thread. While a turn is running the message is queued
    /// behind it, or fed into it with --steer.
    Send {
        thread: String,
        text: String,
        /// Feed the message into the running turn instead of queueing it. Queued
        /// anyway if the provider cannot steer.
        #[arg(long)]
        steer: bool,
        /// Stream the thread's events until the turn that takes this message settles.
        #[arg(long)]
        follow: bool,
    },
    /// Change a thread's permission mode: ask, accept-edits, auto, full, or
    /// `default` for the agent's own configuration.
    Mode { thread: String, mode: String },
    /// Name a thread.
    Rename { thread: String, title: String },
    /// Put a thread into a project (id or name), or take it out with `none`.
    Project { thread: String, project: String },
    /// Take a queued message back before it is delivered and print it. The id is the
    /// `queue_id` that `thread send --json` printed.
    Unqueue { thread: String, queue_id: String },
    /// Go on with another agent or model. Another agent starts a new session and is
    /// told the conversation so far; `qcc provider models` lists the models.
    Agent {
        thread: String,
        provider: String,
        /// Absent: the agent's default model.
        #[arg(long)]
        model: Option<String>,
    },
    /// Interrupt the running turn of a thread.
    Stop {
        thread: String,
        /// Also release the provider session.
        #[arg(long)]
        release: bool,
    },
    /// Wait until threads settle (idle, failed or archived) and print each one's
    /// state and last message. Exit code 0: every last turn completed; 1: one failed
    /// or was interrupted; 124: the timeout ran out first.
    Wait {
        #[arg(required = true)]
        threads: Vec<String>,
        /// Return when any of them settles.
        #[arg(long)]
        any: bool,
        /// Give up after this many seconds.
        #[arg(long, value_name = "SECONDS")]
        timeout: Option<u64>,
    },
    /// List threads, oldest first, children under their parents.
    List {
        /// Include archived threads.
        #[arg(long)]
        archived: bool,
        /// Only the children of this thread.
        #[arg(long, value_name = "THREAD")]
        parent: Option<String>,
    },
    /// List the files a thread changed in its environment.
    Changes { thread: String },
    /// Print everything a thread changed in its environment as a unified diff.
    Diff { thread: String },
    /// Put a thread away: release its session and remove its environment. The
    /// timeline stays readable.
    Archive {
        thread: String,
        /// Remove the environment even if uncommitted work is lost.
        #[arg(long)]
        force: bool,
    },
    /// Show a thread and its whole timeline.
    Show { thread: String },
    /// Print a thread's events.
    Events {
        thread: String,
        /// Only events after this sequence number.
        #[arg(long, default_value_t = 0)]
        after: u64,
        /// Keep printing events as they are appended.
        #[arg(long)]
        follow: bool,
    },
}

#[derive(Subcommand)]
enum InteractionCommand {
    /// Answer a pending approval or question.
    Resolve {
        thread: String,
        interaction: String,
        #[command(flatten)]
        answer: Answer,
    },
}

#[derive(Args)]
#[group(required = true, multiple = false)]
struct Answer {
    /// Pick one of the offered options by id.
    #[arg(long)]
    option: Option<String>,
    /// Answer with free text.
    #[arg(long)]
    text: Option<String>,
    /// Decline to answer.
    #[arg(long)]
    cancel: bool,
}

#[derive(Subcommand)]
enum PluginCommand {
    /// List loaded plugins and what they contribute.
    List,
}

#[derive(Subcommand)]
enum ProjectCommand {
    /// List projects and their folders.
    List,
    /// Make a project. Threads outside any project that work in its folders join it.
    Create {
        name: String,
        /// A repository or folder of the project; repeat for several.
        #[arg(long = "folder", required = true)]
        folders: Vec<PathBuf>,
    },
    /// Add a folder to a project.
    AddFolder {
        project: String,
        folder: PathBuf,
    },
    /// Take a folder out of a project.
    RemoveFolder {
        project: String,
        folder: PathBuf,
    },
    Rename {
        project: String,
        name: String,
    },
    /// Delete a project. Its threads stay, outside any project.
    Delete {
        project: String,
    },
}

#[derive(Subcommand)]
enum SettingsCommand {
    /// Print the settings and which file each agent program is.
    Show,
    /// Run this file whenever a plugin runs the program NAME (`claude`, `codex`,
    /// `opencode`); without a path, go back to looking it up on PATH.
    Program { name: String, path: Option<PathBuf> },
    /// The agent new threads get when none is named; `none` to unset.
    DefaultProvider { provider: String },
    /// Defaults and environment of one agent's threads.
    Provider {
        provider: String,
        /// The model new threads get; `default` to unset.
        #[arg(long)]
        model: Option<String>,
        /// The permission mode new threads get; `default` to unset.
        #[arg(long, value_name = "MODE")]
        permission_mode: Option<String>,
        /// A variable every session of this agent sees: NAME=VALUE. Repeatable.
        #[arg(long = "env", value_name = "NAME=VALUE")]
        env: Vec<String>,
        /// Stop setting this variable. Repeatable.
        #[arg(long = "unset-env", value_name = "NAME")]
        unset_env: Vec<String>,
        /// Leave the agent out of the app's pickers (true or false).
        #[arg(long)]
        hidden: Option<bool>,
    },
}

#[derive(Subcommand)]
enum ProviderCommand {
    /// The models a provider's agent offers, for `thread spawn --model`.
    Models {
        provider: String,
        /// Ask the agent again instead of using what the daemon remembers.
        #[arg(long)]
        refresh: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("qcc: {error}");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(run(cli)) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("qcc: {error:#}");
            ExitCode::FAILURE
        }
    }
}

/// Print a line to stdout. A closed pipe (`qcc … | head`) ends the program quietly,
/// the way command-line tools are expected to.
fn out(line: impl AsRef<str>) {
    use std::io::Write;
    let mut stdout = std::io::stdout().lock();
    if writeln!(stdout, "{}", line.as_ref()).and_then(|()| stdout.flush()).is_err() {
        std::process::exit(0);
    }
}

fn print_json(value: &impl Serialize) -> anyhow::Result<()> {
    out(serde_json::to_string(value)?);
    Ok(())
}

/// Exit code of a wait that ran out of time, as `timeout(1)` has it.
const TIMED_OUT: u8 = 124;

async fn run(cli: Cli) -> anyhow::Result<ExitCode> {
    let socket = cli.socket.unwrap_or_else(paths::default_socket);
    let client = Client::connect(&socket).await.context("is qccd running?")?;
    let json = cli.json;

    match cli.command {
        Command::Daemon(DaemonCommand::Status) => {
            let status = client.call::<DaemonStatus>(Empty {}).await?;
            if json {
                print_json(&status)?;
            } else {
                out(format!("qccd {} (pid {})", status.version, status.pid));
                out(format!("socket    {}", status.socket));
                out(format!("data dir  {}", status.data_dir));
            }
        }

        Command::Thread(ThreadCommand::Spawn {
            provider,
            prompt,
            cwd,
            env,
            base,
            model,
            parent,
            project,
            permission_mode,
            follow,
        }) => {
            let project = match project {
                Some(project) => Some(find_project(&client, &project).await?.id),
                None => None,
            };
            let cwd = match cwd {
                Some(cwd) => std::path::absolute(cwd)?,
                None => std::env::current_dir()?,
            };
            // Without a prompt no turn starts, so there is nothing to follow.
            let follow = follow && prompt.is_some();
            let parent = parent.filter(|parent| !parent.is_empty()).map(ThreadId::new);
            let provider = match (provider, &parent) {
                (Some(provider), _) => ProviderId::new(provider),
                // A child takes after its parent.
                (None, Some(parent)) => {
                    client
                        .call::<ThreadGet>(ThreadGetParams { thread_id: parent.clone() })
                        .await?
                        .thread
                        .provider
                }
                (None, None) => {
                    match client.call::<SettingsGet>(Empty {}).await?.settings.default_provider {
                        Some(provider) => provider,
                        None => bail!(
                            "name a provider with --provider (see `qcc plugin list`), or set one with \
                         `qcc settings default-provider`"
                        ),
                    }
                }
            };
            let result = client
                .call::<ThreadSpawn>(ThreadSpawnParams {
                    provider,
                    cwd: cwd.display().to_string(),
                    environment: env,
                    base,
                    prompt,
                    model,
                    parent,
                    project,
                    permission_mode,
                })
                .await?;
            if follow {
                if !json {
                    eprintln!("thread {}", result.thread.id);
                }
                let stream = client.events(result.thread.id, 0, true).await?;
                return follow_turn(stream, json, None).await;
            }
            if json {
                print_json(&result)?;
            } else {
                out(result.thread.id.as_str());
            }
        }

        Command::Thread(ThreadCommand::Send { thread, text, steer, follow }) => {
            let thread_id = ThreadId::new(thread);
            // Read the cursor first so the stream starts at this message.
            let cursor = client
                .call::<ThreadGet>(ThreadGetParams { thread_id: thread_id.clone() })
                .await?
                .thread
                .last_seq;
            let delivery = if steer { SendMode::Steer } else { SendMode::Queue };
            let sent = client
                .call::<ThreadSend>(ThreadSendParams {
                    thread_id: thread_id.clone(),
                    text,
                    delivery,
                })
                .await?;
            if !json {
                match sent.delivery {
                    Delivery::Started => {}
                    Delivery::Steered => eprintln!("steered into the running turn"),
                    Delivery::Queued => eprintln!("queued behind the running turn"),
                }
            }
            if follow {
                let stream = client.events(thread_id, cursor, true).await?;
                // A queued message's turn is not the one running now.
                return follow_turn(stream, json, sent.queue_id).await;
            }
            if json {
                print_json(&sent)?;
            }
        }

        Command::Thread(ThreadCommand::Mode { thread, mode }) => {
            let permission_mode = match mode.as_str() {
                "default" => None,
                mode => Some(mode.parse::<PermissionMode>().map_err(anyhow::Error::msg)?),
            };
            let result = client
                .call::<ThreadSetMode>(ThreadSetModeParams {
                    thread_id: ThreadId::new(thread),
                    permission_mode,
                })
                .await?;
            if json {
                print_json(&result)?;
            }
        }

        Command::Thread(ThreadCommand::Rename { thread, title }) => {
            let params = ThreadRenameParams { thread_id: ThreadId::new(thread), title };
            let result = client.call::<ThreadRename>(params).await?;
            if json {
                print_json(&result)?;
            }
        }

        Command::Thread(ThreadCommand::Project { thread, project }) => {
            let project = match project.as_str() {
                "none" => None,
                project => Some(find_project(&client, project).await?.id),
            };
            let result = client
                .call::<ThreadSetProject>(ThreadSetProjectParams {
                    thread_id: ThreadId::new(thread),
                    project,
                })
                .await?;
            if json {
                print_json(&result)?;
            }
        }

        Command::Project(command) => return project_command(&client, command, json).await,

        Command::Settings(command) => return settings_command(&client, command, json).await,

        Command::Thread(ThreadCommand::Unqueue { thread, queue_id }) => {
            let result = client
                .call::<ThreadUnqueue>(ThreadUnqueueParams {
                    thread_id: ThreadId::new(thread),
                    queue_id,
                })
                .await?;
            if json {
                print_json(&result)?;
            } else {
                out(result.text);
            }
        }

        Command::Thread(ThreadCommand::Agent { thread, provider, model }) => {
            let result = client
                .call::<ThreadSetAgent>(ThreadSetAgentParams {
                    thread_id: ThreadId::new(thread),
                    provider: ProviderId::new(provider),
                    model,
                })
                .await?;
            if json {
                print_json(&result)?;
            }
        }

        Command::Thread(ThreadCommand::Stop { thread, release }) => {
            let mode = if release { StopMode::Release } else { StopMode::Interrupt };
            client
                .call::<ThreadStop>(ThreadStopParams { thread_id: ThreadId::new(thread), mode })
                .await?;
            if json {
                print_json(&Empty {})?;
            }
        }

        Command::Thread(ThreadCommand::Changes { thread }) => {
            let result = client
                .call::<ThreadChanges>(ThreadGetParams { thread_id: ThreadId::new(thread) })
                .await?;
            if json {
                print_json(&result)?;
            } else {
                out(render::changes(&result.changes));
            }
        }

        Command::Thread(ThreadCommand::Diff { thread }) => {
            let result = client
                .call::<ThreadDiff>(ThreadGetParams { thread_id: ThreadId::new(thread) })
                .await?;
            if json {
                print_json(&result)?;
            } else {
                // The patch as it is, so it can be piped into `git apply`.
                use std::io::Write;
                let mut stdout = std::io::stdout().lock();
                if stdout.write_all(result.diff.as_bytes()).and_then(|()| stdout.flush()).is_err() {
                    std::process::exit(0);
                }
                if result.truncated {
                    eprintln!("qcc: the diff is too large and was cut short");
                }
            }
        }

        Command::Thread(ThreadCommand::Archive { thread, force }) => {
            let result = client
                .call::<ThreadArchive>(ThreadArchiveParams {
                    thread_id: ThreadId::new(thread),
                    force,
                })
                .await?;
            if json {
                print_json(&result)?;
            }
        }

        Command::Thread(ThreadCommand::List { archived, parent }) => {
            let mut result =
                client.call::<ThreadList>(ThreadListParams { include_archived: archived }).await?;
            if let Some(parent) = &parent {
                result
                    .threads
                    .retain(|thread| thread.parent.as_ref().map(ThreadId::as_str) == Some(parent));
            }
            if json {
                print_json(&result)?;
            } else if parent.is_some() {
                for thread in &result.threads {
                    out(render::thread_line(thread));
                }
            } else {
                for line in render::thread_tree(&result.threads) {
                    out(line);
                }
            }
        }

        Command::Thread(ThreadCommand::Wait { threads, any, timeout }) => {
            let result = client
                .call::<ThreadWait>(ThreadWaitParams {
                    thread_ids: threads.into_iter().map(ThreadId::new).collect(),
                    any,
                    timeout_ms: timeout.map(|seconds| seconds.saturating_mul(1000)),
                })
                .await?;
            if json {
                print_json(&result)?;
            } else {
                for (index, waited) in result.threads.iter().enumerate() {
                    if index > 0 {
                        out("");
                    }
                    out(render::waited(waited));
                }
            }
            if result.timed_out {
                if !json {
                    eprintln!("qcc: timed out; the threads still working have not settled");
                }
                return Ok(ExitCode::from(TIMED_OUT));
            }
            let bad = result.threads.iter().any(|waited| {
                waited.state == ThreadState::Failed
                    || (waited.state.is_settled()
                        && matches!(
                            waited.thread.last_turn_status,
                            Some(TurnStatus::Failed | TurnStatus::Interrupted)
                        ))
            });
            return Ok(if bad { ExitCode::FAILURE } else { ExitCode::SUCCESS });
        }

        Command::Thread(ThreadCommand::Show { thread }) => {
            let thread_id = ThreadId::new(thread);
            let result =
                client.call::<ThreadGet>(ThreadGetParams { thread_id: thread_id.clone() }).await?;
            let mut stream = client.events(thread_id, 0, false).await?;
            let mut events = Vec::new();
            while let Some(event) = stream.next().await {
                events.push(event);
            }
            if json {
                #[derive(Serialize)]
                struct Shown<'a> {
                    thread: &'a qcc_domain::Thread,
                    events: &'a [ThreadEvent],
                }
                print_json(&Shown { thread: &result.thread, events: &events })?;
            } else {
                out(render::thread_details(&result.thread));
                for event in &events {
                    if let Some(text) = render::event(event) {
                        out(text);
                    }
                }
            }
        }

        Command::Thread(ThreadCommand::Events { thread, after, follow }) => {
            let mut stream = client.events(ThreadId::new(thread), after, follow).await?;
            while let Some(event) = stream.next().await {
                print_event(&event, json)?;
            }
            if follow {
                bail!("connection to qccd lost");
            }
        }

        Command::Interaction(InteractionCommand::Resolve { thread, interaction, answer }) => {
            let resolution = match answer {
                Answer { option: Some(option_id), .. } => {
                    InteractionResolution::Selected { option_id }
                }
                Answer { text: Some(text), .. } => InteractionResolution::Text { text },
                Answer { .. } => InteractionResolution::Cancelled,
            };
            client
                .call::<InteractionResolve>(InteractionResolveParams {
                    thread_id: ThreadId::new(thread),
                    interaction_id: InteractionId::new(interaction),
                    resolution,
                })
                .await?;
            if json {
                print_json(&Empty {})?;
            }
        }

        Command::PluginCommands(CommandCommand::List) => {
            let result = client.call::<CommandList>(Empty {}).await?;
            if json {
                print_json(&result)?;
            } else {
                for command in &result.commands {
                    out(render::command(command));
                }
            }
        }

        Command::External(mut args) => {
            // A global flag after the command name is still the global flag.
            let json = json || args.iter().any(|arg| arg == "--json");
            args.retain(|arg| arg != "--json");
            let params = plugin_command(args)?;
            let result = client.call::<CommandRun>(params).await?;
            if json {
                print_json(&result)?;
            } else {
                out(&result.message);
                if let Some(thread) = &result.thread_id {
                    out(format!("thread {thread}"));
                }
            }
        }

        Command::Plugin(PluginCommand::List) => {
            let result = client.call::<PluginList>(Empty {}).await?;
            if json {
                print_json(&result)?;
            } else {
                for plugin in &result.plugins {
                    out(render::plugin(plugin));
                }
            }
        }

        Command::Provider(ProviderCommand::Models { provider, refresh }) => {
            let params = ProviderModelsParams { provider: ProviderId::new(provider), refresh };
            let result = client.call::<ProviderModels>(params).await?;
            if json {
                print_json(&result)?;
            } else {
                for model in &result.models {
                    let default = if model.is_default { "  (default)" } else { "" };
                    let name = if model.name == model.id { "" } else { model.name.as_str() };
                    out(format!("{:<32} {name}{default}", model.id).trim_end());
                }
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

async fn settings_command(
    client: &Client,
    command: SettingsCommand,
    json: bool,
) -> anyhow::Result<ExitCode> {
    let SettingsResult { mut settings, programs } = client.call::<SettingsGet>(Empty {}).await?;
    match command {
        SettingsCommand::Show => {
            if json {
                print_json(&SettingsResult { settings, programs })?;
                return Ok(ExitCode::SUCCESS);
            }
            if let Some(provider) = &settings.default_provider {
                out(format!("default provider  {provider}"));
            }
            for program in &programs {
                let at = match (&program.path, &program.error) {
                    (Some(path), _) => path.clone(),
                    (None, Some(error)) => format!("({error})"),
                    (None, None) => "(not found)".to_owned(),
                };
                let set =
                    if settings.programs.contains_key(&program.name) { "  (set)" } else { "" };
                out(format!("program  {:<10} {at}{set}", program.name));
            }
            for (provider, defaults) in &settings.providers {
                out(format!("provider {provider}"));
                if let Some(model) = &defaults.model {
                    out(format!("  model            {model}"));
                }
                if let Some(mode) = defaults.permission_mode {
                    out(format!("  permission mode  {}", mode.as_str()));
                }
                for name in defaults.env.keys() {
                    out(format!("  env              {name}"));
                }
                if defaults.hidden {
                    out("  hidden");
                }
            }
            return Ok(ExitCode::SUCCESS);
        }
        SettingsCommand::Program { name, path } => match path {
            Some(path) => {
                settings.programs.insert(name, absolute(path)?);
            }
            None => {
                settings.programs.remove(&name);
            }
        },
        SettingsCommand::DefaultProvider { provider } => {
            settings.default_provider = (provider != "none").then(|| ProviderId::new(provider));
        }
        SettingsCommand::Provider { provider, model, permission_mode, env, unset_env, hidden } => {
            let defaults = settings.providers.entry(ProviderId::new(provider)).or_default();
            if let Some(model) = model {
                defaults.model = (model != "default").then_some(model);
            }
            if let Some(mode) = permission_mode {
                defaults.permission_mode = match mode.as_str() {
                    "default" => None,
                    mode => Some(mode.parse::<PermissionMode>().map_err(anyhow::Error::msg)?),
                };
            }
            for pair in env {
                let (name, value) =
                    pair.split_once('=').with_context(|| format!("{pair:?} is not NAME=VALUE"))?;
                defaults.env.insert(name.to_owned(), value.to_owned());
            }
            for name in unset_env {
                defaults.env.remove(&name);
            }
            if let Some(hidden) = hidden {
                defaults.hidden = hidden;
            }
        }
    }
    settings.providers.retain(|_, defaults| *defaults != ProviderSettings::default());
    let result = client.call::<SettingsSet>(SettingsSetParams { settings }).await?;
    if json {
        print_json(&result)?;
    }
    Ok(ExitCode::SUCCESS)
}

/// A project by id or by exact name.
async fn find_project(client: &Client, wanted: &str) -> anyhow::Result<Project> {
    let ProjectListResult { projects } = client.call::<ProjectList>(Empty {}).await?;
    let mut matches = projects.into_iter().filter(|p| p.id.as_str() == wanted || p.name == wanted);
    let found =
        matches.next().with_context(|| format!("no project {wanted:?}; see `qcc project list`"))?;
    if matches.next().is_some() {
        bail!("several projects are named {wanted:?}; use the id");
    }
    Ok(found)
}

fn absolute(folder: PathBuf) -> anyhow::Result<String> {
    Ok(std::path::absolute(folder)?.display().to_string())
}

async fn project_command(
    client: &Client,
    command: ProjectCommand,
    json: bool,
) -> anyhow::Result<ExitCode> {
    let update = |project: &Project| ProjectUpdateParams {
        id: project.id.clone(),
        name: None,
        folders: None,
    };
    let changed = match command {
        ProjectCommand::List => {
            let result = client.call::<ProjectList>(Empty {}).await?;
            if json {
                print_json(&result)?;
            } else {
                for project in &result.projects {
                    out(format!("{}  {}", project.id, project.name));
                    for folder in &project.folders {
                        out(format!("  {folder}"));
                    }
                }
            }
            return Ok(ExitCode::SUCCESS);
        }
        ProjectCommand::Create { name, folders } => {
            let folders = folders.into_iter().map(absolute).collect::<anyhow::Result<_>>()?;
            client.call::<ProjectCreate>(ProjectCreateParams { name, folders }).await?
        }
        ProjectCommand::AddFolder { project, folder } => {
            let project = find_project(client, &project).await?;
            let mut folders = project.folders.clone();
            folders.push(absolute(folder)?);
            let params = ProjectUpdateParams { folders: Some(folders), ..update(&project) };
            client.call::<ProjectUpdate>(params).await?
        }
        ProjectCommand::RemoveFolder { project, folder } => {
            let project = find_project(client, &project).await?;
            let folder = absolute(folder)?;
            let folders: Vec<String> =
                project.folders.iter().filter(|known| **known != folder).cloned().collect();
            if folders.len() == project.folders.len() {
                bail!("{folder} is not a folder of {}", project.name);
            }
            let params = ProjectUpdateParams { folders: Some(folders), ..update(&project) };
            client.call::<ProjectUpdate>(params).await?
        }
        ProjectCommand::Rename { project, name } => {
            let project = find_project(client, &project).await?;
            let params = ProjectUpdateParams { name: Some(name), ..update(&project) };
            client.call::<ProjectUpdate>(params).await?
        }
        ProjectCommand::Delete { project } => {
            let project = find_project(client, &project).await?;
            client.call::<ProjectDelete>(ProjectDeleteParams { id: project.id }).await?;
            return Ok(ExitCode::SUCCESS);
        }
    };
    if json {
        print_json(&changed)?;
    } else {
        out(format!("{}  {}", changed.project.id, changed.project.name));
    }
    Ok(ExitCode::SUCCESS)
}

/// `<plugin> <command> [--thread <id>] [--<name> <value> | --<name>=<value>]…`
fn plugin_command(args: Vec<String>) -> anyhow::Result<CommandRunParams> {
    let mut args = args.into_iter();
    let (Some(plugin), Some(command)) = (args.next(), args.next()) else {
        bail!("run a plugin command as `qcc <plugin> <command>`; `qcc command list` shows them");
    };
    let mut thread_id = std::env::var(paths::THREAD_ID_ENV).ok().filter(|id| !id.is_empty());
    let mut values = BTreeMap::new();
    while let Some(arg) = args.next() {
        let Some(name) = arg.strip_prefix("--") else {
            bail!("unexpected {arg:?}: give values as --<name> <value>");
        };
        let (name, value) = match name.split_once('=') {
            Some((name, value)) => (name.to_owned(), value.to_owned()),
            None => {
                let value = args.next().with_context(|| format!("--{name} needs a value"))?;
                (name.to_owned(), value)
            }
        };
        if name == "thread" {
            thread_id = Some(value);
        } else {
            values.insert(name, value);
        }
    }
    Ok(CommandRunParams { plugin, command, thread_id: thread_id.map(ThreadId::new), args: values })
}

fn print_event(event: &ThreadEvent, json: bool) -> anyhow::Result<()> {
    if json {
        print_json(event)?;
    } else if let Some(text) = render::event(event) {
        out(text);
    }
    Ok(())
}

/// Print events until the turn settles. The exit code says how it went, so scripts
/// and agents can branch on it.
async fn follow_turn(
    mut stream: EventStream,
    json: bool,
    queue_id: Option<String>,
) -> anyhow::Result<ExitCode> {
    // For a queued message, turns that settle before it is delivered are not its own.
    let mut waiting_for = queue_id;
    while let Some(event) = stream.next().await {
        print_event(&event, json)?;
        match &event.payload {
            EventPayload::InputSubmitted { queue_id: Some(delivered), .. }
                if waiting_for.as_ref() == Some(delivered) =>
            {
                waiting_for = None;
            }
            EventPayload::InputDropped { queue_id: dropped }
                if waiting_for.as_ref() == Some(dropped) =>
            {
                stream.close().await;
                return Ok(ExitCode::FAILURE);
            }
            EventPayload::TurnCompleted { .. } if waiting_for.is_some() => {}
            EventPayload::TurnCompleted { status, .. } => {
                stream.close().await;
                return Ok(match status {
                    TurnStatus::Completed => ExitCode::SUCCESS,
                    TurnStatus::Failed | TurnStatus::Interrupted => ExitCode::FAILURE,
                });
            }
            // The session never got as far as a turn.
            EventPayload::ThreadStatusChanged { status: ThreadStatus::Failed }
            | EventPayload::SessionEnded { .. } => {
                stream.close().await;
                return Ok(ExitCode::FAILURE);
            }
            _ => {}
        }
    }
    bail!("connection to qccd lost")
}

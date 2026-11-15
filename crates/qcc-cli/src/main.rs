//! qcc: the command-line client of qccd.
//!
//! It is the same interface for people and for agents: every command takes `--json`,
//! and a `qcc thread spawn` run from inside an agent session records that session's
//! thread as the parent.

mod render;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, bail};
use clap::{Args, Parser, Subcommand};
use qcc_client::protocol::methods::{
    DaemonStatus, Delivery, Empty, InteractionResolve, InteractionResolveParams, PluginList,
    SendMode, StopMode, ThreadGet, ThreadGetParams, ThreadList, ThreadSend, ThreadSendParams,
    ThreadSetMode, ThreadSetModeParams, ThreadSpawn, ThreadSpawnParams, ThreadStop,
    ThreadStopParams,
};
use qcc_client::protocol::paths;
use qcc_client::{Client, EventStream};
use qcc_domain::{
    EventPayload, InteractionId, InteractionResolution, PermissionMode, ProviderId, ThreadEvent,
    ThreadId, ThreadStatus, TurnStatus,
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
        /// Provider id, as listed by `qcc plugin list`.
        #[arg(long)]
        provider: String,
        /// The first message to the agent.
        #[arg(long)]
        prompt: Option<String>,
        /// Working directory of the thread [default: the current directory].
        #[arg(long)]
        cwd: Option<PathBuf>,
        #[arg(long)]
        model: Option<String>,
        /// Parent thread [default: $QCC_THREAD_ID, set inside agent sessions].
        #[arg(long, env = paths::THREAD_ID_ENV)]
        parent: Option<String>,
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
    /// Interrupt the running turn of a thread.
    Stop {
        thread: String,
        /// Also release the provider session.
        #[arg(long)]
        release: bool,
    },
    /// List threads, oldest first.
    List,
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
            model,
            parent,
            permission_mode,
            follow,
        }) => {
            let cwd = match cwd {
                Some(cwd) => std::path::absolute(cwd)?,
                None => std::env::current_dir()?,
            };
            // Without a prompt no turn starts, so there is nothing to follow.
            let follow = follow && prompt.is_some();
            let result = client
                .call::<ThreadSpawn>(ThreadSpawnParams {
                    provider: ProviderId::new(provider),
                    cwd: cwd.display().to_string(),
                    prompt,
                    model,
                    parent: parent.map(ThreadId::new),
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

        Command::Thread(ThreadCommand::Stop { thread, release }) => {
            let mode = if release { StopMode::Release } else { StopMode::Interrupt };
            client
                .call::<ThreadStop>(ThreadStopParams { thread_id: ThreadId::new(thread), mode })
                .await?;
            if json {
                print_json(&Empty {})?;
            }
        }

        Command::Thread(ThreadCommand::List) => {
            let result = client.call::<ThreadList>(Empty {}).await?;
            if json {
                print_json(&result)?;
            } else {
                for thread in &result.threads {
                    out(render::thread_line(thread));
                }
            }
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
    }
    Ok(ExitCode::SUCCESS)
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

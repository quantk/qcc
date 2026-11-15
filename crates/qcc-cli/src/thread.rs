//! `qcc thread`: create, drive and inspect threads.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::bail;
use clap::Subcommand;
use qcc_client::protocol::methods::{
    Delivery, Empty, SendMode, SettingsGet, StopMode, ThreadArchive, ThreadArchiveParams,
    ThreadChanges, ThreadDeliver, ThreadDeliverParams, ThreadDiff, ThreadGet, ThreadGetParams,
    ThreadIsolate, ThreadIsolateParams, ThreadList, ThreadListParams, ThreadRename,
    ThreadRenameParams, ThreadSend, ThreadSendParams, ThreadSetAgent, ThreadSetAgentParams,
    ThreadSetMcp, ThreadSetMcpParams, ThreadSetMode, ThreadSetModeParams, ThreadSetParent,
    ThreadSetParentParams, ThreadSetProject, ThreadSetProjectParams, ThreadSpawn,
    ThreadSpawnParams, ThreadStop, ThreadStopParams, ThreadUnqueue, ThreadUnqueueParams,
    ThreadWait, ThreadWaitParams,
};
use qcc_client::protocol::paths;
use qcc_client::{Client, EventStream};
use qcc_domain::{
    DeliverMode, EventPayload, PermissionMode, ProviderId, ThreadEvent, ThreadId, ThreadState,
    ThreadStatus, TurnStatus,
};
use serde::Serialize;

use crate::project::find_project;
use crate::{out, print_json, render};

#[derive(Subcommand)]
pub(crate) enum ThreadCommand {
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
    /// The MCP servers of a thread's session and the ones it keeps off; with --off
    /// and --on, turn servers off or on for this thread alone (not the threads it
    /// starts). Names are those `qcc thread show` lists or `qcc mcp list` sets up.
    Mcp {
        thread: String,
        /// Keep this server off in the thread. Repeatable.
        #[arg(long = "off", value_name = "NAME")]
        off: Vec<String>,
        /// Turn this server on again. Repeatable.
        #[arg(long = "on", value_name = "NAME")]
        on: Vec<String>,
    },
    /// Name a thread.
    Rename { thread: String, title: String },
    /// Bring what a thread did in its worktree back to the directory it was made
    /// from: as uncommitted changes, or with --merge as a merge of its branch.
    Deliver {
        thread: String,
        #[arg(long)]
        merge: bool,
    },
    /// Move a thread that works in its directory into a worktree of its own, between
    /// turns. With --carry its uncommitted changes go along and leave the directory.
    Isolate {
        thread: String,
        #[arg(long, value_name = "ENVIRONMENT", default_value = "git-worktree")]
        env: String,
        #[arg(long)]
        carry: bool,
    },
    /// Put a thread into a project (id or name), or take it out with `none`.
    Project { thread: String, project: String },
    /// Place a thread under another thread, or let it stand on its own with `none`.
    Parent { thread: String, parent: String },
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

/// Exit code of a wait that ran out of time, as `timeout(1)` has it.
const TIMED_OUT: u8 = 124;

pub(crate) async fn run(
    client: &Arc<Client>,
    command: ThreadCommand,
    json: bool,
) -> anyhow::Result<ExitCode> {
    match command {
        ThreadCommand::Spawn {
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
        } => {
            let project = match project {
                Some(project) => Some(find_project(client, &project).await?.id),
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

        ThreadCommand::Send { thread, text, steer, follow } => {
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

        ThreadCommand::Mode { thread, mode } => {
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

        ThreadCommand::Mcp { thread, off, on } => {
            let thread_id = ThreadId::new(thread);
            let mut thread = client
                .call::<ThreadGet>(ThreadGetParams { thread_id: thread_id.clone() })
                .await?
                .thread;
            let switches = off.into_iter().map(|name| (name, false));
            for (server, enabled) in switches.chain(on.into_iter().map(|name| (name, true))) {
                let params = ThreadSetMcpParams { thread_id: thread_id.clone(), server, enabled };
                thread = client.call::<ThreadSetMcp>(params).await?.thread;
            }
            if json {
                print_json(&serde_json::json!({
                    "mcp_servers": thread.mcp_servers,
                    "mcp_disabled": thread.mcp_disabled,
                }))?;
            } else if thread.mcp_servers.is_empty() && thread.mcp_disabled.is_empty() {
                out("no MCP servers reported, none kept off");
            } else {
                out(render::mcp_servers(&thread));
            }
        }

        ThreadCommand::Deliver { thread, merge } => {
            let mode = if merge { DeliverMode::Merge } else { DeliverMode::Apply };
            let params = ThreadDeliverParams { thread_id: ThreadId::new(thread), mode };
            let result = client.call::<ThreadDeliver>(params).await?;
            if json {
                print_json(&result)?;
            } else {
                out(result.message);
            }
        }

        ThreadCommand::Isolate { thread, env, carry } => {
            let params = ThreadIsolateParams {
                thread_id: ThreadId::new(thread),
                environment: env,
                carry_changes: carry,
            };
            let result = client.call::<ThreadIsolate>(params).await?;
            if json {
                print_json(&result)?;
            }
        }

        ThreadCommand::Rename { thread, title } => {
            let params = ThreadRenameParams { thread_id: ThreadId::new(thread), title };
            let result = client.call::<ThreadRename>(params).await?;
            if json {
                print_json(&result)?;
            }
        }

        ThreadCommand::Project { thread, project } => {
            let project = match project.as_str() {
                "none" => None,
                project => Some(find_project(client, project).await?.id),
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

        ThreadCommand::Parent { thread, parent } => {
            let parent = (parent != "none").then(|| ThreadId::new(parent));
            let result = client
                .call::<ThreadSetParent>(ThreadSetParentParams {
                    thread_id: ThreadId::new(thread),
                    parent,
                })
                .await?;
            if json {
                print_json(&result)?;
            }
        }

        ThreadCommand::Unqueue { thread, queue_id } => {
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

        ThreadCommand::Agent { thread, provider, model } => {
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

        ThreadCommand::Stop { thread, release } => {
            let mode = if release { StopMode::Release } else { StopMode::Interrupt };
            client
                .call::<ThreadStop>(ThreadStopParams { thread_id: ThreadId::new(thread), mode })
                .await?;
            if json {
                print_json(&Empty {})?;
            }
        }

        ThreadCommand::Changes { thread } => {
            let result = client
                .call::<ThreadChanges>(ThreadGetParams { thread_id: ThreadId::new(thread) })
                .await?;
            if json {
                print_json(&result)?;
            } else {
                out(render::changes(&result.changes));
            }
        }

        ThreadCommand::Diff { thread } => {
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

        ThreadCommand::Archive { thread, force } => {
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

        ThreadCommand::List { archived, parent } => {
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

        ThreadCommand::Wait { threads, any, timeout } => {
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

        ThreadCommand::Show { thread } => {
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

        ThreadCommand::Events { thread, after, follow } => {
            let mut stream = client.events(ThreadId::new(thread), after, follow).await?;
            while let Some(event) = stream.next().await {
                print_event(&event, json)?;
            }
            if follow {
                bail!("connection to qccd lost");
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

pub(crate) fn print_event(event: &ThreadEvent, json: bool) -> anyhow::Result<()> {
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

//! qcc: the command-line client of qccd.
//!
//! It is the same interface for people and for agents: every command takes `--json`,
//! and a `qcc thread spawn` run from inside an agent session records that session's
//! thread as the parent. Plugin commands run as `qcc <plugin> <command>`.

mod automation;
mod comment;
mod mcp;
mod project;
mod render;
mod settings;
mod skill;
mod thread;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, bail};
use clap::{Args, Parser, Subcommand};
use qcc_client::Client;
use qcc_client::protocol::methods::{
    CommandList, CommandRun, CommandRunParams, DaemonStatus, Empty, InteractionResolve,
    InteractionResolveParams, PluginDisable, PluginDisableParams, PluginEnable, PluginEnableParams,
    PluginList, ProviderModels, ProviderModelsParams, ProviderSkills, ProviderSkillsParams,
    SettingsGet, SettingsSet, SettingsSetParams,
};
use qcc_client::protocol::paths;
use qcc_domain::{InteractionId, InteractionResolution, ProviderId, ThreadId};
use serde::Serialize;

use automation::AutomationCommand;
use comment::CommentCommand;
use mcp::McpCommand;
use project::ProjectCommand;
use settings::SettingsCommand;
use skill::SkillCommand;
use thread::ThreadCommand;

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
    /// Comments on lines of a thread's changes: left by the user, dealt with by
    /// the agent.
    #[command(subcommand)]
    Comment(CommentCommand),
    /// Answer approvals and questions agents are waiting on.
    #[command(subcommand)]
    Interaction(InteractionCommand),
    /// Inspect plugins.
    #[command(subcommand)]
    Plugin(PluginCommand),
    /// Group threads by project: named sets of repositories and folders.
    #[command(subcommand)]
    Project(ProjectCommand),
    /// Prompts the daemon sends an agent on a schedule: every hour, every weekday
    /// at nine, …
    #[command(subcommand)]
    Automation(AutomationCommand),
    /// Show and change the daemon's settings: paths of agent programs, defaults and
    /// environment of each agent.
    #[command(subcommand)]
    Settings(SettingsCommand),
    /// MCP servers every agent session gets on top of those of the agent's own
    /// configuration.
    #[command(subcommand)]
    Mcp(McpCommand),
    /// Skills plugins offer agents: instructions an agent reads when a task needs them.
    #[command(subcommand)]
    Skill(SkillCommand),
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
    /// List installed plugins, whether each is on, and what it contributes.
    List,
    /// Turn a plugin on and start it, now and on later starts of the daemon.
    Enable { plugin: String },
    /// Turn a plugin off and stop it, now and on later starts of the daemon. Its
    /// agents, environments, commands and skills go away; threads made with them
    /// stay readable.
    Disable {
        plugin: String,
        /// Even while its agents are working: their turns are cut short.
        #[arg(long)]
        force: bool,
    },
    /// A plugin's own settings and their values.
    Settings { plugin: String },
    /// Set one of a plugin's own settings; without a value, go back to its default.
    Set { plugin: String, setting: String, value: Option<String> },
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
    /// The skills a provider's agent finds by itself (its own folders, its plugins),
    /// as the agent reports them. qcc shows them; the agent's configuration decides.
    Skills {
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

async fn run(cli: Cli) -> anyhow::Result<ExitCode> {
    let socket = cli.socket.unwrap_or_else(paths::default_socket);
    let client = Client::connect(&socket).await.context("is qccd running?")?;
    let json = cli.json;

    match cli.command {
        Command::Thread(command) => return thread::run(&client, command, json).await,

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

        Command::Project(command) => return project::run(&client, command, json).await,

        Command::Automation(command) => return automation::run(&client, command, json).await,

        Command::Comment(command) => return comment::run(&client, command, json).await,

        Command::Settings(command) => return settings::run(&client, command, json).await,

        Command::Mcp(command) => return mcp::run(&client, command, json).await,

        Command::Skill(command) => return skill::run(&client, command, json).await,

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

        Command::Plugin(PluginCommand::Settings { plugin }) => {
            let plugins = client.call::<PluginList>(Empty {}).await?;
            let Some(info) = plugins.plugins.into_iter().find(|info| info.id == plugin) else {
                bail!("no plugin {plugin:?} is loaded; `qcc plugin list` lists them");
            };
            let settings = client.call::<SettingsGet>(Empty {}).await?.settings;
            let set = settings.plugins.get(&plugin);
            if json {
                print_json(&serde_json::json!({ "fields": info.settings, "values": set }))?;
                return Ok(ExitCode::SUCCESS);
            }
            if info.settings.is_empty() {
                out(format!("plugin {plugin} has no settings"));
            }
            for field in &info.settings {
                let value = match set.and_then(|set| set.get(&field.id)) {
                    Some(value) => value.clone(),
                    None => match &field.default {
                        Some(default) => format!("{default}  (default)"),
                        None => "(not set)".to_owned(),
                    },
                };
                out(format!("{:<20} {value}", field.id));
                let about = field.description.as_deref().unwrap_or(&field.label);
                out(format!("{:<20} {about} [{}]", "", render::setting_kind(&field.kind)));
            }
        }

        Command::Plugin(PluginCommand::Set { plugin, setting, value }) => {
            let mut settings = client.call::<SettingsGet>(Empty {}).await?.settings;
            let values = settings.plugins.entry(plugin.clone()).or_default();
            match value {
                Some(value) => {
                    values.insert(setting, value);
                }
                None => {
                    values.remove(&setting);
                }
            }
            if values.is_empty() {
                settings.plugins.remove(&plugin);
            }
            let result = client.call::<SettingsSet>(SettingsSetParams { settings }).await?;
            if json {
                print_json(&result.settings.plugins.get(&plugin))?;
            }
        }

        Command::Plugin(PluginCommand::Enable { plugin }) => {
            let result = client.call::<PluginEnable>(PluginEnableParams { id: plugin }).await?;
            if json {
                print_json(&result)?;
            } else {
                out(render::plugin(&result.plugin));
            }
        }

        Command::Plugin(PluginCommand::Disable { plugin, force }) => {
            let params = PluginDisableParams { id: plugin, force };
            let result = client.call::<PluginDisable>(params).await?;
            if json {
                print_json(&result)?;
            } else {
                out(render::plugin(&result.plugin));
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

        Command::Provider(ProviderCommand::Skills { provider, refresh }) => {
            let params = ProviderSkillsParams { provider: ProviderId::new(provider), refresh };
            let result = client.call::<ProviderSkills>(params).await?;
            if json {
                print_json(&result)?;
            } else if result.skills.is_empty() {
                out("the agent reports no skills of its own");
            } else {
                for skill in &result.skills {
                    let source =
                        skill.source.as_deref().map(|s| format!("  ({s})")).unwrap_or_default();
                    out(format!("{}{source}", skill.name));
                    let about = skill.path.as_deref().unwrap_or(&skill.description);
                    out(format!("    {about}"));
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
                    if !model.efforts.is_empty() {
                        let levels: Vec<String> = model
                            .efforts
                            .iter()
                            .map(|level| match &model.default_effort {
                                Some(default) if default == level => format!("{level} (default)"),
                                _ => level.clone(),
                            })
                            .collect();
                        out(format!("{:<32} effort: {}", "", levels.join(", ")));
                    }
                }
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn absolute(folder: PathBuf) -> anyhow::Result<String> {
    Ok(std::path::absolute(folder)?.display().to_string())
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

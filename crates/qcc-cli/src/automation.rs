//! `qcc automation`: prompts the daemon sends an agent on a schedule.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, bail};
use clap::{Args, Subcommand, ValueEnum};
use qcc_client::Client;
use qcc_client::protocol::methods::{
    AutomationCreate, AutomationDelete, AutomationEntry, AutomationIdParams, AutomationList,
    AutomationResult, AutomationRunNow, AutomationRuns, AutomationRunsParams, AutomationSetEnabled,
    AutomationSetEnabledParams, AutomationUpdate, AutomationUpdateParams, Empty, SettingsGet,
};
use qcc_domain::{
    AutomationOutcome, AutomationOverlap, AutomationRun, AutomationSpec, AutomationThreadMode,
    AutomationTrigger, PermissionMode, ProviderId,
};

use crate::{absolute, out, print_json};

#[derive(Subcommand)]
pub(crate) enum AutomationCommand {
    /// List automations, when each runs next and how its last run went.
    List,
    /// Make an automation: on its schedule the daemon starts a thread with the prompt.
    Add {
        name: String,
        #[command(flatten)]
        options: Options,
    },
    /// An automation's settings and its latest runs.
    Show {
        automation: String,
        /// How many runs to show.
        #[arg(long, default_value_t = 10)]
        runs: u32,
    },
    /// Change an automation; what is not given stays as it is.
    Edit {
        automation: String,
        /// A new name.
        #[arg(long)]
        name: Option<String>,
        #[command(flatten)]
        options: Options,
    },
    /// Switch an automation on: its schedule counts from now.
    Enable { automation: String },
    /// Switch an automation off; it can still be run by hand.
    Disable { automation: String },
    /// Delete an automation and its history. The threads it started stay.
    Remove { automation: String },
    /// Run an automation now, whether it is on or not. Its schedule goes on as before.
    Run { automation: String },
}

/// What an automation is; for `add`, `--schedule` and `--prompt` are required.
#[derive(Args)]
pub(crate) struct Options {
    /// When it runs: a cron expression (`0 9 * * 1-5`, in local time), @hourly,
    /// @daily, @weekly, @monthly, @yearly, or an interval (`every 30m`, `every 2h`).
    #[arg(long)]
    schedule: Option<String>,
    /// The message each run sends the agent.
    #[arg(long)]
    prompt: Option<String>,
    /// Provider id [default: the default provider of `qcc settings`].
    #[arg(long)]
    provider: Option<String>,
    /// Directory the agent works in [default: the current directory].
    #[arg(long)]
    cwd: Option<PathBuf>,
    /// Give each new thread a directory of its own made by this environment
    /// (`git-worktree`). `--env none` drops it.
    #[arg(long, value_name = "ENVIRONMENT")]
    env: Option<String>,
    /// `--model default` goes back to the agent's default.
    #[arg(long)]
    model: Option<String>,
    /// `--effort default` goes back to the model's default.
    #[arg(long, value_name = "LEVEL")]
    effort: Option<String>,
    /// ask, accept-edits, auto or full; `default` for the agent's own configuration.
    /// Nobody watches a scheduled run: in `ask`, it waits for you.
    #[arg(long, value_name = "MODE")]
    permission_mode: Option<String>,
    /// `new`: every run is a thread of its own; `continue`: every run is a message to
    /// one thread, which the first run starts [default: new].
    #[arg(long, value_enum, value_name = "MODE")]
    threads: Option<ThreadsArg>,
    /// When a run comes due while the previous one still works: skip it, or start
    /// it anyway [default: skip].
    #[arg(long, value_enum, value_name = "POLICY")]
    overlap: Option<OverlapArg>,
    /// Make up for a run missed while the daemon was not running, once
    /// [default: no; `--catch-up false` turns it off].
    #[arg(long, num_args = 0..=1, default_missing_value = "true", value_name = "BOOL")]
    catch_up: Option<bool>,
    /// Make it switched off (`add`).
    #[arg(long)]
    disabled: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum ThreadsArg {
    New,
    Continue,
}

#[derive(Clone, Copy, ValueEnum)]
enum OverlapArg {
    Skip,
    Allow,
}

/// `default` and `none` clear an optional value.
fn optional(value: Option<String>, current: Option<String>) -> Option<String> {
    match value {
        Some(value) if value == "default" || value == "none" || value.is_empty() => None,
        Some(value) => Some(value),
        None => current,
    }
}

fn permission_mode(
    value: Option<String>,
    current: Option<PermissionMode>,
) -> anyhow::Result<Option<PermissionMode>> {
    match value.as_deref() {
        None => Ok(current),
        Some("default") => Ok(None),
        Some(mode) => PermissionMode::ALL
            .into_iter()
            .find(|known| known.as_str() == mode)
            .map(Some)
            .with_context(|| format!("unknown permission mode {mode:?}")),
    }
}

impl Options {
    /// Fold the options given into an automation's settings.
    fn apply(self, mut spec: AutomationSpec) -> anyhow::Result<AutomationSpec> {
        if let Some(schedule) = self.schedule {
            spec.schedule = schedule;
        }
        if let Some(prompt) = self.prompt {
            spec.prompt = prompt;
        }
        if let Some(provider) = self.provider {
            spec.provider = ProviderId::new(provider);
        }
        if let Some(cwd) = self.cwd {
            spec.cwd = absolute(cwd)?;
        }
        spec.environment = optional(self.env, spec.environment);
        spec.model = optional(self.model, spec.model);
        spec.effort = optional(self.effort, spec.effort);
        spec.permission_mode = permission_mode(self.permission_mode, spec.permission_mode)?;
        if let Some(threads) = self.threads {
            spec.thread_mode = match threads {
                ThreadsArg::New => AutomationThreadMode::New,
                ThreadsArg::Continue => AutomationThreadMode::Continue,
            };
        }
        if let Some(overlap) = self.overlap {
            spec.overlap = match overlap {
                OverlapArg::Skip => AutomationOverlap::Skip,
                OverlapArg::Allow => AutomationOverlap::Allow,
            };
        }
        if let Some(catch_up) = self.catch_up {
            spec.catch_up = catch_up;
        }
        Ok(spec)
    }
}

pub(crate) async fn run(
    client: &Client,
    command: AutomationCommand,
    json: bool,
) -> anyhow::Result<ExitCode> {
    let changed = match command {
        AutomationCommand::List => {
            let result = client.call::<AutomationList>(Empty {}).await?;
            if json {
                print_json(&result)?;
            } else if result.automations.is_empty() {
                out("no automations; make one with `qcc automation add`");
            } else {
                for entry in &result.automations {
                    out(summary(entry));
                }
            }
            return Ok(ExitCode::SUCCESS);
        }
        AutomationCommand::Add { name, options } => {
            let (Some(schedule), Some(prompt)) = (options.schedule.clone(), options.prompt.clone())
            else {
                bail!("an automation needs --schedule and --prompt");
            };
            let provider = match &options.provider {
                Some(provider) => ProviderId::new(provider),
                None => match client.call::<SettingsGet>(Empty {}).await?.settings.default_provider
                {
                    Some(provider) => provider,
                    None => bail!(
                        "name a provider with --provider (see `qcc plugin list`), or set one \
                         with `qcc settings default-provider`"
                    ),
                },
            };
            let enabled = !options.disabled;
            let spec = AutomationSpec {
                name,
                schedule,
                prompt,
                provider,
                cwd: std::env::current_dir()?.display().to_string(),
                environment: None,
                model: None,
                effort: None,
                permission_mode: None,
                thread_mode: AutomationThreadMode::New,
                overlap: AutomationOverlap::Skip,
                catch_up: false,
                enabled,
            };
            let spec = options.apply(spec)?;
            client.call::<AutomationCreate>(spec).await?.automation
        }
        AutomationCommand::Show { automation, runs } => {
            let entry = find(client, &automation).await?;
            let params =
                AutomationRunsParams { id: entry.automation.id.clone(), limit: Some(runs) };
            let runs = client.call::<AutomationRuns>(params).await?.runs;
            if json {
                print_json(&serde_json::json!({ "automation": entry, "runs": runs }))?;
            } else {
                out(details(&entry));
                if runs.is_empty() {
                    out("no runs yet");
                } else {
                    out("runs");
                    for run in &runs {
                        out(format!("  {}", run_line(run)));
                    }
                }
            }
            return Ok(ExitCode::SUCCESS);
        }
        AutomationCommand::Edit { automation, name, options } => {
            let entry = find(client, &automation).await?;
            let mut spec = entry.automation.spec.clone();
            if let Some(name) = name {
                spec.name = name;
            }
            if options.disabled {
                spec.enabled = false;
            }
            let spec = options.apply(spec)?;
            let params = AutomationUpdateParams { id: entry.automation.id, spec };
            client.call::<AutomationUpdate>(params).await?.automation
        }
        AutomationCommand::Enable { automation } => set_enabled(client, &automation, true).await?,
        AutomationCommand::Disable { automation } => {
            set_enabled(client, &automation, false).await?
        }
        AutomationCommand::Remove { automation } => {
            let entry = find(client, &automation).await?;
            let id = entry.automation.id;
            client.call::<AutomationDelete>(AutomationIdParams { id: id.clone() }).await?;
            if json {
                print_json(&Empty {})?;
            } else {
                out(format!("removed {id}"));
            }
            return Ok(ExitCode::SUCCESS);
        }
        AutomationCommand::Run { automation } => {
            let entry = find(client, &automation).await?;
            let id = entry.automation.id;
            let result = client.call::<AutomationRunNow>(AutomationIdParams { id }).await?;
            if json {
                print_json(&result)?;
            } else {
                out(run_line(&result.run));
            }
            return Ok(match result.run.outcome {
                AutomationOutcome::Failed => ExitCode::FAILURE,
                _ => ExitCode::SUCCESS,
            });
        }
    };
    if json {
        print_json(&AutomationResult { automation: changed })?;
    } else {
        out(summary(&changed));
    }
    Ok(ExitCode::SUCCESS)
}

async fn set_enabled(
    client: &Client,
    automation: &str,
    enabled: bool,
) -> anyhow::Result<AutomationEntry> {
    let entry = find(client, automation).await?;
    let params = AutomationSetEnabledParams { id: entry.automation.id, enabled };
    Ok(client.call::<AutomationSetEnabled>(params).await?.automation)
}

/// An automation by id or by exact name.
async fn find(client: &Client, wanted: &str) -> anyhow::Result<AutomationEntry> {
    let automations = client.call::<AutomationList>(Empty {}).await?.automations;
    let mut matches = automations.into_iter().filter(|entry| {
        entry.automation.id.as_str() == wanted || entry.automation.spec.name == wanted
    });
    let found = matches
        .next()
        .with_context(|| format!("no automation {wanted:?}; see `qcc automation list`"))?;
    if matches.next().is_some() {
        bail!("several automations are named {wanted:?}; use the id");
    }
    Ok(found)
}

/// A time in the local time zone, to the minute.
fn local_time(ms: u64) -> String {
    let Ok(at) = jiff::Timestamp::from_millisecond(ms.cast_signed()) else {
        return ms.to_string();
    };
    at.to_zoned(jiff::tz::TimeZone::system()).strftime("%Y-%m-%d %H:%M").to_string()
}

fn summary(entry: &AutomationEntry) -> String {
    let automation = &entry.automation;
    let next = match entry.next_run_at {
        Some(at) => format!("next {}", local_time(at)),
        None if !automation.spec.enabled => "off".to_owned(),
        None => "no next run".to_owned(),
    };
    let last = entry
        .last_run
        .as_ref()
        .map(|run| format!(", last {}", outcome(run.outcome)))
        .unwrap_or_default();
    format!(
        "{}  {}  [{}]  {next}{last}",
        automation.id, automation.spec.name, automation.spec.schedule
    )
}

fn details(entry: &AutomationEntry) -> String {
    let automation = &entry.automation;
    let spec = &automation.spec;
    let mut lines = vec![
        format!("automation {}", automation.id),
        format!("name       {}", spec.name),
        format!("schedule   {}", spec.schedule),
        format!("enabled    {}", if spec.enabled { "yes" } else { "no" }),
    ];
    if let Some(at) = entry.next_run_at {
        lines.push(format!("next run   {}", local_time(at)));
    }
    lines.push(format!("provider   {}", spec.provider));
    lines.push(format!("cwd        {}", spec.cwd));
    if let Some(environment) = &spec.environment {
        lines.push(format!("env        {environment}"));
    }
    if let Some(model) = &spec.model {
        lines.push(format!("model      {model}"));
    }
    if let Some(effort) = &spec.effort {
        lines.push(format!("effort     {effort}"));
    }
    if let Some(mode) = spec.permission_mode {
        lines.push(format!("mode       {}", mode.as_str()));
    }
    let threads = match spec.thread_mode {
        AutomationThreadMode::New => "a new thread for every run".to_owned(),
        AutomationThreadMode::Continue => match &automation.thread_id {
            Some(thread) => format!("every run continues thread {thread}"),
            None => "every run continues one thread".to_owned(),
        },
    };
    lines.push(format!("threads    {threads}"));
    let overlap = match spec.overlap {
        AutomationOverlap::Skip => "skipped while the previous run works",
        AutomationOverlap::Allow => "started even while the previous run works",
    };
    lines.push(format!("overlap    {overlap}"));
    let missed = if spec.catch_up { "made up for once" } else { "skipped" };
    lines.push(format!("missed     {missed}"));
    lines.push(String::new());
    lines.push(spec.prompt.clone());
    lines.push(String::new());
    lines.join("\n")
}

fn outcome(outcome: AutomationOutcome) -> &'static str {
    match outcome {
        AutomationOutcome::Started => "started",
        AutomationOutcome::Skipped => "skipped",
        AutomationOutcome::Failed => "failed",
    }
}

fn run_line(run: &AutomationRun) -> String {
    let trigger = match run.trigger {
        AutomationTrigger::Schedule => "scheduled",
        AutomationTrigger::Missed => "missed",
        AutomationTrigger::Manual => "manual",
    };
    let mut line = format!("{}  {trigger:<9}  {:<7}", local_time(run.at), outcome(run.outcome));
    if let Some(thread) = &run.thread_id {
        line.push_str(&format!("  {thread}"));
    }
    if let Some(reason) = &run.reason {
        line.push_str(&format!("  {reason}"));
    }
    line
}

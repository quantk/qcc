//! `qcc settings`: the daemon settings.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Context;
use clap::Subcommand;
use qcc_client::Client;
use qcc_client::protocol::methods::{
    Empty, ProviderSettings, SettingsGet, SettingsResult, SettingsSet, SettingsSetParams,
};
use qcc_domain::{PermissionMode, ProviderId};

use crate::{absolute, out, print_json};

#[derive(Subcommand)]
pub(crate) enum SettingsCommand {
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
        /// The effort new threads get; `default` to unset.
        #[arg(long, value_name = "LEVEL")]
        effort: Option<String>,
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

pub(crate) async fn run(
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
                if let Some(effort) = &defaults.effort {
                    out(format!("  effort           {effort}"));
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
        SettingsCommand::Provider {
            provider,
            model,
            effort,
            permission_mode,
            env,
            unset_env,
            hidden,
        } => {
            let defaults = settings.providers.entry(ProviderId::new(provider)).or_default();
            if let Some(model) = model {
                defaults.model = (model != "default").then_some(model);
            }
            if let Some(effort) = effort {
                defaults.effort = (effort != "default").then_some(effort);
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

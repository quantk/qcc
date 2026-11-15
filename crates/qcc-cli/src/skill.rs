//! `qcc skill`: the skills plugins offer agents, and turning them off for all.

use std::process::ExitCode;

use clap::Subcommand;
use qcc_client::Client;
use qcc_client::protocol::methods::{
    Empty, SettingsGet, SettingsResult, SettingsSet, SettingsSetParams, SkillList,
};

use crate::{out, print_json};

#[derive(Subcommand)]
pub(crate) enum SkillCommand {
    /// The skills plugins offer, and whether each is on. One offered on request
    /// reaches only the threads that ask for it (`thread spawn --skill`).
    List,
    /// Give agents a skill that was turned off. Sessions started from now on get it.
    Enable { name: String },
    /// Give a skill to no session, whatever a thread asks.
    Disable { name: String },
}

pub(crate) async fn run(
    client: &Client,
    command: SkillCommand,
    json: bool,
) -> anyhow::Result<ExitCode> {
    let listed = client.call::<SkillList>(Empty {}).await?;
    let (name, disabled) = match command {
        SkillCommand::List => {
            if json {
                print_json(&listed.skills)?;
                return Ok(ExitCode::SUCCESS);
            }
            if listed.skills.is_empty() {
                out("no plugin offers skills");
            }
            for skill in &listed.skills {
                let mut tags = Vec::new();
                if skill.on_request {
                    tags.push("on request");
                }
                if skill.disabled {
                    tags.push("off");
                }
                let tags = if tags.is_empty() {
                    String::new()
                } else {
                    format!("  ({})", tags.join(", "))
                };
                out(format!("{:<24} {}{tags}", skill.name, skill.plugin));
                out(format!("{:<24} {}", "", skill.description));
            }
            return Ok(ExitCode::SUCCESS);
        }
        SkillCommand::Enable { name } => (name, false),
        SkillCommand::Disable { name } => (name, true),
    };
    if !listed.skills.iter().any(|skill| skill.name == name) {
        anyhow::bail!("no plugin offers a skill called {name:?}; `qcc skill list` lists them");
    }
    let SettingsResult { mut settings, .. } = client.call::<SettingsGet>(Empty {}).await?;
    let entry = settings.skills.entry(name.clone()).or_default();
    entry.disabled = disabled;
    // Nothing set is the same as the default: keep the file free of it.
    if !disabled {
        settings.skills.remove(&name);
    }
    let result = client.call::<SettingsSet>(SettingsSetParams { settings }).await?;
    if json {
        print_json(&result.settings.skills)?;
    }
    Ok(ExitCode::SUCCESS)
}

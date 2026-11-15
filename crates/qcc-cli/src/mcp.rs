//! `qcc mcp`: the MCP servers every agent session gets on top of the agent's own.

use std::collections::BTreeMap;
use std::process::ExitCode;

use anyhow::{Context, bail};
use clap::Subcommand;
use qcc_client::Client;
use qcc_client::protocol::methods::{
    Empty, McpServerSettings, SettingsGet, SettingsResult, SettingsSet, SettingsSetParams,
};
use qcc_domain::McpTransport;

use crate::{out, print_json};

#[derive(Subcommand)]
pub(crate) enum McpCommand {
    /// The MCP servers qcc gives agents, and whether each is on.
    List,
    /// Add an MCP server, or replace the one of that name. Either a program the agent
    /// starts (`qcc mcp add docs -- npx -y docs-mcp`) or an HTTP server
    /// (`qcc mcp add docs --url https://…/mcp`). Sessions started from now on get it.
    Add {
        /// Letters, digits, `_` and `-`.
        name: String,
        /// The server's streamable HTTP endpoint.
        #[arg(long, conflicts_with_all = ["command", "env"])]
        url: Option<String>,
        /// A header the agent sends to an HTTP server: NAME=VALUE. Repeatable.
        #[arg(long = "header", value_name = "NAME=VALUE", requires = "url")]
        headers: Vec<String>,
        /// A variable the server program sees: NAME=VALUE. Repeatable.
        #[arg(long = "env", value_name = "NAME=VALUE")]
        env: Vec<String>,
        /// The program to start and its arguments, after `--`.
        #[arg(last = true, value_name = "COMMAND")]
        command: Vec<String>,
    },
    /// Remove an MCP server.
    Remove { name: String },
    /// Give agents a server that was turned off.
    Enable { name: String },
    /// Keep a server in the settings but give it to no session.
    Disable { name: String },
}

pub(crate) async fn run(
    client: &Client,
    command: McpCommand,
    json: bool,
) -> anyhow::Result<ExitCode> {
    let SettingsResult { mut settings, .. } = client.call::<SettingsGet>(Empty {}).await?;
    match command {
        McpCommand::List => {
            if json {
                print_json(&settings.mcp_servers)?;
                return Ok(ExitCode::SUCCESS);
            }
            if settings.mcp_servers.is_empty() {
                out("no MCP servers; agents get only those of their own configuration");
            }
            for (name, server) in &settings.mcp_servers {
                let what = match &server.transport {
                    McpTransport::Stdio { command, args, .. } => {
                        std::iter::once(command).chain(args).cloned().collect::<Vec<_>>().join(" ")
                    }
                    McpTransport::Http { url, .. } => url.clone(),
                };
                let off = if server.disabled { "  (off)" } else { "" };
                out(format!("{name:<16} {what}{off}"));
            }
            return Ok(ExitCode::SUCCESS);
        }
        McpCommand::Add { name, url, headers, env, command } => {
            let transport = match url {
                Some(url) => McpTransport::Http { url, headers: pairs(headers)? },
                None => {
                    let mut command = command.into_iter();
                    let Some(program) = command.next() else {
                        bail!("give the server's program after `--`, or its --url");
                    };
                    McpTransport::Stdio {
                        command: program,
                        args: command.collect(),
                        env: pairs(env)?,
                    }
                }
            };
            settings.mcp_servers.insert(name, McpServerSettings { transport, disabled: false });
        }
        McpCommand::Remove { name } => {
            if settings.mcp_servers.remove(&name).is_none() {
                bail!("there is no MCP server {name:?}");
            }
        }
        McpCommand::Enable { name } => switch(&mut settings.mcp_servers, &name, false)?,
        McpCommand::Disable { name } => switch(&mut settings.mcp_servers, &name, true)?,
    }
    let result = client.call::<SettingsSet>(SettingsSetParams { settings }).await?;
    if json {
        print_json(&result.settings.mcp_servers)?;
    }
    Ok(ExitCode::SUCCESS)
}

fn switch(
    servers: &mut BTreeMap<String, McpServerSettings>,
    name: &str,
    disabled: bool,
) -> anyhow::Result<()> {
    let Some(server) = servers.get_mut(name) else { bail!("there is no MCP server {name:?}") };
    server.disabled = disabled;
    Ok(())
}

fn pairs(pairs: Vec<String>) -> anyhow::Result<BTreeMap<String, String>> {
    pairs
        .into_iter()
        .map(|pair| {
            let (name, value) =
                pair.split_once('=').with_context(|| format!("{pair:?} is not NAME=VALUE"))?;
            Ok((name.to_owned(), value.to_owned()))
        })
        .collect()
}

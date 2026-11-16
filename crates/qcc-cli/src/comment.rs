//! `qcc comment`: comments left on lines of a thread's changes. The user leaves them
//! in the app; the agent reads them here and says when each is dealt with.

use std::process::ExitCode;

use anyhow::Context;
use clap::{Args, Subcommand};
use qcc_client::Client;
use qcc_client::protocol::methods::{
    CommentAdd, CommentAddParams, CommentDelete, CommentDeleteParams, CommentList,
    CommentListParams, CommentResolve, CommentResolveParams, Empty,
};
use qcc_client::protocol::paths;
use qcc_domain::{CommentId, DiffSide, LineComment, ThreadId};

use crate::{out, print_json};

#[derive(Subcommand)]
pub(crate) enum CommentCommand {
    /// The open comments on a thread's changes.
    List {
        #[command(flatten)]
        thread: ThreadArg,
        /// Resolved comments too.
        #[arg(long)]
        all: bool,
    },
    /// Leave a comment on a line of a thread's changes.
    Add {
        #[command(flatten)]
        thread: ThreadArg,
        /// The file, from the top of the checkout as the thread's diff names it.
        path: String,
        /// The line number in the changed file.
        line: u32,
        text: String,
        /// The line number counts in the code before the change: a removed line.
        #[arg(long)]
        old: bool,
    },
    /// Mark comments dealt with.
    Resolve {
        #[arg(required = true)]
        ids: Vec<String>,
        /// What was done about them.
        #[arg(long)]
        note: Option<String>,
    },
    /// Open resolved comments again.
    Reopen {
        #[arg(required = true)]
        ids: Vec<String>,
    },
    /// Take a comment back.
    Delete { id: String },
}

#[derive(Args)]
pub(crate) struct ThreadArg {
    /// The thread [default: $QCC_THREAD_ID, set inside agent sessions].
    #[arg(long, env = paths::THREAD_ID_ENV)]
    thread: Option<String>,
}

impl ThreadArg {
    fn id(self) -> anyhow::Result<ThreadId> {
        let id = self.thread.filter(|id| !id.is_empty());
        id.map(ThreadId::new).context("say which thread with --thread")
    }
}

pub(crate) async fn run(
    client: &Client,
    command: CommentCommand,
    json: bool,
) -> anyhow::Result<ExitCode> {
    let comments = match command {
        CommentCommand::List { thread, all } => {
            let params = CommentListParams { thread_id: thread.id()?, include_resolved: all };
            let result = client.call::<CommentList>(params).await?;
            if !json && result.comments.is_empty() {
                out(if all { "no comments" } else { "no open comments" });
            }
            result.comments
        }
        CommentCommand::Add { thread, path, line, text, old } => {
            let side = if old { DiffSide::Old } else { DiffSide::New };
            let params =
                CommentAddParams { thread_id: thread.id()?, path, line, side, quote: None, text };
            vec![client.call::<CommentAdd>(params).await?.comment]
        }
        CommentCommand::Resolve { ids, note } => {
            let params = CommentResolveParams {
                comment_ids: ids.into_iter().map(CommentId::new).collect(),
                resolution: note,
                reopen: false,
            };
            client.call::<CommentResolve>(params).await?.comments
        }
        CommentCommand::Reopen { ids } => {
            let params = CommentResolveParams {
                comment_ids: ids.into_iter().map(CommentId::new).collect(),
                resolution: None,
                reopen: true,
            };
            client.call::<CommentResolve>(params).await?.comments
        }
        CommentCommand::Delete { id } => {
            let params = CommentDeleteParams { comment_id: CommentId::new(id) };
            client.call::<CommentDelete>(params).await?;
            if json {
                print_json(&Empty {})?;
            }
            return Ok(ExitCode::SUCCESS);
        }
    };
    if json {
        print_json(&comments)?;
    } else {
        for (index, comment) in comments.iter().enumerate() {
            if index > 0 {
                out("");
            }
            print(comment);
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn print(comment: &LineComment) {
    let state = if comment.is_open() { "" } else { "  (resolved)" };
    out(format!("{}  {}{state}", comment.id, comment.location()));
    if let Some(quote) = comment.quote.as_deref().filter(|quote| !quote.trim().is_empty()) {
        out(format!("  > {}", quote.trim_end()));
    }
    for line in comment.text.lines() {
        out(format!("  {line}"));
    }
    if let Some(resolution) = &comment.resolution {
        out(format!("  resolved: {resolution}"));
    }
}

//! Live progress of `thread archive` on a terminal.
//!
//! The daemon reports every step of the archive. Nothing is drawn for an archive
//! that is over before [`DELAY`]: most are, and a flash of lines would only be
//! noise. Once drawn, the block is redrawn in place and erased at the end, so what
//! is printed afterwards is exactly what a pipe would get.

use std::io::{IsTerminal, Write};
use std::time::Duration;

use qcc_client::Client;
use qcc_client::protocol::methods::{
    ArchiveProgress, ArchiveStep, ThreadArchive, ThreadArchiveParams, ThreadArchiveResult,
};
use tokio::sync::mpsc;

/// How long an archive runs before its progress is shown.
const DELAY: Duration = Duration::from_millis(300);
/// How often the spinner turns.
const TICK: Duration = Duration::from_millis(100);
const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
/// Trees larger than this show the threads being worked on, not all of them.
const MAX_LISTED: usize = 8;
const TITLE_WIDTH: usize = 48;

/// Archive as `params` say. On a terminal, stderr shows how it goes while it does.
pub async fn archive(
    client: &Client,
    params: ThreadArchiveParams,
) -> Result<ThreadArchiveResult, qcc_client::ClientError> {
    let mut stderr = std::io::stderr();
    if !stderr.is_terminal() {
        return client.call::<ThreadArchive>(params).await;
    }
    let (sender, mut reports) = mpsc::unbounded_channel();
    let call = client.archive(params, sender);
    tokio::pin!(call);
    let started = tokio::time::Instant::now();
    let mut tick = tokio::time::interval(TICK);
    let mut latest: Vec<ArchiveProgress> = Vec::new();
    let mut frame = 0usize;
    // Lines on screen now.
    let mut drawn = 0usize;
    let result = loop {
        tokio::select! {
            result = &mut call => break result,
            Some(report) = reports.recv() => latest = report,
            _ = tick.tick() => frame += 1,
        }
        if started.elapsed() >= DELAY && !latest.is_empty() {
            let lines = progress_lines(&latest, SPINNER[frame % SPINNER.len()]);
            drawn = redraw(&mut stderr, drawn, &lines);
        }
    };
    redraw(&mut stderr, drawn, &[]);
    result
}

/// Replace the `drawn` lines above the cursor with `lines`. Lines longer than the
/// terminal are cut rather than wrapped, so the count of lines to go back up stays
/// right. Returns how many lines are on screen now.
fn redraw(out: &mut impl Write, drawn: usize, lines: &[String]) -> usize {
    let mut text = String::new();
    if drawn > 0 {
        text.push_str(&format!("\x1b[{drawn}A"));
    }
    text.push_str("\r\x1b[J\x1b[?7l");
    for line in lines {
        text.push_str(line);
        text.push('\n');
    }
    text.push_str("\x1b[?7h");
    let _ = out.write_all(text.as_bytes()).and_then(|()| out.flush());
    lines.len()
}

/// The block that shows where an archive is: a headline, then one line per thread.
pub fn progress_lines(threads: &[ArchiveProgress], spinner: char) -> Vec<String> {
    let total = threads.len();
    let done = threads.iter().filter(|thread| thread.step.is_final()).count();
    let mut lines = vec![if total == 1 {
        format!("{spinner} Archiving {}", title(&threads[0]))
    } else {
        format!("{spinner} Archiving {total} threads: {done} of {total} done")
    }];
    let listed: Vec<&ArchiveProgress> = if total <= MAX_LISTED {
        threads.iter().collect()
    } else {
        let busy = |t: &&ArchiveProgress| {
            !matches!(t.step, ArchiveStep::Waiting | ArchiveStep::Archived | ArchiveStep::Kept)
        };
        threads.iter().filter(busy).collect()
    };
    for thread in &listed {
        let mark = match &thread.step {
            ArchiveStep::Waiting => '·',
            ArchiveStep::StoppingSession { .. } | ArchiveStep::RemovingEnvironment { .. } => {
                spinner
            }
            ArchiveStep::Archived => '✓',
            ArchiveStep::Refused { .. } => '✗',
            ArchiveStep::Kept => '-',
        };
        lines.push(format!(
            "  {mark} {}  {}  {}",
            thread.thread_id,
            title(thread),
            step(&thread.step)
        ));
    }
    if listed.len() < total {
        lines.push(format!("  … {} more", total - listed.len()));
    }
    lines
}

fn title(thread: &ArchiveProgress) -> String {
    let title = thread.title.as_deref().unwrap_or("(untitled)");
    match title.char_indices().nth(TITLE_WIDTH) {
        Some((at, _)) => format!("{}…", &title[..at]),
        None => title.to_owned(),
    }
}

/// What is happening to a thread, in a few words.
pub fn step(step: &ArchiveStep) -> String {
    match step {
        ArchiveStep::Waiting => "waiting".into(),
        ArchiveStep::StoppingSession { working: true } => "stopping the agent".into(),
        ArchiveStep::StoppingSession { working: false } => "closing the session".into(),
        ArchiveStep::RemovingEnvironment { environment } => format!("removing its {environment}"),
        ArchiveStep::Archived => "archived".into(),
        ArchiveStep::Refused { message, .. } => format!("not archived: {message}"),
        ArchiveStep::Kept => "kept, a thread under it stays".into(),
    }
}

#[cfg(test)]
mod tests {
    use qcc_domain::ThreadId;

    use super::*;

    fn thread(id: &str, step: ArchiveStep) -> ArchiveProgress {
        ArchiveProgress {
            thread_id: ThreadId::new(id),
            title: Some(format!("title of {id}")),
            parent: None,
            step,
        }
    }

    #[test]
    fn a_tree_shows_each_thread_and_how_many_are_done() {
        let threads = [
            thread("a", ArchiveStep::Archived),
            thread("b", ArchiveStep::RemovingEnvironment { environment: "Git worktree".into() }),
            thread("c", ArchiveStep::Waiting),
        ];
        let lines = progress_lines(&threads, '*');
        assert_eq!(
            lines,
            [
                "* Archiving 3 threads: 1 of 3 done",
                "  ✓ a  title of a  archived",
                "  * b  title of b  removing its Git worktree",
                "  · c  title of c  waiting",
            ]
        );
    }

    #[test]
    fn one_thread_is_named_in_the_headline() {
        let threads = [thread("a", ArchiveStep::StoppingSession { working: true })];
        let lines = progress_lines(&threads, '*');
        assert_eq!(lines, ["* Archiving title of a", "  * a  title of a  stopping the agent"]);
    }

    #[test]
    fn a_large_tree_lists_only_the_threads_in_hand() {
        let mut threads: Vec<_> =
            (0..12).map(|i| thread(&format!("t{i}"), ArchiveStep::Waiting)).collect();
        threads[0].step = ArchiveStep::Archived;
        threads[1].step = ArchiveStep::Refused { code: 1, message: "dirty".into() };
        threads[2].step = ArchiveStep::StoppingSession { working: false };
        let lines = progress_lines(&threads, '*');
        assert_eq!(lines[0], "* Archiving 12 threads: 2 of 12 done");
        assert_eq!(lines[1], "  ✗ t1  title of t1  not archived: dirty");
        assert_eq!(lines[2], "  * t2  title of t2  closing the session");
        assert_eq!(lines[3], "  … 10 more");
        assert_eq!(lines.len(), 4);
    }

    #[test]
    fn redrawing_goes_back_over_the_lines_drawn_before() {
        let mut out = Vec::new();
        assert_eq!(redraw(&mut out, 0, &["one".into(), "two".into()]), 2);
        assert_eq!(redraw(&mut out, 2, &[]), 0);
        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with("\r\x1b[J\x1b[?7lone\ntwo\n\x1b[?7h"), "{text:?}");
        assert!(text.ends_with("\x1b[2A\r\x1b[J\x1b[?7l\x1b[?7h"), "{text:?}");
    }
}

//! What reviewers are told, and what the reviewed agent is told back.

use crate::diff;
use crate::findings::Finding;
use crate::profile::{Profile, covers};

/// How much of the diff goes into a prompt; past it the reviewer reads the rest.
pub const MAX_DIFF_CHARS: usize = 60_000;
const MAX_TASK_CHARS: usize = 2_000;

/// A remark from an earlier round, as the reviewer sees it again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Earlier {
    pub location: String,
    pub text: String,
    /// What the agent said it did about it, once resolved.
    pub resolution: Option<String>,
    pub resolved: bool,
}

/// Everything a reviewer's prompt is made of.
pub struct Review<'a> {
    pub thread: &'a str,
    pub cwd: &'a str,
    pub task: Option<&'a str>,
    pub profile: &'a Profile,
    pub round: u32,
    /// The whole diff of the thread.
    pub diff: &'a str,
    /// The environment cut the diff short.
    pub truncated: bool,
    /// Files changed since the last round; `None` in the first.
    pub changed: Option<&'a [String]>,
    pub earlier: &'a [Earlier],
}

fn clip(text: &str, max: usize) -> (String, bool) {
    if text.chars().count() <= max {
        return (text.to_owned(), false);
    }
    (text.chars().take(max).collect(), true)
}

/// The part of the diff a reviewer is given: the files its profile covers and, after
/// the first round, only those changed since. `None` when nothing is left.
pub fn scoped_diff(diff: &str, profile: &Profile, changed: Option<&[String]>) -> Option<String> {
    let mut scoped = String::new();
    for file in diff::files(diff) {
        let fresh = changed.is_none_or(|changed| changed.contains(&file.path));
        if fresh && covers(profile, &file.path) {
            scoped.push_str(file.text);
        }
    }
    (!scoped.trim().is_empty()).then_some(scoped)
}

pub fn critic(review: &Review<'_>, scoped: &str) -> String {
    let Review { thread, cwd, task, profile, round, truncated, changed, earlier, .. } = review;
    let mut prompt = format!(
        "You are a code reviewer, \"{name}\", working for the user of qcc. Review the changes \
         another agent made in qcc thread {thread}, in {cwd}. You may read any file there and \
         run commands that only look (git log, git show, grep). Do not change any file, do not \
         commit, and do not start other agents.\n\n",
        name = profile.name,
    );
    if let Some(task) = task {
        let (task, _) = clip(task.trim(), MAX_TASK_CHARS);
        prompt.push_str(&format!("The task the agent was given:\n\n<task>\n{task}\n</task>\n\n"));
    }
    match changed {
        None => prompt.push_str("Its changes so far, as a diff:\n\n"),
        Some(changed) => {
            prompt.push_str(&format!(
                "This is review round {round}. Since the last review the agent changed: {}. \
                 Review only those changes, and say again only what is still wrong. The diff \
                 of those files as they are now:\n\n",
                changed.join(", ")
            ));
        }
    }
    let (body, cut) = clip(scoped, MAX_DIFF_CHARS);
    prompt.push_str(&format!("```diff\n{}\n```\n\n", body.trim_end()));
    if cut || *truncated {
        prompt.push_str(&format!(
            "The diff above is cut short. Read the rest with `qcc thread diff {thread}` (if `qcc` \
             is not found, use \"$QCC_BIN\") or with git in {cwd}.\n\n"
        ));
    }
    if !profile.paths.is_empty() {
        prompt.push_str(&format!(
            "Look only at files matching {}.\n\n",
            profile.paths.iter().map(|glob| format!("`{glob}`")).collect::<Vec<_>>().join(", ")
        ));
    }
    if !earlier.is_empty() {
        prompt.push_str("What you found in earlier rounds, and what became of it:\n\n");
        for remark in *earlier {
            let state = match (remark.resolved, &remark.resolution) {
                (true, Some(note)) => format!("resolved: {note}"),
                (true, None) => "resolved".to_owned(),
                (false, _) => "still open".to_owned(),
            };
            prompt.push_str(&format!("- {}: {} ({state})\n", remark.location, remark.text));
        }
        prompt.push_str(
            "\nDo not repeat a remark that was dealt with; check that it really was. A remark \
             still open that is still true may be repeated.\n\n",
        );
    }
    prompt.push_str(&format!(
        "Report only findings of severity {} or above. End your reply with one JSON object \
         and nothing after it:\n\n\
         {{\"findings\": [{{\"file\": \"path/from/the/diff\", \"line\": 12, \"severity\": \
         \"high\", \"text\": \"what is wrong and how to fix it\"}}]}}\n\n\
         `line` is the line number in the file as it is now; leave out `file` and `line` for a \
         remark about the change as a whole. `severity` is one of info, low, medium, high, \
         critical. An empty list means you found nothing.\n\n",
        profile.severity
    ));
    prompt.push_str(&format!("What to look for:\n\n{}\n", profile.body.trim()));
    prompt
}

/// One finding in the message to the agent.
pub struct Sent<'a> {
    pub reviewer: &'a str,
    pub finding: &'a Finding,
    /// The comment that holds it, if one does.
    pub comment: Option<&'a str>,
}

/// The one message a round sends the agent.
pub fn summary(round: u32, sent: &[Sent<'_>], stale: bool, problems: &[String]) -> String {
    let mut text = format!("qcc reviewers finished review round {round} of your changes.");
    if stale {
        text.push_str(
            " You changed files while they worked, so some lines they point at may have moved.",
        );
    }
    if sent.is_empty() {
        text.push_str(" They found nothing to change.");
    } else {
        text.push_str(" Their findings:\n");
        for item in sent {
            let place = item.finding.location().map(|at| format!(" at {at}")).unwrap_or_default();
            let comment = item.comment.map(|id| format!(" [{id}]")).unwrap_or_default();
            text.push_str(&format!(
                "\n- ({}, from {}){comment}{place}: {}",
                item.finding.severity, item.reviewer, item.finding.text
            ));
        }
        text.push_str(
            "\n\nFix what you agree with. Each finding in [brackets] is a comment on your \
             changes: when it is dealt with, resolve it with `qcc comment resolve <id> --note \
             \"<what you did, or why not>\"` (if `qcc` is not found, use \"$QCC_BIN\").",
        );
    }
    for problem in problems {
        text.push_str(&format!("\n\n(Note: {problem}.)"));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::findings::Severity;
    use crate::profile::{Source, parse};

    const DIFF: &str = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n\
                        @@ -1 +1 @@\n-a\n+b\ndiff --git a/README.md b/README.md\n--- a/README.md\n\
                        +++ b/README.md\n@@ -1 +1 @@\n-x\n+y\n";

    fn profile(front: &str) -> Profile {
        parse("p.md", &format!("---\n{front}\n---\nCheck errors."), Source::Project).unwrap()
    }

    #[test]
    fn the_diff_is_scoped_to_the_profile_and_the_round() {
        let rust = profile("paths: [\"*.rs\"]");
        let scoped = scoped_diff(DIFF, &rust, None).unwrap();
        assert!(scoped.contains("src/a.rs") && !scoped.contains("README"));
        assert_eq!(scoped_diff(DIFF, &rust, Some(&["README.md".to_owned()])), None);
        let all = profile("name: all");
        let later = scoped_diff(DIFF, &all, Some(&["README.md".to_owned()])).unwrap();
        assert!(later.contains("README") && !later.contains("src/a.rs"));
    }

    #[test]
    fn a_prompt_says_what_and_how_to_answer() {
        let profile = profile("severity: medium\npaths: [\"*.rs\"]");
        let earlier = [Earlier {
            location: "src/a.rs:1".into(),
            text: "bad name".into(),
            resolution: Some("renamed".into()),
            resolved: true,
        }];
        let changed = ["src/a.rs".to_owned()];
        let review = Review {
            thread: "thr_1",
            cwd: "/w",
            task: Some("fix it"),
            profile: &profile,
            round: 2,
            diff: DIFF,
            truncated: true,
            changed: Some(&changed),
            earlier: &earlier,
        };
        let prompt = critic(&review, "+b\n");
        assert!(prompt.contains("qcc thread thr_1, in /w"));
        assert!(prompt.contains("<task>\nfix it\n</task>"));
        assert!(prompt.contains("review round 2") && prompt.contains("changed: src/a.rs"));
        assert!(prompt.contains("cut short"));
        assert!(prompt.contains("- src/a.rs:1: bad name (resolved: renamed)"));
        assert!(prompt.contains("severity medium or above"));
        assert!(prompt.contains("Do not change any file"));
        assert!(prompt.ends_with("Check errors.\n"), "the profile's words come last");
    }

    #[test]
    fn the_summary_lists_findings_with_their_comments() {
        let found = Finding {
            file: Some("a.rs".into()),
            line: Some(3),
            severity: Severity::High,
            text: "boom".into(),
        };
        let general =
            Finding { file: None, line: None, severity: Severity::Low, text: "no tests".into() };
        let sent = [
            Sent { reviewer: "general", finding: &found, comment: Some("cmt_1") },
            Sent { reviewer: "security", finding: &general, comment: None },
        ];
        let text = summary(2, &sent, true, &["security did not answer in the format".into()]);
        assert!(text.contains("round 2"));
        assert!(text.contains("may have moved"));
        assert!(text.contains("- (high, from general) [cmt_1] at a.rs:3: boom"));
        assert!(text.contains("- (low, from security): no tests"));
        assert!(text.contains("qcc comment resolve"));
        assert!(text.contains("(Note: security did not answer"));
        assert!(summary(1, &[], false, &[]).contains("found nothing"));
    }
}

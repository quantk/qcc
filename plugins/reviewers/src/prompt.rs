//! What reviewers are told, and what the reviewed agent is told back.

use crate::findings::Finding;
use crate::profile::{Profile, covers};

const MAX_TASK_CHARS: usize = 2_000;
/// How many files a prompt lists by name; the reviewer lists the rest itself.
const MAX_FILES: usize = 200;

/// A remark from an earlier round, as the reviewer sees it again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Earlier {
    pub location: String,
    pub text: String,
    /// What the agent said it did about it, once resolved.
    pub resolution: Option<String>,
    pub resolved: bool,
}

/// A file the thread changed, as `qcc thread changes` lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Changed {
    /// From the top of the checkout.
    pub path: String,
    /// `added`, `modified`, `deleted` or `untracked`.
    pub kind: &'static str,
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
}

/// Everything a reviewer's prompt is made of.
pub struct Review<'a> {
    pub thread: &'a str,
    pub cwd: &'a str,
    pub task: Option<&'a str>,
    pub profile: &'a Profile,
    pub round: u32,
    /// What the changes are measured against (`HEAD`, a branch, a commit).
    pub base: Option<&'a str>,
    pub branch: Option<&'a str>,
    /// Every file the thread changed.
    pub files: &'a [Changed],
    /// Files changed since the last round; `None` in the first.
    pub changed: Option<&'a [String]>,
    pub earlier: &'a [Earlier],
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    text.chars().take(max).collect::<String>() + " […]"
}

/// The files a reviewer looks at: those its profile covers and, after the first
/// round, only those changed since. Empty when nothing is left.
pub fn scoped_files<'a>(
    files: &'a [Changed],
    profile: &Profile,
    changed: Option<&[String]>,
) -> Vec<&'a Changed> {
    files
        .iter()
        .filter(|file| changed.is_none_or(|changed| changed.contains(&file.path)))
        .filter(|file| covers(profile, &file.path))
        .collect()
}

fn file_line(file: &Changed) -> String {
    let counts = match (file.additions, file.deletions) {
        (Some(added), Some(deleted)) => format!(" +{added} -{deleted}"),
        _ => String::new(),
    };
    format!("- `{}` ({}{counts})\n", file.path, file.kind)
}

/// The prompt: where to look and how, not the diff itself; the reviewer reads
/// what it needs, so a large change does not fill its context up front.
pub fn critic(review: &Review<'_>, files: &[&Changed]) -> String {
    let Review { thread, cwd, task, profile, round, base, branch, changed, earlier, .. } = review;
    // The first line names the thread in lists.
    let mut prompt = format!(
        "Review round {round} by {name} of qcc thread {thread}\n\n\
         You are a code reviewer, \"{name}\", working for the user of qcc. Review the changes \
         another agent made in qcc thread {thread}, which works in `{cwd}`. You run read-only: \
         read files and run commands that only look; nothing that changes a file will run, and \
         nobody is there to approve anything. Do not try to fix what you find: report it.\n\n",
        name = profile.name,
    );
    if let Some(task) = task {
        prompt.push_str(&format!(
            "The task the agent was given:\n\n<task>\n{}\n</task>\n\n",
            clip(task.trim(), MAX_TASK_CHARS)
        ));
    }
    let measured = match (base, branch) {
        (Some(base), Some(branch)) => format!(" on branch `{branch}`, against `{base}`"),
        (Some(base), None) => format!(" against `{base}`"),
        (None, Some(branch)) => format!(" on branch `{branch}`"),
        (None, None) => String::new(),
    };
    match changed {
        None => prompt.push_str(&format!(
            "The files it changed{measured} (paths from the top of the repository), with lines \
             added and removed:\n\n"
        )),
        Some(changed) => prompt.push_str(&format!(
            "This is review round {round}. Since the last review the agent changed {}. Review \
             only these files{measured}, and report again only what is still wrong:\n\n",
            changed.len()
        )),
    }
    for file in files.iter().take(MAX_FILES) {
        prompt.push_str(&file_line(file));
    }
    if files.len() > MAX_FILES {
        prompt.push_str(&format!("- … and {} more\n", files.len() - MAX_FILES));
    }
    let base_ref = base.unwrap_or("HEAD");
    prompt.push_str(&format!(
        "\nLook at them yourself, a few files at a time, as much as you need:\n\n\
         - `git diff {base_ref} -- ':/<path>'` in `{cwd}`: the changes of a file (an untracked \
           file is not in it: read the file);\n\
         - read any file, and search with your tools, to see the code around a change;\n\
         - `git log`, `git show`, `git blame` for history;\n\
         - `qcc thread diff {thread} <path>…` (`--full` shows each file whole, its changes \
           marked) and `qcc thread changes {thread}`: the same as qcc sees it. If `qcc` is not \
           found, write \"$QCC_BIN\" in its place; a sandbox may keep it from reaching qcc, \
           and then git is the way.\n\n"
    ));
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
         {{\"findings\": [{{\"file\": \"path/as/listed/above\", \"line\": 12, \"severity\": \
         \"high\", \"text\": \"what is wrong and how to fix it\"}}]}}\n\n\
         `line` is the line number in the file as it is now; leave out `file` and `line` for a \
         remark about the change as a whole. `severity` is one of info, low, medium, high, \
         critical. An empty list means you found nothing.\n\n",
        profile.severity
    ));
    prompt.push_str(&format!("What to look for:\n\n{}\n", profile.body.trim()));
    prompt
}

/// A remark in the message to the agent: a finding with no comment to carry it.
pub struct Remark<'a> {
    pub reviewer: &'a str,
    pub finding: &'a Finding,
}

/// The message that hands findings to the agent. The findings on lines go along as
/// comments, which the daemon spells out in full with how to resolve them; the
/// message says so, and lists the remarks that have no comment.
pub fn findings_message(
    intro: &str,
    comments: usize,
    remarks: &[Remark<'_>],
    stale: bool,
    problems: &[String],
) -> String {
    let mut text = intro.to_owned();
    if stale {
        text.push_str(
            " You changed files while the reviewers worked, so some lines they point at may \
             have moved.",
        );
    }
    if comments == 0 && remarks.is_empty() {
        text.push_str(" They found nothing to change.");
    }
    if comments > 0 {
        text.push_str(&format!(
            " {} on lines of your changes {} below, as comments. Fix what you agree with, and \
             resolve each comment when it is dealt with, saying what you did or why not.",
            if comments == 1 { "One finding".to_owned() } else { format!("{comments} findings") },
            if comments == 1 { "is" } else { "are" },
        ));
    }
    if !remarks.is_empty() {
        text.push_str("\n\nRemarks about the change as a whole, with no comment to resolve:\n");
        for item in remarks {
            let place = item.finding.location().map(|at| format!(" at {at}")).unwrap_or_default();
            text.push_str(&format!(
                "\n- ({}, from {}){place}: {}",
                item.finding.severity, item.reviewer, item.finding.text
            ));
        }
        text.push_str("\n\nSay in your reply what you did about each remark.");
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

    fn file(path: &str, kind: &'static str, counts: Option<(u32, u32)>) -> Changed {
        Changed {
            path: path.into(),
            kind,
            additions: counts.map(|c| c.0),
            deletions: counts.map(|c| c.1),
        }
    }

    fn files() -> Vec<Changed> {
        vec![file("src/a.rs", "modified", Some((3, 1))), file("README.md", "untracked", None)]
    }

    fn profile(front: &str) -> Profile {
        parse("p.md", &format!("---\n{front}\n---\nCheck errors."), Source::Project).unwrap()
    }

    fn paths(files: Vec<&Changed>) -> Vec<&str> {
        files.into_iter().map(|file| file.path.as_str()).collect()
    }

    #[test]
    fn the_files_are_scoped_to_the_profile_and_the_round() {
        let files = files();
        let rust = profile("paths: [\"*.rs\"]");
        assert_eq!(paths(scoped_files(&files, &rust, None)), ["src/a.rs"]);
        assert!(scoped_files(&files, &rust, Some(&["README.md".to_owned()])).is_empty());
        let all = profile("name: all");
        assert_eq!(
            paths(scoped_files(&files, &all, Some(&["README.md".to_owned()]))),
            ["README.md"]
        );
    }

    #[test]
    fn a_prompt_says_where_to_look_and_how_to_answer() {
        let profile = profile("severity: medium\npaths: [\"*.rs\"]");
        let earlier = [Earlier {
            location: "src/a.rs:1".into(),
            text: "bad name".into(),
            resolution: Some("renamed".into()),
            resolved: true,
        }];
        let files = files();
        let changed = ["src/a.rs".to_owned()];
        let review = Review {
            thread: "thr_1",
            cwd: "/w",
            task: Some("fix it"),
            profile: &profile,
            round: 2,
            base: Some("main"),
            branch: Some("task/x"),
            files: &files,
            changed: Some(&changed),
            earlier: &earlier,
        };
        let prompt = critic(&review, &scoped_files(&files, &profile, Some(&changed)));
        assert!(prompt.starts_with("Review round 2 by p of qcc thread thr_1\n\n"), "{prompt}");
        assert!(prompt.contains("qcc thread thr_1, which works in `/w`"));
        assert!(prompt.contains("<task>\nfix it\n</task>"));
        assert!(prompt.contains("review round 2") && prompt.contains("changed 1"));
        assert!(prompt.contains("on branch `task/x`, against `main`"), "{prompt}");
        assert!(prompt.contains("- `src/a.rs` (modified +3 -1)\n"), "{prompt}");
        assert!(!prompt.contains("README.md"), "outside the profile's paths: {prompt}");
        assert!(prompt.contains("`qcc thread diff thr_1 <path>…`"));
        assert!(prompt.contains("`qcc thread changes thr_1`"));
        assert!(prompt.contains("`git diff main -- ':/<path>'`"));
        assert!(!prompt.contains("```diff"), "no diff in the prompt");
        assert!(prompt.contains("- src/a.rs:1: bad name (resolved: renamed)"));
        assert!(prompt.contains("severity medium or above"));
        assert!(prompt.contains("You run read-only"));
        assert!(prompt.ends_with("Check errors.\n"), "the profile's words come last");

        let first = Review { changed: None, base: None, branch: None, ..review };
        let refs: Vec<&Changed> = files.iter().collect();
        let prompt = critic(&first, &refs);
        assert!(prompt.contains("- `README.md` (untracked)\n"), "{prompt}");
        assert!(prompt.contains("`git diff HEAD -- "), "{prompt}");
    }

    #[test]
    fn the_message_points_to_the_comments_and_lists_the_remarks() {
        let general =
            Finding { file: None, line: None, severity: Severity::Low, text: "no tests".into() };
        let elsewhere = Finding {
            file: Some("b.rs".into()),
            line: Some(4),
            severity: Severity::High,
            text: "boom".into(),
        };
        let remarks = [
            Remark { reviewer: "security", finding: &general },
            Remark { reviewer: "general", finding: &elsewhere },
        ];
        let problems = ["security did not answer in the format".to_owned()];
        let text = findings_message("Round 2 is done.", 2, &remarks, true, &problems);
        assert!(text.starts_with("Round 2 is done."));
        assert!(text.contains("may have moved"));
        assert!(text.contains("2 findings on lines of your changes are below, as comments"));
        assert!(text.contains("- (low, from security): no tests"));
        assert!(text.contains("- (high, from general) at b.rs:4: boom"));
        assert!(text.contains("(Note: security did not answer"));
        let one = findings_message("Done.", 1, &[], false, &[]);
        assert!(
            one.contains("One finding on lines of your changes is below")
                && !one.contains("Remarks")
        );
        assert!(findings_message("Done.", 0, &[], false, &[]).contains("found nothing"));
    }
}

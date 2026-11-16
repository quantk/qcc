//! What the plugin does, as state machines over finished commands.
//!
//! Nothing here touches the host: a job says which commands to run and is told how
//! each one ended. [`crate`] moves the bytes.

use std::collections::HashMap;

use qcc_plugin_sdk::types::{ChangeKind, ChangedFile, EnvironmentChanges};
use serde_json::{Value, json};

/// A setup or teardown script may install dependencies; give it time.
pub const SCRIPT_TIMEOUT_MS: u64 = 15 * 60 * 1000;
const GIT_TIMEOUT_MS: u64 = 2 * 60 * 1000;
/// Output a command may produce before it is cut off.
const OUTPUT_LIMIT: usize = 1024 * 1024;
pub const DIFF_LIMIT: usize = 2 * 1024 * 1024;
/// New files beyond this many are left out of a diff.
const MAX_UNTRACKED_DIFFS: usize = 100;

pub const SETUP_SCRIPT: &str = ".qcc-env-setup.sh";
pub const TEARDOWN_SCRIPT: &str = ".qcc-env-teardown.sh";
pub const INCLUDE_FILE: &str = ".worktreeinclude";

/// Copies the untracked files of the source checkout that `.worktreeinclude` names
/// into the new worktree. `$1` is the source, `$2` the worktree. Never replaces a
/// file the worktree has and never follows a symlink.
const INCLUDE_SCRIPT: &str = r#"
src=$1; dst=$2
[ -f "$dst/.worktreeinclude" ] || exit 0
git -C "$src" ls-files -z --others --ignored --exclude-from="$dst/.worktreeinclude" |
while IFS= read -r -d "" file; do
  [ -L "$src/$file" ] && continue
  { [ -e "$dst/$file" ] || [ -L "$dst/$file" ]; } && continue
  mkdir -p "$(dirname "$dst/$file")" && cp -p "$src/$file" "$dst/$file" ||
    echo "cannot copy $file" >&2
done
"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cmd {
    /// Names the command to the job that asked for it.
    pub tag: u32,
    pub program: &'static str,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub env: Vec<(String, String)>,
    /// Standard output beyond this is dropped and the command is stopped.
    pub limit: usize,
    pub timeout_ms: u64,
}

fn git(tag: u32, dir: &str, args: &[&str]) -> Cmd {
    let mut all = vec!["-C".to_owned(), dir.to_owned()];
    all.extend(args.iter().map(|arg| (*arg).to_owned()));
    Cmd {
        tag,
        program: "git",
        args: all,
        cwd: None,
        limit: OUTPUT_LIMIT,
        timeout_ms: GIT_TIMEOUT_MS,
        // The jobs read git's messages; a translated one would not match.
        env: vec![("LC_ALL".to_owned(), "C".to_owned())],
    }
}

/// Run `script` from the root of `dir` if the checkout has it.
fn script(tag: u32, dir: &str, script: &str, env: Vec<(String, String)>) -> Cmd {
    Cmd {
        tag,
        program: "bash",
        args: vec![
            "-c".into(),
            r#"[ -f "$1" ] || exit 0; exec bash "$1""#.into(),
            "qcc".into(),
            script.to_owned(),
        ],
        cwd: Some(dir.to_owned()),
        env,
        limit: OUTPUT_LIMIT,
        timeout_ms: SCRIPT_TIMEOUT_MS,
    }
}

/// How a command ended.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Exit {
    /// Absent when a signal ended it or it could not be started.
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    /// The end of standard error.
    pub stderr: String,
    /// It produced more than its limit and was stopped.
    pub truncated: bool,
    pub timed_out: bool,
}

impl Exit {
    fn ok(&self) -> bool {
        self.code == Some(0)
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    /// Why it failed, in its own words.
    fn reason(&self) -> String {
        if self.timed_out {
            return "timed out".into();
        }
        if self.truncated {
            return "produced too much output and was stopped".into();
        }
        let stderr = self.stderr.trim();
        match (stderr.is_empty(), self.code) {
            (false, _) => stderr.to_owned(),
            (true, Some(code)) => format!("exit code {code}"),
            (true, None) => "killed by a signal".into(),
        }
    }
}

/// Whether a failed `git status` in a worktree says the worktree no longer exists:
/// its directory is gone, or git no longer knows it.
fn worktree_gone(exit: &Exit) -> bool {
    exit.code == Some(128)
        && !exit.truncated
        && ["cannot change to", "not a git repository"]
            .iter()
            .any(|message| exit.stderr.contains(message))
}

/// What the plugin remembers about a worktree it made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// Root of the checkout the worktree was made from.
    pub toplevel: String,
    pub worktree: String,
    /// The thread's working directory: the worktree, or the subdirectory of it that
    /// mirrors where in the checkout the thread was started.
    pub cwd: String,
    pub branch: String,
    /// The commit the branch started at.
    pub base: String,
}

impl Record {
    pub fn to_json(&self) -> Vec<u8> {
        json!({
            "toplevel": self.toplevel,
            "worktree": self.worktree,
            "cwd": self.cwd,
            "branch": self.branch,
            "base": self.base,
        })
        .to_string()
        .into_bytes()
    }

    pub fn from_json(bytes: &[u8]) -> Option<Self> {
        let value: Value = serde_json::from_slice(bytes).ok()?;
        let field = |name: &str| Some(value.get(name)?.as_str()?.to_owned());
        Some(Self {
            toplevel: field("toplevel")?,
            worktree: field("worktree")?,
            cwd: field("cwd")?,
            branch: field("branch")?,
            base: field("base")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    Ready(Record),
    Removed,
    Changes(EnvironmentChanges),
    Diff {
        diff: String,
        truncated: bool,
    },
    /// What delivering did, in a sentence.
    Delivered(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Run(Vec<Cmd>),
    /// Other commands of the job are still running.
    Wait,
    Finish(Result<Answer, String>),
}

pub enum Job {
    Prepare(Prepare),
    Remove(Remove),
    Changes(Changes),
    Diff(Diff),
    Deliver(Deliver),
    Inspect(Inspect),
}

impl Job {
    pub fn start(&mut self) -> Step {
        match self {
            Self::Prepare(job) => job.start(),
            Self::Remove(job) => job.start(),
            Self::Changes(job) => job.start(),
            Self::Diff(job) => job.start(),
            Self::Deliver(job) => job.start(),
            Self::Inspect(job) => job.start(),
        }
    }

    pub fn on_exit(&mut self, tag: u32, exit: Exit) -> Step {
        match self {
            Self::Prepare(job) => job.on_exit(tag, exit),
            Self::Remove(job) => job.on_exit(tag, exit),
            Self::Changes(job) => job.on_exit(tag, exit),
            Self::Diff(job) => job.on_exit(tag, exit),
            Self::Deliver(job) => job.on_exit(tag, exit),
            Self::Inspect(job) => job.on_exit(tag, exit),
        }
    }
}

fn join(dir: &str, name: &str) -> String {
    format!("{}/{name}", dir.trim_end_matches('/'))
}

// ----- prepare ------------------------------------------------------------------------

mod prepare {
    pub const TOPLEVEL: u32 = 0;
    pub const BASE: u32 = 1;
    pub const ADD: u32 = 2;
    pub const INCLUDE: u32 = 3;
    pub const SETUP: u32 = 4;
    pub const UNDO_WORKTREE: u32 = 5;
    pub const UNDO_BRANCH: u32 = 6;
    /// Set the source's uncommitted work aside, to carry it into the worktree.
    pub const STASH: u32 = 7;
    /// Bring that work into the worktree.
    pub const POP: u32 = 8;
    /// Give it back to the source when the worktree could not be made.
    pub const RESTORE: u32 = 9;
}

/// `git worktree add` on a new branch, then the files `.worktreeinclude` names, then
/// the repository's setup script. A failed setup takes the worktree down again.
///
/// A thread moving in from the source checkout takes its uncommitted work along:
/// it is stashed in the source before the worktree is made and popped in the
/// worktree after setup. If anything fails, the worktree goes and the stash is
/// popped back where it came from.
pub struct Prepare {
    thread: String,
    source: String,
    root: String,
    base: Option<String>,
    /// Where in the checkout the thread was started, with a trailing slash; empty at
    /// the root.
    prefix: String,
    record: Record,
    failure: Option<String>,
    carry: bool,
    /// Something was stashed in the source and is not back anywhere yet.
    stashed: bool,
}

impl Prepare {
    /// Carry the source's uncommitted work into the worktree.
    pub fn carrying(mut self, carry: bool) -> Self {
        self.carry = carry;
        self
    }

    /// Take the worktree down again, give the stash back, and fail with `reason`.
    fn undo(&mut self, reason: String) -> Step {
        self.failure = Some(reason);
        Step::Run(vec![git(
            prepare::UNDO_WORKTREE,
            &self.record.toplevel,
            &["worktree", "remove", "--force", &self.record.worktree],
        )])
    }

    fn fail(&mut self) -> Step {
        Step::Finish(Err(self.failure.take().unwrap_or_else(|| "setup failed".into())))
    }

    pub fn new(thread: String, source: String, root: String, base: Option<String>) -> Self {
        let record = Record {
            toplevel: String::new(),
            worktree: String::new(),
            cwd: String::new(),
            branch: format!("qcc/{thread}"),
            base: String::new(),
        };
        Self {
            thread,
            source,
            root,
            base,
            prefix: String::new(),
            record,
            failure: None,
            carry: false,
            stashed: false,
        }
    }

    fn start(&mut self) -> Step {
        Step::Run(vec![git(
            prepare::TOPLEVEL,
            &self.source,
            &["rev-parse", "--show-toplevel", "--show-prefix"],
        )])
    }

    fn on_exit(&mut self, tag: u32, exit: Exit) -> Step {
        use prepare::*;
        let record = &mut self.record;
        match tag {
            TOPLEVEL => {
                let text = exit.text();
                let mut lines = text.lines();
                let toplevel = lines.next().unwrap_or("").to_owned();
                if !exit.ok() || toplevel.is_empty() {
                    return Step::Finish(Err(if exit.code == Some(128) {
                        format!("{} is not inside a git repository", self.source)
                    } else {
                        format!("git rev-parse in {}: {}", self.source, exit.reason())
                    }));
                }
                self.prefix = lines.next().unwrap_or("").to_owned();
                let name = toplevel.trim_end_matches('/').rsplit('/').next().unwrap_or("repo");
                record.worktree = join(&join(&self.root, &self.thread), name);
                record.cwd = match self.prefix.trim_end_matches('/') {
                    "" => record.worktree.clone(),
                    prefix => join(&record.worktree, prefix),
                };
                record.toplevel = toplevel;
                let commit = format!("{}^{{commit}}", self.base.as_deref().unwrap_or("HEAD"));
                Step::Run(vec![git(
                    BASE,
                    &record.toplevel,
                    &["rev-parse", "--verify", "--quiet", "--end-of-options", &commit],
                )])
            }
            BASE => {
                let commit = exit.text().trim().to_owned();
                // `--quiet` makes a missing commit a bare exit code 1; anything else is
                // git failing for its own reasons.
                if exit.timed_out || !matches!(exit.code, Some(0 | 1)) {
                    return Step::Finish(Err(format!("git rev-parse: {}", exit.reason())));
                }
                if !exit.ok() || commit.is_empty() {
                    return Step::Finish(Err(match &self.base {
                        Some(base) => {
                            format!("{base:?} does not name a commit in {}", record.toplevel)
                        }
                        None => format!(
                            "{} has no commits yet; a worktree needs one to start from",
                            record.toplevel
                        ),
                    }));
                }
                record.base = commit;
                if self.carry {
                    let message = format!("qcc: thread {} moves into a worktree", self.thread);
                    return Step::Run(vec![git(
                        STASH,
                        &record.toplevel,
                        &["stash", "push", "--include-untracked", "-m", &message],
                    )]);
                }
                Step::Run(vec![self.add()])
            }
            STASH => {
                if !exit.ok() {
                    return Step::Finish(Err(format!(
                        "cannot set the uncommitted changes aside: {}",
                        exit.reason()
                    )));
                }
                self.stashed = !exit.text().contains("No local changes to save");
                Step::Run(vec![self.add()])
            }
            ADD => {
                if !exit.ok() {
                    let reason = format!("git worktree add: {}", exit.reason());
                    if self.stashed {
                        self.failure = Some(reason);
                        return Step::Run(vec![git(
                            RESTORE,
                            &self.record.toplevel,
                            &["stash", "pop", "--index"],
                        )]);
                    }
                    return Step::Finish(Err(reason));
                }
                Step::Run(vec![Cmd {
                    tag: INCLUDE,
                    program: "bash",
                    args: vec![
                        "-c".into(),
                        INCLUDE_SCRIPT.into(),
                        "qcc".into(),
                        record.toplevel.clone(),
                        record.worktree.clone(),
                    ],
                    cwd: None,
                    env: Vec::new(),
                    limit: OUTPUT_LIMIT,
                    timeout_ms: SCRIPT_TIMEOUT_MS,
                }])
            }
            INCLUDE => {
                // A file that could not be copied is the setup script's problem to
                // notice, not a reason to give up on the worktree.
                let env = vec![
                    ("QCC_SOURCE_DIR".to_owned(), record.toplevel.clone()),
                    ("QCC_THREAD_ID".to_owned(), self.thread.clone()),
                ];
                Step::Run(vec![script(SETUP, &record.worktree, SETUP_SCRIPT, env)])
            }
            SETUP => {
                if !exit.ok() {
                    return self.undo(format!("{SETUP_SCRIPT}: {}", exit.reason()));
                }
                if self.stashed {
                    return Step::Run(vec![git(
                        POP,
                        &record.worktree,
                        &["stash", "pop", "--index"],
                    )]);
                }
                Step::Finish(Ok(Answer::Ready(record.clone())))
            }
            POP => {
                if !exit.ok() {
                    return self.undo(format!(
                        "cannot bring the uncommitted changes into the worktree: {}",
                        exit.reason()
                    ));
                }
                self.stashed = false;
                Step::Finish(Ok(Answer::Ready(record.clone())))
            }
            UNDO_WORKTREE => Step::Run(vec![git(
                UNDO_BRANCH,
                &record.toplevel,
                &["branch", "-D", &record.branch],
            )]),
            UNDO_BRANCH if self.stashed => {
                Step::Run(vec![git(RESTORE, &record.toplevel, &["stash", "pop", "--index"])])
            }
            RESTORE if !exit.ok() => {
                let reason = self.failure.take().unwrap_or_default();
                Step::Finish(Err(format!(
                    "{reason}; the uncommitted changes could not be put back either ({}) \
                     and are kept in `git stash` of {}",
                    exit.reason(),
                    record.toplevel
                )))
            }
            _ => self.fail(),
        }
    }

    fn add(&self) -> Cmd {
        let record = &self.record;
        // `--no-track`: the branch is the thread's own, not a follower of its base,
        // whatever `branch.autoSetupMerge` says.
        git(
            prepare::ADD,
            &record.toplevel,
            &[
                "worktree",
                "add",
                "--no-track",
                "-b",
                &record.branch,
                &record.worktree,
                &record.base,
            ],
        )
    }
}

// ----- remove -------------------------------------------------------------------------

mod remove {
    pub const STATUS: u32 = 0;
    pub const TEARDOWN: u32 = 1;
    pub const WORKTREE: u32 = 2;
    pub const BRANCH: u32 = 3;
    pub const RMDIR: u32 = 4;
}

/// The teardown script, `git worktree remove`, and the branch if nothing on it would
/// be lost. Uncommitted work stops it unless forced.
pub struct Remove {
    record: Record,
    force: bool,
}

impl Remove {
    pub fn new(record: Record, force: bool) -> Self {
        Self { record, force }
    }

    fn teardown(&self) -> Step {
        Step::Run(vec![script(
            remove::TEARDOWN,
            &self.record.worktree,
            TEARDOWN_SCRIPT,
            vec![("QCC_SOURCE_DIR".to_owned(), self.record.toplevel.clone())],
        )])
    }

    fn start(&mut self) -> Step {
        if self.force {
            return self.teardown();
        }
        Step::Run(vec![git(remove::STATUS, &self.record.worktree, &["status", "--porcelain"])])
    }

    fn on_exit(&mut self, tag: u32, exit: Exit) -> Step {
        use remove::*;
        let record = &self.record;
        match tag {
            STATUS => {
                // Only a worktree that is really gone lets a failed status through:
                // anything else (a timeout, too much output, git not starting) says
                // nothing about uncommitted work, and the next step is `--force`.
                if !exit.ok() {
                    if worktree_gone(&exit) {
                        return self.teardown();
                    }
                    return Step::Finish(Err(format!(
                        "cannot check {} for uncommitted changes: {}",
                        record.worktree,
                        exit.reason()
                    )));
                }
                let dirty = exit.text().lines().filter(|line| !line.is_empty()).count();
                if dirty > 0 {
                    return Step::Finish(Err(format!(
                        "{} has {dirty} uncommitted change{}; commit them or force the removal",
                        record.worktree,
                        if dirty == 1 { "" } else { "s" }
                    )));
                }
                self.teardown()
            }
            // A failing teardown is the repository's problem; it never keeps the
            // worktree alive.
            TEARDOWN => Step::Run(vec![git(
                WORKTREE,
                &record.toplevel,
                &["worktree", "remove", "--force", &record.worktree],
            )]),
            WORKTREE => {
                if !exit.ok() && !exit.stderr.contains("is not a working tree") {
                    return Step::Finish(Err(format!("git worktree remove: {}", exit.reason())));
                }
                // `-d`, not `-D`: git refuses when the branch has commits that are
                // nowhere else, and then the branch stays.
                Step::Run(vec![git(BRANCH, &record.toplevel, &["branch", "-d", &record.branch])])
            }
            BRANCH => {
                let parent = record.worktree.rsplit_once('/').map_or("", |(parent, _)| parent);
                Step::Run(vec![Cmd {
                    tag: RMDIR,
                    program: "rmdir",
                    args: vec![parent.to_owned()],
                    cwd: None,
                    env: Vec::new(),
                    limit: OUTPUT_LIMIT,
                    timeout_ms: GIT_TIMEOUT_MS,
                }])
            }
            _ => Step::Finish(Ok(Answer::Removed)),
        }
    }
}

// ----- inspect ------------------------------------------------------------------------

/// The changes or the diff of a directory that is not one of our worktrees: the
/// thread works in a checkout of the user's own. Its uncommitted work against HEAD,
/// measured from the top of the checkout so paths read the same as a worktree's.
pub struct Inspect {
    cwd: String,
    diff: bool,
    /// The job that does the work, once the top of the checkout is known.
    inner: Option<Box<Job>>,
}

/// The tag of `rev-parse`, apart from the tags of the job it leads to.
const TOPLEVEL_OF: u32 = u32::MAX;

impl Inspect {
    pub fn new(cwd: String, diff: bool) -> Self {
        Self { cwd, diff, inner: None }
    }

    fn start(&mut self) -> Step {
        Step::Run(vec![git(TOPLEVEL_OF, &self.cwd, &["rev-parse", "--show-toplevel"])])
    }

    fn on_exit(&mut self, tag: u32, exit: Exit) -> Step {
        if let Some(inner) = &mut self.inner {
            return inner.on_exit(tag, exit);
        }
        if !exit.ok() {
            return Step::Finish(Err(format!(
                "{} is not in a git checkout: {}",
                self.cwd,
                exit.reason()
            )));
        }
        let toplevel = exit.text().trim().to_owned();
        let record = Record {
            toplevel: toplevel.clone(),
            worktree: toplevel,
            cwd: self.cwd.clone(),
            branch: String::new(),
            base: "HEAD".to_owned(),
        };
        let mut inner = Box::new(if self.diff {
            Job::Diff(Diff::new(record))
        } else {
            Job::Changes(Changes::new(record))
        });
        let step = inner.start();
        self.inner = Some(inner);
        step
    }
}

// ----- changes ------------------------------------------------------------------------

const NO_RENAMES: &str = "--no-renames";

fn untracked(tag: u32, worktree: &str) -> Cmd {
    git(tag, worktree, &["ls-files", "-z", "--others", "--exclude-standard"])
}

fn nul_separated(bytes: &[u8]) -> Vec<String> {
    bytes
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .map(|part| String::from_utf8_lossy(part).into_owned())
        .collect()
}

/// `git diff --name-status -z`: status and path alternate.
pub fn parse_name_status(bytes: &[u8]) -> Vec<(ChangeKind, String)> {
    let parts = nul_separated(bytes);
    parts
        .as_chunks::<2>()
        .0
        .iter()
        .map(|[status, path]| {
            let kind = match status.as_str() {
                "A" => ChangeKind::Added,
                "D" => ChangeKind::Deleted,
                _ => ChangeKind::Modified,
            };
            (kind, path.clone())
        })
        .collect()
}

/// `git diff --numstat -z`: `added<TAB>deleted<TAB>path`; binary files have dashes.
pub fn parse_numstat(bytes: &[u8]) -> Vec<(String, Option<u32>, Option<u32>)> {
    nul_separated(bytes)
        .into_iter()
        .filter_map(|entry| {
            let mut fields = entry.splitn(3, '\t');
            let additions = fields.next()?.parse().ok();
            let deletions = fields.next()?.parse().ok();
            Some((fields.next()?.to_owned(), additions, deletions))
        })
        .collect()
}

mod changes {
    pub const NAMES: u32 = 0;
    pub const STATS: u32 = 1;
    pub const UNTRACKED: u32 = 2;
    pub const BRANCH: u32 = 3;
    pub const COUNT: usize = 4;
}

/// Everything since the base commit, committed or not, plus files git does not
/// know yet.
pub struct Changes {
    record: Record,
    exits: Vec<Option<Exit>>,
}

impl Changes {
    pub fn new(record: Record) -> Self {
        Self { record, exits: vec![None; changes::COUNT] }
    }

    fn start(&mut self) -> Step {
        use changes::*;
        let (worktree, base) = (&self.record.worktree, &self.record.base);
        Step::Run(vec![
            git(NAMES, worktree, &["diff", NO_RENAMES, "--name-status", "-z", base]),
            git(STATS, worktree, &["diff", NO_RENAMES, "--numstat", "-z", base]),
            untracked(UNTRACKED, worktree),
            git(BRANCH, worktree, &["rev-parse", "--abbrev-ref", "HEAD"]),
        ])
    }

    fn on_exit(&mut self, tag: u32, exit: Exit) -> Step {
        use changes::*;
        if let Some(slot) = self.exits.get_mut(tag as usize) {
            *slot = Some(exit);
        }
        if self.exits.iter().any(Option::is_none) {
            return Step::Wait;
        }
        let exits: Vec<Exit> = self.exits.drain(..).flatten().collect();
        // The branch is decoration; the rest is the answer.
        if let Some(failed) = exits[..BRANCH as usize].iter().find(|exit| !exit.ok()) {
            return Step::Finish(Err(format!("git: {}", failed.reason())));
        }
        let stats: HashMap<String, (Option<u32>, Option<u32>)> =
            parse_numstat(&exits[STATS as usize].stdout)
                .into_iter()
                .map(|(name, additions, deletions)| (name, (additions, deletions)))
                .collect();
        let mut files: Vec<ChangedFile> = parse_name_status(&exits[NAMES as usize].stdout)
            .into_iter()
            .map(|(kind, path)| {
                let (additions, deletions) = stats.get(&path).copied().unwrap_or_default();
                ChangedFile { additions, deletions, path, kind }
            })
            .collect();
        files.extend(nul_separated(&exits[UNTRACKED as usize].stdout).into_iter().map(|path| {
            ChangedFile { path, kind: ChangeKind::Untracked, additions: None, deletions: None }
        }));
        files.sort_by(|a, b| a.path.cmp(&b.path));
        let branch = exits[BRANCH as usize].text().trim().to_owned();
        Step::Finish(Ok(Answer::Changes(EnvironmentChanges {
            // A detached HEAD has no branch name.
            branch: (exits[BRANCH as usize].ok() && branch != "HEAD" && !branch.is_empty())
                .then_some(branch),
            base: Some(self.record.base.clone()),
            files,
        })))
    }
}

// ----- deliver ------------------------------------------------------------------------

/// Applies everything the worktree has since its base (commits, uncommitted and new
/// files) to the source checkout as uncommitted changes: all of it or, when it does
/// not fit, nothing. The worktree's own index is left alone: new files are added to
/// a copy of it. `$1` worktree, `$2` base commit, `$3` source checkout. The last
/// line of output says how many files changed.
const APPLY_SCRIPT: &str = r#"
set -euo pipefail
wt=$1; base=$2; src=$3
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
cp "$(git -C "$wt" rev-parse --path-format=absolute --git-path index)" "$tmp/index"
GIT_INDEX_FILE="$tmp/index" git -C "$wt" add -A
GIT_INDEX_FILE="$tmp/index" git -C "$wt" diff --cached --binary --no-color --no-ext-diff --no-renames "$base" >"$tmp/patch"
if [ ! -s "$tmp/patch" ]; then echo 0; exit 0; fi
if ! git -C "$src" apply --check "$tmp/patch" 2>"$tmp/err"; then
  echo "the changes do not apply to $src as it is now: $(tail -n 3 "$tmp/err" | tr '
' ' ')" >&2
  exit 3
fi
git -C "$src" apply "$tmp/patch"
git -C "$src" apply --numstat "$tmp/patch" | wc -l
"#;

/// Commits what the worktree has uncommitted on the thread's branch, then merges
/// the branch into whatever the source checkout has checked out. A conflict undoes
/// the merge. `$1` worktree, `$2` branch, `$3` source checkout, `$4` thread. The
/// last line of output names the branch merged into.
const MERGE_SCRIPT: &str = r#"
set -euo pipefail
wt=$1; branch=$2; src=$3; thread=$4
target=$(git -C "$src" symbolic-ref --quiet --short HEAD) || {
  echo "$src has no branch checked out (detached HEAD); check one out to merge into" >&2
  exit 3
}
if [ -n "$(git -C "$wt" status --porcelain)" ]; then
  git -C "$wt" add -A
  git -C "$wt" commit --quiet -m "qcc: work of thread $thread" || {
    echo "cannot commit the uncommitted work in the worktree (is git user.name/user.email set?)" >&2
    exit 3
  }
fi
if ! out=$(git -C "$src" merge --no-ff --no-edit "$branch" 2>&1); then
  files=$(git -C "$src" diff --name-only --diff-filter=U | head -n 10 | tr '
' ' ')
  git -C "$src" merge --abort >/dev/null 2>&1 || true
  if [ -n "$files" ]; then
    echo "merging $branch into $target conflicts in: $files; nothing was merged" >&2
  else
    echo "git merge refused: $(printf '%s' "$out" | tail -n 3 | tr '
' ' ')" >&2
  fi
  exit 3
fi
echo "$target"
"#;

/// Brings the worktree's work back to the source checkout. See [`APPLY_SCRIPT`] and
/// [`MERGE_SCRIPT`].
pub struct Deliver {
    record: Record,
    thread: String,
    merge: bool,
}

impl Deliver {
    pub fn new(record: Record, thread: String, merge: bool) -> Self {
        Self { record, thread, merge }
    }

    fn start(&mut self) -> Step {
        let record = &self.record;
        let (script, args) = if self.merge {
            (
                MERGE_SCRIPT,
                vec![
                    record.worktree.clone(),
                    record.branch.clone(),
                    record.toplevel.clone(),
                    self.thread.clone(),
                ],
            )
        } else {
            (
                APPLY_SCRIPT,
                vec![record.worktree.clone(), record.base.clone(), record.toplevel.clone()],
            )
        };
        let mut all = vec!["-c".to_owned(), script.to_owned(), "qcc".to_owned()];
        all.extend(args);
        Step::Run(vec![Cmd {
            tag: 0,
            program: "bash",
            args: all,
            cwd: None,
            env: vec![("LC_ALL".to_owned(), "C".to_owned())],
            limit: OUTPUT_LIMIT,
            timeout_ms: GIT_TIMEOUT_MS,
        }])
    }

    fn on_exit(&mut self, _tag: u32, exit: Exit) -> Step {
        if !exit.ok() {
            return Step::Finish(Err(exit.reason()));
        }
        let text = exit.text();
        let last = text.lines().last().unwrap_or("").trim();
        let source = &self.record.toplevel;
        Step::Finish(Ok(Answer::Delivered(if self.merge {
            format!("Merged {} into {last} in {source}", self.record.branch)
        } else {
            match last.parse::<u32>().unwrap_or(0) {
                0 => "Nothing to apply: the thread changed no files".to_owned(),
                1 => format!("Applied the changes to 1 file in {source}, uncommitted"),
                count => format!("Applied the changes to {count} files in {source}, uncommitted"),
            }
        })))
    }
}

// ----- diff ---------------------------------------------------------------------------

mod diff {
    pub const TRACKED: u32 = 0;
    pub const UNTRACKED: u32 = 1;
    /// One new file, shown as an addition.
    pub const NEW_FILE: u32 = 2;
}

/// The same changes as one patch. New files are diffed against nothing, one by
/// one, because git only diffs what it tracks.
pub struct Diff {
    record: Record,
    tracked: Option<Exit>,
    untracked: Option<Exit>,
    /// New files still to diff, last first.
    pending: Vec<String>,
    out: String,
    truncated: bool,
}

impl Diff {
    pub fn new(record: Record) -> Self {
        Self {
            record,
            tracked: None,
            untracked: None,
            pending: Vec::new(),
            out: String::new(),
            truncated: false,
        }
    }

    fn start(&mut self) -> Step {
        let (worktree, base) = (&self.record.worktree, &self.record.base);
        let mut tracked = git(
            diff::TRACKED,
            worktree,
            &["diff", "--no-color", "--no-ext-diff", NO_RENAMES, base],
        );
        tracked.limit = DIFF_LIMIT;
        Step::Run(vec![tracked, untracked(diff::UNTRACKED, worktree)])
    }

    fn append(&mut self, exit: &Exit) {
        let room = DIFF_LIMIT.saturating_sub(self.out.len());
        let text = exit.text();
        if exit.truncated || text.len() > room {
            self.truncated = true;
        }
        let mut cut = text.len().min(room);
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        self.out.push_str(&text[..cut]);
    }

    fn next_file(&mut self) -> Step {
        if self.truncated {
            self.pending.clear();
        }
        let Some(path) = self.pending.pop() else {
            return Step::Finish(Ok(Answer::Diff {
                diff: std::mem::take(&mut self.out),
                truncated: self.truncated,
            }));
        };
        let mut cmd = git(
            diff::NEW_FILE,
            &self.record.worktree,
            &["diff", "--no-color", "--no-ext-diff", "--no-index", "--", "/dev/null", &path],
        );
        cmd.limit = DIFF_LIMIT;
        Step::Run(vec![cmd])
    }

    fn on_exit(&mut self, tag: u32, exit: Exit) -> Step {
        match tag {
            diff::TRACKED => self.tracked = Some(exit),
            diff::UNTRACKED => self.untracked = Some(exit),
            _ => {
                // `--no-index` exits with 1 when the files differ, which they do.
                self.append(&exit);
                return self.next_file();
            }
        }
        if self.tracked.is_none() || self.untracked.is_none() {
            return Step::Wait;
        }
        let (Some(tracked), Some(untracked)) = (self.tracked.take(), self.untracked.take()) else {
            unreachable!("both exits were just checked");
        };
        if !tracked.ok() && !tracked.truncated {
            return Step::Finish(Err(format!("git diff: {}", tracked.reason())));
        }
        if !untracked.ok() {
            return Step::Finish(Err(format!("git ls-files: {}", untracked.reason())));
        }
        let mut files = nul_separated(&untracked.stdout);
        if files.len() > MAX_UNTRACKED_DIFFS {
            files.truncate(MAX_UNTRACKED_DIFFS);
            self.truncated = true;
        }
        files.reverse();
        self.append(&tracked);
        self.pending = files;
        self.next_file()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn out(text: &str) -> Exit {
        Exit { code: Some(0), stdout: text.as_bytes().to_vec(), ..Exit::default() }
    }

    fn failed(stderr: &str) -> Exit {
        Exit { code: Some(1), stderr: stderr.into(), ..Exit::default() }
    }

    /// How git itself fails: exit code 128.
    fn fatal(stderr: &str) -> Exit {
        Exit { code: Some(128), stderr: stderr.into(), ..Exit::default() }
    }

    fn run(step: Step) -> Vec<Cmd> {
        match step {
            Step::Run(cmds) => cmds,
            other => panic!("expected commands, got {other:?}"),
        }
    }

    fn record() -> Record {
        Record {
            toplevel: "/src/app".into(),
            worktree: "/data/thr_1/app".into(),
            cwd: "/data/thr_1/app".into(),
            branch: "qcc/thr_1".into(),
            base: "abc123".into(),
        }
    }

    #[test]
    fn prepare_makes_a_worktree_on_its_own_branch() {
        let mut job = Prepare::new("thr_1".into(), "/src/app/web".into(), "/data".into(), None);
        let first = run(job.start());
        assert_eq!(
            first[0].args,
            ["-C", "/src/app/web", "rev-parse", "--show-toplevel", "--show-prefix"]
        );

        let base = run(job.on_exit(prepare::TOPLEVEL, out("/src/app\nweb/\n")));
        assert_eq!(base[0].args.last().unwrap(), "HEAD^{commit}");

        let add = run(job.on_exit(prepare::BASE, out("abc123\n")));
        assert_eq!(
            add[0].args,
            [
                "-C",
                "/src/app",
                "worktree",
                "add",
                "--no-track",
                "-b",
                "qcc/thr_1",
                "/data/thr_1/app",
                "abc123"
            ]
        );

        let include = run(job.on_exit(prepare::ADD, out("")));
        assert_eq!(include[0].program, "bash");
        assert_eq!(&include[0].args[3..], ["/src/app", "/data/thr_1/app"]);

        // A file that could not be copied does not stop the setup.
        let setup = run(job.on_exit(prepare::INCLUDE, failed("cannot copy .env")));
        assert_eq!(setup[0].cwd.as_deref(), Some("/data/thr_1/app"));
        assert_eq!(setup[0].args.last().unwrap(), SETUP_SCRIPT);
        assert_eq!(setup[0].timeout_ms, SCRIPT_TIMEOUT_MS);

        // The thread works where it was started: in the same subdirectory.
        let Step::Finish(Ok(Answer::Ready(record))) = job.on_exit(prepare::SETUP, out("")) else {
            panic!("not ready");
        };
        assert_eq!(record.cwd, "/data/thr_1/app/web");
        assert_eq!(Record::from_json(&record.to_json()), Some(record));
    }

    #[test]
    fn prepare_explains_what_is_wrong_with_the_source() {
        let mut job = Prepare::new("thr_1".into(), "/tmp/x".into(), "/data".into(), None);
        job.start();
        let step = job.on_exit(prepare::TOPLEVEL, fatal("fatal: not a git repository"));
        assert_eq!(step, Step::Finish(Err("/tmp/x is not inside a git repository".into())));

        let mut job = Prepare::new("thr_1".into(), "/tmp/x".into(), "/data".into(), None);
        job.start();
        let step = job.on_exit(
            prepare::TOPLEVEL,
            Exit { stderr: "no such program".into(), ..Exit::default() },
        );
        assert_eq!(step, Step::Finish(Err("git rev-parse in /tmp/x: no such program".into())));

        let mut job = Prepare::new("thr_1".into(), "/src/app".into(), "/data".into(), None);
        job.start();
        job.on_exit(prepare::TOPLEVEL, out("/src/app\n\n"));
        let Step::Finish(Err(message)) = job.on_exit(prepare::BASE, failed("")) else {
            panic!("an empty repository was accepted");
        };
        assert!(message.contains("no commits yet"), "{message}");

        let base = Some("nope".to_owned());
        let mut job = Prepare::new("thr_1".into(), "/src/app".into(), "/data".into(), base);
        job.start();
        job.on_exit(prepare::TOPLEVEL, out("/src/app\n\n"));
        let Step::Finish(Err(message)) = job.on_exit(prepare::BASE, failed("")) else {
            panic!("an unknown base was accepted");
        };
        assert!(message.contains("\"nope\" does not name a commit"), "{message}");

        let mut job = Prepare::new("thr_1".into(), "/src/app".into(), "/data".into(), None);
        job.start();
        job.on_exit(prepare::TOPLEVEL, out("/src/app\n\n"));
        let timed_out = Exit { timed_out: true, ..Exit::default() };
        let step = job.on_exit(prepare::BASE, timed_out);
        assert_eq!(step, Step::Finish(Err("git rev-parse: timed out".into())));
    }

    #[test]
    fn carried_changes_go_into_the_worktree_or_back_where_they_were() {
        let carrying = || {
            let mut job = Prepare::new("thr_1".into(), "/src/app".into(), "/data".into(), None)
                .carrying(true);
            job.start();
            job.on_exit(prepare::TOPLEVEL, out("/src/app\n\n"));
            job
        };
        let mut job = carrying();
        let stash = run(job.on_exit(prepare::BASE, out("abc123\n")));
        assert_eq!(&stash[0].args[2..5], ["stash", "push", "--include-untracked"]);
        run(job.on_exit(prepare::STASH, out("Saved working directory and index state")));
        run(job.on_exit(prepare::ADD, out("")));
        run(job.on_exit(prepare::INCLUDE, out("")));
        // Popped in the worktree, after its setup.
        let pop = run(job.on_exit(prepare::SETUP, out("")));
        assert_eq!(pop[0].args, ["-C", "/data/thr_1/app", "stash", "pop", "--index"]);
        assert!(matches!(job.on_exit(prepare::POP, out("")), Step::Finish(Ok(Answer::Ready(_)))));

        // Nothing to carry: no pop.
        let mut job = carrying();
        job.on_exit(prepare::BASE, out("abc123\n"));
        job.on_exit(prepare::STASH, out("No local changes to save\n"));
        job.on_exit(prepare::ADD, out(""));
        job.on_exit(prepare::INCLUDE, out(""));
        assert!(matches!(job.on_exit(prepare::SETUP, out("")), Step::Finish(Ok(Answer::Ready(_)))));

        // A failed pop takes the worktree down and gives the work back to the source.
        let mut job = carrying();
        job.on_exit(prepare::BASE, out("abc123\n"));
        job.on_exit(prepare::STASH, out("Saved"));
        job.on_exit(prepare::ADD, out(""));
        job.on_exit(prepare::INCLUDE, out(""));
        job.on_exit(prepare::SETUP, out(""));
        let undo = run(job.on_exit(prepare::POP, failed("conflict")));
        assert_eq!(&undo[0].args[2..4], ["worktree", "remove"]);
        run(job.on_exit(prepare::UNDO_WORKTREE, out("")));
        let restore = run(job.on_exit(prepare::UNDO_BRANCH, out("")));
        assert_eq!(restore[0].args, ["-C", "/src/app", "stash", "pop", "--index"]);
        let Step::Finish(Err(message)) = job.on_exit(prepare::RESTORE, out("")) else {
            panic!("a failed pop was accepted");
        };
        assert!(message.contains("cannot bring the uncommitted changes"), "{message}");
    }

    #[test]
    fn deliver_says_what_it_did() {
        let mut apply = Deliver::new(record(), "thr_1".into(), false);
        let script = run(apply.start());
        assert_eq!(&script[0].args[3..], ["/data/thr_1/app", "abc123", "/src/app"]);
        let Step::Finish(Ok(Answer::Delivered(said))) = apply.on_exit(0, out("2\n")) else {
            panic!("not delivered");
        };
        assert_eq!(said, "Applied the changes to 2 files in /src/app, uncommitted");

        let mut merge = Deliver::new(record(), "thr_1".into(), true);
        merge.start();
        let Step::Finish(Ok(Answer::Delivered(said))) = merge.on_exit(0, out("main\n")) else {
            panic!("not merged");
        };
        assert_eq!(said, "Merged qcc/thr_1 into main in /src/app");
        let refused = merge.on_exit(0, failed("conflicts in: a.txt"));
        assert_eq!(refused, Step::Finish(Err("conflicts in: a.txt".into())));
    }

    #[test]
    fn a_failed_setup_takes_the_worktree_down_again() {
        let mut job = Prepare::new("thr_1".into(), "/src/app".into(), "/data".into(), None);
        job.start();
        job.on_exit(prepare::TOPLEVEL, out("/src/app\n\n"));
        job.on_exit(prepare::BASE, out("abc123\n"));
        job.on_exit(prepare::ADD, out(""));
        job.on_exit(prepare::INCLUDE, out(""));

        let undo = run(job.on_exit(prepare::SETUP, failed("npm: command not found")));
        assert_eq!(&undo[0].args[2..], ["worktree", "remove", "--force", "/data/thr_1/app"]);
        let branch = run(job.on_exit(prepare::UNDO_WORKTREE, out("")));
        assert_eq!(&branch[0].args[2..], ["branch", "-D", "qcc/thr_1"]);
        assert_eq!(
            job.on_exit(prepare::UNDO_BRANCH, out("")),
            Step::Finish(Err(".qcc-env-setup.sh: npm: command not found".into()))
        );
    }

    #[test]
    fn remove_refuses_to_lose_uncommitted_work_unless_forced() {
        let mut job = Remove::new(record(), false);
        let status = run(job.start());
        assert_eq!(&status[0].args[2..], ["status", "--porcelain"]);
        let Step::Finish(Err(message)) = job.on_exit(remove::STATUS, out(" M a.rs\n?? b.rs\n"))
        else {
            panic!("dirty worktree was removed");
        };
        assert!(message.contains("2 uncommitted changes"), "{message}");

        let mut job = Remove::new(record(), true);
        let teardown = run(job.start());
        assert_eq!(teardown[0].args.last().unwrap(), TEARDOWN_SCRIPT);
    }

    #[test]
    fn remove_runs_teardown_then_removes_worktree_branch_and_directory() {
        let mut job = Remove::new(record(), false);
        job.start();
        let teardown = run(job.on_exit(remove::STATUS, out("")));
        assert_eq!(teardown[0].cwd.as_deref(), Some("/data/thr_1/app"));
        // A failing teardown does not keep the worktree alive.
        let worktree = run(job.on_exit(remove::TEARDOWN, failed("docker: not found")));
        assert_eq!(&worktree[0].args[2..], ["worktree", "remove", "--force", "/data/thr_1/app"]);
        let branch = run(job.on_exit(remove::WORKTREE, out("")));
        // The safe delete: git keeps a branch with commits that are nowhere else.
        assert_eq!(&branch[0].args[2..], ["branch", "-d", "qcc/thr_1"]);
        let rmdir = run(job.on_exit(remove::BRANCH, failed("not fully merged")));
        assert_eq!(
            (rmdir[0].program, rmdir[0].args.as_slice()),
            ("rmdir", &["/data/thr_1".to_owned()][..])
        );
        assert_eq!(job.on_exit(remove::RMDIR, out("")), Step::Finish(Ok(Answer::Removed)));
    }

    #[test]
    fn remove_never_forces_a_worktree_it_could_not_check() {
        // Each of these says nothing about uncommitted work; a dirty worktree with
        // more than the output limit of changes is the case that matters most.
        let unknown = [
            Exit { truncated: true, ..Exit::default() },
            Exit { timed_out: true, ..Exit::default() },
            Exit { stderr: "cannot run git".into(), ..Exit::default() },
            fatal("fatal: unable to read index"),
        ];
        for exit in unknown {
            let mut job = Remove::new(record(), false);
            job.start();
            let step = job.on_exit(remove::STATUS, exit.clone());
            let Step::Finish(Err(message)) = &step else { panic!("{exit:?} let {step:?} through") };
            assert!(message.starts_with("cannot check /data/thr_1/app"), "{message}");
        }
    }

    #[test]
    fn remove_cleans_up_after_a_worktree_that_is_already_gone() {
        let mut job = Remove::new(record(), false);
        job.start();
        let missing = fatal("fatal: cannot change to '/data/thr_1/app': No such file or directory");
        run(job.on_exit(remove::STATUS, missing));
        let gone = failed("fatal: '/data/thr_1/app' is not a working tree");
        run(job.on_exit(remove::TEARDOWN, Exit::default()));
        let branch = run(job.on_exit(remove::WORKTREE, gone));
        assert_eq!(&branch[0].args[2..], ["branch", "-d", "qcc/thr_1"]);

        let mut job = Remove::new(record(), true);
        job.start();
        job.on_exit(remove::TEARDOWN, out(""));
        let step = job.on_exit(remove::WORKTREE, failed("fatal: permission denied"));
        assert_eq!(step, Step::Finish(Err("git worktree remove: fatal: permission denied".into())));
    }

    #[test]
    fn changes_join_names_stats_and_untracked_files() {
        let mut job = Changes::new(record());
        assert_eq!(run(job.start()).len(), changes::COUNT);
        assert_eq!(job.on_exit(changes::BRANCH, out("qcc/thr_1\n")), Step::Wait);
        assert_eq!(
            job.on_exit(changes::NAMES, out("M\0src/a.rs\0A\0img.png\0D\0old.txt\0")),
            Step::Wait
        );
        assert_eq!(job.on_exit(changes::UNTRACKED, out("notes.md\0")), Step::Wait);
        let step =
            job.on_exit(changes::STATS, out("3\t1\tsrc/a.rs\0-\t-\timg.png\x000\t7\told.txt\0"));
        let Step::Finish(Ok(Answer::Changes(changes))) = step else { panic!("{step:?}") };
        assert_eq!(changes.branch.as_deref(), Some("qcc/thr_1"));
        assert_eq!(changes.base.as_deref(), Some("abc123"));
        let rows: Vec<_> = changes
            .files
            .iter()
            .map(|f| (f.path.as_str(), f.kind, f.additions, f.deletions))
            .collect();
        assert_eq!(
            rows,
            [
                ("img.png", ChangeKind::Added, None, None),
                ("notes.md", ChangeKind::Untracked, None, None),
                ("old.txt", ChangeKind::Deleted, Some(0), Some(7)),
                ("src/a.rs", ChangeKind::Modified, Some(3), Some(1)),
            ]
        );
    }

    #[test]
    fn a_checkout_of_the_users_own_is_measured_from_its_top_against_head() {
        let mut job = Inspect::new("/src/app/crates".into(), false);
        let first = run(job.start());
        assert_eq!(first[0].args, ["-C", "/src/app/crates", "rev-parse", "--show-toplevel"]);
        let next = run(job.on_exit(TOPLEVEL_OF, out("/src/app\n")));
        assert_eq!(next.len(), changes::COUNT);
        assert_eq!(next[0].args[..2], ["-C", "/src/app"]);
        assert_eq!(next[0].args.last().map(String::as_str), Some("HEAD"));
        job.on_exit(changes::BRANCH, out("main\n"));
        job.on_exit(changes::NAMES, out("M\0src/a.rs\0"));
        job.on_exit(changes::UNTRACKED, out(""));
        let step = job.on_exit(changes::STATS, out("1\t1\tsrc/a.rs\0"));
        let Step::Finish(Ok(Answer::Changes(changes))) = step else { panic!("{step:?}") };
        assert_eq!(
            (changes.branch.as_deref(), changes.base.as_deref()),
            (Some("main"), Some("HEAD"))
        );
        assert_eq!(changes.files.len(), 1);
    }

    #[test]
    fn a_directory_outside_git_says_so() {
        let mut job = Inspect::new("/tmp/notes".into(), true);
        job.start();
        let step = job.on_exit(TOPLEVEL_OF, fatal("fatal: not a git repository"));
        let Step::Finish(Err(error)) = step else { panic!("{step:?}") };
        assert!(error.contains("/tmp/notes is not in a git checkout"), "{error}");
    }

    #[test]
    fn changes_report_a_git_failure() {
        let mut job = Changes::new(record());
        job.start();
        job.on_exit(changes::BRANCH, out("HEAD\n"));
        job.on_exit(changes::NAMES, failed("fatal: bad object abc123"));
        job.on_exit(changes::STATS, failed("fatal: bad object abc123"));
        let step = job.on_exit(changes::UNTRACKED, out(""));
        assert_eq!(step, Step::Finish(Err("git: fatal: bad object abc123".into())));
    }

    #[test]
    fn diff_appends_new_files_to_the_tracked_patch() {
        let mut job = Diff::new(record());
        let first = run(job.start());
        assert_eq!(first[0].limit, DIFF_LIMIT);
        assert_eq!(job.on_exit(diff::UNTRACKED, out("b.txt\0a.txt\0")), Step::Wait);
        let next = run(job.on_exit(diff::TRACKED, out("diff --git a/x b/x\n")));
        assert_eq!(&next[0].args[5..], ["--no-index", "--", "/dev/null", "b.txt"]);
        // Exit code 1 is how `--no-index` says "they differ".
        let differ = |text: &str| Exit { code: Some(1), ..out(text) };
        let next = run(job.on_exit(diff::NEW_FILE, differ("diff --git a/b.txt b/b.txt\n")));
        assert_eq!(next[0].args.last().unwrap(), "a.txt");
        let step = job.on_exit(diff::NEW_FILE, differ("diff --git a/a.txt b/a.txt\n"));
        assert_eq!(
            step,
            Step::Finish(Ok(Answer::Diff {
                diff:
                    "diff --git a/x b/x\ndiff --git a/b.txt b/b.txt\ndiff --git a/a.txt b/a.txt\n"
                        .into(),
                truncated: false,
            }))
        );
    }

    #[test]
    fn an_oversized_diff_is_cut_and_says_so() {
        let mut job = Diff::new(record());
        job.start();
        job.on_exit(diff::UNTRACKED, out("new.txt\0"));
        // The tracked diff hit its limit and was stopped: no exit code.
        let big = Exit { code: None, truncated: true, ..out("diff --git a/x b/x\n") };
        let step = job.on_exit(diff::TRACKED, big);
        assert_eq!(
            step,
            Step::Finish(Ok(Answer::Diff { diff: "diff --git a/x b/x\n".into(), truncated: true }))
        );
    }
}

//! The reviewers plugin on the real wasm host, with the test playing the daemon:
//! it answers for threads, diffs and comments and tells the plugin of turns.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use qcc_domain::{
    ChangeKind, ChangedFile, EnvironmentChanges, ModelInfo, ProviderCapabilities, ProviderInfo,
};
use qcc_domain::{
    CommentAuthor, CommentId, LineComment, PermissionMode, Role, ThreadId, ThreadState,
    TranscriptEntry, TurnStatus,
};
use qcc_plugin_host::{Grant, Outcome, PluginError, PluginRequest, SettledThread, UiRequest};
use qcc_plugin_testkit::{Bench, ThreadSummary, ThreadsAnswer, ThreadsCall};
use qcc_runtime::NewThread;

const SOURCE: &str = "thr_src";

const DIFF: &str = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n\
@@ -1,2 +1,2 @@\n fn a() {\n-    old();\n+    new();\n";

fn summary(id: &str, state: ThreadState, last: Option<TurnStatus>) -> ThreadSummary {
    let mut thread = NewThread {
        id: ThreadId::new(id),
        provider: "echo".into(),
        cwd: "/work".into(),
        environment: None,
        model: None,
        effort: None,
        parent: None,
        project: None,
        permission_mode: None,
        skills_enabled: Vec::new(),
        skills_disabled: Vec::new(),
        started_by: None,
        hidden: false,
        automation: None,
        fork: None,
    }
    .into_thread(0);
    thread.last_turn_status = last;
    ThreadSummary { thread, state }
}

/// What the pretend daemon knows.
#[derive(Default)]
struct World {
    states: HashMap<String, ThreadState>,
    spawned: u32,
    comments: Vec<LineComment>,
}

struct Fixture {
    bench: Bench,
    world: Arc<Mutex<World>>,
}

impl Fixture {
    fn new(grant: Grant) -> Self {
        let world: Arc<Mutex<World>> = Arc::default();
        world.lock().unwrap().states.insert(SOURCE.into(), ThreadState::Idle);
        let shared = world.clone();
        let bench = Bench::bundled("reviewers")
            .grant(grant)
            .daemon(move |call| {
                let mut world = shared.lock().unwrap();
                Ok(match call {
                    ThreadsCall::Get(id) => {
                        let state = *world.states.get(id.as_str()).ok_or("no such thread")?;
                        ThreadsAnswer::Thread(Box::new(summary(
                            id.as_str(),
                            state,
                            Some(TurnStatus::Completed),
                        )))
                    }
                    ThreadsCall::Spawn(spawn) => {
                        world.spawned += 1;
                        let id = format!("thr_critic{}", world.spawned);
                        world.states.insert(id.clone(), ThreadState::Working);
                        let mut made = summary(&id, ThreadState::Working, None);
                        made.thread.parent = spawn.parent.clone();
                        ThreadsAnswer::Thread(Box::new(made))
                    }
                    ThreadsCall::Transcript(_) => {
                        ThreadsAnswer::Transcript(vec![TranscriptEntry {
                            role: Role::User,
                            text: "make a() better".into(),
                        }])
                    }
                    ThreadsCall::Providers => ThreadsAnswer::Providers(vec![
                        provider("echo", &[PermissionMode::ReadOnly, PermissionMode::Ask]),
                        provider("plain", &[PermissionMode::Ask]),
                    ]),
                    ThreadsCall::ListFiles { .. } => ThreadsAnswer::Names(Vec::new()),
                    ThreadsCall::ReadFile { .. } => ThreadsAnswer::File(None),
                    ThreadsCall::AddComment(new) => {
                        let comment = LineComment {
                            id: CommentId::new(format!("cmt_{}", world.comments.len() + 1)),
                            thread_id: new.thread.clone(),
                            path: new.path.clone(),
                            line: new.line,
                            side: new.side,
                            quote: new.quote.clone(),
                            text: new.text.clone(),
                            created_at: 0,
                            resolved_at: None,
                            resolution: None,
                            sent_at: None,
                            author: Some(new.author.clone()),
                        };
                        world.comments.push(comment.clone());
                        ThreadsAnswer::Comment(Box::new(comment))
                    }
                    ThreadsCall::ListComments { .. } => {
                        ThreadsAnswer::Comments(world.comments.clone())
                    }
                    ThreadsCall::ResolveComments { ids, resolution, reopen } => {
                        for comment in world.comments.iter_mut().filter(|c| ids.contains(&c.id)) {
                            comment.resolved_at = (!reopen).then_some(1);
                            comment.resolution = resolution.clone().filter(|_| !reopen);
                        }
                        ThreadsAnswer::Done
                    }
                    ThreadsCall::SendComments { ids, .. } => {
                        for comment in world.comments.iter_mut().filter(|c| ids.contains(&c.id)) {
                            comment.sent_at = Some(1);
                        }
                        ThreadsAnswer::Done
                    }
                    ThreadsCall::Watch(_)
                    | ThreadsCall::Diff { .. }
                    | ThreadsCall::Changes { .. }
                    | ThreadsCall::Models { .. }
                    | ThreadsCall::RefreshView { .. }
                    | ThreadsCall::SetSkill { .. }
                    | ThreadsCall::Send { .. } => ThreadsAnswer::Done,
                })
            })
            .build();
        Self { bench, world }
    }

    fn command(&self, command: &str, args: &[(&str, &str)]) -> Result<String, PluginError> {
        let request = PluginRequest::Command {
            command: command.into(),
            thread: Some(ThreadId::new(SOURCE)),
            args: args.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect(),
        };
        match self.bench.call(request)? {
            Outcome::CommandResult { message, .. } => Ok(message),
            other => panic!("unexpected {other:?}"),
        }
    }

    fn set_state(&self, thread: &str, state: ThreadState) {
        self.world.lock().unwrap().states.insert(thread.into(), state);
    }

    fn turn_ended(&self, status: TurnStatus) {
        let state = self.world.lock().unwrap().states[SOURCE];
        self.bench.post(PluginRequest::TurnEnded {
            summary: Box::new(summary(SOURCE, state, Some(status))),
            status,
        });
    }

    /// Wait for the next diff the plugin asks for and give it `diff`.
    /// Start a round: give the plugin `diff`, then the files it touches.
    fn start_round(&self, diff: &str) {
        self.answer_diff(diff);
        let calls = self.bench.wait_for_call("asked for the changed files", |call| {
            matches!(call, ThreadsCall::Changes { .. })
        });
        self.bench.take_calls();
        let request = calls
            .iter()
            .find_map(|call| match call {
                ThreadsCall::Changes { request, .. } => Some(*request),
                _ => None,
            })
            .unwrap();
        let files = vec![
            ChangedFile {
                path: "src/a.rs".into(),
                kind: ChangeKind::Modified,
                additions: Some(1),
                deletions: Some(1),
            },
            ChangedFile {
                path: "notes.md".into(),
                kind: ChangeKind::Untracked,
                additions: None,
                deletions: None,
            },
        ];
        let changes = EnvironmentChanges {
            branch: Some("task/x".into()),
            base: Some("main".into()),
            files,
            revisions: Vec::new(),
        };
        self.bench.post(PluginRequest::ThreadChanges {
            request,
            thread: ThreadId::new(SOURCE),
            answer: Ok(changes),
        });
    }

    fn answer_diff(&self, diff: &str) {
        let calls = self
            .bench
            .wait_for_call("asked for the diff", |call| matches!(call, ThreadsCall::Diff { .. }));
        self.bench.take_calls();
        let (thread, request) = calls
            .iter()
            .rev()
            .find_map(|call| match call {
                ThreadsCall::Diff { thread, request } => Some((thread.clone(), *request)),
                _ => None,
            })
            .unwrap();
        self.bench.post(PluginRequest::ThreadDiff {
            request,
            thread,
            answer: Ok((diff.to_owned(), false)),
        });
    }

    /// The reviewer finished with `reply`.
    fn critic_answers(&self, critic: &str, reply: &str) {
        self.set_state(critic, ThreadState::Idle);
        let settled = SettledThread {
            summary: summary(critic, ThreadState::Idle, Some(TurnStatus::Completed)),
            last_message: Some(reply.to_owned()),
        };
        self.bench.post(PluginRequest::ThreadSettled(Box::new(settled)));
    }

    fn spawns(&self) -> u32 {
        self.world.lock().unwrap().spawned
    }

    fn comments(&self) -> Vec<LineComment> {
        self.world.lock().unwrap().comments.clone()
    }
}

const FINDING: &str = "Done. {\"findings\": [{\"file\": \"src/a.rs\", \"line\": 2, \
                       \"severity\": \"high\", \"text\": \"new() can fail\"}, \
                       {\"file\": \"src/a.rs\", \"line\": 1, \"severity\": \"info\", \
                       \"text\": \"a nit below the bar\"}, \
                       {\"file\": \"elsewhere.rs\", \"line\": 3, \"severity\": \"high\", \
                       \"text\": \"not in the diff\"}]}";

#[test]
fn a_review_waits_for_the_agent_to_stay_idle_and_leaves_comments() {
    let fixture = Fixture::new(Grant::AllRequested);
    let told = fixture
        .command("configure", &[("enabled", "on"), ("delay", "300ms"), ("profiles", "general")])
        .unwrap();
    assert!(told.contains("reviews on") && told.contains("after 300ms"), "{told}");
    assert!(fixture.bench.calls().iter().any(|call| matches!(call,
        ThreadsCall::SetSkill { skill, enabled: true, .. } if skill == "qcc-reviewers")));

    // A turn ends, and a new one starts before the delay is over: no review.
    fixture.turn_ended(TurnStatus::Completed);
    fixture.set_state(SOURCE, ThreadState::Working);
    std::thread::sleep(Duration::from_millis(500));
    fixture.bench.handle().flush_blocking();
    assert!(!fixture.bench.calls().iter().any(|c| matches!(c, ThreadsCall::Diff { .. })));

    // That turn ends too, and the agent stays idle: the review starts.
    fixture.set_state(SOURCE, ThreadState::Idle);
    fixture.turn_ended(TurnStatus::Completed);
    std::thread::sleep(Duration::from_millis(100));
    fixture.bench.handle().flush_blocking();
    assert!(
        !fixture.bench.calls().iter().any(|c| matches!(c, ThreadsCall::Diff { .. })),
        "not before the delay"
    );
    fixture.start_round(DIFF);
    let calls =
        fixture.bench.wait_for_call("started a reviewer", |c| matches!(c, ThreadsCall::Spawn(_)));
    let spawn = calls
        .iter()
        .find_map(|call| match call {
            ThreadsCall::Spawn(spawn) => Some(spawn.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(spawn.parent.as_ref().map(ThreadId::as_str), Some(SOURCE));
    assert_eq!(spawn.permission_mode, Some(PermissionMode::ReadOnly), "echo runs read-only");
    assert!(spawn.hidden, "a reviewer is a service thread, followed in the panel");
    assert_eq!(spawn.cwd.as_deref(), Some("/work"));
    let prompt = spawn.prompt.unwrap();
    // No diff in the prompt: the files, the base, and how to look.
    assert!(!prompt.contains("new();"), "{prompt}");
    assert!(prompt.contains("make a() better"), "{prompt}");
    assert!(prompt.contains("- `src/a.rs` (modified +1 -1)"), "{prompt}");
    assert!(prompt.contains("- `notes.md` (untracked)"), "{prompt}");
    assert!(prompt.contains("against `main`") && prompt.contains("qcc thread diff thr_src"));

    // The reviewer answers; the diff is read again, the same: the comments are not stale.
    fixture.critic_answers("thr_critic1", FINDING);
    fixture.answer_diff(DIFF);
    fixture.bench.wait_for_call("left a comment", |c| matches!(c, ThreadsCall::AddComment(_)));
    let comments = fixture.comments();
    assert_eq!(
        comments.len(),
        1,
        "below the profile's bar or outside the changes, no comment: {comments:?}"
    );
    let comment = &comments[0];
    assert_eq!((comment.path.as_str(), comment.line), ("src/a.rs", 2));
    assert_eq!(comment.quote.as_deref(), Some("    new();"));
    assert_eq!(comment.text, "new() can fail");
    let author: &CommentAuthor = comment.author.as_ref().unwrap();
    assert_eq!(author.name, "general");
    assert_eq!(author.thread.as_ref().map(ThreadId::as_str), Some("thr_critic1"));
    assert_eq!(author.severity.as_deref(), Some("high"));
    assert!(!author.stale);
    // Collect: nothing is sent to the agent.
    assert!(!fixture.bench.calls().iter().any(|c| matches!(c, ThreadsCall::Send { .. })));

    // Another turn with the same changes: the diff is read, but nobody reviews it again.
    fixture.turn_ended(TurnStatus::Completed);
    fixture.answer_diff(DIFF);
    fixture.bench.handle().flush_blocking();
    assert_eq!(fixture.spawns(), 1);
    std::thread::scope(|scope| {
        let asked = scope.spawn(|| fixture.command("request", &[]));
        fixture.answer_diff(DIFF);
        let error = asked.join().unwrap().unwrap_err();
        assert!(error.to_string().contains("nothing changed since review round 1"), "{error}");
    });
    let status = fixture.command("configure", &[]).unwrap();
    assert!(status.contains("rounds: 1 of 3"), "{status}");
}

#[test]
fn a_request_by_hand_reports_dedup_and_the_round_limit() {
    let fixture = Fixture::new(Grant::AllRequested);
    fixture
        .command(
            "configure",
            &[("trigger", "manual"), ("max-rounds", "1"), ("profiles", "general")],
        )
        .unwrap();
    // A finished turn starts nothing with a manual trigger, even when reviews are on.
    fixture.command("configure", &[("enabled", "on")]).unwrap();
    fixture.turn_ended(TurnStatus::Completed);
    std::thread::sleep(Duration::from_millis(100));
    fixture.bench.handle().flush_blocking();
    assert!(!fixture.bench.calls().iter().any(|c| matches!(c, ThreadsCall::Diff { .. })));

    // `request` answers once the diff is in and the round started.
    let bench = &fixture.bench;
    std::thread::scope(|scope| {
        let asked = scope.spawn(|| fixture.command("request", &[]));
        fixture.start_round(DIFF);
        let message = asked.join().unwrap().unwrap();
        assert!(
            message.contains("review round 1 started: general in thread thr_critic1"),
            "{message}"
        );
    });
    // While it runs, no second round.
    let busy = fixture.command("request", &[]).unwrap_err();
    assert!(busy.to_string().contains("still running"), "{busy}");

    fixture.critic_answers("thr_critic1", "{\"findings\": []}");
    fixture.answer_diff(DIFF);
    bench.handle().flush_blocking();
    let limit = fixture.command("request", &[]).unwrap_err();
    assert!(limit.to_string().contains("limit of 1 review rounds"), "{limit}");

    // Counted again, the same diff is still not reviewed twice.
    fixture.command("reset", &[]).unwrap();
    fixture.command("configure", &[("max-rounds", "0")]).unwrap();
    std::thread::scope(|scope| {
        let asked = scope.spawn(|| fixture.command("request", &[]));
        fixture.answer_diff("");
        let error = asked.join().unwrap().unwrap_err();
        assert!(error.to_string().contains("no changes to review"), "{error}");
    });
}

#[test]
fn auto_delivery_sends_one_message_and_its_answer_is_not_reviewed_again() {
    let fixture = Fixture::new(Grant::AllRequested);
    fixture
        .command(
            "configure",
            &[
                ("enabled", "on"),
                ("trigger", "every-turn"),
                ("delivery", "auto"),
                ("profiles", "general,security"),
            ],
        )
        .unwrap();
    fixture.turn_ended(TurnStatus::Completed);
    fixture.start_round(DIFF);
    fixture.bench.wait_for_call("started two reviewers", |_| fixture.spawns() == 2);

    // The first one answers; the round waits for the second.
    fixture.critic_answers("thr_critic1", FINDING);
    fixture.bench.handle().flush_blocking();
    assert!(!fixture.bench.calls().iter().any(|c| matches!(c, ThreadsCall::Diff { .. })));
    fixture.critic_answers("thr_critic2", "I could not decide on anything worth a JSON object.");
    // The changes moved on while they worked: the comments are marked stale.
    fixture.answer_diff(&DIFF.replace("new();", "newer();"));
    let calls = fixture
        .bench
        .wait_for_call("sent the findings", |c| matches!(c, ThreadsCall::SendComments { .. }));
    let sent: Vec<(&Vec<CommentId>, &String)> = calls
        .iter()
        .filter_map(|call| match call {
            ThreadsCall::SendComments { ids, text, .. } => Some((ids, text)),
            _ => None,
        })
        .collect();
    assert_eq!(sent.len(), 1, "one message for the whole round");
    assert!(!calls.iter().any(|c| matches!(c, ThreadsCall::Send { .. })));
    let (ids, message) = sent[0];
    // The comment goes along as a comment, the way the user's "send" hands it over.
    assert_eq!(ids, &vec![CommentId::new("cmt_1")]);
    assert!(message.contains("review round 1"), "{message}");
    assert!(message.contains("One finding on lines of your changes is below"), "{message}");
    assert!(message.contains("from security): I could not decide"), "{message}");
    // A finding on a file the changes do not touch is no comment, only a remark.
    assert!(
        message.contains("(high, from general) at elsewhere.rs:3: not in the diff"),
        "{message}"
    );
    assert!(message.contains("may have moved"), "{message}");
    assert!(fixture.comments()[0].author.as_ref().unwrap().stale);

    // The agent's answer to the findings starts no new review, though files changed.
    fixture.bench.take_calls();
    fixture.turn_ended(TurnStatus::Completed);
    std::thread::sleep(Duration::from_millis(100));
    fixture.bench.handle().flush_blocking();
    assert!(!fixture.bench.calls().iter().any(|c| matches!(c, ThreadsCall::Diff { .. })));
    // The turn after it does.
    fixture.turn_ended(TurnStatus::Completed);
    fixture.start_round(&DIFF.replace("new();", "newer();"));
    fixture.bench.wait_for_call("started round 2", |_| fixture.spawns() == 4);
}

#[test]
fn without_the_comments_permission_findings_still_reach_the_agent() {
    let grant = qcc_plugin_host::Permissions {
        kv: true,
        timers: true,
        threads: true,
        turn_events: true,
        files: true,
        skills: true,
        ..Default::default()
    };
    let fixture = Fixture::new(Grant::Only(grant));
    fixture
        .command("configure", &[("enabled", "on"), ("trigger", "every-turn"), ("delivery", "auto")])
        .unwrap();
    fixture.turn_ended(TurnStatus::Completed);
    fixture.start_round(DIFF);
    fixture.bench.wait_for_call("started a reviewer", |_| fixture.spawns() == 1);
    fixture.critic_answers("thr_critic1", FINDING);
    fixture.answer_diff(DIFF);
    let calls =
        fixture.bench.wait_for_call("sent the findings", |c| matches!(c, ThreadsCall::Send { .. }));
    assert!(
        !calls.iter().any(|c| matches!(c, ThreadsCall::AddComment(_))),
        "the host refused them"
    );
    assert!(fixture.comments().is_empty());
}

fn model(id: &str, default: bool, efforts: &[&str]) -> ModelInfo {
    ModelInfo {
        id: id.into(),
        name: id.to_uppercase(),
        description: None,
        is_default: default,
        efforts: efforts.iter().map(|e| (*e).to_owned()).collect(),
        default_effort: None,
    }
}

impl Fixture {
    /// The panel of the reviewed thread, as the app asks for it.
    fn panel(&self) -> serde_json::Value {
        let request = PluginRequest::Ui(UiRequest::Render {
            view: "reviewers".into(),
            thread: Some(ThreadId::new(SOURCE)),
        });
        match self.bench.call(request).unwrap() {
            Outcome::UiTree(tree) => serde_json::from_str(&tree).unwrap(),
            other => panic!("unexpected {other:?}"),
        }
    }

    fn act(&self, action: &str, value: &str) {
        let request = PluginRequest::Ui(UiRequest::Action {
            view: "reviewers".into(),
            thread: Some(ThreadId::new(SOURCE)),
            action: action.into(),
            value: Some(value.into()),
        });
        self.bench.call(request).unwrap();
    }

    /// The request id of the last `threads.models` call for `provider`.
    fn models_request(&self, provider: &str) -> u64 {
        self.bench
            .calls()
            .iter()
            .rev()
            .find_map(|call| match call {
                ThreadsCall::Models { provider: asked, request, .. }
                    if asked.as_str() == provider =>
                {
                    Some(*request)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("never asked for the models of {provider}"))
    }

    fn models_come(&self, provider: &str, answer: Result<Vec<ModelInfo>, String>) {
        let request = self.models_request(provider);
        self.bench.post(PluginRequest::ProviderModels {
            request,
            provider: provider.into(),
            answer,
        });
    }
}

/// Every node of a tree of the given type, depth first.
fn nodes<'a>(tree: &'a serde_json::Value, kind: &str, out: &mut Vec<&'a serde_json::Value>) {
    if tree["type"] == kind {
        out.push(tree);
    }
    for child in tree["children"].as_array().into_iter().flatten() {
        nodes(child, kind, out);
    }
}

fn select<'a>(tree: &'a serde_json::Value, action: &str) -> Option<&'a serde_json::Value> {
    let mut found = Vec::new();
    nodes(tree, "select", &mut found);
    found.into_iter().find(|node| node["action"] == action)
}

fn values(select: &serde_json::Value) -> Vec<&str> {
    select["options"].as_array().unwrap().iter().map(|o| o["value"].as_str().unwrap()).collect()
}

fn text_of(tree: &serde_json::Value) -> String {
    tree.to_string()
}

#[test]
fn the_panel_offers_the_models_of_the_agent_chosen_once_they_come() {
    let fixture = Fixture::new(Grant::AllRequested);
    // No agent chosen: no models to pick from, and nobody is asked.
    let panel = fixture.panel();
    assert!(select(&panel, "model").is_none());
    assert!(!fixture.bench.calls().iter().any(|c| matches!(c, ThreadsCall::Models { .. })));

    // An agent: its models are asked for, and the panel says it waits for them.
    fixture.act("provider", "codex");
    let panel = fixture.panel();
    assert!(text_of(&panel).contains("Loading the models of codex"), "{panel}");
    let asked =
        fixture.bench.calls().iter().filter(|c| matches!(c, ThreadsCall::Models { .. })).count();
    assert_eq!(asked, 1, "asked once, however often the panel is drawn");

    // They come: the panel of the thread is redrawn, with a select of them.
    fixture.models_come(
        "codex",
        Ok(vec![model("mini", false, &[]), model("big", true, &["low", "high"])]),
    );
    assert!(fixture.bench.calls().iter().any(|call| matches!(call,
        ThreadsCall::RefreshView { view, thread: Some(thread) }
            if view == "reviewers" && thread.as_str() == SOURCE)));
    let panel = fixture.panel();
    let models = select(&panel, "model").expect("a select of models");
    assert_eq!(values(models), ["", "mini", "big"], "the profile's choice first");
    // The default model's efforts until a model is chosen.
    assert_eq!(values(select(&panel, "effort").unwrap()), ["", "low", "high"]);

    fixture.act("effort", "high");
    fixture.act("model", "mini");
    let status = fixture.command("configure", &[]).unwrap();
    assert!(
        status.contains("model: mini, effort: from the profile"),
        "an effort mini does not take is dropped: {status}"
    );
    assert!(select(&fixture.panel(), "effort").is_none(), "mini takes no effort");

    // Another agent starts afresh; one that lists no models gets text fields.
    fixture.act("provider", "echo");
    let status = fixture.command("configure", &[]).unwrap();
    assert!(status.contains("agent: echo, model: from the profile"), "{status}");
    fixture.models_come("echo", Ok(Vec::new()));
    let panel = fixture.panel();
    assert!(select(&panel, "model").is_none());
    let mut inputs = Vec::new();
    nodes(&panel, "input", &mut inputs);
    assert!(inputs.iter().any(|input| input["action"] == "model"), "{panel}");

    // One that cannot list them says why and may be asked again.
    fixture.act("provider", "opencode");
    fixture.models_come("opencode", Err("opencode is not installed".into()));
    let panel = fixture.panel();
    assert!(text_of(&panel).contains("opencode is not installed"), "{panel}");
    fixture.act("models-refresh", "");
    let refreshed = fixture.bench.calls().iter().any(|call| {
        matches!(call, ThreadsCall::Models { provider, refresh: true, .. } if provider.as_str() == "opencode")
    });
    assert!(refreshed);
}

fn provider(id: &str, modes: &[PermissionMode]) -> ProviderInfo {
    ProviderInfo {
        id: id.into(),
        name: id.into(),
        capabilities: ProviderCapabilities {
            permission_modes: modes.to_vec(),
            ..ProviderCapabilities::default()
        },
    }
}

#[test]
fn an_agent_that_cannot_run_read_only_reviews_in_ask_mode_and_says_so() {
    let fixture = Fixture::new(Grant::AllRequested);
    fixture.command("configure", &[("provider", "plain"), ("trigger", "manual")]).unwrap();
    // The panel warns before any round.
    let panel = fixture.panel().to_string();
    assert!(
        panel.contains("plain cannot run read-only, so reviewer general runs in ask mode"),
        "{panel}"
    );
    std::thread::scope(|scope| {
        let asked = scope.spawn(|| fixture.command("request", &[]));
        fixture.start_round(DIFF);
        let message = asked.join().unwrap().unwrap();
        assert!(message.contains("note: plain cannot run read-only"), "{message}");
    });
    let spawn = fixture
        .bench
        .calls()
        .into_iter()
        .find_map(|call| match call {
            ThreadsCall::Spawn(spawn) => Some(spawn),
            _ => None,
        })
        .expect("the reviewer started");
    assert_eq!(spawn.permission_mode, Some(PermissionMode::Ask));
}

/// The findings as `qcc reviewers findings --json` gives them.
fn findings(fixture: &Fixture) -> Vec<serde_json::Value> {
    let request = PluginRequest::Command {
        command: "findings".into(),
        thread: Some(ThreadId::new(SOURCE)),
        args: Vec::new(),
    };
    match fixture.bench.call(request).unwrap() {
        Outcome::CommandResult { data: Some(data), .. } => serde_json::from_str(&data).unwrap(),
        other => panic!("unexpected {other:?}"),
    }
}

fn status_of(fixture: &Fixture, id: &str) -> String {
    let all = findings(fixture);
    let found = all.iter().find(|f| f["id"] == id).unwrap_or_else(|| panic!("no {id}: {all:?}"));
    found["status"].as_str().unwrap().to_owned()
}

#[test]
fn findings_are_sent_dismissed_restored_and_fixed_with_their_comments() {
    let fixture = Fixture::new(Grant::AllRequested);
    fixture.command("configure", &[("enabled", "on"), ("trigger", "every-turn")]).unwrap();
    fixture.turn_ended(TurnStatus::Completed);
    fixture.start_round(DIFF);
    fixture.bench.wait_for_call("started a reviewer", |_| fixture.spawns() == 1);
    fixture.critic_answers("thr_critic1", FINDING);
    fixture.answer_diff(DIFF);
    fixture.bench.wait_for_call("left a comment", |c| matches!(c, ThreadsCall::AddComment(_)));
    // The panel is told to draw itself again as the round goes.
    assert!(fixture.bench.calls().iter().any(|c| matches!(c, ThreadsCall::RefreshView { .. })));

    // f1 sits on a line, with a comment; f2 is about a file the changes do not touch.
    let all = findings(&fixture);
    assert_eq!(all.len(), 2, "{all:?}");
    assert_eq!(all[0]["id"], "f1");
    assert_eq!(all[0]["comment"], "cmt_1");
    assert_eq!(all[0]["reviewer"], "general");
    assert_eq!(all[1]["id"], "f2");
    assert!(all[1]["comment"].is_null());
    assert!(all.iter().all(|f| f["status"] == "open"), "{all:?}");

    // Sent with the user's "send all open": the comment through the daemon, the remark
    // in the text, in one message.
    fixture.bench.take_calls();
    let sent = fixture.command("send", &[]).unwrap();
    assert!(sent.contains("sent 2 findings"), "{sent}");
    let calls = fixture.bench.calls();
    let text = calls
        .iter()
        .find_map(|call| match call {
            ThreadsCall::SendComments { ids, text, .. }
                if ids == &vec![CommentId::new("cmt_1")] =>
            {
                Some(text.clone())
            }
            _ => None,
        })
        .expect("sent with its comment");
    assert!(text.contains("at elsewhere.rs:3: not in the diff"), "{text}");
    assert_eq!(status_of(&fixture, "f1"), "sent");
    assert_eq!(status_of(&fixture, "f2"), "sent");
    assert!(fixture.command("send", &[]).unwrap_err().to_string().contains("no open findings"));

    // Dismissed: its comment is resolved with the user's words; restored: open again.
    fixture.command("dismiss", &[("ids", "f1 f2")]).unwrap();
    assert_eq!(status_of(&fixture, "f1"), "dismissed");
    assert_eq!(status_of(&fixture, "f2"), "dismissed");
    assert_eq!(fixture.comments()[0].resolution.as_deref(), Some("Dismissed by the user"));
    fixture.command("restore", &[("ids", "f1")]).unwrap();
    assert_eq!(status_of(&fixture, "f1"), "sent", "sent before, and open again");
    assert!(fixture.comments()[0].resolved_at.is_none());

    // The agent resolves the comment: fixed, with its note.
    {
        let mut world = fixture.world.lock().unwrap();
        world.comments[0].resolved_at = Some(2);
        world.comments[0].resolution = Some("handled the error".into());
    }
    let all = findings(&fixture);
    assert_eq!(all[0]["status"], "fixed");
    assert_eq!(all[0]["note"], "handled the error");
    let bogus = fixture.command("dismiss", &[("ids", "f9")]).unwrap_err();
    assert!(bogus.to_string().contains("no finding f9"), "{bogus}");

    // The panel lists them with their statuses and a way to the place in Changes.
    let panel = fixture.panel().to_string();
    assert!(panel.contains("Fixed") && panel.contains("Dismissed"), "{panel}");
    assert!(
        panel.contains(r#""type":"link""#) && panel.contains(r#""path":"src/a.rs""#),
        "{panel}"
    );
    assert!(panel.contains("Agent: handled the error"), "{panel}");
}

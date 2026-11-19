//! The reviewers plugin on the real wasm host, with the test playing the daemon:
//! it answers for threads, diffs and comments and tells the plugin of turns.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use qcc_domain::{
    CommentAuthor, CommentId, LineComment, PermissionMode, Role, ThreadId, ThreadState,
    TranscriptEntry, TurnStatus,
};
use qcc_plugin_host::{Grant, Outcome, PluginError, PluginRequest, SettledThread};
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
                    ThreadsCall::Providers => ThreadsAnswer::Providers(Vec::new()),
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
                            author: Some(new.author.clone()),
                        };
                        world.comments.push(comment.clone());
                        ThreadsAnswer::Comment(Box::new(comment))
                    }
                    ThreadsCall::ListComments { .. } => {
                        ThreadsAnswer::Comments(world.comments.clone())
                    }
                    ThreadsCall::Watch(_)
                    | ThreadsCall::Diff { .. }
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
    fixture.answer_diff(DIFF);
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
    assert_eq!(spawn.permission_mode, Some(PermissionMode::Ask));
    assert_eq!(spawn.cwd.as_deref(), Some("/work"));
    let prompt = spawn.prompt.unwrap();
    assert!(prompt.contains("+    new();") && prompt.contains("make a() better"), "{prompt}");
    assert!(prompt.contains("Do not change any file"), "{prompt}");

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
        fixture.answer_diff(DIFF);
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
    fixture.answer_diff(DIFF);
    fixture.bench.wait_for_call("started two reviewers", |_| fixture.spawns() == 2);

    // The first one answers; the round waits for the second.
    fixture.critic_answers("thr_critic1", FINDING);
    fixture.bench.handle().flush_blocking();
    assert!(!fixture.bench.calls().iter().any(|c| matches!(c, ThreadsCall::Diff { .. })));
    fixture.critic_answers("thr_critic2", "I could not decide on anything worth a JSON object.");
    // The changes moved on while they worked: the comments are marked stale.
    fixture.answer_diff(&DIFF.replace("new();", "newer();"));
    let calls =
        fixture.bench.wait_for_call("sent the findings", |c| matches!(c, ThreadsCall::Send { .. }));
    let sent: Vec<&String> = calls
        .iter()
        .filter_map(|call| match call {
            ThreadsCall::Send { text, .. } => Some(text),
            _ => None,
        })
        .collect();
    assert_eq!(sent.len(), 1, "one message for the whole round");
    let message = sent[0];
    assert!(message.contains("review round 1"), "{message}");
    assert!(
        message.contains("(high, from general) [cmt_1] at src/a.rs:2: new() can fail"),
        "{message}"
    );
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
    fixture.answer_diff(&DIFF.replace("new();", "newer();"));
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
    fixture.answer_diff(DIFF);
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

//! The bridge replayed against traffic recorded from Claude Code 2.1.325, through the
//! real wasm host and the real assembler. The recordings are in `fixtures/`; the
//! `.timeline.jsonl` next to each is the timeline it must assemble into.

use std::path::PathBuf;
use std::sync::Arc;

use qcc_domain::{
    EventPayload, InteractionResolution, ItemShape, ItemStatus, PermissionMode, TurnStatus,
};
use qcc_plugin_testkit::{Harness, ScriptStep, ScriptedProcesses, script_from_recording};
use serde_json::Value;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures").join(name)
}

fn recording(name: &str) -> String {
    std::fs::read_to_string(fixture(&format!("{name}.ndjson"))).unwrap()
}

/// A harness whose `claude` processes replay the named recordings, one per spawn.
fn replay(names: &[&str]) -> (Harness, Arc<ScriptedProcesses>) {
    let recordings: Vec<String> = names.iter().map(|name| recording(name)).collect();
    let processes = ScriptedProcesses::from_recordings(recordings.iter().map(String::as_str));
    let harness = Harness::bundled("provider-claude-code").processes(processes.clone()).build();
    (harness, processes)
}

fn golden(harness: &Harness, name: &str) {
    harness.assert_conformant();
    harness.assert_golden(fixture(&format!("{name}.timeline.jsonl")));
}

#[test]
fn basic_turn() {
    let (mut harness, processes) = replay(&["basic"]);
    harness.start(Some("Reply with exactly: pong")).unwrap();
    harness.wait_turns(1);
    golden(&harness, "basic");
    assert_eq!(harness.agent_messages(), ["pong"]);
    // The agent named its session in its first message; that is the resume handle.
    assert_eq!(
        harness.thread().provider_session.as_deref(),
        Some("9fbcae9c-fb05-4333-b6a0-9c4a46977ea7")
    );

    let spawn = &processes.spawns()[0];
    assert_eq!(spawn.program, "claude");
    assert_eq!(spawn.cwd.as_deref(), Some("/work"));
    for flag in ["-p", "stream-json", "--permission-prompt-tool", "--include-partial-messages"] {
        assert!(spawn.args.iter().any(|arg| arg == flag), "missing {flag}: {:?}", spawn.args);
    }
    assert!(!spawn.args.iter().any(|arg| arg == "--resume"));
    assert!(spawn.env.iter().any(|(name, value)| name == "QCC_THREAD_ID" && value == "thread-1"));
    let sent = processes.stdin_messages();
    assert_eq!(sent[0]["message"]["content"], "Reply with exactly: pong");
}

/// An agent that exits right after a last line with no newline: that line is still
/// read, and it is the one that completes the turn.
#[test]
fn the_last_line_counts_without_a_newline() {
    let mut script = script_from_recording(&recording("basic"));
    let Some(ScriptStep::Stdout(last)) = script.last_mut() else { panic!("ends in output") };
    assert_eq!(last.pop(), Some(b'\n'));
    script.push(ScriptStep::Exit(Some(0)));
    let processes = ScriptedProcesses::new([script]);
    let mut harness = Harness::bundled("provider-claude-code").processes(processes).build();
    harness.start(Some("Reply with exactly: pong")).unwrap();
    harness.wait_turns(1);
    harness.assert_conformant();
    assert_eq!(harness.turn_statuses(), [(TurnStatus::Completed, None)]);
}

#[test]
fn tool_call_that_needs_no_permission() {
    let (mut harness, _) = replay(&["tool"]);
    harness.start(Some("Run the shell command `cat note.txt`")).unwrap();
    harness.wait_turns(1);
    golden(&harness, "tool");

    let items = harness.completed_items();
    let command = items
        .iter()
        .find_map(|item| match &item.shape {
            ItemShape::Command { command, output, .. } => Some((command, output, item.status)),
            _ => None,
        })
        .expect("a command item");
    assert_eq!(command.0, "cat note.txt");
    assert_eq!(command.1.as_deref(), Some("hello fixture"));
    assert_eq!(command.2, ItemStatus::Completed);
    assert_eq!(harness.agent_messages(), ["The command printed:\n\n```\nhello fixture\n```"]);
}

#[test]
fn permission_allowed() {
    let (mut harness, processes) = replay(&["permission-allow"]);
    harness.start(Some("Create a file named made.txt")).unwrap();
    harness.wait_until("the permission prompt", |h| h.pending_interaction().is_some());
    assert_eq!(harness.turn_statuses(), [], "the turn waits for the answer");
    let requested = harness
        .events()
        .iter()
        .find_map(|event| match &event.payload {
            EventPayload::InteractionRequested { item_id, request, .. } => {
                Some((item_id.clone(), serde_json::to_value(request).unwrap()))
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(requested.1["title"], "Allow Write?");
    assert_eq!(requested.1["detail"], "/work/made.txt");

    harness.resolve(InteractionResolution::Selected { option_id: "allow".into() }).unwrap();
    harness.wait_turns(1);
    golden(&harness, "permission-allow");

    // The prompt pointed at the file change that then went through.
    let change = harness
        .completed_items()
        .into_iter()
        .find(|item| matches!(item.shape, ItemShape::FileChange { .. }))
        .unwrap();
    assert_eq!(requested.0, Some(change.id));
    assert_eq!(change.status, ItemStatus::Completed);
    assert_eq!(harness.agent_messages(), ["done"]);

    let response = processes
        .stdin_messages()
        .into_iter()
        .find(|message| message["type"] == "control_response")
        .unwrap();
    assert_eq!(response["response"]["response"]["behavior"], "allow");
    // The input goes back unchanged: allowing is not editing.
    assert_eq!(response["response"]["response"]["updatedInput"]["content"], "ok");
}

#[test]
fn permission_denied() {
    let (mut harness, processes) = replay(&["permission-deny"]);
    harness.start(Some("Create a file named made.txt")).unwrap();
    harness.wait_until("the permission prompt", |h| h.pending_interaction().is_some());
    harness.resolve(InteractionResolution::Selected { option_id: "deny".into() }).unwrap();
    harness.wait_turns(1);
    golden(&harness, "permission-deny");

    let change = harness
        .completed_items()
        .into_iter()
        .find(|item| matches!(item.shape, ItemShape::FileChange { .. }))
        .unwrap();
    assert_eq!(change.status, ItemStatus::Declined);
    assert_eq!(harness.turn_statuses(), [(TurnStatus::Completed, None)]);
    let response: Value = processes
        .stdin_messages()
        .into_iter()
        .find(|message| message["type"] == "control_response")
        .unwrap();
    assert_eq!(response["response"]["response"]["behavior"], "deny");
}

#[test]
fn interrupt_then_continue_then_resume() {
    let (mut harness, processes) = replay(&["interrupt", "resume"]);
    harness.start(Some("Write the numbers from 1 to 400")).unwrap();
    harness.wait_until("streamed text", |h| {
        h.count(|payload| matches!(payload, EventPayload::ItemTextDelta { .. })) > 0
    });

    harness.interrupt().unwrap();
    // The bridge settles the turn itself; the agent's late output is not replayed
    // into the timeline.
    assert_eq!(harness.turn_statuses(), [(TurnStatus::Interrupted, None)]);
    let streamed = harness.agent_messages().remove(0);
    assert!(streamed.starts_with("1\n2\n3\n"), "{streamed:?}");

    harness.input("Reply with exactly: still here").unwrap();
    harness.wait_turns(2);
    assert_eq!(harness.turn_statuses()[1], (TurnStatus::Completed, None));
    assert_eq!(harness.agent_messages().last().unwrap(), "still here");
    let session = harness.thread().provider_session.clone().unwrap();

    // Release the session and come back: the same provider session is resumed.
    harness.stop().unwrap();
    assert_eq!(processes.kills(), 1);
    harness.resume(Some("What exact phrase did I ask you to reply with last time?")).unwrap();
    harness.wait_turns(3);
    golden(&harness, "interrupt-resume");

    let resumed = &processes.spawns()[1];
    let flag = resumed.args.iter().position(|arg| arg == "--resume").expect("--resume");
    assert_eq!(resumed.args[flag + 1], session);
    assert_eq!(harness.thread().provider_session, Some(session));
    assert_eq!(harness.agent_messages().last().unwrap(), "still here");
    // Resuming a known session announces nothing new.
    assert_eq!(harness.count(|p| matches!(p, EventPayload::SessionIdentified { .. })), 1);
}

#[test]
fn a_dying_agent_fails_the_turn_with_its_last_words() {
    use qcc_plugin_testkit::ScriptStep;
    let processes = ScriptedProcesses::new([vec![
        ScriptStep::AwaitStdin,
        ScriptStep::Stderr(b"warming up\nNo conversation found with session ID: gone\n".to_vec()),
        ScriptStep::Exit(Some(1)),
    ]]);
    let mut harness = Harness::bundled("provider-claude-code").processes(processes).build();
    harness.start(Some("hello")).unwrap();
    harness.wait_turns(1);
    harness.assert_conformant();
    let reason = "claude exited with code 1: No conversation found with session ID: gone";
    assert_eq!(harness.turn_statuses(), [(TurnStatus::Failed, Some(reason.into()))]);
    assert_eq!(harness.count(|p| matches!(p, EventPayload::SessionEnded { .. })), 1);
    // The session is gone on both sides.
    assert!(harness.input("anyone?").is_err());
}

#[test]
fn a_missing_executable_fails_the_start() {
    // No script: the scripted backend refuses the spawn like a missing binary would.
    let processes = ScriptedProcesses::new([]);
    let mut harness = Harness::bundled("provider-claude-code").processes(processes).build();
    assert!(harness.start(Some("hello")).is_err());
    harness.assert_conformant();
}

#[test]
fn a_message_sent_mid_turn_is_steered_into_it() {
    let (mut harness, processes) = replay(&["steer"]);
    harness.start(Some("First run `cat note.txt`, then run `cat other.txt`, then count")).unwrap();
    harness.steer("Also end your answer with the word BANANA.").unwrap();
    harness.wait_turns(1);
    golden(&harness, "steer");

    // Two user messages went to the agent; it answered them in one turn.
    let sent = processes.stdin_messages();
    assert_eq!(sent.iter().filter(|message| message["type"] == "user").count(), 2);
    assert_eq!(harness.count(|p| matches!(p, EventPayload::TurnStarted { .. })), 1);
    assert!(harness.agent_messages().last().unwrap().ends_with("BANANA"));
    let steered =
        harness.count(|p| matches!(p, EventPayload::InputSubmitted { steered: true, .. }));
    assert_eq!(steered, 1);
}

#[test]
fn accept_edits_mode_applies_an_edit_without_asking_and_reports_the_patch() {
    let (mut harness, processes) = replay(&["edit"]);
    harness.set_permission_mode(Some(PermissionMode::AcceptEdits));
    harness.start(Some("In list.txt replace the line beta")).unwrap();
    harness.wait_turns(1);
    golden(&harness, "edit");

    let args = &processes.spawns()[0].args;
    let flag = args.iter().position(|arg| arg == "--permission-mode").expect("--permission-mode");
    assert_eq!(args[flag + 1], "acceptEdits");
    assert_eq!(harness.count(|p| matches!(p, EventPayload::InteractionRequested { .. })), 0);

    let (path, diff) = harness
        .completed_items()
        .into_iter()
        .find_map(|item| match item.shape {
            ItemShape::FileChange { path, diff } => Some((path, diff)),
            _ => None,
        })
        .expect("a file change");
    assert_eq!(path, "/work/list.txt");
    // The patch the agent applied, with line numbers and context.
    assert_eq!(diff.as_deref(), Some("@@ -1,3 +1,4 @@\n alpha\n-beta\n+BETA\n+delta\n gamma"));
}

#[test]
fn each_permission_mode_maps_to_a_claude_flag() {
    let flag_for = |mode| {
        let processes = ScriptedProcesses::new([vec![]]);
        let mut harness =
            Harness::bundled("provider-claude-code").processes(processes.clone()).build();
        harness.set_permission_mode(mode);
        harness.start(None).unwrap();
        let args = processes.spawns()[0].args.clone();
        args.iter().position(|arg| arg == "--permission-mode").map(|at| args[at + 1].clone())
    };
    assert_eq!(flag_for(Some(PermissionMode::Ask)).as_deref(), Some("manual"));
    assert_eq!(flag_for(Some(PermissionMode::AcceptEdits)).as_deref(), Some("acceptEdits"));
    assert_eq!(flag_for(Some(PermissionMode::Auto)).as_deref(), Some("auto"));
    assert_eq!(flag_for(Some(PermissionMode::Full)).as_deref(), Some("bypassPermissions"));
    // No mode: the user's own Claude Code settings decide.
    assert_eq!(flag_for(None), None);
}

//! The bridge replayed against traffic recorded from opencode 1.19.4 (`opencode acp`),
//! through the real wasm host and the real assembler. The recordings are in
//! `fixtures/`; the `.timeline.jsonl` next to each is the timeline it must assemble
//! into.

use std::path::PathBuf;
use std::sync::Arc;

use qcc_domain::{EventPayload, InteractionResolution, ItemShape, ItemStatus, TurnStatus};
use qcc_plugin_testkit::{Harness, ScriptStep, ScriptedProcesses};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures").join(name)
}

/// A harness whose agent processes replay the named recordings, one per spawn.
fn replay(names: &[&str]) -> (Harness, Arc<ScriptedProcesses>) {
    let recordings: Vec<String> = names
        .iter()
        .map(|name| std::fs::read_to_string(fixture(&format!("{name}.ndjson"))).unwrap())
        .collect();
    let processes = ScriptedProcesses::from_recordings(recordings.iter().map(String::as_str));
    let harness = Harness::bundled("provider-acp").processes(processes.clone()).build();
    (harness, processes)
}

fn golden(harness: &Harness, name: &str) {
    harness.assert_conformant();
    harness.assert_golden(fixture(&format!("{name}.timeline.jsonl")));
}

fn file_change(harness: &Harness) -> (String, Option<String>, ItemStatus) {
    harness
        .completed_items()
        .into_iter()
        .find_map(|item| match item.shape {
            ItemShape::FileChange { path, diff } => Some((path, diff, item.status)),
            _ => None,
        })
        .expect("a file change item")
}

#[test]
fn basic_turn() {
    let (mut harness, processes) = replay(&["basic"]);
    harness.start(Some("Reply with exactly: pong")).unwrap();
    harness.wait_turns(1);
    golden(&harness, "basic");
    assert_eq!(harness.agent_messages(), ["pong"]);
    // The agent names the session during the handshake; that is the resume handle.
    assert_eq!(
        harness.thread().provider_session.as_deref(),
        Some("ses_e218e1cb4ffe8VcDFv3Peb287P")
    );

    let spawn = &processes.spawns()[0];
    assert_eq!((spawn.program.as_str(), &spawn.args[..]), ("opencode", &["acp".to_owned()][..]));
    assert_eq!(spawn.cwd.as_deref(), Some("/work"));
    let sent: Vec<String> = processes
        .stdin_messages()
        .iter()
        .map(|message| message["method"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(sent, ["initialize", "session/new", "session/prompt"]);
    assert_eq!(processes.stdin_messages()[1]["params"]["cwd"], "/work");
}

#[test]
fn tool_call_that_needs_no_permission() {
    let (mut harness, _) = replay(&["tool"]);
    harness.start(Some("Create a file named made.txt")).unwrap();
    harness.wait_turns(1);
    golden(&harness, "tool");
    let (path, _, status) = file_change(&harness);
    assert_eq!((path.as_str(), status), ("/work/made.txt", ItemStatus::Completed));
    assert_eq!(harness.agent_messages(), ["done"]);
}

#[test]
fn permission_allowed() {
    let (mut harness, processes) = replay(&["permission-allow"]);
    harness.start(Some("Create a file named made.txt")).unwrap();
    harness.wait_until("the permission request", |h| h.pending_interaction().is_some());
    assert_eq!(harness.turn_statuses(), [], "the turn waits for the answer");
    let options: Vec<String> = harness
        .events()
        .iter()
        .find_map(|event| match &event.payload {
            EventPayload::InteractionRequested { request, .. } => {
                let request = serde_json::to_value(request).unwrap();
                let options = request["options"].as_array().unwrap().clone();
                Some(options.iter().map(|o| format!("{}:{}", o["id"], o["kind"])).collect())
            }
            _ => None,
        })
        .unwrap();
    // The agent's own options, with their meaning translated.
    assert_eq!(options, ["\"once\":\"allow\"", "\"always\":\"allow\"", "\"reject\":\"deny\""]);

    harness.resolve(InteractionResolution::Selected { option_id: "once".into() }).unwrap();
    harness.wait_turns(1);
    golden(&harness, "permission-allow");
    let (path, diff, status) = file_change(&harness);
    assert_eq!((path.as_str(), status), ("/work/made.txt", ItemStatus::Completed));
    assert!(diff.unwrap().contains("+ok"));

    let sent = processes.stdin_messages();
    let answer = sent.iter().find(|m| m.get("result").is_some()).expect("a response");
    assert_eq!(answer["id"], 0, "the agent's request id is echoed");
    assert_eq!(answer["result"]["outcome"]["optionId"], "once");
    // opencode asked us to write the file although we offered no such capability;
    // it is refused instead of left hanging.
    let refusal = sent.iter().find(|m| m.get("error").is_some()).expect("a refusal");
    assert_eq!(refusal["error"]["code"], -32601);
}

#[test]
fn permission_rejected() {
    let (mut harness, _) = replay(&["permission-deny"]);
    harness.start(Some("Create a file named made.txt")).unwrap();
    harness.wait_until("the permission request", |h| h.pending_interaction().is_some());
    harness.resolve(InteractionResolution::Selected { option_id: "reject".into() }).unwrap();
    harness.wait_turns(1);
    golden(&harness, "permission-deny");
    assert_eq!(file_change(&harness).2, ItemStatus::Declined);
    assert_eq!(harness.turn_statuses(), [(TurnStatus::Completed, None)]);
}

#[test]
fn cancel_then_continue_then_resume() {
    let (mut harness, processes) = replay(&["cancel", "resume"]);
    harness.start(Some("Write the numbers from 1 to 400")).unwrap();
    harness.wait_until("streamed text", |h| {
        h.count(|payload| matches!(payload, EventPayload::ItemTextDelta { .. })) > 0
    });

    harness.interrupt().unwrap();
    assert_eq!(harness.turn_statuses(), [(TurnStatus::Interrupted, None)]);

    harness.input("Reply with exactly: still here").unwrap();
    harness.wait_turns(2);
    assert_eq!(harness.turn_statuses()[1], (TurnStatus::Completed, None));
    assert_eq!(harness.agent_messages().last().unwrap(), "still here");
    let session = harness.thread().provider_session.clone().unwrap();

    // Release the session and come back, this time asking for a model.
    harness.stop().unwrap();
    assert_eq!(processes.kills(), 1);
    harness.set_model(Some("opencode/big-pickle"));
    harness.resume(Some("What exact phrase did I ask you to reply with before?")).unwrap();
    harness.wait_turns(3);
    golden(&harness, "cancel-resume");
    assert_eq!(harness.agent_messages().last().unwrap(), "still here");
    assert_eq!(harness.thread().provider_session, Some(session.clone()));

    let sent = processes.stdin_messages();
    let resume = sent.iter().find(|m| m["method"] == "session/resume").expect("session/resume");
    assert_eq!(resume["params"]["sessionId"], session.as_str());
    let model = sent.iter().find(|m| m["method"] == "session/set_config_option").unwrap();
    assert_eq!(model["params"]["value"], "opencode/big-pickle");
    assert_eq!(processes.unused_scripts(), 0);
}

#[test]
fn a_dying_agent_fails_the_turn_with_its_last_words() {
    let processes = ScriptedProcesses::new([vec![
        ScriptStep::AwaitStdin,
        ScriptStep::line(r#"{"jsonrpc":"2.0","id":1,"result":{"agentCapabilities":{}}}"#),
        ScriptStep::AwaitMessage("method:session/new".into()),
        ScriptStep::line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s"}}"#),
        ScriptStep::AwaitMessage("method:session/prompt".into()),
        ScriptStep::Stderr(b"error: ACP connection closed\n".to_vec()),
        ScriptStep::Exit(Some(1)),
    ]]);
    let mut harness = Harness::bundled("provider-acp").processes(processes).build();
    harness.start(Some("hello")).unwrap();
    harness.wait_turns(1);
    harness.assert_conformant();
    let reason = "opencode exited with code 1: error: ACP connection closed";
    assert_eq!(harness.turn_statuses(), [(TurnStatus::Failed, Some(reason.into()))]);
    assert!(harness.input("anyone?").is_err());
}

#[test]
fn an_agent_that_needs_login_ends_the_session_with_the_reason() {
    let processes = ScriptedProcesses::new([vec![
        ScriptStep::AwaitStdin,
        ScriptStep::line(r#"{"jsonrpc":"2.0","id":1,"result":{"agentCapabilities":{}}}"#),
        ScriptStep::AwaitMessage("method:session/new".into()),
        ScriptStep::line(
            r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32000,"message":"Authentication required"}}"#,
        ),
    ]]);
    let mut harness = Harness::bundled("provider-acp").processes(processes.clone()).build();
    harness.start(Some("hello")).unwrap();
    harness.wait_until("the session to end", |h| {
        h.count(|payload| matches!(payload, EventPayload::SessionEnded { .. })) > 0
    });
    harness.assert_conformant();
    assert!(harness.events().iter().any(|event| matches!(
        &event.payload,
        EventPayload::ProviderError { message } if message.contains("Authentication required")
    )));
    // No prompt was sent, so no turn was ever opened.
    assert_eq!(harness.turn_statuses(), []);
    assert_eq!(processes.kills(), 1);
}

//! The bridge replayed against traffic recorded from codex-cli 0.166.0
//! (`codex app-server`), through the real wasm host and the real assembler. The
//! recordings are in `fixtures/`; the `.timeline.jsonl` next to each is the timeline
//! it must assemble into.

use std::path::PathBuf;
use std::sync::Arc;

use qcc_domain::{EventPayload, InteractionResolution, ItemShape, ItemStatus, TurnStatus};
use qcc_plugin_testkit::{Harness, ScriptStep, ScriptedProcesses};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures").join(name)
}

/// A harness whose `codex` processes replay the named recordings, one per spawn.
fn replay(names: &[&str]) -> (Harness, Arc<ScriptedProcesses>) {
    let recordings: Vec<String> = names
        .iter()
        .map(|name| std::fs::read_to_string(fixture(&format!("{name}.ndjson"))).unwrap())
        .collect();
    let processes = ScriptedProcesses::from_recordings(recordings.iter().map(String::as_str));
    let harness = Harness::bundled("provider-codex").processes(processes.clone()).build();
    (harness, processes)
}

fn golden(harness: &Harness, name: &str) {
    harness.assert_conformant();
    harness.assert_golden(fixture(&format!("{name}.timeline.jsonl")));
}

fn command(harness: &Harness) -> (String, Option<i32>, ItemStatus) {
    harness
        .completed_items()
        .into_iter()
        .find_map(|item| match item.shape {
            ItemShape::Command { command, exit_code, .. } => {
                Some((command, exit_code, item.status))
            }
            _ => None,
        })
        .expect("a command item")
}

fn sent_methods(processes: &ScriptedProcesses) -> Vec<String> {
    processes
        .stdin_messages()
        .iter()
        .map(|message| message["method"].as_str().unwrap_or("response").to_owned())
        .collect()
}

#[test]
fn basic_turn() {
    let (mut harness, processes) = replay(&["basic"]);
    harness.start(Some("Reply with exactly: pong")).unwrap();
    harness.wait_turns(1);
    golden(&harness, "basic");
    assert_eq!(harness.agent_messages(), ["pong"]);
    // The thread id codex returned is the resume handle.
    assert_eq!(
        harness.thread().provider_session.as_deref(),
        Some("01a1de76-7974-79d2-83bc-e0f6ca5b0025")
    );

    let spawn = &processes.spawns()[0];
    assert_eq!(
        (spawn.program.as_str(), &spawn.args[..]),
        ("codex", &["app-server".to_owned()][..])
    );
    assert_eq!(
        sent_methods(&processes),
        ["initialize", "initialized", "thread/start", "turn/start"]
    );
    let thread_start = &processes.stdin_messages()[2]["params"];
    assert_eq!(thread_start["cwd"], "/work");
    // Sandboxing and approvals are the user's Codex configuration, not ours.
    assert!(thread_start.get("sandbox").is_none());
}

#[test]
fn command_approved() {
    let (mut harness, processes) = replay(&["approve"]);
    harness.start(Some("Run the shell command `touch made.txt`")).unwrap();
    harness.wait_until("the approval request", |h| h.pending_interaction().is_some());
    assert_eq!(harness.turn_statuses(), [], "the turn waits for the answer");
    let (item_id, request) = harness
        .events()
        .iter()
        .find_map(|event| match &event.payload {
            EventPayload::InteractionRequested { item_id, request, .. } => {
                Some((item_id.clone(), serde_json::to_value(request).unwrap()))
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(request["title"], "May I run the requested command to create made.txt?");
    assert!(request["detail"].as_str().unwrap().ends_with("'touch made.txt'"));

    harness.resolve(InteractionResolution::Selected { option_id: "accept".into() }).unwrap();
    harness.wait_turns(1);
    golden(&harness, "approve");

    let (command_line, exit_code, status) = command(&harness);
    assert!(command_line.ends_with("'touch made.txt'"));
    assert_eq!((exit_code, status), (Some(0), ItemStatus::Completed));
    // The request pointed at the command item that then ran.
    let ran = harness
        .completed_items()
        .into_iter()
        .find(|item| matches!(item.shape, ItemShape::Command { .. }))
        .unwrap();
    assert_eq!(item_id, Some(ran.id));
    assert_eq!(harness.agent_messages(), ["I’ll run the command.", "done"]);

    let sent = processes.stdin_messages();
    let answer = sent.iter().find(|m| m.get("result").is_some()).expect("a response");
    assert_eq!((&answer["id"], &answer["result"]["decision"]), (&0.into(), &"accept".into()));
}

#[test]
fn command_declined() {
    let (mut harness, processes) = replay(&["decline"]);
    harness.start(Some("Run the shell command `touch made.txt`")).unwrap();
    harness.wait_until("the approval request", |h| h.pending_interaction().is_some());
    harness.resolve(InteractionResolution::Selected { option_id: "decline".into() }).unwrap();
    harness.wait_turns(1);
    golden(&harness, "decline");
    let (_, exit_code, status) = command(&harness);
    assert_eq!((exit_code, status), (None, ItemStatus::Declined));
    // Declining one command does not end the turn.
    assert_eq!(harness.turn_statuses(), [(TurnStatus::Completed, None)]);
    let sent = processes.stdin_messages();
    let answer = sent.iter().find(|m| m.get("result").is_some()).unwrap();
    assert_eq!(answer["result"]["decision"], "decline");
}

#[test]
fn interrupt_then_continue_then_resume() {
    let (mut harness, processes) = replay(&["interrupt", "resume"]);
    harness.start(Some("Write the numbers from 1 to 400")).unwrap();
    harness.wait_until("streamed text", |h| {
        h.count(|payload| matches!(payload, EventPayload::ItemTextDelta { .. })) > 0
    });

    harness.interrupt().unwrap();
    assert_eq!(harness.turn_statuses(), [(TurnStatus::Interrupted, None)]);
    let streamed = harness.agent_messages().remove(0);
    assert!(streamed.starts_with("1\n2\n3\n"), "{streamed:?}");

    harness.input("Reply with exactly: still here").unwrap();
    harness.wait_turns(2);
    assert_eq!(harness.turn_statuses()[1], (TurnStatus::Completed, None));
    assert_eq!(harness.agent_messages().last().unwrap(), "still here");
    let session = harness.thread().provider_session.clone().unwrap();

    harness.stop().unwrap();
    assert_eq!(processes.kills(), 1);
    harness.resume(Some("What exact phrase did I ask you to reply with last time?")).unwrap();
    harness.wait_turns(3);
    golden(&harness, "interrupt-resume");
    assert_eq!(harness.agent_messages().last().unwrap(), "still here");
    assert_eq!(harness.thread().provider_session, Some(session.clone()));
    // Resuming a known thread announces nothing new.
    assert_eq!(harness.count(|p| matches!(p, EventPayload::SessionIdentified { .. })), 1);

    let sent = processes.stdin_messages();
    let interrupt = sent.iter().find(|m| m["method"] == "turn/interrupt").expect("turn/interrupt");
    assert_eq!(interrupt["params"]["turnId"], "01a1de77-21f4-71e2-b473-e126ee77ae82");
    let resume = sent.iter().find(|m| m["method"] == "thread/resume").expect("thread/resume");
    assert_eq!(resume["params"]["threadId"], session.as_str());
    assert_eq!(processes.unused_scripts(), 0);
}

#[test]
fn a_dying_agent_fails_the_turn_with_its_last_words() {
    let processes = ScriptedProcesses::new([vec![
        ScriptStep::AwaitStdin,
        ScriptStep::line(r#"{"id":1,"result":{}}"#),
        ScriptStep::AwaitMessage("method:thread/start".into()),
        ScriptStep::line(r#"{"id":2,"result":{"thread":{"id":"th"}}}"#),
        ScriptStep::AwaitMessage("method:turn/start".into()),
        ScriptStep::Stderr(b"thread 'main' panicked\nerror: stream disconnected\n".to_vec()),
        ScriptStep::Exit(Some(101)),
    ]]);
    let mut harness = Harness::bundled("provider-codex").processes(processes).build();
    harness.start(Some("hello")).unwrap();
    harness.wait_turns(1);
    harness.assert_conformant();
    let reason = "codex exited with code 101: error: stream disconnected";
    assert_eq!(harness.turn_statuses(), [(TurnStatus::Failed, Some(reason.into()))]);
    assert!(harness.input("anyone?").is_err());
}

#[test]
fn a_message_sent_mid_turn_is_steered_into_it() {
    let (mut harness, processes) = replay(&["steer"]);
    harness.start(Some("First run `cat note.txt`, then run `cat other.txt`, then count")).unwrap();
    harness.wait_until("the turn to start", |h| {
        h.count(|payload| matches!(payload, EventPayload::TurnStarted { .. })) > 0
    });
    harness.steer("Also end your answer with the word BANANA.").unwrap();
    harness.wait_turns(1);
    golden(&harness, "steer");

    assert_eq!(harness.count(|p| matches!(p, EventPayload::TurnStarted { .. })), 1);
    assert!(harness.agent_messages().last().unwrap().ends_with("BANANA"));
    let sent = processes.stdin_messages();
    let steer = sent.iter().find(|m| m["method"] == "turn/steer").expect("turn/steer");
    // Codex only accepts a steer that names the turn it is meant for.
    assert_eq!(steer["params"]["expectedTurnId"], "01a1de9e-fd6e-7770-a949-51ee730040f4");
    assert_eq!(sent.iter().filter(|m| m["method"] == "turn/start").count(), 1);
}

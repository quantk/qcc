//! The echo provider driven through the real wasm host. Besides checking the provider
//! itself, these tests are the host's integration tests: permissions, timers,
//! processes, the key-value store and crash isolation.

use std::path::PathBuf;

use qcc_domain::{
    EventPayload, InteractionResolution, ItemStatus, PermissionMode, ProviderCapabilities,
    TurnStatus,
};
use qcc_plugin_host::services::DirKv;
use qcc_plugin_host::{Contribution, Grant, Outcome, Permissions, PluginError, ProviderRequest};
use qcc_plugin_testkit::{Harness, ScriptStep, ScriptedProcesses};

fn echo() -> Harness {
    Harness::bundled("provider-echo").build()
}

fn golden(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(name)
}

#[test]
fn declares_its_provider_and_its_ui() {
    let harness = echo();
    let [
        Contribution::Provider(info),
        Contribution::UiView(view),
        Contribution::UiView(canvas),
        Contribution::UiItem(kind),
        Contribution::Skill(skill),
    ] = harness.handle().contributions()
    else {
        panic!("unexpected contributions: {:?}", harness.handle().contributions());
    };
    assert_eq!((view.id.as_str(), view.title.as_str()), ("playground", "Echo"));
    assert_eq!(canvas.web.as_deref(), Some("index.html"));
    assert_eq!(kind, "provider-echo/card");
    assert!(skill.on_request, "{skill:?}");
    assert_eq!(info.id.as_str(), "echo");
    assert_eq!(
        info.capabilities,
        ProviderCapabilities {
            resume: true,
            fork: false,
            steer: true,
            permission_modes: PermissionMode::ALL.to_vec(),
            mcp_toggle: true,
            compact: true,
        }
    );
    assert_eq!(harness.handle().id(), "provider-echo");
}

#[test]
fn streamed_turn_matches_the_recorded_timeline() {
    let mut harness = echo();
    harness.start(Some("hello brave new world")).unwrap();
    harness.wait_turns(1);
    harness.assert_conformant();
    harness.assert_golden(golden("echo-streamed-turn.jsonl"));
    assert_eq!(harness.agent_messages(), ["hello brave new world"]);
    assert_eq!(harness.thread().provider_session.as_deref(), Some("echo-thread-1"));
    assert_eq!(harness.thread().title.as_deref(), Some("hello brave new world"));
}

#[test]
fn tool_call_splits_the_message_stream() {
    let mut harness = echo();
    harness.start(None).unwrap();
    harness.input("/tool grep").unwrap();
    harness.input("thanks").unwrap();
    harness.wait_turns(2);
    harness.assert_conformant();
    harness.assert_golden(golden("echo-tool-call.jsonl"));
    assert_eq!(harness.agent_messages(), ["Calling a tool.", "Done.", "thanks"]);
}

#[test]
fn question_is_answered_through_the_provider() {
    let mut harness = echo();
    harness.start(Some("/ask Proceed?")).unwrap();
    harness.wait_until("the question", |h| h.pending_interaction().is_some());
    assert_eq!(harness.turn_statuses(), []);

    harness.resolve(InteractionResolution::Text { text: "go ahead".into() }).unwrap();
    harness.wait_turns(1);
    harness.assert_conformant();
    assert_eq!(harness.agent_messages(), ["You said: go ahead"]);
}

#[test]
fn declined_approval_settles_the_item_as_declined() {
    let mut harness = echo();
    harness.start(Some("/approve rm -rf build")).unwrap();
    harness.wait_until("the approval", |h| h.pending_interaction().is_some());
    let requested = harness
        .events()
        .iter()
        .find_map(|event| match &event.payload {
            EventPayload::InteractionRequested { item_id, .. } => Some(item_id.clone()),
            _ => None,
        })
        .unwrap();

    harness.resolve(InteractionResolution::Selected { option_id: "deny".into() }).unwrap();
    harness.wait_turns(1);
    harness.assert_conformant();
    let settled = harness
        .events()
        .iter()
        .find_map(|event| match &event.payload {
            EventPayload::ItemCompleted { item, .. } => Some(item.clone()),
            _ => None,
        })
        .unwrap();
    // The request pointed at the very item that then settled.
    assert_eq!(requested, Some(settled.id.clone()));
    assert_eq!(settled.status, ItemStatus::Declined);
}

#[test]
fn timers_drive_a_slow_stream_and_interrupt_stops_it() {
    let mut harness = echo();
    let long = (0..2000).map(|n| n.to_string()).collect::<Vec<_>>().join(" ");
    harness.start(Some(&format!("/slow {long}"))).unwrap();
    harness.wait_until("streamed text", |h| {
        h.count(|payload| matches!(payload, EventPayload::ItemTextDelta { .. })) >= 3
    });
    harness.interrupt().unwrap();
    harness.wait_turns(1);
    assert_eq!(harness.turn_statuses(), [(TurnStatus::Interrupted, None)]);
    let partial = harness.agent_messages().remove(0);
    assert!(partial.starts_with("0 1 2 "), "{partial:?}");
    assert!(partial.len() < long.len());

    // The cancelled timer never fires into the next turn.
    harness.input("/slow x y").unwrap();
    harness.wait_turns(2);
    harness.assert_conformant();
    assert_eq!(harness.agent_messages()[1], "x y ");
}

#[test]
fn a_plugin_busy_on_zero_delay_timers_still_takes_calls() {
    let mut harness = echo();
    harness.start(Some("/spin")).unwrap();
    // The interrupt is a call queued behind an endless stream of due timers.
    harness.interrupt().unwrap();
    harness.wait_turns(1);
    assert_eq!(harness.turn_statuses(), [(TurnStatus::Interrupted, None)]);
}

#[test]
fn kv_store_outlives_the_plugin_instance() {
    let dir = tempfile::tempdir().unwrap();
    let run = |prompts: &[&str]| {
        let mut harness =
            Harness::bundled("provider-echo").kv(DirKv::new(dir.path().join("kv"))).build();
        harness.start(None).unwrap();
        for prompt in prompts {
            harness.input(prompt).unwrap();
        }
        harness.wait_turns(prompts.len());
        harness.assert_conformant();
        harness.agent_messages()
    };
    assert_eq!(run(&["/kv", "/kv"]), ["count 1", "count 2"]);
    assert_eq!(run(&["/kv"]), ["count 3"]);
}

#[test]
fn runs_an_allowed_process_and_streams_its_output() {
    let mut harness = echo();
    harness.start(Some("/exec echo from a real process")).unwrap();
    harness.wait_turns(1);
    harness.assert_conformant();
    assert_eq!(harness.agent_messages(), ["from a real process\n"]);
    assert_eq!(harness.turn_statuses(), [(TurnStatus::Completed, None)]);
}

#[test]
fn recorded_process_traffic_replaces_the_real_process() {
    let processes = ScriptedProcesses::new([vec![
        ScriptStep::line("recorded line 1"),
        ScriptStep::Stderr(b"noise".to_vec()),
        ScriptStep::line("recorded line 2"),
        ScriptStep::Exit(Some(2)),
    ]]);
    let mut harness = Harness::bundled("provider-echo").processes(processes.clone()).build();
    harness.start(Some("/exec echo live")).unwrap();
    harness.wait_turns(1);
    harness.assert_conformant();
    assert_eq!(harness.agent_messages(), ["recorded line 1\nrecorded line 2\n"]);
    assert_eq!(harness.turn_statuses(), [(TurnStatus::Failed, Some("exit code 2".into()))]);
    let spawns = processes.spawns();
    assert_eq!(
        (spawns[0].program.as_str(), &spawns[0].args[..]),
        ("echo", &["live".to_owned()][..])
    );
}

#[test]
fn a_program_outside_the_manifest_is_refused() {
    let mut harness = echo();
    harness.start(Some("/exec sh -c true")).unwrap();
    harness.wait_turns(1);
    harness.assert_conformant();
    assert_eq!(
        harness.turn_statuses(),
        [(TurnStatus::Failed, Some("permission denied: process \"sh\"".into()))]
    );
}

#[test]
fn ungranted_permissions_are_refused() {
    // The manifest requests everything; the user granted only deltas.
    let granted = Permissions { deltas: true, ..Permissions::default() };
    let mut harness = Harness::bundled("provider-echo").grant(Grant::Only(granted.clone())).build();
    assert_eq!(harness.handle().permissions(), &granted);
    harness.start(None).unwrap();

    assert_eq!(harness.input("/kv"), Err(PluginError::Rejected("permission denied: kv".into())));
    assert_eq!(
        harness.input("/slow a b"),
        Err(PluginError::Rejected("permission denied: timers".into()))
    );
    harness.input("/exec echo hi").unwrap();
    harness.wait_turns(1);
    assert_eq!(
        harness.turn_statuses(),
        [(TurnStatus::Failed, Some("permission denied: process \"echo\"".into()))]
    );
    // What was granted keeps working.
    harness.input("still here").unwrap();
    harness.wait_turns(2);
    assert_eq!(harness.agent_messages(), ["still here"]);
}

#[test]
fn a_plugin_without_the_deltas_permission_cannot_touch_the_timeline() {
    let mut harness =
        Harness::bundled("provider-echo").grant(Grant::Only(Permissions::default())).build();
    assert_eq!(
        harness.start(Some("hi")),
        Err(PluginError::Rejected("permission denied: deltas".into()))
    );
    harness.pump();
    assert!(harness.events().iter().all(|event| !matches!(
        event.payload,
        EventPayload::TurnStarted { .. } | EventPayload::ItemStarted { .. }
    )));
}

#[test]
fn a_crash_is_contained_and_reported() {
    let mut harness = echo();
    harness.start(None).unwrap();
    assert!(matches!(harness.input("/crash"), Err(PluginError::Crashed(_))));
    assert!(harness.crash().is_some());
    // The instance is never entered again.
    assert!(matches!(harness.input("hello"), Err(PluginError::Crashed(_))));
}

#[test]
fn stop_settles_everything_and_resume_restores_the_session() {
    let mut harness = echo();
    harness.start(Some("/ask Still there?")).unwrap();
    harness.wait_until("the question", |h| h.pending_interaction().is_some());
    harness.stop().unwrap();
    harness.assert_conformant();
    assert_eq!(harness.turn_statuses(), [(TurnStatus::Interrupted, None)]);
    assert_eq!(harness.count(|p| matches!(p, EventPayload::SessionEnded { .. })), 1);
    assert_eq!(
        harness.input("anyone?"),
        Err(PluginError::Rejected("the session was not started".into()))
    );

    harness.resume(Some("back again")).unwrap();
    harness.wait_turns(2);
    assert_eq!(harness.agent_messages(), ["back again"]);
    assert_eq!(harness.thread().provider_session.as_deref(), Some("echo-thread-1"));
    assert!(
        harness.events().iter().any(|event| matches!(
            event.payload,
            EventPayload::SessionStarted { resumed: true, .. }
        ))
    );
}

#[test]
fn lists_models() {
    let mut harness = echo();
    let Outcome::Models(models) = harness.call(ProviderRequest::ListModels).unwrap() else {
        panic!("expected models");
    };
    assert_eq!(models[0].id, "echo-1");
}

#[test]
fn steering_adds_to_the_running_turn() {
    let mut harness = echo();
    harness.start(Some("/slow one two three")).unwrap();
    harness.steer("four five").unwrap();
    harness.wait_turns(1);
    harness.assert_conformant();
    assert_eq!(harness.agent_messages(), ["one two three four five "]);
    // One turn took both messages.
    assert_eq!(harness.count(|p| matches!(p, EventPayload::TurnStarted { .. })), 1);
    assert_eq!(harness.steer("too late"), Err(PluginError::Rejected("no turn is running".into())));
}

#[test]
fn the_permission_mode_decides_whether_the_user_is_asked() {
    let run = |mode| {
        let mut harness = echo();
        harness.set_permission_mode(mode);
        harness.start(Some("/approve make deploy")).unwrap();
        harness.pump();
        harness
    };
    for mode in [None, Some(PermissionMode::Ask), Some(PermissionMode::AcceptEdits)] {
        assert!(run(mode).pending_interaction().is_some(), "{mode:?} asks");
    }
    for mode in [PermissionMode::Auto, PermissionMode::Full] {
        let mut harness = run(Some(mode));
        harness.wait_turns(1);
        harness.assert_conformant();
        assert_eq!(harness.count(|p| matches!(p, EventPayload::InteractionRequested { .. })), 0);
        assert_eq!(harness.completed_items()[0].status, ItemStatus::Completed);
    }
}

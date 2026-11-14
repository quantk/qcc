use qcc_domain::{
    Delta, EventPayload, InteractionRequest, InteractionResolution, ItemKey, ItemShape,
    TextChannel, Thread, ThreadEvent, ThreadStatus, TurnStatus,
};
use qcc_runtime::{EngineError, NewThread, SequentialMint, Supervisor, SupervisorError, Timeline};

fn supervisor() -> Supervisor {
    Supervisor::new(|| Box::new(SequentialMint::default()))
}

fn new_thread(id: &str) -> NewThread {
    NewThread {
        id: id.into(),
        provider: "echo".into(),
        cwd: "/work".into(),
        model: None,
        parent: None,
    }
}

fn types(events: &[ThreadEvent]) -> Vec<String> {
    events
        .iter()
        .map(|event| {
            let value = serde_json::to_value(event).unwrap();
            value["type"].as_str().unwrap().to_owned()
        })
        .collect()
}

fn text(text: &str) -> Delta {
    Delta::ItemTextDelta {
        key: ItemKey::Stream("assistant".into()),
        channel: TextChannel::AgentMessage,
        text: text.into(),
    }
}

/// Replaying the journal must rebuild the projection the engine holds.
fn assert_projection(journal: &[ThreadEvent], thread: &Thread) {
    let EventPayload::ThreadCreated {
        thread: mut replayed,
    } = journal[0].payload.clone()
    else {
        panic!("the first event must be thread/created");
    };
    for event in journal {
        replayed.apply(event);
    }
    assert_eq!(&replayed, thread);
    for (index, event) in journal.iter().enumerate() {
        assert_eq!(event.seq, index as u64 + 1, "seq has no gaps");
    }
    assert_eq!(Timeline::check(journal.iter().map(|e| &e.payload)), vec![]);
}

#[test]
fn a_turn_drives_status_and_projection() {
    let mut supervisor = supervisor();
    let id = "t1".into();
    let mut journal = supervisor.create(new_thread("t1"), 100).unwrap();
    assert_eq!(types(&journal), ["thread/created"]);
    assert_eq!(
        supervisor.create(new_thread("t1"), 100),
        Err(SupervisorError::ThreadExists("t1".into()))
    );

    let engine = supervisor.engine(&id).unwrap();
    assert!(!engine.session_live());
    journal.extend(engine.submit_input("\n  fix the bug  \nplease".into(), 110));
    journal.extend(engine.session_started(Some("sess-1".into()), false, 120));
    assert!(engine.session_live());

    let events = engine.apply_deltas([Delta::TurnOpen, text("ok")], 130);
    assert_eq!(
        types(&events),
        [
            "turn/started",
            "item/started",
            "item/text/delta",
            "thread/status"
        ]
    );
    assert_eq!(engine.thread().status, ThreadStatus::Active);
    journal.extend(events);

    let events = engine.apply_deltas(
        [Delta::TurnBoundary {
            status: TurnStatus::Completed,
            error: None,
            claim_if_idle: false,
        }],
        140,
    );
    assert_eq!(
        types(&events),
        ["item/completed", "turn/completed", "thread/status"]
    );
    journal.extend(events);

    let thread = engine.thread().clone();
    assert_eq!(thread.status, ThreadStatus::Idle);
    assert_eq!(thread.title.as_deref(), Some("fix the bug"));
    assert_eq!(thread.provider_session.as_deref(), Some("sess-1"));
    assert_eq!((thread.created_at, thread.updated_at), (100, 140));
    assert_projection(&journal, &thread);
}

#[test]
fn failed_session_settles_open_work_and_recovers_on_next_start() {
    let mut supervisor = supervisor();
    let mut journal = supervisor.create(new_thread("t1"), 1).unwrap();
    let engine = supervisor.engine(&"t1".into()).unwrap();
    journal.extend(engine.session_started(None, false, 2));
    journal.extend(engine.apply_deltas([Delta::TurnOpen, text("partial")], 3));

    let events = engine.session_failed("agent crashed".into(), 4);
    assert_eq!(
        types(&events),
        [
            "provider/error",
            "item/completed",
            "turn/completed",
            "thread/status"
        ]
    );
    assert!(matches!(
        &events[2].payload,
        EventPayload::TurnCompleted { status: TurnStatus::Failed, error: Some(e), .. } if e == "agent crashed"
    ));
    assert_eq!(engine.thread().status, ThreadStatus::Failed);
    assert!(!engine.session_live());
    journal.extend(events);

    let events = engine.session_started(None, true, 5);
    assert_eq!(types(&events), ["session/started", "thread/status"]);
    assert_eq!(engine.thread().status, ThreadStatus::Idle);
    journal.extend(events);
    assert_projection(&journal, &engine.thread().clone());
}

#[test]
fn interaction_roundtrip() {
    let mut supervisor = supervisor();
    supervisor.create(new_thread("t1"), 1).unwrap();
    let engine = supervisor.engine(&"t1".into()).unwrap();
    engine.session_started(None, false, 2);
    let events = engine.apply_deltas(
        [
            Delta::TurnOpen,
            Delta::InteractionRequest {
                request_id: "p1".into(),
                item: None,
                request: InteractionRequest::Question {
                    question: "which?".into(),
                    options: vec![],
                    allow_text: true,
                },
            },
        ],
        3,
    );
    let EventPayload::InteractionRequested { interaction_id, .. } = &events[1].payload else {
        panic!("expected interaction/requested, got {:?}", events[1]);
    };

    let resolution = InteractionResolution::Text {
        text: "that one".into(),
    };
    let (request_id, events) = engine
        .resolve_interaction(interaction_id.clone(), resolution.clone(), 4)
        .unwrap();
    assert_eq!(request_id, "p1");
    assert_eq!(types(&events), ["interaction/resolved"]);
    assert_eq!(
        engine.resolve_interaction(interaction_id.clone(), resolution, 5),
        Err(EngineError::InteractionNotPending(interaction_id.clone()))
    );
}

#[test]
fn force_settle_is_a_noop_when_the_bridge_settled_itself() {
    let mut supervisor = supervisor();
    supervisor.create(new_thread("t1"), 1).unwrap();
    let engine = supervisor.engine(&"t1".into()).unwrap();
    engine.session_started(None, false, 2);
    engine.apply_deltas([Delta::TurnOpen, text("a")], 3);
    assert_eq!(
        types(&engine.force_settle(4)),
        ["item/completed", "turn/completed", "thread/status"]
    );
    assert_eq!(engine.force_settle(5), vec![]);

    assert_eq!(
        types(&engine.session_released(Some("stopped".into()), 6)),
        ["session/ended"]
    );
    assert!(!engine.session_live());
    assert_eq!(engine.session_released(None, 7), vec![]);
}

#[test]
fn restart_settles_what_the_dead_session_left_open() {
    let mut first = supervisor();
    let mut journal = first.create(new_thread("t1"), 1).unwrap();
    let engine = first.engine(&"t1".into()).unwrap();
    journal.extend(engine.session_started(Some("sess".into()), false, 2));
    journal.extend(engine.apply_deltas(
        [
            Delta::TurnOpen,
            text("thinking"),
            Delta::ItemOpen {
                key: ItemKey::Provider("call".into()),
                shape: ItemShape::Command {
                    command: "sleep 100".into(),
                    cwd: None,
                    output: None,
                    exit_code: None,
                },
            },
            Delta::InteractionRequest {
                request_id: "p".into(),
                item: Some(ItemKey::Provider("call".into())),
                request: InteractionRequest::Approval {
                    title: "Run?".into(),
                    detail: None,
                    options: vec![],
                },
            },
        ],
        3,
    ));
    let persisted = engine.thread().clone();
    assert_eq!(persisted.status, ThreadStatus::Active);

    // The daemon dies here. A new one adopts the thread from the journal.
    let mut second = Supervisor::new(|| Box::new(SequentialMint::starting_after(100)));
    let events = second.recover(persisted, journal.iter().map(|e| &e.payload), 50);
    assert_eq!(
        types(&events),
        [
            "interaction/resolved",
            "item/completed",
            "turn/completed",
            "thread/status"
        ]
    );
    assert_eq!(events[0].seq, journal.last().unwrap().seq + 1);
    journal.extend(events);

    let engine = second.engine(&"t1".into()).unwrap();
    assert!(!engine.session_live());
    let thread = engine.thread().clone();
    assert_eq!(thread.status, ThreadStatus::Idle);
    assert_eq!(thread.provider_session.as_deref(), Some("sess"));
    assert_projection(&journal, &thread);

    // The thread keeps working: the resumed session opens a new turn.
    journal.extend(engine.session_started(None, true, 60));
    journal.extend(engine.apply_deltas([Delta::TurnOpen], 61));
    assert_projection(&journal, &engine.thread().clone());
}

#[test]
fn unknown_thread() {
    let mut supervisor = supervisor();
    assert!(matches!(
        supervisor.engine(&"nope".into()),
        Err(SupervisorError::UnknownThread(id)) if id.as_str() == "nope"
    ));
}

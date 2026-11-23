//! The assembler against recorded delta sequences, written in the wire format a bridge
//! uses, so the grammar's serialization is exercised too.

use qcc_domain::{Delta, EventPayload, Item, ItemShape};
use qcc_runtime::{DeltaAssembler, GrammarViolation, SequentialMint, Timeline};
use serde_json::json;

fn assembler() -> DeltaAssembler {
    DeltaAssembler::new(Box::new(SequentialMint::default()))
}

fn deltas(value: serde_json::Value) -> Vec<Delta> {
    serde_json::from_value(value).expect("valid deltas")
}

/// Feed deltas, check the assembled timeline against the grammar, render it compactly.
fn run(assembler: &mut DeltaAssembler, value: serde_json::Value) -> Vec<String> {
    let mut payloads = Vec::new();
    for delta in deltas(value) {
        payloads.extend(assembler.apply(delta));
    }
    payloads.iter().map(render).collect()
}

fn assemble(value: serde_json::Value) -> (Vec<String>, Vec<GrammarViolation>) {
    let mut assembler = assembler();
    let mut payloads = Vec::new();
    for delta in deltas(value) {
        payloads.extend(assembler.apply(delta));
    }
    assert_eq!(Timeline::check(&payloads), vec![], "assembled timeline breaks the grammar");
    (payloads.iter().map(render).collect(), assembler.take_violations())
}

fn item(item: &Item) -> String {
    let body = match &item.shape {
        ItemShape::AgentMessage { text } => format!("agentMessage {text:?}"),
        ItemShape::Reasoning { text } => format!("reasoning {text:?}"),
        ItemShape::Tool { name, output, .. } => format!("tool {name} -> {output:?}"),
        ItemShape::Command { command, exit_code, .. } => {
            format!("command {command:?} -> {exit_code:?}")
        }
        other => format!("{other:?}"),
    };
    format!("{} {:?} {body}", item.id, item.status)
}

fn render(payload: &EventPayload) -> String {
    match payload {
        EventPayload::TurnStarted { turn_id } => format!("turn/started {turn_id}"),
        EventPayload::TurnCompleted { turn_id, status, error, .. } => match error {
            Some(error) => format!("turn/completed {turn_id} {status:?} {error:?}"),
            None => format!("turn/completed {turn_id} {status:?}"),
        },
        EventPayload::ItemStarted { item: i, .. } => format!("item/started {}", item(i)),
        EventPayload::ItemTextDelta { item_id, text } => {
            format!("item/text/delta {item_id} {text:?}")
        }
        EventPayload::ItemCompleted { item: i, .. } => format!("item/completed {}", item(i)),
        EventPayload::InteractionRequested { interaction_id, item_id, .. } => {
            format!("interaction/requested {interaction_id} item={item_id:?}")
        }
        EventPayload::InteractionResolved { interaction_id, resolution } => {
            format!("interaction/resolved {interaction_id} {resolution:?}")
        }
        EventPayload::SessionEnded { reason } => format!("session/ended {reason:?}"),
        EventPayload::UsageUpdated { usage } => {
            format!(
                "usage/updated in={} out={}",
                usage.total.input_tokens, usage.total.output_tokens
            )
        }
        other => format!("{other:?}"),
    }
}

#[test]
fn streamed_message_turn() {
    let (events, violations) = assemble(json!([
        {"kind": "turn.open"},
        {"kind": "item.text-delta", "key": {"stream": "assistant"}, "channel": "agentMessage", "text": "Hel"},
        {"kind": "item.text-delta", "key": {"stream": "assistant"}, "channel": "agentMessage", "text": "lo"},
        {"kind": "item.text-close", "key": {"stream": "assistant"}, "channel": "agentMessage"},
        {"kind": "usage", "usage": {
            "total": {"input_tokens": 10, "output_tokens": 2},
            "last": {"input_tokens": 10, "output_tokens": 2}
        }},
        {"kind": "turn.boundary", "status": "completed"},
    ]));
    assert_eq!(violations, vec![]);
    assert_eq!(
        events,
        [
            "turn/started turn_1",
            "item/started item_1 Pending agentMessage \"\"",
            "item/text/delta item_1 \"Hel\"",
            "item/text/delta item_1 \"lo\"",
            "item/completed item_1 Completed agentMessage \"Hello\"",
            "usage/updated in=10 out=2",
            "turn/completed turn_1 Completed",
        ]
    );
}

#[test]
fn provider_final_text_wins_over_the_stream() {
    let (events, _) = assemble(json!([
        {"kind": "turn.open"},
        {"kind": "item.text-delta", "key": {"provider": "msg-1"}, "channel": "agentMessage", "text": "Helo"},
        {"kind": "item.text-close", "key": {"provider": "msg-1"}, "channel": "agentMessage", "text": "Hello"},
        {"kind": "turn.boundary", "status": "completed"},
    ]));
    assert_eq!(events[3], "item/completed item_1 Completed agentMessage \"Hello\"");
}

#[test]
fn tool_open_releases_the_anonymous_stream() {
    let (events, violations) = assemble(json!([
        {"kind": "turn.open"},
        {"kind": "item.text-delta", "key": {"stream": "assistant"}, "channel": "agentMessage", "text": "Let me look."},
        {"kind": "item.open", "key": {"provider": "call-1"},
         "shape": {"type": "tool", "name": "Read", "input": {"path": "a.rs"}}},
        {"kind": "item.close", "key": {"provider": "call-1"}, "status": "completed",
         "shape": {"type": "tool", "name": "Read", "input": {"path": "a.rs"}, "output": "fn main() {}"}},
        {"kind": "item.text-delta", "key": {"stream": "assistant"}, "channel": "agentMessage", "text": "Done."},
        {"kind": "turn.boundary", "status": "completed"},
    ]));
    assert_eq!(violations, vec![]);
    assert_eq!(
        events,
        [
            "turn/started turn_1",
            "item/started item_1 Pending agentMessage \"\"",
            "item/text/delta item_1 \"Let me look.\"",
            "item/completed item_1 Completed agentMessage \"Let me look.\"",
            "item/started item_2 Pending tool Read -> None",
            "item/completed item_2 Completed tool Read -> Some(\"fn main() {}\")",
            // The same stream key after the tool call is a new item.
            "item/started item_3 Pending agentMessage \"\"",
            "item/text/delta item_3 \"Done.\"",
            // The bridge never closed it: the boundary settles it with what it received.
            "item/completed item_3 Completed agentMessage \"Done.\"",
            "turn/completed turn_1 Completed",
        ]
    );
}

#[test]
fn items_never_open_a_turn() {
    let (events, violations) = assemble(json!([
        {"kind": "item.text-delta", "key": {"stream": "assistant"}, "channel": "agentMessage", "text": "hi"},
        {"kind": "item.open", "key": {"provider": "c"}, "shape": {"type": "command", "command": "ls"}},
        {"kind": "item.close", "key": {"provider": "c"}, "status": "completed",
         "shape": {"type": "command", "command": "ls"}},
        {"kind": "item.text-close", "key": {"stream": "assistant"}, "channel": "agentMessage", "text": "hi"},
    ]));
    assert_eq!(events, Vec::<String>::new());
    assert_eq!(
        violations,
        [
            GrammarViolation::NoOpenTurn("item.text-delta"),
            GrammarViolation::NoOpenTurn("item.open"),
            GrammarViolation::NoOpenTurn("item.close"),
            GrammarViolation::NoOpenTurn("item.text-close"),
        ]
    );
}

#[test]
fn close_without_open_builds_the_whole_item() {
    let (events, violations) = assemble(json!([
        {"kind": "turn.open"},
        {"kind": "item.close", "key": {"provider": "c"}, "status": "failed",
         "shape": {"type": "command", "command": "false", "exit_code": 1}},
        {"kind": "turn.boundary", "status": "completed"},
    ]));
    assert_eq!(violations, vec![]);
    assert_eq!(
        events,
        [
            "turn/started turn_1",
            "item/started item_1 Pending command \"false\" -> Some(1)",
            "item/completed item_1 Failed command \"false\" -> Some(1)",
            "turn/completed turn_1 Completed",
        ]
    );
}

#[test]
fn repeated_close_is_deduplicated_and_reopen_keeps_the_id() {
    let close = json!({"kind": "item.close", "key": {"provider": "c"}, "status": "completed",
        "shape": {"type": "command", "command": "ls", "exit_code": 0}});
    let (events, violations) = assemble(json!([
        {"kind": "turn.open"},
        {"kind": "item.open", "key": {"provider": "c"}, "shape": {"type": "command", "command": "ls"}},
        close,
        close,
        {"kind": "item.open", "key": {"provider": "c"}, "shape": {"type": "command", "command": "ls"}},
        {"kind": "item.open", "key": {"provider": "c"}, "shape": {"type": "command", "command": "ls"}},
        {"kind": "turn.boundary", "status": "interrupted"},
    ]));
    assert_eq!(
        violations,
        [GrammarViolation::ItemAlreadyOpen(qcc_domain::ItemKey::Provider("c".into()))]
    );
    assert_eq!(
        events,
        [
            "turn/started turn_1",
            "item/started item_1 Pending command \"ls\" -> None",
            "item/completed item_1 Completed command \"ls\" -> Some(0)",
            "item/started item_1 Pending command \"ls\" -> None",
            "item/completed item_1 Interrupted command \"ls\" -> None",
            "turn/completed turn_1 Interrupted",
        ]
    );
}

#[test]
fn session_reset_starts_a_fresh_provider_id_space() {
    let (events, _) = assemble(json!([
        {"kind": "turn.open"},
        {"kind": "item.close", "key": {"provider": "1"}, "status": "completed",
         "shape": {"type": "fileRead", "path": "a"}},
        {"kind": "turn.boundary", "status": "completed"},
        {"kind": "session.reset"},
        {"kind": "turn.open"},
        {"kind": "item.close", "key": {"provider": "1"}, "status": "completed",
         "shape": {"type": "fileRead", "path": "b"}},
        {"kind": "turn.boundary", "status": "completed"},
    ]));
    let completed: Vec<_> = events.iter().filter(|e| e.starts_with("item/completed")).collect();
    assert!(completed[0].starts_with("item/completed item_1 "));
    assert!(completed[1].starts_with("item/completed item_2 "));
}

#[test]
fn duplicate_turn_open_and_stray_boundary_are_dropped() {
    let (events, violations) = assemble(json!([
        {"kind": "turn.boundary", "status": "completed"},
        {"kind": "turn.open"},
        {"kind": "turn.open"},
        {"kind": "turn.boundary", "status": "failed", "error": "boom"},
        {"kind": "turn.boundary", "status": "completed"},
    ]));
    assert_eq!(events, ["turn/started turn_1", "turn/completed turn_1 Failed \"boom\""]);
    assert_eq!(
        violations,
        [
            GrammarViolation::BoundaryWithoutTurn,
            GrammarViolation::TurnAlreadyOpen,
            GrammarViolation::BoundaryWithoutTurn,
        ]
    );
}

#[test]
fn claiming_boundary_owns_a_turn_only_when_input_is_waiting() {
    let claim = json!([{"kind": "turn.boundary", "status": "completed", "claim_if_idle": true}]);

    let mut idle = assembler();
    assert_eq!(run(&mut idle, claim.clone()), Vec::<String>::new());
    assert_eq!(idle.take_violations(), vec![]);

    // A prompt the provider handled without doing work still settles a turn.
    let mut waiting = assembler();
    waiting.note_input();
    assert_eq!(
        run(&mut waiting, claim.clone()),
        ["turn/started turn_1", "turn/completed turn_1 Completed"]
    );
    assert_eq!(run(&mut waiting, claim), Vec::<String>::new());
}

#[test]
fn interaction_is_mapped_and_cancelled_when_the_turn_settles() {
    let mut assembler = assembler();
    let events = run(
        &mut assembler,
        json!([
            {"kind": "turn.open"},
            {"kind": "item.open", "key": {"provider": "c"}, "shape": {"type": "command", "command": "rm -rf x"}},
            {"kind": "interaction.request", "request_id": "perm-1", "item": {"provider": "c"},
             "request": {"type": "approval", "title": "Run command?", "options": [
                {"id": "allow", "label": "Allow", "kind": "allow"},
                {"id": "deny", "label": "Deny", "kind": "deny"}
             ]}},
            {"kind": "interaction.request", "request_id": "perm-1",
             "request": {"type": "question", "question": "again?"}},
            {"kind": "interaction.request", "request_id": "perm-2",
             "request": {"type": "question", "question": "sure?", "allow_text": true}},
        ]),
    );
    assert_eq!(
        events,
        [
            "turn/started turn_1",
            "item/started item_1 Pending command \"rm -rf x\" -> None",
            "interaction/requested int_1 item=Some(ItemId(\"item_1\"))",
            "interaction/requested int_2 item=None",
        ]
    );
    assert_eq!(
        assembler.take_violations(),
        [GrammarViolation::InteractionAlreadyPending("perm-1".into())]
    );

    // The answer goes back with the provider's own handle, exactly once.
    assert_eq!(assembler.take_interaction(&"int_1".into()), Some("perm-1".to_owned()));
    assert_eq!(assembler.take_interaction(&"int_1".into()), None);

    let events = run(&mut assembler, json!([{"kind": "session.ended", "reason": "exit"}]));
    assert_eq!(
        events,
        [
            "interaction/resolved int_2 Cancelled",
            "item/completed item_1 Interrupted command \"rm -rf x\" -> None",
            "turn/completed turn_1 Interrupted",
            "session/ended Some(\"exit\")",
        ]
    );
}

#[test]
fn text_delta_into_a_non_text_item_is_rejected() {
    let (events, violations) = assemble(json!([
        {"kind": "turn.open"},
        {"kind": "item.open", "key": {"provider": "c"}, "shape": {"type": "command", "command": "ls"}},
        {"kind": "item.text-delta", "key": {"provider": "c"}, "channel": "agentMessage", "text": "x"},
        {"kind": "item.close", "key": {"provider": "c"}, "status": "pending",
         "shape": {"type": "command", "command": "ls"}},
    ]));
    assert_eq!(events.len(), 2);
    assert_eq!(
        violations,
        [
            GrammarViolation::NotATextItem(qcc_domain::ItemKey::Provider("c".into())),
            GrammarViolation::CloseWithPendingStatus(qcc_domain::ItemKey::Provider("c".into())),
        ]
    );
}

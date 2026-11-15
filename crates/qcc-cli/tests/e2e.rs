//! End to end: a real daemon in a temporary directory, driven by the real `qcc`
//! binary over the socket, with the echo provider running as a wasm plugin.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::Value;

/// Compiled plugins are shared by every daemon these tests start.
fn cache_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("qcc-cache")
}

/// A daemon running on its own runtime thread, like a separate process would.
struct Daemon {
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<anyhow::Result<()>>>,
    socket: PathBuf,
}

impl Daemon {
    fn start(data_dir: &Path) -> Self {
        let config = qcc_daemon::Config {
            data_dir: data_dir.to_path_buf(),
            socket: data_dir.join("qccd.sock"),
            cache_dir: cache_dir(),
        };
        let socket = config.socket.clone();
        let (shutdown, stop) = tokio::sync::oneshot::channel::<()>();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
            runtime.block_on(qcc_daemon::serve(
                config,
                move || {
                    let _ = ready_tx.send(());
                },
                async {
                    let _ = stop.await;
                },
            ))
        });
        ready_rx.recv_timeout(Duration::from_secs(60)).expect("daemon did not start");
        Self { shutdown: Some(shutdown), thread: Some(thread), socket }
    }

    fn stop(mut self) {
        self.stop_inner().expect("daemon exited with an error");
    }

    fn stop_inner(&mut self) -> anyhow::Result<()> {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        match self.thread.take() {
            Some(thread) => thread.join().expect("daemon thread panicked"),
            None => Ok(()),
        }
    }

    /// Run `qcc` against this daemon.
    fn qcc(&self, args: &[&str]) -> Output {
        self.qcc_env(args, &[])
    }

    fn qcc_env(&self, args: &[&str], env: &[(&str, &str)]) -> Output {
        let output = Command::new(env!("CARGO_BIN_EXE_qcc"))
            .arg("--socket")
            .arg(&self.socket)
            .args(args)
            .env_remove("QCC_THREAD_ID")
            .env_remove("QCC_SOCKET")
            .envs(env.iter().copied())
            .output()
            .expect("qcc runs");
        Output {
            ok: output.status.success(),
            stdout: String::from_utf8(output.stdout).unwrap(),
            stderr: String::from_utf8(output.stderr).unwrap(),
        }
    }

    /// Spawn a thread without waiting for it; returns its id.
    fn spawn(&self, prompt: &str) -> String {
        let output =
            self.qcc(&["thread", "spawn", "--provider", "echo", "--prompt", prompt, "--json"]);
        assert!(output.ok, "{}", output.stderr);
        output.json()["thread"]["id"].as_str().unwrap().to_owned()
    }

    fn events(&self, thread: &str) -> Vec<Value> {
        let output = self.qcc(&["thread", "events", thread, "--json"]);
        assert!(output.ok, "{}", output.stderr);
        output.json_lines()
    }

    /// Poll the journal until an event of `kind` shows up.
    fn wait_for(&self, thread: &str, kind: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(event) = self.events(thread).into_iter().find(|e| e["type"] == kind) {
                return event;
            }
            assert!(Instant::now() < deadline, "no {kind} event: {:#?}", self.events(thread));
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn thread(&self, thread: &str) -> Value {
        let output = self.qcc(&["thread", "show", thread, "--json"]);
        assert!(output.ok, "{}", output.stderr);
        output.json()["thread"].clone()
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.stop_inner();
    }
}

struct Output {
    ok: bool,
    stdout: String,
    stderr: String,
}

impl Output {
    fn json(&self) -> Value {
        serde_json::from_str(&self.stdout).unwrap_or_else(|e| panic!("{e}: {:?}", self.stdout))
    }

    fn json_lines(&self) -> Vec<Value> {
        self.stdout.lines().map(|line| serde_json::from_str(line).unwrap()).collect()
    }
}

fn types(events: &[Value]) -> Vec<&str> {
    events.iter().map(|event| event["type"].as_str().unwrap()).collect()
}

fn messages(events: &[Value]) -> Vec<&str> {
    events
        .iter()
        .filter(|e| e["type"] == "item/completed" && e["item"]["type"] == "agentMessage")
        .map(|e| e["item"]["text"].as_str().unwrap())
        .collect()
}

#[test]
fn spawn_streams_a_turn_and_the_journal_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path());

    let output = daemon.qcc(&[
        "thread",
        "spawn",
        "--provider",
        "echo",
        "--prompt",
        "hi there",
        "--json",
        "--follow",
    ]);
    assert!(output.ok, "{}", output.stderr);
    let streamed = output.json_lines();
    assert_eq!(
        types(&streamed),
        [
            "thread/created",
            "input/submitted",
            "session/started",
            "turn/started",
            "item/started",
            "item/text/delta",
            "item/text/delta",
            "item/completed",
            "usage/updated",
            "turn/completed",
        ]
    );
    assert_eq!(messages(&streamed), ["hi there"]);
    let seqs: Vec<u64> = streamed.iter().map(|e| e["seq"].as_u64().unwrap()).collect();
    assert_eq!(seqs, (1..=10).collect::<Vec<_>>());
    let id = streamed[0]["thread_id"].as_str().unwrap().to_owned();

    let listed = daemon.qcc(&["thread", "list", "--json"]).json();
    assert_eq!(listed["threads"][0]["id"], id.as_str());
    assert_eq!(listed["threads"][0]["status"], "idle");
    assert_eq!(listed["threads"][0]["title"], "hi there");

    // What a client was told is exactly what the journal holds.
    assert_eq!(daemon.events(&id), streamed);
    let human = daemon.qcc(&["thread", "show", &id]);
    assert!(human.stdout.contains("> hi there\nhi there"), "{}", human.stdout);

    daemon.stop();
    let daemon = Daemon::start(dir.path());

    // The timeline is read back from SQLite, plus the note that the session ended.
    let after_restart = daemon.events(&id);
    assert_eq!(after_restart[..streamed.len()], streamed[..]);
    assert_eq!(types(&after_restart[streamed.len()..]), ["session/ended"]);
    assert_eq!(daemon.thread(&id)["title"], "hi there");

    // The thread keeps working: input resumes the provider session.
    let output = daemon.qcc(&["thread", "send", &id, "still there?", "--json", "--follow"]);
    assert!(output.ok, "{}", output.stderr);
    let resumed = output.json_lines();
    assert_eq!(types(&resumed)[..2], ["input/submitted", "session/started"]);
    assert_eq!(resumed[1]["resumed"], true);
    assert_eq!(resumed[1]["provider_session"], format!("echo-{id}").as_str());
    assert_eq!(messages(&resumed), ["still there?"]);
    assert_eq!(resumed[0]["seq"].as_u64().unwrap(), after_restart.len() as u64 + 1);
}

#[test]
fn question_is_answered_from_the_cli() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path());
    let id = daemon.spawn("/ask Deploy to production?");

    let request = daemon.wait_for(&id, "interaction/requested");
    assert_eq!(request["request"]["question"], "Deploy to production?");
    assert_eq!(daemon.thread(&id)["status"], "active");
    let interaction = request["interaction_id"].as_str().unwrap();

    let human = daemon.qcc(&["thread", "show", &id]);
    assert!(human.stdout.contains(&format!("qcc interaction resolve {id} {interaction}")));

    let answer =
        daemon.qcc(&["interaction", "resolve", &id, interaction, "--text", "not on a Friday"]);
    assert!(answer.ok, "{}", answer.stderr);
    daemon.wait_for(&id, "turn/completed");
    assert_eq!(messages(&daemon.events(&id)), ["You said: not on a Friday"]);
    assert_eq!(daemon.thread(&id)["status"], "idle");

    // Answering twice is an error, not a second answer.
    let again = daemon.qcc(&["interaction", "resolve", &id, interaction, "--cancel"]);
    assert!(!again.ok);
    assert!(again.stderr.contains("is not pending"), "{}", again.stderr);
}

#[test]
fn stop_interrupts_a_running_turn() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path());
    let long = (0..5000).map(|n| n.to_string()).collect::<Vec<_>>().join(" ");
    let id = daemon.spawn(&format!("/slow {long}"));
    daemon.wait_for(&id, "item/text/delta");

    let stopped = daemon.qcc(&["thread", "stop", &id]);
    assert!(stopped.ok, "{}", stopped.stderr);
    // `stop` returns only once the turn is settled.
    let events = daemon.events(&id);
    let turn = events.iter().find(|e| e["type"] == "turn/completed").expect("turn settled");
    assert_eq!(turn["status"], "interrupted");
    assert_eq!(daemon.thread(&id)["status"], "idle");

    // The session is still attached: the next message is a plain input, not a resume.
    let output = daemon.qcc(&["thread", "send", &id, "next", "--json", "--follow"]);
    assert_eq!(
        types(&output.json_lines()),
        [
            "input/submitted",
            "turn/started",
            "item/started",
            "item/text/delta",
            "item/completed",
            "usage/updated",
            "turn/completed",
        ]
    );

    // Releasing detaches the session; stopping an idle thread is fine.
    assert!(daemon.qcc(&["thread", "stop", &id, "--release"]).ok);
    assert_eq!(daemon.events(&id).last().unwrap()["type"], "session/ended");
    assert!(daemon.qcc(&["thread", "stop", &id]).ok);
}

#[test]
fn shutdown_settles_a_turn_in_flight() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path());
    let long = (0..5000).map(|n| n.to_string()).collect::<Vec<_>>().join(" ");
    let id = daemon.spawn(&format!("/slow {long}"));
    daemon.wait_for(&id, "item/text/delta");
    daemon.stop();

    let daemon = Daemon::start(dir.path());
    let events = daemon.events(&id);
    let tail = types(&events);
    assert_eq!(
        tail[tail.len() - 4..],
        ["item/completed", "turn/completed", "thread/status", "session/ended"]
    );
    assert_eq!(daemon.thread(&id)["status"], "idle");
}

#[test]
fn a_crashing_plugin_fails_its_thread_not_the_daemon() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path());
    let output =
        daemon.qcc(&["thread", "spawn", "--provider", "echo", "--prompt", "/crash", "--follow"]);
    assert!(!output.ok);
    assert!(
        output.stdout.contains("error: plugin provider-echo crashed: wasm trap"),
        "{}",
        output.stdout
    );

    let listed = daemon.qcc(&["thread", "list", "--json"]).json();
    assert_eq!(listed["threads"][0]["status"], "failed");
    assert!(daemon.qcc(&["daemon", "status"]).ok);
}

#[test]
fn errors_are_reported_with_a_failing_exit_code() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path());

    let unknown_provider = daemon.qcc(&["thread", "spawn", "--provider", "nope"]);
    assert!(!unknown_provider.ok);
    assert_eq!(unknown_provider.stderr.trim(), "qcc: no plugin provides \"nope\" (code -32002)");

    let unknown_thread = daemon.qcc(&["thread", "send", "thr_missing", "hello"]);
    assert!(!unknown_thread.ok);
    assert!(unknown_thread.stderr.contains("thread thr_missing does not exist"));

    let bad_cwd =
        daemon.qcc(&["thread", "spawn", "--provider", "echo", "--cwd", "/no/such/directory"]);
    assert!(!bad_cwd.ok);
    assert!(bad_cwd.stderr.contains("is not an absolute path to a directory"));

    let failed = daemon.qcc(&[
        "thread",
        "spawn",
        "--provider",
        "echo",
        "--prompt",
        "/fail nope",
        "--follow",
    ]);
    assert!(!failed.ok);
    assert!(failed.stdout.contains("turn failed: nope"), "{}", failed.stdout);

    let socket = dir.path().join("nobody-home.sock");
    let no_daemon = Command::new(env!("CARGO_BIN_EXE_qcc"))
        .args(["--socket", socket.to_str().unwrap(), "daemon", "status"])
        .output()
        .unwrap();
    assert!(!no_daemon.status.success());
    assert!(String::from_utf8_lossy(&no_daemon.stderr).contains("is qccd running?"));
}

#[test]
fn a_spawn_from_inside_an_agent_session_records_its_parent() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path());
    let parent = daemon.spawn("parent");

    // The daemon hands every agent session its thread id in the environment.
    let child = daemon
        .qcc_env(
            &["thread", "spawn", "--provider", "echo", "--prompt", "child", "--json"],
            &[("QCC_THREAD_ID", &parent)],
        )
        .json();
    assert_eq!(child["thread"]["parent"], parent.as_str());

    let orphan = daemon.qcc_env(
        &["thread", "spawn", "--provider", "echo", "--json"],
        &[("QCC_THREAD_ID", "thr_gone")],
    );
    assert!(!orphan.ok);
}

#[test]
fn plugins_and_daemon_are_listed() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path());

    let plugins = daemon.qcc(&["plugin", "list", "--json"]).json();
    let ids: Vec<&str> =
        plugins["plugins"].as_array().unwrap().iter().map(|p| p["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["provider-acp", "provider-claude-code", "provider-codex", "provider-echo"]);
    let echo = &plugins["plugins"][3];
    assert_eq!(echo["bundled"], true);
    assert_eq!(echo["providers"][0]["id"], "echo");
    assert_eq!(echo["providers"][0]["capabilities"]["resume"], true);
    assert_eq!(echo["permissions"]["process"][0], "echo");
    // Bundled plugins are installed like any other.
    assert!(dir.path().join("plugins/provider-echo/plugin.wasm").is_file());

    let status = daemon.qcc(&["daemon", "status", "--json"]).json();
    assert_eq!(status["data_dir"], dir.path().to_str().unwrap());

    // A second daemon on the same socket refuses to start.
    let config = qcc_daemon::Config {
        data_dir: dir.path().to_path_buf(),
        socket: daemon.socket.clone(),
        cache_dir: cache_dir(),
    };
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let second = runtime.block_on(qcc_daemon::serve(config, || {}, std::future::pending()));
    assert!(second.unwrap_err().to_string().contains("already listening"));
}

fn words(count: usize) -> String {
    (0..count).map(|n| n.to_string()).collect::<Vec<_>>().join(" ")
}

#[test]
fn a_message_sent_during_a_turn_is_queued_and_runs_after_it() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path());
    let id = daemon.spawn(&format!("/slow {}", words(60)));
    daemon.wait_for(&id, "item/text/delta");

    // `--follow` on a queued message waits for the turn that takes it, not for the
    // one that is running.
    let output = daemon.qcc(&["thread", "send", &id, "second message", "--json", "--follow"]);
    assert!(output.ok, "{}", output.stderr);
    let followed = output.json_lines();
    let kinds = types(&followed);
    assert_eq!(kinds[0], "input/queued");
    assert_eq!(kinds.iter().filter(|kind| **kind == "turn/completed").count(), 2);
    let delivered = followed.iter().position(|e| e["type"] == "input/submitted").unwrap();
    let first_done = followed.iter().position(|e| e["type"] == "turn/completed").unwrap();
    assert!(first_done < delivered, "delivered only after the running turn settled");
    assert_eq!(followed[delivered]["queue_id"], followed[0]["queue_id"]);
    assert_eq!(*messages(&followed).last().unwrap(), "second message");

    let sent = daemon.qcc(&["thread", "send", &id, "third", "--json"]).json();
    assert_eq!(sent["delivery"], "started", "an idle thread takes the message at once");
}

#[test]
fn a_message_can_be_steered_into_the_running_turn() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path());
    let id = daemon.spawn(&format!("/slow {}", words(40)));
    daemon.wait_for(&id, "item/text/delta");

    let sent = daemon.qcc(&["thread", "send", &id, "LAST WORDS", "--steer", "--json"]).json();
    assert_eq!(sent["delivery"], "steered");
    daemon.wait_for(&id, "turn/completed");
    let events = daemon.events(&id);
    assert_eq!(types(&events).iter().filter(|kind| **kind == "turn/started").count(), 1);
    assert!(messages(&events)[0].ends_with("LAST WORDS "), "{:?}", messages(&events));
    let steered = events.iter().find(|e| e["type"] == "input/submitted" && e["steered"] == true);
    assert_eq!(steered.unwrap()["text"], "LAST WORDS");
}

#[test]
fn stop_drops_the_queue_and_a_restart_keeps_it() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path());
    let stopped = daemon.spawn(&format!("/slow {}", words(3000)));
    let kept = daemon.spawn(&format!("/slow {}", words(3000)));
    for id in [&stopped, &kept] {
        daemon.wait_for(id, "item/text/delta");
        let sent = daemon.qcc(&["thread", "send", id, "run me later", "--json"]).json();
        assert_eq!(sent["delivery"], "queued");
    }
    assert_eq!(daemon.thread(&kept)["queued_inputs"], 1);

    // Stopping a thread means its queued messages are not run behind the user's back.
    assert!(daemon.qcc(&["thread", "stop", &stopped]).ok);
    let events = daemon.events(&stopped);
    assert!(types(&events).contains(&"input/dropped"));
    assert_eq!(types(&events).iter().filter(|kind| **kind == "turn/started").count(), 1);

    // A daemon restart interrupts the turn but still owes the queued message.
    daemon.stop();
    let daemon = Daemon::start(dir.path());
    daemon.wait_for(&kept, "session/started");
    let deadline = Instant::now() + Duration::from_secs(20);
    while messages(&daemon.events(&kept)).last() != Some(&"run me later") {
        assert!(Instant::now() < deadline, "{:#?}", types(&daemon.events(&kept)));
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(daemon.thread(&kept).get("queued_inputs").is_none());
    assert_eq!(types(&daemon.events(&stopped)).iter().filter(|k| **k == "turn/started").count(), 1);
}

#[test]
fn permission_mode_is_set_at_spawn_and_can_be_changed() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path());

    // In `auto` the provider approves by itself: nothing is asked.
    let output = daemon.qcc(&[
        "thread",
        "spawn",
        "--provider",
        "echo",
        "--permission-mode",
        "auto",
        "--prompt",
        "/approve make deploy",
        "--json",
        "--follow",
    ]);
    assert!(output.ok, "{}", output.stderr);
    let events = output.json_lines();
    assert!(!types(&events).contains(&"interaction/requested"));
    let id = events[0]["thread_id"].as_str().unwrap().to_owned();
    assert_eq!(daemon.thread(&id)["permission_mode"], "auto");

    // Switching to `ask` releases the session; the next message resumes it and asks.
    assert!(daemon.qcc(&["thread", "mode", &id, "ask"]).ok);
    assert_eq!(daemon.thread(&id)["permission_mode"], "ask");
    assert!(daemon.qcc(&["thread", "send", &id, "/approve make deploy"]).ok);
    daemon.wait_for(&id, "interaction/requested");
    let kinds = daemon.events(&id);
    let kinds = types(&kinds);
    let changed = kinds.iter().position(|kind| *kind == "thread/permission-mode").unwrap();
    assert!(kinds[changed..].contains(&"session/ended"));
    assert!(kinds[changed..].contains(&"session/started"));

    // Not while a turn is running.
    let busy = daemon.qcc(&["thread", "mode", &id, "full"]);
    assert!(!busy.ok);
    assert!(busy.stderr.contains("stop it before changing the permission mode"), "{}", busy.stderr);
    assert!(daemon.qcc(&["thread", "stop", &id]).ok);
    assert!(daemon.qcc(&["thread", "mode", &id, "default"]).ok);
    assert!(daemon.thread(&id).get("permission_mode").is_none());

    // A provider is never started in a mode it cannot enforce.
    let unsupported =
        daemon.qcc(&["thread", "spawn", "--provider", "opencode", "--permission-mode", "auto"]);
    assert!(!unsupported.ok);
    assert!(
        unsupported.stderr.contains(
            "cannot enforce permission mode \"auto\"; it supports: ask, accept-edits, full"
        ),
        "{}",
        unsupported.stderr
    );
    let nonsense = daemon.qcc(&["thread", "mode", &id, "yolo"]);
    assert!(!nonsense.ok);
    assert!(nonsense.stderr.contains("unknown permission mode"), "{}", nonsense.stderr);

    let plugins = daemon.qcc(&["plugin", "list", "--json"]).json();
    let codex = &plugins["plugins"][2]["providers"][0]["capabilities"];
    assert_eq!(codex["steer"], true);
    assert_eq!(
        codex["permission_modes"],
        serde_json::json!(["ask", "accept-edits", "auto", "full"])
    );
}

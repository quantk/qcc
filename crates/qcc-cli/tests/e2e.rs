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
fn projects_group_threads_by_their_folders() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(&dir.path().join("data"));
    let app = dir.path().join("app");
    let api = dir.path().join("api");
    std::fs::create_dir_all(api.join("src")).unwrap();
    std::fs::create_dir_all(&app).unwrap();
    let (app, api) = (app.to_str().unwrap(), api.to_str().unwrap());
    let spawn_in = |cwd: &str, extra: &[&str]| {
        let mut args = vec!["thread", "spawn", "--provider", "echo", "--cwd", cwd, "--json"];
        args.extend(extra);
        let output = daemon.qcc(&args);
        assert!(output.ok, "{}", output.stderr);
        output.json()["thread"]["id"].as_str().unwrap().to_owned()
    };

    // A thread from before the project joins it when the project is made.
    let early = spawn_in(app, &[]);
    assert!(daemon.thread(&early).get("project").is_none());
    let made =
        daemon.qcc(&["project", "create", "Shop", "--folder", app, "--folder", api, "--json"]);
    assert!(made.ok, "{}", made.stderr);
    let project = made.json()["project"]["id"].as_str().unwrap().to_owned();
    assert_eq!(daemon.thread(&early)["project"], project.as_str());

    // New threads in any of its folders belong to it, children follow their parent.
    let nested = spawn_in(&format!("{api}/src"), &[]);
    assert_eq!(daemon.thread(&nested)["project"], project.as_str());
    let elsewhere = dir.path().to_str().unwrap();
    let child = spawn_in(elsewhere, &["--parent", &nested]);
    assert_eq!(daemon.thread(&child)["project"], project.as_str());
    let outside = spawn_in(elsewhere, &[]);
    assert!(daemon.thread(&outside).get("project").is_none());

    // By name as well as by id; and out again.
    assert!(daemon.qcc(&["thread", "project", &outside, "Shop"]).ok);
    assert_eq!(daemon.thread(&outside)["project"], project.as_str());
    assert!(daemon.qcc(&["thread", "project", &outside, "none"]).ok);
    assert!(daemon.thread(&outside).get("project").is_none());

    assert!(daemon.qcc(&["thread", "rename", &outside, "  Billing  "]).ok);
    assert_eq!(daemon.thread(&outside)["title"], "Billing");
    assert!(!daemon.qcc(&["thread", "rename", &outside, " "]).ok);

    assert!(daemon.qcc(&["project", "rename", "Shop", "Store"]).ok);
    assert!(daemon.qcc(&["project", "remove-folder", "Store", api]).ok);
    let listed = daemon.qcc(&["project", "list", "--json"]).json();
    assert_eq!(listed["projects"][0]["name"], "Store");
    assert_eq!(listed["projects"][0]["folders"], serde_json::json!([app]));
    let missing = daemon.qcc(&["project", "add-folder", "Store", "/no/such/dir"]);
    assert!(!missing.ok);
    assert!(missing.stderr.contains("not an absolute path to a directory"), "{}", missing.stderr);

    // A thread sees the project's other folders, not the one it works in.
    assert!(daemon.qcc(&["project", "add-folder", "Store", api]).ok);
    let told = spawn_in(app, &["--prompt", "/skills"]);
    daemon.wait_for(&told, "turn/completed");
    let reply = daemon.qcc(&["thread", "show", &told]).stdout;
    assert!(reply.contains(&format!("- {api}")), "{reply}");
    assert!(!reply.contains(&format!("- {app}")), "{reply}");

    // Deleting a project leaves its threads, outside any project.
    assert!(daemon.qcc(&["project", "delete", "Store"]).ok);
    assert!(daemon.thread(&early).get("project").is_none());
    assert!(
        daemon.qcc(&["project", "list", "--json"]).json()["projects"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn settings_choose_programs_and_fill_in_defaults() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(&dir.path().join("data"));
    let fake = dir.path().join("my-echo");
    std::fs::write(&fake, "#!/bin/sh\necho \"mine: $*\"\n").unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();

    // The plugin runs `echo`; the user says which file that is.
    assert!(daemon.qcc(&["settings", "program", "echo", fake.to_str().unwrap()]).ok);
    let shown = daemon.qcc(&["settings", "show", "--json"]).json();
    let echo = shown["programs"].as_array().unwrap().iter().find(|p| p["name"] == "echo").unwrap();
    assert_eq!(echo["path"], fake.to_str().unwrap());
    assert_eq!(echo["plugins"][0], "provider-echo");
    let id = daemon.spawn("/exec echo hello");
    daemon.wait_for(&id, "turn/completed");
    let shown = daemon.qcc(&["thread", "show", &id]).stdout;
    assert!(shown.contains("mine: hello"), "{shown}");

    // Defaults of an agent fill in what a spawn leaves out.
    assert!(
        daemon
            .qcc(&["settings", "provider", "echo", "--model", "echo-2", "--permission-mode", "ask"])
            .ok
    );
    assert!(daemon.qcc(&["settings", "default-provider", "echo"]).ok);
    let output = daemon.qcc(&["thread", "spawn", "--json"]);
    assert!(output.ok, "{}", output.stderr);
    let thread = &output.json()["thread"];
    assert_eq!(thread["provider"], "echo");
    assert_eq!(thread["model"], "echo-2");
    assert_eq!(thread["permission_mode"], "ask");

    // Back to PATH, and settings that could not work are refused.
    assert!(daemon.qcc(&["settings", "program", "echo"]).ok);
    let refused = daemon.qcc(&["settings", "provider", "echo", "--env", "LD_PRELOAD=/x.so"]);
    assert!(!refused.ok);
    assert!(refused.stderr.contains("cannot be set"), "{}", refused.stderr);
    let saved: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.path().join("data/settings.json")).unwrap(),
    )
    .unwrap();
    assert!(saved.get("programs").is_none());
    assert_eq!(saved["providers"]["echo"]["model"], "echo-2");
}

#[test]
fn mcp_servers_set_up_in_qcc_reach_new_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path());
    let told = |id: &str| {
        let waited = daemon.qcc(&["thread", "wait", id, "--json"]).json();
        waited["threads"][0]["last_message"].as_str().unwrap().to_owned()
    };
    assert_eq!(told(&daemon.spawn("/mcp")), "no MCP servers");

    let added = daemon.qcc(&["mcp", "add", "docs", "--env", "TOKEN=t", "--", "npx", "-y", "docs"]);
    assert!(added.ok, "{}", added.stderr);
    let url = "https://example.com/mcp";
    assert!(daemon.qcc(&["mcp", "add", "web", "--url", url, "--header", "X-Key=k"]).ok);
    let listed = daemon.qcc(&["mcp", "list"]).stdout;
    assert!(listed.contains("docs") && listed.contains("npx -y docs"), "{listed}");
    let listed = daemon.qcc(&["mcp", "list", "--json"]).json();
    assert_eq!(listed["docs"]["transport"]["env"]["TOKEN"], "t");
    assert_eq!(listed["web"]["transport"]["headers"]["X-Key"], "k");
    assert_eq!(
        told(&daemon.spawn("/mcp")),
        "docs stdio npx -y docs\nweb http https://example.com/mcp"
    );

    // A server turned off stays in the settings but no session gets it.
    assert!(daemon.qcc(&["mcp", "disable", "docs"]).ok);
    assert!(daemon.qcc(&["mcp", "list"]).stdout.contains("(off)"));
    assert_eq!(told(&daemon.spawn("/mcp")), "web http https://example.com/mcp");
    assert!(daemon.qcc(&["mcp", "enable", "docs"]).ok);
    assert!(daemon.qcc(&["mcp", "remove", "web"]).ok);
    assert_eq!(told(&daemon.spawn("/mcp")), "docs stdio npx -y docs");

    // The agent says how each server is doing; the thread shows it.
    assert!(daemon.qcc(&["mcp", "add", "broken", "--", "/bin/fail-mcp"]).ok);
    let probe = daemon.spawn("/mcp");
    told(&probe);
    let thread = daemon.thread(&probe);
    let servers = &thread["mcp_servers"];
    assert_eq!(
        servers[0],
        serde_json::json!({"name": "broken", "state": "failed",
        "error": "echo was told to fail /bin/fail-mcp"})
    );
    assert_eq!(servers[1], serde_json::json!({"name": "docs", "state": "ready"}));
    let shown = daemon.qcc(&["thread", "show", &probe]).stdout;
    assert!(shown.contains("docs ready"), "{shown}");
    assert!(shown.contains("broken failed: echo was told to fail"), "{shown}");

    // A switch reaches the running session a moment after the call returns.
    let state_of = |thread: &str, index: usize, state: &str| {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let found = daemon.thread(thread)["mcp_servers"][index]["state"].clone();
            if found == state {
                return;
            }
            assert!(Instant::now() < deadline, "{thread}: server {index} is {found}, not {state}");
            std::thread::sleep(Duration::from_millis(20));
        }
    };

    // Only qcc's servers are switched; the agent's own are its configuration's.
    let theirs = daemon.qcc(&["thread", "mcp", &probe, "--off", "playwright"]);
    assert!(!theirs.ok);
    assert!(theirs.stderr.contains("not an MCP server set up in qcc"), "{}", theirs.stderr);

    // One thread turns a server off for itself, in its running session.
    let off = daemon.qcc(&["thread", "mcp", &probe, "--off", "docs", "--json"]);
    assert!(off.ok, "{}", off.stderr);
    assert_eq!(off.json()["mcp_disabled"], serde_json::json!(["docs"]));
    state_of(&probe, 1, "disabled");
    let shown = daemon.qcc(&["thread", "mcp", &probe]).stdout;
    assert!(shown.contains("docs off"), "{shown}");
    // Its next session starts without it; other threads and its children keep it.
    let other = daemon.spawn("/mcp");
    told(&other);
    assert_eq!(daemon.thread(&other)["mcp_servers"][1]["state"], "ready");
    let child = daemon.qcc(&["thread", "spawn", "--parent", &probe, "--prompt", "hi", "--json"]);
    assert!(child.ok, "{}", child.stderr);
    assert!(child.json()["thread"].get("mcp_disabled").is_none());
    assert!(daemon.qcc(&["thread", "stop", &probe, "--release"]).ok);
    assert!(daemon.qcc(&["thread", "send", &probe, "/mcp"]).ok);
    told(&probe);
    state_of(&probe, 1, "disabled");
    let on = daemon.qcc(&["thread", "mcp", &probe, "--on", "docs", "--json"]);
    assert!(on.ok, "{}", on.stderr);
    state_of(&probe, 1, "ready");

    // What could not work is refused.
    let refused = daemon.qcc(&["mcp", "add", "my docs", "--", "npx"]);
    assert!(!refused.ok);
    assert!(refused.stderr.contains("not an MCP server name"), "{}", refused.stderr);
    assert!(!daemon.qcc(&["mcp", "add", "docs"]).ok);
    assert!(!daemon.qcc(&["mcp", "remove", "nope"]).ok);
}

#[test]
fn a_queued_message_can_be_taken_back() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path());
    let long = (0..5000).map(|n| n.to_string()).collect::<Vec<_>>().join(" ");
    let id = daemon.spawn(&format!("/slow {long}"));
    daemon.wait_for(&id, "item/text/delta");

    let sent = daemon.qcc(&["thread", "send", &id, "later", "--json"]).json();
    assert_eq!(sent["delivery"], "queued");
    let queue_id = sent["queue_id"].as_str().unwrap().to_owned();
    assert_eq!(daemon.thread(&id)["queued_inputs"], 1);

    let taken = daemon.qcc(&["thread", "unqueue", &id, &queue_id]);
    assert!(taken.ok, "{}", taken.stderr);
    assert_eq!(taken.stdout.trim(), "later");
    assert!(daemon.thread(&id).get("queued_inputs").is_none());
    let dropped = daemon.events(&id);
    let dropped = dropped.iter().find(|event| event["type"] == "input/dropped").unwrap();
    assert_eq!(dropped["queue_id"], queue_id.as_str());

    // Gone from the queue: neither taken twice nor delivered after the turn.
    assert!(!daemon.qcc(&["thread", "unqueue", &id, &queue_id]).ok);
    assert!(daemon.qcc(&["thread", "stop", &id]).ok);
    let events = daemon.events(&id);
    assert!(
        events.iter().all(|event| event["text"] != "later" || event["type"] != "input/submitted")
    );
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
    assert_eq!(
        ids,
        [
            "env-git-worktree",
            "orchestration",
            "provider-acp",
            "provider-claude-code",
            "provider-codex",
            "provider-echo"
        ]
    );
    assert_eq!(plugins["plugins"][0]["environments"][0]["id"], "git-worktree");
    let orchestration = &plugins["plugins"][1];
    assert_eq!(orchestration["permissions"]["threads"], true);
    assert_eq!(orchestration["skills"][0], "qcc-orchestration");
    assert_eq!(orchestration["commands"][0]["id"], "handoff");
    let echo = &plugins["plugins"][5];
    assert_eq!(echo["bundled"], true);
    assert_eq!(echo["example"], true);
    assert!(plugins["plugins"][0].get("example").is_none());
    assert_eq!(echo["providers"][0]["id"], "echo");
    assert_eq!(echo["providers"][0]["capabilities"]["resume"], true);
    assert_eq!(echo["permissions"]["process"][0], "echo");
    // Bundled plugins are installed like any other.
    assert!(dir.path().join("plugins/provider-echo/plugin.wasm").is_file());

    let models = daemon.qcc(&["provider", "models", "echo", "--json"]).json();
    assert_eq!(models["models"][0]["id"], "echo-1");
    assert_eq!(models["models"][0]["is_default"], true);
    let unknown = daemon.qcc(&["provider", "models", "nobody"]);
    assert!(!unknown.ok);
    assert!(unknown.stderr.contains("no plugin provides"), "{}", unknown.stderr);

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
    // Output of the running turn may slip in before the message is queued.
    let queued = kinds.iter().position(|kind| *kind == "input/queued").expect("it was queued");
    assert_eq!(kinds.iter().filter(|kind| **kind == "turn/completed").count(), 2);
    let delivered = followed.iter().position(|e| e["type"] == "input/submitted").unwrap();
    let first_done = followed.iter().position(|e| e["type"] == "turn/completed").unwrap();
    assert!(first_done < delivered, "delivered only after the running turn settled");
    assert!(queued < first_done, "queued while the first turn was running");
    assert_eq!(followed[delivered]["queue_id"], followed[queued]["queue_id"]);
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
    let codex = plugins["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .find(|plugin| plugin["id"] == "provider-codex")
        .unwrap();
    let codex = &codex["providers"][0]["capabilities"];
    assert_eq!(codex["steer"], true);
    assert_eq!(
        codex["permission_modes"],
        serde_json::json!(["ask", "accept-edits", "auto", "full"])
    );
}

// ----- environments -----------------------------------------------------------------

/// Run git in `dir`, isolated from the configuration of whoever runs the tests.
fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .output()
        .expect("git runs");
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap()
}

fn write(path: PathBuf, contents: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

/// A repository with one commit and the given tracked files.
fn repo(dir: &Path, files: &[(&str, &str)]) -> PathBuf {
    let repo = dir.join("app");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "--quiet", "--initial-branch", "main"]);
    for (name, contents) in files {
        write(repo.join(name), contents);
    }
    git(&repo, &["add", "--all"]);
    git(&repo, &["commit", "--quiet", "--message", "initial"]);
    repo
}

#[test]
fn a_worktree_gives_its_work_back_applied_or_merged() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let repo = repo(dir.path(), &[("README.md", "hello\n"), ("src/a.txt", "one\n")]);
    // The merge commits in the worktree; the repository knows who commits.
    git(&repo, &["config", "user.name", "Test"]);
    git(&repo, &["config", "user.email", "test@example.invalid"]);
    let daemon = Daemon::start(&data);

    let spawned = |daemon: &Daemon| {
        let output = spawn_in_worktree(daemon, &repo, &["--prompt", "hi", "--follow"]);
        assert!(output.ok, "{}", output.stderr);
        let id = output.json_lines()[0]["thread_id"].as_str().unwrap().to_owned();
        let worktree = data.join("environments/git-worktree").join(&id).join("app");
        (id, worktree)
    };

    // Applied: a commit, an uncommitted edit and a new file arrive uncommitted.
    let (id, worktree) = spawned(&daemon);
    write(worktree.join("src/a.txt"), "one\ntwo\n");
    git(&worktree, &["commit", "--quiet", "--all", "--message", "two"]);
    write(worktree.join("README.md"), "hello\nworld\n");
    write(worktree.join("docs/new.md"), "new\n");
    let applied = daemon.qcc(&["thread", "deliver", &id]);
    assert!(applied.ok, "{}", applied.stderr);
    assert!(applied.stdout.contains("Applied the changes to 3 files"), "{}", applied.stdout);
    assert_eq!(std::fs::read_to_string(repo.join("src/a.txt")).unwrap(), "one\ntwo\n");
    assert_eq!(std::fs::read_to_string(repo.join("docs/new.md")).unwrap(), "new\n");
    assert_eq!(git(&repo, &["log", "--oneline"]).lines().count(), 1, "nothing was committed");
    // The worktree is as it was: its own index untouched.
    assert!(git(&worktree, &["status", "--porcelain"]).contains("?? docs/"));

    // A second time it no longer fits: refused, and nothing changes.
    let again = daemon.qcc(&["thread", "deliver", &id]);
    assert!(!again.ok);
    assert!(again.stderr.contains("do not apply"), "{}", again.stderr);
    git(&repo, &["checkout", "--quiet", "--", "."]);
    git(&repo, &["clean", "--quiet", "-fd"]);

    // Merged: uncommitted work is committed on the branch, which is merged.
    let (id, worktree) = spawned(&daemon);
    write(worktree.join("README.md"), "hello\nmerged\n");
    let merged = daemon.qcc(&["thread", "deliver", &id, "--merge"]);
    assert!(merged.ok, "{}", merged.stderr);
    assert!(merged.stdout.contains(&format!("Merged qcc/{id} into main")), "{}", merged.stdout);
    assert_eq!(std::fs::read_to_string(repo.join("README.md")).unwrap(), "hello\nmerged\n");
    assert!(git(&repo, &["log", "--oneline", "-3"]).contains(&format!("work of thread {id}")));

    // A conflict leaves the source as it was.
    let (id, worktree) = spawned(&daemon);
    write(worktree.join("README.md"), "theirs\n");
    write(repo.join("README.md"), "ours\n");
    git(&repo, &["commit", "--quiet", "--all", "--message", "ours"]);
    let conflict = daemon.qcc(&["thread", "deliver", &id, "--merge"]);
    assert!(!conflict.ok);
    assert!(conflict.stderr.contains("conflicts in: README.md"), "{}", conflict.stderr);
    assert_eq!(git(&repo, &["status", "--porcelain"]), "");

    // A thread without a worktree has nothing to give back.
    let plain = daemon.spawn("hi");
    assert!(!daemon.qcc(&["thread", "deliver", &plain]).ok);
}

#[test]
fn a_thread_moves_into_a_worktree_with_its_uncommitted_work() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let repo = repo(dir.path(), &[("README.md", "hello\n")]);
    let daemon = Daemon::start(&data);
    let output = daemon.qcc(&[
        "thread",
        "spawn",
        "--provider",
        "echo",
        "--cwd",
        repo.to_str().unwrap(),
        "--prompt",
        "hi",
        "--json",
        "--follow",
    ]);
    assert!(output.ok, "{}", output.stderr);
    let id = output.json_lines()[0]["thread_id"].as_str().unwrap().to_owned();

    // Work in the source checkout, some of it new.
    write(repo.join("README.md"), "hello\nedited\n");
    write(repo.join("notes.md"), "draft\n");

    let moved = daemon.qcc(&["thread", "isolate", &id, "--carry"]);
    assert!(moved.ok, "{}", moved.stderr);
    daemon.wait_for(&id, "environment/ready");
    let worktree = data.join("environments/git-worktree").join(&id).join("app");
    let thread = daemon.thread(&id);
    assert_eq!(thread["cwd"], worktree.to_str().unwrap());
    assert_eq!(thread["handover"], true, "the next session is told the conversation");
    assert_eq!(std::fs::read_to_string(worktree.join("README.md")).unwrap(), "hello\nedited\n");
    assert_eq!(std::fs::read_to_string(worktree.join("notes.md")).unwrap(), "draft\n");
    assert_eq!(git(&repo, &["status", "--porcelain"]), "", "the work left the source");

    // The next message starts a session in the worktree, told what came before.
    let next = daemon.qcc(&["thread", "send", &id, "go on", "--follow", "--json"]);
    assert!(next.ok, "{} {}", next.stderr, next.stdout);
    let reply = daemon.qcc(&["thread", "show", &id]).stdout;
    assert!(reply.contains("moved into a working directory of its own"), "{reply}");

    // Only once; and a directory that is no repository leaves the thread where it is.
    assert!(!daemon.qcc(&["thread", "isolate", &id]).ok);
    let loose = dir.path().join("loose");
    std::fs::create_dir_all(&loose).unwrap();
    let plain = daemon.qcc(&[
        "thread",
        "spawn",
        "--provider",
        "echo",
        "--cwd",
        loose.to_str().unwrap(),
        "--json",
    ]);
    let plain = plain.json()["thread"]["id"].as_str().unwrap().to_owned();
    assert!(daemon.qcc(&["thread", "isolate", &plain]).ok);
    daemon.wait_for(&plain, "environment/detached");
    assert!(daemon.thread(&plain).get("environment").is_none());
}

fn spawn_in_worktree(daemon: &Daemon, repo: &Path, extra: &[&str]) -> Output {
    let mut args =
        vec!["thread", "spawn", "--provider", "echo", "--env", "git-worktree", "--json", "--cwd"];
    args.push(repo.to_str().unwrap());
    args.extend(extra);
    daemon.qcc(&args)
}

#[test]
fn a_thread_gets_a_worktree_of_its_own_and_reports_what_it_changed() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let repo = repo(
        dir.path(),
        &[
            ("README.md", "hello\n"),
            ("src/a.txt", "one\ntwo\n"),
            (".gitignore", ".env\nscratch/\n"),
            (".worktreeinclude", ".env\n"),
            (
                ".qcc-env-setup.sh",
                "mkdir -p scratch && echo \"$QCC_THREAD_ID from $QCC_SOURCE_DIR\" > scratch/setup\n",
            ),
        ],
    );
    // Local, untracked, and wanted in every worktree.
    write(repo.join(".env"), "SECRET=1\n");
    let daemon = Daemon::start(&data);

    let output = spawn_in_worktree(&daemon, &repo.join("src"), &["--prompt", "hi", "--follow"]);
    assert!(output.ok, "{}", output.stderr);
    let streamed = output.json_lines();
    // The directory comes first: no agent runs before it exists.
    assert_eq!(
        types(&streamed)[..4],
        ["thread/created", "input/submitted", "environment/ready", "session/started"]
    );
    let id = streamed[0]["thread_id"].as_str().unwrap().to_owned();
    let worktree = data.join("environments/git-worktree").join(&id).join("app");

    // Started in a subdirectory, the thread works in the same one of its worktree.
    let thread = daemon.thread(&id);
    assert_eq!(thread["cwd"], worktree.join("src").to_str().unwrap());
    assert_eq!(thread["environment"]["state"], "ready");
    assert_eq!(thread["environment"]["source_dir"], repo.join("src").to_str().unwrap());
    assert_eq!(std::fs::read_to_string(worktree.join("src/a.txt")).unwrap(), "one\ntwo\n");
    assert_eq!(std::fs::read_to_string(worktree.join(".env")).unwrap(), "SECRET=1\n");
    assert_eq!(
        std::fs::read_to_string(worktree.join("scratch/setup")).unwrap(),
        format!("{id} from {}\n", repo.display())
    );
    let branch = format!("qcc/{id}");
    assert_eq!(git(&worktree, &["rev-parse", "--abbrev-ref", "HEAD"]).trim(), branch);

    // The worktree outlives the daemon that made it.
    daemon.stop();
    let daemon = Daemon::start(&data);
    assert_eq!(daemon.thread(&id)["cwd"], worktree.join("src").to_str().unwrap());

    let clean = daemon.qcc(&["thread", "changes", &id, "--json"]).json();
    assert_eq!(clean["changes"]["files"], serde_json::json!([]));
    assert_eq!(clean["changes"]["branch"], branch.as_str());

    // One committed change, one uncommitted, one deletion, one file git has not seen.
    write(worktree.join("src/a.txt"), "one\ntwo\nthree\n");
    git(&worktree, &["commit", "--quiet", "--all", "--message", "three"]);
    write(worktree.join("README.md"), "hello\nworld\n");
    std::fs::remove_file(worktree.join(".worktreeinclude")).unwrap();
    write(worktree.join("docs/new.md"), "brand new\n");

    let changes = daemon.qcc(&["thread", "changes", &id, "--json"]).json();
    assert_eq!(
        changes["changes"]["files"],
        serde_json::json!([
            { "path": ".worktreeinclude", "kind": "deleted", "additions": 0, "deletions": 1 },
            { "path": "README.md", "kind": "modified", "additions": 1, "deletions": 0 },
            { "path": "docs/new.md", "kind": "untracked" },
            { "path": "src/a.txt", "kind": "modified", "additions": 1, "deletions": 0 },
        ])
    );
    let human = daemon.qcc(&["thread", "changes", &id]);
    assert!(human.stdout.contains("M  src/a.txt  +1 -0\n"), "{}", human.stdout);
    assert!(human.stdout.contains("?  docs/new.md"), "{}", human.stdout);

    let diff = daemon.qcc(&["thread", "diff", &id]);
    assert!(diff.ok, "{}", diff.stderr);
    for expected in ["+three\n", "+world\n", "+++ b/docs/new.md\n", "+brand new\n", "-.env\n"] {
        assert!(diff.stdout.contains(expected), "no {expected:?} in:\n{}", diff.stdout);
    }
    // The patch is a real one: it applies to the checkout the thread started from.
    write(dir.path().join("thread.patch"), &diff.stdout);
    git(&repo, &["apply", "--check", dir.path().join("thread.patch").to_str().unwrap()]);

    // None of this touched the main checkout.
    assert_eq!(std::fs::read_to_string(repo.join("src/a.txt")).unwrap(), "one\ntwo\n");
    assert_eq!(git(&repo, &["status", "--porcelain"]), "");

    // Archiving would throw the uncommitted work away, so it has to be forced.
    let refused = daemon.qcc(&["thread", "archive", &id]);
    assert!(!refused.ok);
    assert!(refused.stderr.contains("uncommitted changes"), "{}", refused.stderr);
    assert!(worktree.is_dir());
    assert_eq!(daemon.thread(&id).get("archived"), None);

    let archived = daemon.qcc(&["thread", "archive", &id, "--force", "--json"]);
    assert!(archived.ok, "{}", archived.stderr);
    assert_eq!(archived.json()["thread"]["archived"], true);
    assert_eq!(archived.json()["thread"]["environment"]["state"], "removed");
    assert!(!worktree.exists());
    assert!(!worktree.parent().unwrap().exists(), "the thread's directory was left behind");
    // The commit exists nowhere else, so the branch that holds it stays.
    assert_eq!(git(&repo, &["branch", "--list", &branch]).trim(), branch);

    assert_eq!(daemon.qcc(&["thread", "list", "--json"]).json()["threads"], serde_json::json!([]));
    let all = daemon.qcc(&["thread", "list", "--archived", "--json"]).json();
    assert_eq!(all["threads"][0]["id"], id.as_str());
    let sent = daemon.qcc(&["thread", "send", &id, "anyone?"]);
    assert!(!sent.ok);
    assert!(sent.stderr.contains("is archived"), "{}", sent.stderr);
    // The timeline is still there to read.
    let kinds = types(&daemon.events(&id)).join(" ");
    assert!(kinds.ends_with("session/ended environment/removed thread/archived"), "{kinds}");
}

#[test]
fn an_untouched_worktree_is_archived_with_its_branch_and_runs_teardown() {
    let dir = tempfile::tempdir().unwrap();
    let repo = repo(
        dir.path(),
        &[
            ("a.txt", "one\n"),
            (".qcc-env-teardown.sh", "echo gone > \"$QCC_SOURCE_DIR/../torn\"\n"),
        ],
    );
    git(&repo, &["branch", "release"]);
    write(repo.join("a.txt"), "main moved on\n");
    git(&repo, &["commit", "--quiet", "--all", "--message", "more"]);
    let daemon = Daemon::start(&dir.path().join("data"));

    // No prompt: the worktree is made right away all the same, from the base asked for.
    let output = spawn_in_worktree(&daemon, &repo, &["--base", "release"]);
    assert!(output.ok, "{}", output.stderr);
    let id = output.json()["thread"]["id"].as_str().unwrap().to_owned();
    let ready = daemon.wait_for(&id, "environment/ready");
    let worktree = PathBuf::from(ready["cwd"].as_str().unwrap());
    assert_eq!(std::fs::read_to_string(worktree.join("a.txt")).unwrap(), "one\n");
    daemon.wait_for(&id, "session/started");

    let archived = daemon.qcc(&["thread", "archive", &id]);
    assert!(archived.ok, "{}", archived.stderr);
    assert!(!worktree.exists());
    assert_eq!(std::fs::read_to_string(dir.path().join("torn")).unwrap(), "gone\n");
    // Nothing was committed on it, so nothing is lost with the branch.
    assert_eq!(git(&repo, &["branch", "--list", "qcc/*"]), "");
    assert_eq!(git(&repo, &["worktree", "list", "--porcelain"]).matches("worktree ").count(), 1);
    // Archiving twice is not an error.
    assert!(daemon.qcc(&["thread", "archive", &id]).ok);
}

#[test]
fn a_thread_never_runs_in_the_source_when_its_environment_fails() {
    let dir = tempfile::tempdir().unwrap();
    let plain = dir.path().join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    let daemon = Daemon::start(&dir.path().join("data"));

    let output = spawn_in_worktree(&daemon, &plain, &["--prompt", "hi", "--follow"]);
    assert!(!output.ok);
    let events = output.json_lines();
    let kinds = types(&events);
    assert!(kinds.contains(&"environment/failed"), "{kinds:?}");
    assert!(!kinds.contains(&"session/started"), "{kinds:?}");
    let id = events[0]["thread_id"].as_str().unwrap().to_owned();
    let failed = daemon.wait_for(&id, "environment/failed");
    assert!(failed["message"].as_str().unwrap().contains("is not inside a git repository"));
    let thread = daemon.thread(&id);
    assert_eq!(thread["status"], "failed");
    assert_eq!(thread["environment"]["state"], "failed");
    let changes = daemon.qcc(&["thread", "changes", &id]);
    assert!(changes.stderr.contains("has no environment"), "{}", changes.stderr);

    // Once the directory is a repository, the next message tries again and runs.
    git(&plain, &["init", "--quiet", "--initial-branch", "main"]);
    write(plain.join("a.txt"), "one\n");
    git(&plain, &["add", "--all"]);
    git(&plain, &["commit", "--quiet", "--message", "initial"]);
    let output = daemon.qcc(&["thread", "send", &id, "again", "--json", "--follow"]);
    assert!(output.ok, "{}", output.stderr);
    assert_eq!(
        types(&output.json_lines())[..4],
        ["input/submitted", "environment/preparing", "environment/ready", "session/started"]
    );
    assert_ne!(daemon.thread(&id)["cwd"], plain.to_str().unwrap());

    // A setup script that fails leaves no worktree and no branch behind.
    let broken = repo(
        dir.path(),
        &[("a.txt", "one\n"), (".qcc-env-setup.sh", "echo no deps >&2; exit 3\n")],
    );
    let output = spawn_in_worktree(&daemon, &broken, &[]);
    let id = output.json()["thread"]["id"].as_str().unwrap().to_owned();
    let failed = daemon.wait_for(&id, "environment/failed");
    assert_eq!(failed["message"], ".qcc-env-setup.sh: no deps");
    assert_eq!(git(&broken, &["branch", "--list", "qcc/*"]), "");
    assert_eq!(git(&broken, &["worktree", "list", "--porcelain"]).matches("worktree ").count(), 1);
    // A thread whose environment was never made archives without one.
    assert!(daemon.qcc(&["thread", "archive", &id]).ok);

    // Requests that cannot mean anything are refused up front.
    let unknown = daemon.qcc(&["thread", "spawn", "--provider", "echo", "--env", "docker"]);
    assert!(unknown.stderr.contains("no plugin provides environment \"docker\""));
    let plain_thread = daemon.spawn("no environment");
    let diff = daemon.qcc(&["thread", "diff", &plain_thread]);
    assert!(!diff.ok);
    assert!(diff.stderr.contains("has no environment"), "{}", diff.stderr);
    let bad_base = spawn_in_worktree(&daemon, &broken, &["--base", "nope"]);
    let id = bad_base.json()["thread"]["id"].as_str().unwrap().to_owned();
    let failed = daemon.wait_for(&id, "environment/failed");
    assert!(failed["message"].as_str().unwrap().contains("\"nope\" does not name a commit"));
}

/// What the desktop app does: one raw connection that watches every thread and
/// receives notifications untouched.
#[test]
fn a_watching_client_sees_threads_appear_and_change() {
    use qcc_client::Client;
    use serde_json::json;

    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path());
    let before = daemon.spawn("first");
    daemon.wait_for(&before, "turn/completed");

    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let (client, mut notifications) = Client::connect_raw(&daemon.socket).await.unwrap();
        let listed = client.call_raw("thread.watch", json!({ "subscription": 7 })).await.unwrap();
        let ids: Vec<&str> = listed["threads"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, [before.as_str()]);
        assert_eq!(listed["threads"][0]["last_turn_status"], "completed");
        assert_eq!(listed["threads"][0]["activity"]["kind"], "agentMessage");

        // A thread that asks for approval: the watcher learns about the thread,
        // then about the request, with everything needed to answer it.
        let spawned = client
            .call_raw(
                "thread.spawn",
                json!({ "provider": "echo", "cwd": dir.path(), "prompt": "/approve rm -rf build" }),
            )
            .await
            .unwrap();
        let id = spawned["thread"]["id"].as_str().unwrap().to_owned();
        let pending = loop {
            let notification = tokio::time::timeout(Duration::from_secs(30), notifications.recv())
                .await
                .expect("the watcher is told about the approval")
                .expect("the connection stays open");
            assert_eq!(notification.method, "thread.updated");
            assert_eq!(notification.params["subscription"], 7);
            let thread = &notification.params["thread"];
            assert_eq!(thread["id"], id.as_str());
            if let Some(pending) = thread["pending_interactions"].as_array() {
                assert!(thread["turn_started_at"].is_u64());
                break pending[0].clone();
            }
        };
        assert_eq!(pending["request"]["type"], "approval");
        let allow = pending["request"]["options"]
            .as_array()
            .unwrap()
            .iter()
            .find(|option| option["kind"] == "allow")
            .unwrap();

        client
            .call_raw(
                "interaction.resolve",
                json!({
                    "thread_id": id,
                    "interaction_id": pending["id"],
                    "resolution": { "type": "selected", "option_id": allow["id"] },
                }),
            )
            .await
            .unwrap();
        loop {
            let notification = tokio::time::timeout(Duration::from_secs(30), notifications.recv())
                .await
                .expect("the watcher is told when the turn ends")
                .unwrap();
            let thread = &notification.params["thread"];
            if thread["last_turn_status"] == "completed" {
                assert!(thread.get("pending_interactions").is_none());
                assert!(thread.get("turn_started_at").is_none());
                break;
            }
        }

        // After unsubscribing, nothing more arrives.
        client.call_raw("thread.unsubscribe", json!({ "subscription": 7 })).await.unwrap();
        // Whatever was sent before the unsubscribe was answered is already here.
        while notifications.try_recv().is_ok() {}
        client
            .call_raw(
                "thread.spawn",
                json!({ "provider": "echo", "cwd": dir.path(), "prompt": "x" }),
            )
            .await
            .unwrap();
        let quiet = tokio::time::timeout(Duration::from_millis(400), notifications.recv()).await;
        assert!(quiet.is_err(), "an unsubscribed watcher got {quiet:?}");
    });
    daemon.stop();
}

/// A message sent while the session is being released waits for the release, then
/// starts a fresh session, instead of reaching the one going away.
#[test]
fn the_model_changes_between_turns_and_the_session_resumes() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path());
    let id = daemon.spawn("hello");
    daemon.wait_for(&id, "turn/completed");

    assert!(daemon.qcc(&["thread", "agent", &id, "echo", "--model", "echo-2"]).ok);
    let thread = daemon.thread(&id);
    assert_eq!(thread["model"], "echo-2");
    assert!(thread.get("handover").is_none(), "the same agent resumes: {thread}");

    assert!(daemon.qcc(&["thread", "send", &id, "again"]).ok);
    let deadline = Instant::now() + Duration::from_secs(20);
    let events = loop {
        let events = daemon.events(&id);
        if types(&events).iter().filter(|kind| **kind == "turn/completed").count() == 2 {
            break events;
        }
        assert!(Instant::now() < deadline, "the second turn never ran: {events:#?}");
        std::thread::sleep(Duration::from_millis(20));
    };
    let changed = events.iter().position(|event| event["type"] == "thread/agent").unwrap();
    let resumed = events[changed..].iter().find(|event| event["type"] == "session/started");
    assert_eq!(resumed.unwrap()["resumed"], true);

    let unknown = daemon.qcc(&["thread", "agent", &id, "nobody"]);
    assert!(!unknown.ok);
    assert!(unknown.stderr.contains("no plugin provides"), "{}", unknown.stderr);
}

#[test]
fn the_effort_reaches_the_session_and_changes_between_turns() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path());
    // The user's default for the agent fills in a spawn that names none.
    assert!(daemon.qcc(&["settings", "provider", "echo", "--effort", "high"]).ok);
    let id = daemon.spawn("/agent");
    daemon.wait_for(&id, "turn/completed");
    assert_eq!(daemon.thread(&id)["effort"], "high");
    assert_eq!(messages(&daemon.events(&id)), ["model default, effort high"]);

    // Another level for the same model resumes the session the new way.
    assert!(daemon.qcc(&["thread", "agent", &id, "echo", "--effort", "low"]).ok);
    assert_eq!(daemon.thread(&id)["effort"], "low");
    assert!(daemon.qcc(&["thread", "send", &id, "/agent"]).ok);
    let deadline = Instant::now() + Duration::from_secs(20);
    while messages(&daemon.events(&id)).len() < 2 {
        assert!(Instant::now() < deadline, "the second turn never ran");
        std::thread::sleep(Duration::from_millis(20));
    }
    let events = daemon.events(&id);
    assert_eq!(messages(&events)[1], "model default, effort low");
    let changed = events.iter().position(|event| event["type"] == "thread/agent").unwrap();
    let resumed = events[changed..].iter().find(|event| event["type"] == "session/started");
    assert_eq!(resumed.unwrap()["resumed"], true);

    // The models say which levels they take.
    let models = daemon.qcc(&["provider", "models", "echo"]).stdout;
    assert!(models.contains("effort: low (default), high"), "{models}");
}

#[test]
fn a_message_tells_the_agent_what_it_is_about() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path());
    let other = daemon.spawn("hello");
    daemon.wait_for(&other, "turn/completed");
    let id = daemon.spawn("first");
    daemon.wait_for(&id, "turn/completed");

    let sent = daemon.qcc(&["thread", "send", &id, "/notes", "--about", &other]);
    assert!(sent.ok, "{}", sent.stderr);
    let deadline = Instant::now() + Duration::from_secs(20);
    while messages(&daemon.events(&id)).len() < 2 {
        assert!(Instant::now() < deadline, "the second turn never ran");
        std::thread::sleep(Duration::from_millis(20));
    }
    let events = daemon.events(&id);
    let reply = messages(&events)[1];
    assert!(reply.contains(&format!("looking at qcc thread {other}")), "{reply}");
    assert!(reply.contains("\"hello\", run by echo, idle"), "{reply}");
    // The note stays with the message in the timeline, apart from its text.
    let submitted = events.iter().rfind(|event| event["type"] == "input/submitted").unwrap();
    assert_eq!(submitted["text"], "/notes");
    assert_eq!(submitted["notes"][0].as_str(), Some(reply));
    // A message without context carries no notes.
    let first = events.iter().find(|event| event["type"] == "input/submitted").unwrap();
    assert!(first.get("notes").is_none(), "{first}");
}

#[test]
fn a_message_sent_during_a_release_starts_a_new_session() {
    use qcc_client::Client;
    use serde_json::json;

    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path());
    let id = daemon.spawn("hello");
    daemon.wait_for(&id, "turn/completed");

    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let client = Client::connect(&daemon.socket).await.unwrap();
        // Both requests go out before the release can be answered: the daemon reads
        // them in order, and the provider call runs on its own.
        let (mode, sent) = tokio::join!(
            client
                .call_raw("thread.set_mode", json!({ "thread_id": id, "permission_mode": "ask" })),
            client.call_raw("thread.send", json!({ "thread_id": id, "text": "again" })),
        );
        mode.unwrap();
        assert_eq!(sent.unwrap()["delivery"], "queued");
    });

    let deadline = Instant::now() + Duration::from_secs(20);
    let events = loop {
        let events = daemon.events(&id);
        if types(&events).iter().filter(|kind| **kind == "turn/completed").count() == 2 {
            break events;
        }
        assert!(Instant::now() < deadline, "the queued message never ran: {events:#?}");
        std::thread::sleep(Duration::from_millis(20));
    };
    let kinds = types(&events);
    let changed = kinds.iter().position(|kind| *kind == "thread/permission-mode").unwrap();
    let after = &kinds[changed..];
    assert!(!after.contains(&"provider/error"), "{kinds:?}");
    let ended = after.iter().position(|kind| *kind == "session/ended").unwrap();
    let started = after.iter().position(|kind| *kind == "session/started").unwrap();
    assert!(ended < started, "{kinds:?}");
}

/// A plugin's own UI: views it contributes, actions on them, and its timeline items.
#[test]
fn a_plugin_draws_its_views_and_items_from_ui_nodes() {
    use qcc_client::Client;
    use serde_json::json;

    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path());
    let id = daemon.spawn("/card hello there");
    daemon.wait_for(&id, "turn/completed");
    let item = daemon
        .events(&id)
        .into_iter()
        .find(|event| event["type"] == "item/completed")
        .expect("the card item")["item"]
        .clone();
    assert_eq!(item["type"], "extension");
    assert_eq!(item["kind"], "provider-echo/card");

    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let client = Client::connect(&daemon.socket).await.unwrap();
        let call = |method: &'static str, params: Value| {
            let client = client.clone();
            async move { client.call_raw(method, params).await }
        };

        let contributions = call("ui.contributions", json!({})).await.unwrap();
        assert_eq!(
            contributions,
            json!({
                "views": [{
                    "plugin": "provider-echo",
                    "id": "playground",
                    "title": "Echo",
                    "placement": "threadTab",
                    "scope": "ownThreads",
                    "providers": ["echo"],
                }, {
                    "plugin": "provider-echo",
                    "id": "canvas",
                    "title": "Echo canvas",
                    "placement": "threadTab",
                    "scope": "ownThreads",
                    "web": "index.html",
                    "providers": ["echo"],
                }],
                "items": [{ "plugin": "provider-echo", "kind": "provider-echo/card" }],
            })
        );

        let view = json!({ "plugin": "provider-echo", "view": "playground", "thread_id": id });
        let presses = |tree: &Value| {
            tree["tree"]["children"][2]["items"][1]["value"].as_str().unwrap().to_owned()
        };
        let tree = call("ui.render", view.clone()).await.unwrap();
        assert_eq!(tree["tree"]["type"], "stack");
        assert_eq!(presses(&tree), "0");

        // An action changes the plugin's state and answers with the view redrawn.
        let mut press = view.clone();
        press["action"] = "press".into();
        call("ui.action", press.clone()).await.unwrap();
        assert_eq!(presses(&call("ui.action", press).await.unwrap()), "2");
        let mut note = view.clone();
        note["action"] = "note".into();
        note["value"] = "  remember this ".into();
        let tree = call("ui.action", note).await.unwrap();
        assert_eq!(tree["tree"]["children"][2]["items"][2]["value"], "remember this");
        assert_eq!(presses(&call("ui.render", view).await.unwrap()), "2");

        let drawn = call(
            "ui.render_item",
            json!({ "plugin": "provider-echo", "kind": item["kind"], "payload": item["payload"] }),
        )
        .await
        .unwrap();
        assert_eq!(drawn["tree"]["children"][1], json!({ "type": "text", "text": "hello there" }));

        // The web bundle is served file by file, and only from inside the bundle.
        let asset = |path: &'static str| {
            call("ui.asset", json!({ "plugin": "provider-echo", "path": path }))
        };
        let page = asset("index.html").await.unwrap();
        assert_eq!(page["content_type"], "text/html; charset=utf-8");
        assert!(page["data"].as_str().unwrap().len() > 100);
        assert_eq!(
            asset("app.js").await.unwrap()["content_type"],
            "text/javascript; charset=utf-8"
        );
        for path in ["../plugin.toml", "/etc/passwd", "missing.js", "", "./index.html"] {
            let error = asset(path).await.unwrap_err().to_string();
            assert!(error.contains("-32008"), "{path}: {error}");
        }
        let error = call("ui.asset", json!({ "plugin": "env-git-worktree", "path": "index.html" }))
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("has no web bundle"), "{error}");

        // The page keeps its state in the plugin through actions.
        let canvas = json!({ "plugin": "provider-echo", "view": "canvas", "thread_id": id });
        let mut save = canvas.clone();
        save["action"] = "save".into();
        save["value"] = "[[[1,2],[3,4]]]".into();
        assert_eq!(
            call("ui.action", save).await.unwrap()["tree"],
            json!({ "strokes": [[[1, 2], [3, 4]]] })
        );
        let mut load = canvas;
        load["action"] = "load".into();
        assert_eq!(call("ui.action", load).await.unwrap()["tree"]["strokes"][0][1], json!([3, 4]));

        // What a plugin does not contribute cannot be asked of it.
        for (method, params) in [
            ("ui.render", json!({ "plugin": "provider-echo", "view": "nope" })),
            ("ui.render", json!({ "plugin": "nope", "view": "playground" })),
            ("ui.render", json!({ "plugin": "env-git-worktree", "view": "playground" })),
            (
                "ui.render_item",
                json!({ "plugin": "provider-echo", "kind": "other-plugin/card", "payload": {} }),
            ),
        ] {
            let error = call(method, params.clone()).await.unwrap_err().to_string();
            assert!(error.contains("-32008"), "{params}: {error}");
        }
    });
    daemon.stop();
}

/// Poll until `check` holds for a thread's events.
fn eventually(daemon: &Daemon, thread: &str, what: &str, check: impl Fn(&[Value]) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let events = daemon.events(thread);
        if check(&events) {
            return;
        }
        assert!(Instant::now() < deadline, "never {what}: {events:#?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn inputs(events: &[Value]) -> Vec<&str> {
    events
        .iter()
        .filter(|e| e["type"] == "input/submitted")
        .map(|e| e["text"].as_str().unwrap())
        .collect()
}

#[test]
fn a_parent_waits_for_its_children_and_reads_their_answers() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path());
    let parent = daemon.spawn("parent");
    let as_agent = [("QCC_THREAD_ID", parent.as_str())];

    // From inside an agent session a child needs only a prompt: the provider is
    // the parent's.
    let child = |prompt: &str| {
        let output = daemon.qcc_env(&["thread", "spawn", "--prompt", prompt, "--json"], &as_agent);
        assert!(output.ok, "{}", output.stderr);
        assert_eq!(output.json()["thread"]["provider"], "echo");
        output.json()["thread"]["id"].as_str().unwrap().to_owned()
    };
    let slow = child("/slow one two three");
    let quick = child("second child");

    let waited = daemon.qcc(&["thread", "wait", &slow, &quick, "--json"]);
    assert!(waited.ok, "{}", waited.stderr);
    let waited = waited.json();
    assert_eq!(waited["timed_out"], Value::Null);
    let threads = waited["threads"].as_array().unwrap();
    assert_eq!(threads[0]["thread"]["id"], slow.as_str());
    assert_eq!(threads[0]["state"], "idle");
    assert_eq!(threads[0]["last_message"].as_str().unwrap().trim(), "one two three");
    assert_eq!(threads[1]["last_message"], "second child");

    // In text, each thread with its answer.
    let text = daemon.qcc(&["thread", "wait", &quick]);
    assert!(text.ok);
    assert!(text.stdout.contains("idle, last turn completed"), "{}", text.stdout);
    assert!(text.stdout.ends_with("second child\n"), "{}", text.stdout);

    let children = daemon.qcc(&["thread", "list", "--parent", &parent, "--json"]).json();
    assert_eq!(children["threads"].as_array().unwrap().len(), 2);
    let tree = daemon.qcc(&["thread", "list"]).stdout;
    let lines: Vec<&str> = tree.lines().collect();
    assert_eq!(lines.len(), 3, "{tree}");
    assert!(lines[0].starts_with(&parent), "{tree}");
    assert!(lines[1].starts_with(&format!("  {slow}")), "{tree}");

    // A thread waiting for the user has not settled: the wait runs out of time and
    // says where it stands.
    let asking = child("/ask which one?");
    daemon.wait_for(&asking, "interaction/requested");
    let timed_out = daemon.qcc(&["thread", "wait", &asking, "--timeout", "1", "--json"]);
    assert!(!timed_out.ok);
    assert_eq!(timed_out.json()["timed_out"], true);
    assert_eq!(timed_out.json()["threads"][0]["state"], "waiting");
    let text = daemon.qcc(&["thread", "wait", &asking, "--timeout", "1"]);
    assert!(text.stdout.contains("waiting for the user"), "{}", text.stdout);

    // `--any` returns as soon as one is done.
    let any = daemon.qcc(&["thread", "wait", &asking, &quick, "--any", "--json"]);
    assert!(any.ok, "{}", any.stderr);
    assert_eq!(any.json()["threads"][1]["state"], "idle");

    // A failed turn fails the wait.
    let failing = child("/fail no luck");
    let failed = daemon.qcc(&["thread", "wait", &failing]);
    assert!(!failed.ok);
    assert!(failed.stdout.contains("last turn failed"), "{}", failed.stdout);
    assert!(!daemon.qcc(&["thread", "wait", "thr_gone"]).ok);

    // Delegation nests only so deep.
    let mut current = parent.clone();
    for level in 1..=5 {
        let output = daemon.qcc(&["thread", "spawn", "--parent", &current, "--json"]);
        if level <= 4 {
            assert!(output.ok, "level {level}: {}", output.stderr);
            current = output.json()["thread"]["id"].as_str().unwrap().to_owned();
        } else {
            assert!(!output.ok);
            assert!(output.stderr.contains("may not delegate any deeper"), "{}", output.stderr);
        }
    }
}

#[test]
fn a_child_takes_after_its_parent_and_never_runs_looser() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path());
    let spawn = |args: &[&str]| daemon.qcc(&[&["thread", "spawn", "--json"], args].concat());
    let id = |output: &Output| output.json()["thread"]["id"].as_str().unwrap().to_owned();

    let parent =
        spawn(&["--provider", "echo", "--permission-mode", "accept-edits", "--model", "echo-2"]);
    assert!(parent.ok, "{}", parent.stderr);
    let parent = id(&parent);

    // Left unsaid, the mode and the model are the parent's, even over the user's
    // defaults for the agent.
    assert!(daemon.qcc(&["settings", "provider", "echo", "--permission-mode", "full"]).ok);
    let child = spawn(&["--parent", &parent]);
    assert!(child.ok, "{}", child.stderr);
    assert_eq!(child.json()["thread"]["permission_mode"], "accept-edits");
    assert_eq!(child.json()["thread"]["model"], "echo-2");
    let child = id(&child);

    // Stricter is fine; looser is refused, at spawn and later.
    let strict = spawn(&["--parent", &parent, "--permission-mode", "ask"]);
    assert!(strict.ok, "{}", strict.stderr);
    for mode in ["auto", "full"] {
        let loose = spawn(&["--parent", &parent, "--permission-mode", mode]);
        assert!(!loose.ok);
        assert!(loose.stderr.contains("may not run looser"), "{}", loose.stderr);
    }
    let loosened = daemon.qcc(&["thread", "mode", &child, "full"]);
    assert!(!loosened.ok);
    assert!(
        loosened.stderr.contains("its children may use at most accept-edits"),
        "{}",
        loosened.stderr
    );
    // The agent's own configuration may allow anything, so it counts as looser too.
    assert!(!daemon.qcc(&["thread", "mode", &child, "default"]).ok);
    assert!(daemon.qcc(&["thread", "mode", &child, "ask"]).ok);

    // A parent without a mode runs as its agent is configured; its children get
    // nothing looser than `ask`, and the user's default is held to that.
    assert!(daemon.qcc(&["settings", "provider", "echo", "--permission-mode", "default"]).ok);
    let free = spawn(&["--provider", "echo"]);
    assert!(free.ok, "{}", free.stderr);
    let free = id(&free);
    let child = spawn(&["--parent", &free]);
    assert!(child.json()["thread"].get("permission_mode").is_none());
    assert!(!spawn(&["--parent", &free, "--permission-mode", "accept-edits"]).ok);
    assert!(daemon.qcc(&["settings", "provider", "echo", "--permission-mode", "full"]).ok);
    let child = spawn(&["--parent", &free]);
    assert_eq!(child.json()["thread"]["permission_mode"], "ask");
}

#[test]
fn the_user_moves_threads_between_families() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path());
    let spawn = |args: &[&str]| {
        let output =
            daemon.qcc(&[&["thread", "spawn", "--provider", "echo", "--json"], args].concat());
        assert!(output.ok, "{}", output.stderr);
        output.json()["thread"]["id"].as_str().unwrap().to_owned()
    };
    let parent_of = |id: &str| daemon.thread(id).get("parent").cloned();
    let a = spawn(&["--permission-mode", "accept-edits"]);
    let child = spawn(&["--parent", &a]);
    let b = spawn(&["--permission-mode", "ask"]);

    // Out on its own, then under another thread, which the event log keeps.
    assert!(daemon.qcc(&["thread", "parent", &child, "none"]).ok);
    assert_eq!(parent_of(&child), None);
    assert!(daemon.qcc(&["thread", "parent", &b, &a]).ok);
    assert_eq!(parent_of(&b), Some(Value::from(a.clone())));
    assert!(types(&daemon.events(&b)).contains(&"thread/parent"));

    // Never under itself or under what it started.
    for (thread, parent) in [(&a, &a), (&a, &b)] {
        let looped = daemon.qcc(&["thread", "parent", thread, parent]);
        assert!(!looped.ok);
        assert!(looped.stderr.contains("was started under it"), "{}", looped.stderr);
    }
    // Never under a thread stricter than itself.
    let strict = spawn(&["--permission-mode", "ask"]);
    let looser = daemon.qcc(&["thread", "parent", &a, &strict]);
    assert!(!looser.ok);
    assert!(looser.stderr.contains("may not run looser"), "{}", looser.stderr);
}

#[test]
fn agents_learn_the_orchestration_skill_and_plugins_hand_off_and_review() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path());

    // Every session is told about the skill and where to read it.
    let probe = daemon.spawn("/skills");
    let told = daemon.qcc(&["thread", "wait", &probe, "--json"]).json();
    let told = told["threads"][0]["last_message"].as_str().unwrap().to_owned();
    assert!(told.contains("qcc-orchestration:"), "{told}");
    let path = told.split("(file: ").nth(1).unwrap().trim_end_matches(')');
    assert!(path.starts_with(dir.path().to_str().unwrap()), "{path}");
    let skill = std::fs::read_to_string(path).unwrap();
    assert!(skill.starts_with("---\nname: qcc-orchestration\ndescription: \""), "{skill}");
    assert!(skill.contains("qcc thread wait"), "{skill}");

    let commands = daemon.qcc(&["command", "list", "--json"]).json();
    let ids: Vec<&str> = commands["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["handoff", "review"]);
    assert!(
        daemon.qcc(&["command", "list"]).stdout.contains("qcc orchestration review --thread <id>")
    );

    let source = daemon.spawn("fix the parser bug");
    assert!(daemon.qcc(&["thread", "wait", &source]).ok);

    // Handoff: a new thread in the same directory with the conversation so far.
    let handoff = daemon.qcc(&[
        "orchestration",
        "handoff",
        "--thread",
        &source,
        "--note",
        "add a regression test",
        "--json",
    ]);
    assert!(handoff.ok, "{}", handoff.stderr);
    let next = handoff.json()["thread_id"].as_str().unwrap().to_owned();
    let next_thread = daemon.thread(&next);
    assert_eq!(next_thread["parent"], source.as_str());
    assert_eq!(next_thread["cwd"], daemon.thread(&source)["cwd"]);
    let waited = daemon.qcc(&["thread", "wait", &next, "--json"]).json();
    let echoed = waited["threads"][0]["last_message"].as_str().unwrap();
    assert!(echoed.contains("taking over a task"), "{echoed}");
    assert!(echoed.contains("fix the parser bug"), "{echoed}");
    assert!(echoed.ends_with("What to do now: add a regression test"), "{echoed}");

    // Review: a reviewer child whose findings go back to the reviewed thread.
    let review = daemon.qcc(&["orchestration", "review", "--thread", &source, "--focus=errors"]);
    assert!(review.ok, "{}", review.stderr);
    let reviewer =
        review.stdout.lines().last().unwrap().strip_prefix("thread ").unwrap().to_owned();
    assert_eq!(daemon.thread(&reviewer)["parent"], source.as_str());
    eventually(&daemon, &source, "received the review", |events| {
        inputs(events)
            .iter()
            .any(|text| text.starts_with(&format!("A reviewer (qcc thread {reviewer})")))
    });
    let reviewed = daemon.qcc(&["thread", "wait", &source, "--json"]).json();
    let findings = reviewed["threads"][0]["last_message"].as_str().unwrap();
    // The echo reviewer's findings are its prompt, echoed.
    assert!(findings.contains("Pay particular attention to: errors"), "{findings}");

    // A thread that is still working is neither handed off nor reviewed.
    let busy = daemon.spawn("/ask ready?");
    daemon.wait_for(&busy, "interaction/requested");
    let refused = daemon.qcc(&["orchestration", "review", "--thread", &busy]);
    assert!(!refused.ok);
    assert!(refused.stderr.contains("still working"), "{}", refused.stderr);

    // The daemon checks commands before the plugin sees them.
    for (args, message) in [
        (vec!["orchestration", "review"], "acts on a thread"),
        (vec!["orchestration", "review", "--thread", &source, "--bogus", "1"], "no parameter"),
        (vec!["orchestration", "nothing"], "has no command"),
        (vec!["nosuch", "thing"], "no plugin"),
        (vec!["orchestration"], "qcc <plugin> <command>"),
    ] {
        let output = daemon.qcc(&args);
        assert!(!output.ok, "{args:?}");
        assert!(output.stderr.contains(message), "{args:?}: {}", output.stderr);
    }
    // Inside an agent session the thread is the agent's own.
    let own =
        daemon.qcc_env(&["orchestration", "handoff", "--json"], &[("QCC_THREAD_ID", &source)]);
    assert!(own.ok, "{}", own.stderr);
}

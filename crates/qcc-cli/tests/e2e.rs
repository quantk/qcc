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

    /// Run `qcc` from `dir`, which is what relative paths in `args` are counted from.
    fn qcc_in(&self, dir: &Path, args: &[&str]) -> Output {
        self.run(args, &[], Some(dir))
    }

    fn qcc_env(&self, args: &[&str], env: &[(&str, &str)]) -> Output {
        self.run(args, env, None)
    }

    fn run(&self, args: &[&str], env: &[(&str, &str)], dir: Option<&Path>) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_qcc"));
        if let Some(dir) = dir {
            command.current_dir(dir);
        }
        let output = command
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
        let events = self.eventually(thread, &format!("saw a {kind} event"), |events| {
            events.iter().any(|e| e["type"] == kind)
        });
        events.into_iter().find(|e| e["type"] == kind).unwrap()
    }

    /// Poll a thread's events until `check` holds for them, and return them.
    fn eventually(&self, thread: &str, what: &str, check: impl Fn(&[Value]) -> bool) -> Vec<Value> {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let events = self.events(thread);
            if check(&events) {
                return events;
            }
            assert!(Instant::now() < deadline, "never {what}: {events:#?}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Poll a thread until the MCP servers of its session are `expected`, each as
    /// `name state`, in the order the agent reported them.
    fn mcp_servers_become(&self, thread: &str, expected: &[&str]) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let found: Vec<String> = self.thread(thread)["mcp_servers"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|server| {
                    format!(
                        "{} {}",
                        server["name"].as_str().unwrap(),
                        server["state"].as_str().unwrap()
                    )
                })
                .collect();
            if found == expected {
                return;
            }
            assert!(Instant::now() < deadline, "{thread}: MCP servers {found:?}, not {expected:?}");
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
fn a_project_icon_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let folder = dir.path().to_str().unwrap();
    let daemon = Daemon::start(&data);
    let made = daemon.qcc(&[
        "project",
        "create",
        "Games",
        "--folder",
        folder,
        "--icon",
        "gamepad-2",
        "--color",
        "violet",
        "--json",
    ]);
    assert!(made.ok, "{}", made.stderr);
    let icon = serde_json::json!({"kind": "lucide", "name": "gamepad-2", "color": "violet"});
    assert_eq!(made.json()["project"]["icon"], icon);
    daemon.stop();

    let daemon = Daemon::start(&data);
    let listed = daemon.qcc(&["project", "list", "--json"]).json();
    assert_eq!(listed["projects"][0]["icon"], icon);
    let text = daemon.qcc(&["project", "list"]).stdout;
    assert!(text.contains("Games  (gamepad-2, violet)"), "{text}");
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

    // An icon: set, kept by other changes, replaced, refused when malformed, cleared.
    assert!(listed["projects"][0].get("icon").is_none());
    let icon = |name: &str| {
        let listed = daemon.qcc(&["project", "list", "--json"]).json();
        let project = listed["projects"].as_array().unwrap().iter().find(|p| p["name"] == name);
        project.unwrap().get("icon").cloned()
    };
    assert!(daemon.qcc(&["project", "icon", "Store", "rocket", "--color", "teal"]).ok);
    assert_eq!(
        icon("Store"),
        Some(serde_json::json!({"kind": "lucide", "name": "rocket", "color": "teal"}))
    );
    assert!(daemon.qcc(&["project", "rename", "Store", "Shop"]).ok);
    assert_eq!(icon("Shop").unwrap()["name"], "rocket");
    assert!(daemon.qcc(&["project", "rename", "Shop", "Store"]).ok);
    assert!(daemon.qcc(&["project", "icon", "Store", "🛒"]).ok);
    assert_eq!(icon("Store"), Some(serde_json::json!({"kind": "emoji", "emoji": "🛒"})));
    let bad = daemon.qcc(&["project", "icon", "Store", "two words"]);
    assert!(!bad.ok);
    assert!(bad.stderr.contains("spaces"), "{}", bad.stderr);
    let bad = daemon.qcc(&["project", "icon", "Store", "🛒", "--color", "red"]);
    assert!(!bad.ok);
    let bad = daemon.qcc(&["project", "icon", "Store", "rocket", "--color", "plaid"]);
    assert!(!bad.ok);
    assert_eq!(icon("Store").unwrap()["emoji"], "🛒");
    assert!(daemon.qcc(&["project", "icon", "Store", "none"]).ok);
    assert_eq!(icon("Store"), None);

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

    // Only qcc's servers are switched; the agent's own are its configuration's.
    let theirs = daemon.qcc(&["thread", "mcp", &probe, "--off", "playwright"]);
    assert!(!theirs.ok);
    assert!(theirs.stderr.contains("not an MCP server set up in qcc"), "{}", theirs.stderr);

    // One thread turns a server off for itself: its running session drops it.
    let off = daemon.qcc(&["thread", "mcp", &probe, "--off", "docs", "--json"]);
    assert!(off.ok, "{}", off.stderr);
    assert_eq!(off.json()["mcp_disabled"], serde_json::json!(["docs"]));
    daemon.mcp_servers_become(&probe, &["broken failed"]);
    let shown = daemon.qcc(&["thread", "mcp", &probe]).stdout;
    assert!(shown.contains("docs off"), "{shown}");
    // Its next session starts without it; other threads and its children keep it.
    let other = daemon.spawn("/mcp");
    told(&other);
    daemon.mcp_servers_become(&other, &["broken failed", "docs ready"]);
    let child = daemon.qcc(&["thread", "spawn", "--parent", &probe, "--prompt", "hi", "--json"]);
    assert!(child.ok, "{}", child.stderr);
    assert!(child.json()["thread"].get("mcp_disabled").is_none());
    assert!(daemon.qcc(&["thread", "stop", &probe, "--release"]).ok);
    assert!(daemon.qcc(&["thread", "send", &probe, "/mcp"]).ok);
    assert_eq!(told(&probe), "broken stdio /bin/fail-mcp ");
    daemon.mcp_servers_become(&probe, &["broken failed"]);
    let on = daemon.qcc(&["thread", "mcp", &probe, "--on", "docs", "--json"]);
    assert!(on.ok, "{}", on.stderr);
    daemon.mcp_servers_become(&probe, &["broken failed", "docs ready"]);

    // What could not work is refused.
    let refused = daemon.qcc(&["mcp", "add", "my docs", "--", "npx"]);
    assert!(!refused.ok);
    assert!(refused.stderr.contains("not an MCP server name"), "{}", refused.stderr);
    assert!(!daemon.qcc(&["mcp", "add", "docs"]).ok);
    assert!(!daemon.qcc(&["mcp", "remove", "nope"]).ok);
}

#[test]
fn mcp_settings_reach_running_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path());
    let told = |id: &str| {
        let waited = daemon.qcc(&["thread", "wait", id, "--json"]).json();
        waited["threads"][0]["last_message"].as_str().unwrap().to_owned()
    };
    let send = |id: &str, text: &str| {
        let sent = daemon.qcc(&["thread", "send", id, text]);
        assert!(sent.ok, "{}", sent.stderr);
    };
    let mcp = |args: &[&str]| {
        let mut all = vec!["mcp"];
        all.extend(args);
        let output = daemon.qcc(&all);
        assert!(output.ok, "{}", output.stderr);
    };
    let first = daemon.spawn("/mcp");
    let second = daemon.spawn("/mcp");
    assert_eq!(told(&first), "no MCP servers");
    told(&second);

    // A server added in the settings joins the running sessions at once.
    mcp(&["add", "docs", "--", "npx", "-y", "docs"]);
    daemon.mcp_servers_become(&first, &["docs ready"]);
    daemon.mcp_servers_become(&second, &["docs ready"]);
    send(&first, "/mcp");
    assert_eq!(told(&first), "docs stdio npx -y docs");

    // Each thread keeps its own choice: one kept off stays off whatever the settings do.
    assert!(daemon.qcc(&["thread", "mcp", &second, "--off", "docs"]).ok);
    daemon.mcp_servers_become(&second, &[]);
    mcp(&["add", "web", "--url", "https://example.com/mcp"]);
    daemon.mcp_servers_become(&first, &["docs ready", "web ready"]);
    daemon.mcp_servers_become(&second, &["web ready"]);

    // Turned off for every thread, it leaves every session; turned on, it is back
    // where the thread does not keep it off.
    mcp(&["disable", "docs"]);
    daemon.mcp_servers_become(&first, &["web ready"]);
    mcp(&["enable", "docs"]);
    daemon.mcp_servers_become(&first, &["web ready", "docs ready"]);
    daemon.mcp_servers_become(&second, &["web ready"]);

    // A server set up anew is connected again with its new configuration, even
    // while a turn runs.
    send(&first, &format!("/slow {}", words(40)));
    daemon.wait_for(&first, "turn/started");
    mcp(&["add", "web", "--url", "https://example.com/fail"]);
    daemon.mcp_servers_become(&first, &["web failed", "docs ready"]);
    assert!(daemon.thread(&first)["turn_started_at"].is_u64(), "the turn goes on");
    assert!(daemon.qcc(&["thread", "stop", &first]).ok);

    // Removed, it is gone from every session.
    mcp(&["remove", "web"]);
    daemon.mcp_servers_become(&first, &["docs ready"]);
    daemon.mcp_servers_become(&second, &[]);
    send(&first, "/mcp");
    assert_eq!(told(&first), "docs stdio npx -y docs");
    send(&second, "/mcp");
    assert_eq!(told(&second), "no MCP servers");
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
fn an_agent_starts_threads_of_the_users_own_and_lists_them_by_state() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path());
    // A strict thread, as an assistant might run.
    let output = daemon.qcc(&[
        "thread",
        "spawn",
        "--provider",
        "echo",
        "--permission-mode",
        "ask",
        "--hidden",
        "--json",
    ]);
    let helper = output.json()["thread"]["id"].as_str().unwrap().to_owned();

    // From inside its session, a thread of the user's own: under no thread and not
    // held to its mode, but recorded as its doing.
    let spawned = daemon.qcc_env(
        &[
            "thread",
            "spawn",
            "--provider",
            "echo",
            "--root",
            "--permission-mode",
            "full",
            "--prompt",
            "/ask ready?",
            "--json",
        ],
        &[("QCC_THREAD_ID", &helper)],
    );
    assert!(spawned.ok, "{}", spawned.stderr);
    let thread = &spawned.json()["thread"];
    let id = thread["id"].as_str().unwrap().to_owned();
    assert!(thread.get("parent").is_none(), "{thread}");
    assert_eq!(thread["started_by"], helper.as_str());
    assert!(
        daemon
            .qcc(&["thread", "show", &id])
            .stdout
            .contains(&format!("started   by thread {helper}"))
    );
    daemon.wait_for(&id, "interaction/requested");

    // The hidden thread is left out unless asked for; a state picks threads out.
    let ids = |args: &[&str]| -> Vec<String> {
        let mut all = vec!["thread", "list", "--json"];
        all.extend_from_slice(args);
        let listed = daemon.qcc(&all).json();
        listed["threads"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["id"].as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(ids(&[]), std::slice::from_ref(&id));
    assert_eq!(ids(&["--hidden"]), [helper.clone(), id.clone()]);
    assert_eq!(ids(&["--state", "waiting"]), std::slice::from_ref(&id));
    assert_eq!(ids(&["--state", "idle", "--hidden"]), std::slice::from_ref(&helper));
    let unknown = daemon.qcc(&["thread", "list", "--state", "sleepy"]);
    assert!(!unknown.ok && unknown.stderr.contains("unknown state"), "{}", unknown.stderr);

    // The last lines of a timeline, for a reader that should not take all of it.
    let shown = daemon.qcc(&["thread", "show", &id, "--last", "1"]).stdout;
    assert!(shown.contains("earlier lines left out"), "{shown}");
    assert!(!shown.contains("> /ask ready?"), "{shown}");
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
            "provider-echo",
            "reviewers"
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

#[test]
fn a_plugin_is_turned_off_and_on_without_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path());
    let echo = |daemon: &Daemon| {
        let plugins = daemon.qcc(&["plugin", "list", "--json"]).json();
        plugins["plugins"].as_array().unwrap().iter().find(|p| p["id"] == "provider-echo").cloned()
    };
    let old = daemon.spawn("before");
    daemon.wait_for(&old, "turn/completed");

    // A working agent keeps its plugin on, unless forced.
    let busy = daemon.spawn(&format!("/slow {}", words(5000)));
    daemon.wait_for(&busy, "item/text/delta");
    let refused = daemon.qcc(&["plugin", "disable", "provider-echo"]);
    assert!(!refused.ok);
    assert!(refused.stderr.contains(&busy), "{}", refused.stderr);
    assert!(refused.stderr.contains("(code -32005)"), "{}", refused.stderr);
    let forced = daemon.qcc(&["plugin", "disable", "provider-echo", "--force"]);
    assert!(forced.ok, "{}", forced.stderr);
    assert!(forced.stdout.contains("provider-echo 0.1.0  disabled"), "{}", forced.stdout);
    let events = daemon.events(&busy);
    let turn = events.iter().find(|e| e["type"] == "turn/completed").expect("turn settled");
    assert_eq!(turn["status"], "interrupted");
    let ended = events.last().unwrap();
    assert_eq!(ended["type"], "session/ended");
    assert_eq!(ended["reason"], "plugin provider-echo was turned off");

    // Off: listed with what its manifest says, contributing nothing.
    let off = echo(&daemon).unwrap();
    assert_eq!(off["enabled"], false);
    assert_eq!(off["version"], "0.1.0");
    assert!(off["description"].as_str().unwrap().contains("test agent"));
    assert_eq!(off["permissions"]["process"][0], "echo");
    assert_eq!(off["providers"].as_array().unwrap().len(), 0);
    let commands = daemon.qcc(&["command", "list", "--json"]).json();
    assert!(!commands.to_string().contains("provider-echo"), "{commands}");

    // New threads cannot use it; old ones are still read, and say why they do not run.
    let spawned = daemon.qcc(&["thread", "spawn", "--provider", "echo", "--prompt", "hi"]);
    assert!(!spawned.ok);
    assert!(
        spawned.stderr.contains("plugin provider-echo, which is turned off"),
        "{}",
        spawned.stderr
    );
    assert!(spawned.stderr.contains("(code -32009)"), "{}", spawned.stderr);
    assert!(messages(&daemon.events(&old)).contains(&"before"));
    let sent = daemon.qcc(&["thread", "send", &old, "again"]);
    assert!(!sent.ok);
    assert!(sent.stderr.contains("qcc plugin enable provider-echo"), "{}", sent.stderr);

    // Off survives a restart.
    daemon.stop();
    let daemon = Daemon::start(dir.path());
    assert_eq!(echo(&daemon).unwrap()["enabled"], false);
    let settings: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.path().join("settings.json")).unwrap())
            .unwrap();
    assert_eq!(settings["disabled_plugins"][0], "provider-echo");
    // Saving the other settings does not turn it back on.
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let client = qcc_client::Client::connect(&daemon.socket).await.unwrap();
        let params = serde_json::json!({ "settings": {} });
        let saved = client.call_raw("settings.set", params).await.unwrap();
        assert_eq!(saved["settings"]["disabled_plugins"][0], "provider-echo");
    });
    assert_eq!(echo(&daemon).unwrap()["enabled"], false);

    // On again: it works, for new threads and old ones.
    let enabled = daemon.qcc(&["plugin", "enable", "provider-echo"]);
    assert!(enabled.ok, "{}", enabled.stderr);
    assert_eq!(echo(&daemon).unwrap()["providers"][0]["id"], "echo");
    let id = daemon.spawn("after");
    daemon.wait_for(&id, "turn/completed");
    let output = daemon.qcc(&["thread", "send", &old, "again", "--json", "--follow"]);
    assert!(output.ok, "{}", output.stderr);
    assert!(messages(&daemon.events(&old)).contains(&"again"));

    // An environment goes away with its plugin too.
    assert!(daemon.qcc(&["plugin", "disable", "env-git-worktree"]).ok);
    let source = repo(&dir.path().join("src"), &[("a.txt", "a\n")]);
    let isolated = spawn_in_worktree(&daemon, &source, &[]);
    assert!(!isolated.ok);
    assert!(
        isolated.stderr.contains("plugin env-git-worktree, which is turned off"),
        "{}",
        isolated.stderr
    );
    assert!(daemon.qcc(&["plugin", "enable", "env-git-worktree"]).ok);
    assert!(spawn_in_worktree(&daemon, &source, &[]).ok);

    let unknown = daemon.qcc(&["plugin", "disable", "nope"]);
    assert!(!unknown.ok);
    assert!(unknown.stderr.contains("no plugin \"nope\" is installed"), "{}", unknown.stderr);
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
    daemon.eventually(&kept, "ran the queued message", |events| {
        messages(events).last() == Some(&"run me later")
    });
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

/// Start a thread in a worktree at `place` (a path as the user types it, counted
/// from `from`); returns the id of the thread.
fn spawn_placed(daemon: &Daemon, from: &Path, repo: &Path, place: &str, extra: &[&str]) -> String {
    let mut args =
        vec!["thread", "spawn", "--provider", "echo", "--env", "git-worktree", "--json", "--cwd"];
    args.extend([repo.to_str().unwrap(), "--env-dir", place]);
    args.extend(extra);
    let output = daemon.qcc_in(from, &args);
    assert!(output.ok, "{}", output.stderr);
    output.json()["thread"]["id"].as_str().unwrap().to_owned()
}

/// Why the environment of `thread` could not be made.
fn refusal(daemon: &Daemon, thread: &str) -> String {
    daemon.wait_for(thread, "environment/failed")["message"].as_str().unwrap().to_owned()
}

/// The worktrees and the branches of `repo`, to tell that a refusal left it alone.
fn worktrees_and_branches(repo: &Path) -> (usize, String) {
    let worktrees = git(repo, &["worktree", "list", "--porcelain"]).matches("worktree ").count();
    (worktrees, git(repo, &["branch", "--format", "%(refname:short)"]))
}

#[test]
fn worktrees_of_one_task_gather_in_a_folder_on_a_branch_of_the_users_naming() {
    let dir = tempfile::tempdir().unwrap();
    // Spelled as the commands see the directory they run in.
    let root = dir.path().canonicalize().unwrap();
    // The user's root directory is a checkout itself: it holds the clones and
    // ignores them and the folder of tasks.
    let dev = root.join("dev");
    write(dev.join(".gitignore"), "BGT/\ntasks/\n");
    git(&dev, &["init", "--quiet", "--initial-branch", "main"]);
    git(&dev, &["add", "--all"]);
    git(&dev, &["commit", "--quiet", "--message", "root"]);
    let auth = repo(&dev.join("BGT/auth"), &[("README.md", "hello\n"), ("src/a.txt", "one\n")]);
    let front = repo(&dev.join("BGT/front"), &[("index.html", "<p>\n")]);
    for repo in [&auth, &front] {
        git(repo, &["config", "user.name", "Test"]);
        git(repo, &["config", "user.email", "test@example.invalid"]);
    }
    let daemon = Daemon::start(&root.join("data"));
    let task = dev.join("tasks/BGT-123");
    let branch = ["--branch", "feature/BGT-123"];

    // Two repositories, one folder, one branch name. The directory is counted from
    // where the command runs, and `..` in it is taken out.
    let first = spawn_placed(&daemon, &dev, &auth, "tasks/BGT-123/auth", &branch);
    let ready = daemon.wait_for(&first, "environment/ready");
    let worktree = task.join("auth");
    assert_eq!(ready["cwd"], worktree.to_str().unwrap());
    let second = spawn_placed(&daemon, &task, &front, "../BGT-123/./front", &branch);
    daemon.wait_for(&second, "environment/ready");
    assert_eq!(daemon.thread(&second)["cwd"], task.join("front").to_str().unwrap());
    let thread = daemon.thread(&first);
    assert_eq!(thread["environment"]["dir"], worktree.to_str().unwrap());
    assert_eq!(thread["environment"]["branch"], "feature/BGT-123");
    for worktree in [&worktree, &task.join("front")] {
        assert_eq!(git(worktree, &["branch", "--show-current"]).trim(), "feature/BGT-123");
    }
    // The checkout around them sees none of it.
    assert_eq!(git(&dev, &["status", "--porcelain"]), "");

    // Changes are counted from where the branch started, committed or not.
    write(worktree.join("src/a.txt"), "one\ntwo\n");
    git(&worktree, &["commit", "--quiet", "--all", "--message", "two"]);
    write(worktree.join("README.md"), "hello\nworld\n");
    let changes = daemon.qcc(&["thread", "changes", &first, "--json"]).json();
    assert_eq!(changes["changes"]["branch"], "feature/BGT-123");
    assert_eq!(
        changes["changes"]["files"],
        serde_json::json!([
            { "path": "README.md", "kind": "modified", "additions": 1, "deletions": 0 },
            { "path": "src/a.txt", "kind": "modified", "additions": 1, "deletions": 0 },
        ])
    );
    let diff = daemon.qcc(&["thread", "diff", &first]);
    assert!(diff.stdout.contains("+two\n") && diff.stdout.contains("+world\n"), "{}", diff.stdout);

    // Delivered as uncommitted changes, like the work of any worktree.
    let applied = daemon.qcc(&["thread", "deliver", &first]);
    assert!(applied.ok, "{}", applied.stderr);
    assert!(applied.stdout.contains("Applied the changes to 2 files"), "{}", applied.stdout);
    assert_eq!(std::fs::read_to_string(auth.join("src/a.txt")).unwrap(), "one\ntwo\n");
    git(&auth, &["checkout", "--quiet", "--", "."]);

    // Archiving takes the worktree and keeps the branch: it has a commit, and may
    // be pushed yet. The folder of the task stays while the other worktree is in it.
    let refused = daemon.qcc(&["thread", "archive", &first]);
    assert!(refused.stderr.contains("uncommitted change"), "{}", refused.stderr);
    git(&worktree, &["commit", "--quiet", "--all", "--message", "world"]);
    let tip = git(&worktree, &["rev-parse", "HEAD"]);
    let archived = daemon.qcc(&["thread", "archive", &first]);
    assert!(archived.ok, "{}", archived.stderr);
    assert!(!worktree.exists() && task.is_dir());
    assert_eq!(git(&auth, &["rev-parse", "feature/BGT-123"]), tip);

    // Merged from the other worktree; then the branch holds a commit and stays too.
    write(task.join("front/index.html"), "<p>\n<b>\n");
    let merged = daemon.qcc(&["thread", "deliver", &second, "--merge"]);
    assert!(merged.ok, "{}", merged.stderr);
    assert!(merged.stdout.contains("Merged feature/BGT-123 into main"), "{}", merged.stdout);
    assert_eq!(std::fs::read_to_string(front.join("index.html")).unwrap(), "<p>\n<b>\n");
    assert!(daemon.qcc(&["thread", "archive", &second]).ok);
    assert!(git(&front, &["branch", "--list", "feature/*"]).contains("feature/BGT-123"));
    // The folder of the task was made for these worktrees and goes with the last.
    assert!(!task.exists(), "the emptied folder of the task was left behind");
    assert!(dev.join("tasks").is_dir(), "only the folder made for the worktrees goes");

    // The task goes on later. A branch that is there is not taken over unasked...
    let unasked = spawn_placed(&daemon, &dev, &auth, "tasks/BGT-123/auth", &branch);
    let why = refusal(&daemon, &unasked);
    assert!(why.contains("branch feature/BGT-123 exists already"), "{why}");
    assert!(why.contains("--reuse-branch"), "{why}");
    assert!(!task.exists(), "a refusal left a directory behind");
    assert!(daemon.qcc(&["thread", "archive", &unasked]).ok);
    // ...but continued when asked: the worktree starts where the branch stands, and
    // the changes of the thread are counted from there.
    let reuse = ["--branch", "feature/BGT-123", "--reuse-branch"];
    let again = spawn_placed(&daemon, &dev, &auth, "tasks/BGT-123/auth", &reuse);
    daemon.wait_for(&again, "environment/ready");
    assert_eq!(git(&worktree, &["rev-parse", "HEAD"]), tip);
    let changes = daemon.qcc(&["thread", "changes", &again, "--json"]).json();
    assert_eq!(changes["changes"]["files"], serde_json::json!([]));
    assert_eq!(changes["changes"]["base"], tip.trim());
    // A branch that was there before the worktree is never deleted with it.
    assert!(daemon.qcc(&["thread", "archive", &again]).ok);
    assert_eq!(git(&auth, &["rev-parse", "feature/BGT-123"]), tip);
    assert_eq!(worktrees_and_branches(&auth).0, 1);
}

#[test]
fn a_place_that_is_not_free_is_refused_and_nothing_is_touched() {
    let dir = tempfile::tempdir().unwrap();
    // Spelled as the commands see the directory they run in.
    let root = dir.path().canonicalize().unwrap();
    let repo = repo(&root, &[("a.txt", "one\n")]);
    git(&repo, &["branch", "taken"]);
    let daemon = Daemon::start(&root.join("data"));
    let before = worktrees_and_branches(&repo);
    let refused = |place: &str, extra: &[&str]| {
        let id = spawn_placed(&daemon, &root, &repo, place, extra);
        let why = refusal(&daemon, &id);
        assert!(daemon.qcc(&["thread", "archive", &id]).ok);
        why
    };

    // A directory with something in it, a file, and a checkout nobody told qcc about.
    write(root.join("full/notes.md"), "mine\n");
    let why = refused("full", &[]);
    assert!(why.contains("full exists and is not empty"), "{why}");
    assert_eq!(std::fs::read_to_string(root.join("full/notes.md")).unwrap(), "mine\n");
    let why = refused("full/notes.md", &[]);
    assert!(why.contains("exists and is not a directory"), "{why}");
    let why = refused("app", &[]);
    assert!(why.contains("is a git checkout already"), "{why}");

    // Inside a checkout that does not ignore it, the worktree would show up there
    // as untracked files. Ignored, it is fine.
    let why = refused("app/nested", &["--branch", "nested"]);
    assert!(why.contains("which does not ignore it"), "{why}");
    assert!(!repo.join("nested").exists());
    write(repo.join(".git/info/exclude"), "nested/\n");
    let nested = spawn_placed(&daemon, &root, &repo, "app/nested", &["--branch", "nested"]);
    daemon.wait_for(&nested, "environment/ready");
    assert_eq!(git(&repo, &["status", "--porcelain"]), "");
    // Nothing was done on the branch, so it goes with its worktree.
    assert!(daemon.qcc(&["thread", "archive", &nested]).ok);
    assert!(!repo.join("nested").exists());

    // A branch that exists, a name qcc keeps for itself, a name git would not take.
    let why = refused("wt", &["--branch", "taken"]);
    assert!(why.contains("branch taken exists already"), "{why}");
    let why = refused("wt", &["--branch", "qcc/mine"]);
    assert!(why.contains("branch names under qcc/"), "{why}");
    let why = refused("wt", &["--branch", "no..good"]);
    assert!(why.contains("no..good is not a valid branch name"), "{why}");
    // The branch checked out in the source cannot be worked on in two places.
    let why = refused("wt", &["--branch", "main", "--reuse-branch"]);
    assert!(why.contains("branch main is checked out in"), "{why}");
    assert!(!root.join("wt").exists());
    assert_eq!(worktrees_and_branches(&repo), before, "a refusal changed the repository");

    // Two threads cannot have one directory, nor one branch of one repository.
    let held = spawn_placed(&daemon, &root, &repo, "wt", &["--branch", "topic"]);
    daemon.wait_for(&held, "environment/ready");
    let same_dir = daemon.qcc(&[
        "thread",
        "spawn",
        "--provider",
        "echo",
        "--env",
        "git-worktree",
        "--cwd",
        repo.to_str().unwrap(),
        "--env-dir",
        root.join("wt").to_str().unwrap(),
    ]);
    assert!(!same_dir.ok);
    assert!(
        same_dir.stderr.contains(&format!("thread {held} already has its environment in")),
        "{}",
        same_dir.stderr
    );
    let why = refused("other", &["--branch", "topic", "--reuse-branch"]);
    assert!(why.contains("branch topic is checked out in"), "{why}");
    assert!(root.join("wt/a.txt").is_file(), "the thread that has the place lost it");

    // The flags mean something only to an environment.
    let bare = daemon.qcc(&["thread", "spawn", "--provider", "echo", "--env-dir", "x"]);
    assert!(!bare.ok && bare.stderr.contains("--env"), "{}", bare.stderr);
    let unnamed = spawn_in_worktree(&daemon, &repo, &["--reuse-branch"]);
    assert!(!unnamed.ok && unnamed.stderr.contains("--branch"), "{}", unnamed.stderr);
}

#[test]
fn a_folder_of_the_users_own_and_a_branch_from_a_remote_are_theirs_to_keep() {
    let dir = tempfile::tempdir().unwrap();
    // Spelled as the commands see the directory they run in.
    let root = dir.path().canonicalize().unwrap();
    let repo = repo(&root, &[("a.txt", "one\n")]);
    let daemon = Daemon::start(&root.join("data"));

    // An empty directory is taken as it is. Its folder was the user's before, so
    // it stays when the worktree goes, and the branch nobody committed on goes.
    let place = root.join("tasks/T-1/app");
    std::fs::create_dir_all(&place).unwrap();
    let id = spawn_placed(&daemon, &root, &repo, "tasks/T-1/app", &["--branch", "topic"]);
    daemon.wait_for(&id, "environment/ready");
    assert_eq!(git(&place, &["branch", "--show-current"]).trim(), "topic");
    assert!(daemon.qcc(&["thread", "archive", &id]).ok);
    assert!(!place.exists());
    assert!(place.parent().unwrap().is_dir(), "a folder the user made was removed");
    assert_eq!(git(&repo, &["branch", "--list", "topic"]), "");

    // Without a name the branch is qcc's, wherever the worktree is.
    let id = spawn_placed(&daemon, &root, &repo, "tasks/T-1/app", &[]);
    daemon.wait_for(&id, "environment/ready");
    assert_eq!(git(&place, &["branch", "--show-current"]).trim(), format!("qcc/{id}"));
    assert!(daemon.qcc(&["thread", "archive", &id]).ok);
    assert_eq!(git(&repo, &["branch", "--list", "qcc/*"]), "");

    // A colleague's branch, known only as fetched from the remote.
    let remote = root.join("remote.git");
    git(&root, &["clone", "--quiet", "--bare", "app", "remote.git"]);
    git(&repo, &["remote", "add", "origin", remote.to_str().unwrap()]);
    git(&repo, &["checkout", "--quiet", "-b", "feature/shared"]);
    write(repo.join("a.txt"), "one\ntheirs\n");
    git(&repo, &["commit", "--quiet", "--all", "--message", "theirs"]);
    git(&repo, &["push", "--quiet", "origin", "feature/shared"]);
    git(&repo, &["checkout", "--quiet", "main"]);
    git(&repo, &["branch", "--quiet", "-D", "feature/shared"]);

    // A new branch of that name would part ways with it: refused unless asked.
    let branch = ["--branch", "feature/shared"];
    let unasked = spawn_placed(&daemon, &root, &repo, "tasks/T-1/app", &branch);
    let why = refusal(&daemon, &unasked);
    assert!(why.contains("exists already on the remote origin"), "{why}");
    assert_eq!(git(&repo, &["branch", "--list", "feature/*"]), "");
    assert!(daemon.qcc(&["thread", "archive", &unasked]).ok);

    // Asked, the thread goes on from the remote branch and follows it.
    let reuse = ["--branch", "feature/shared", "--reuse-branch", "--base", "main"];
    let id = spawn_placed(&daemon, &root, &repo, "tasks/T-1/app", &reuse);
    daemon.wait_for(&id, "environment/ready");
    assert_eq!(std::fs::read_to_string(place.join("a.txt")).unwrap(), "one\ntheirs\n");
    let upstream = git(&place, &["rev-parse", "--abbrev-ref", "feature/shared@{upstream}"]);
    assert_eq!(upstream.trim(), "origin/feature/shared");
    // With a base the changes are those of the whole branch since it left the base.
    let changes = daemon.qcc(&["thread", "changes", &id, "--json"]).json();
    assert_eq!(changes["changes"]["files"][0]["path"], "a.txt");
    assert_eq!(changes["changes"]["base"], git(&repo, &["rev-parse", "main"]).trim());

    // A commit that was never pushed outlives the worktree.
    write(place.join("a.txt"), "one\ntheirs\nmine\n");
    git(&place, &["commit", "--quiet", "--all", "--message", "mine"]);
    let tip = git(&place, &["rev-parse", "HEAD"]);
    assert!(daemon.qcc(&["thread", "archive", &id]).ok);
    assert!(!place.exists());
    assert_eq!(git(&repo, &["rev-parse", "feature/shared"]), tip);
}

#[test]
fn a_thread_moves_into_a_worktree_where_the_user_says() {
    let dir = tempfile::tempdir().unwrap();
    // Spelled as the commands see the directory they run in.
    let root = dir.path().canonicalize().unwrap();
    let repo = repo(&root, &[("README.md", "hello\n")]);
    let daemon = Daemon::start(&root.join("data"));
    let spawned = daemon.qcc(&[
        "thread",
        "spawn",
        "--provider",
        "echo",
        "--json",
        "--cwd",
        repo.to_str().unwrap(),
    ]);
    let id = spawned.json()["thread"]["id"].as_str().unwrap().to_owned();
    write(repo.join("README.md"), "hello\nedited\n");

    // A place that is taken leaves the thread, and its work, where they were.
    write(root.join("full/notes.md"), "mine\n");
    let full = ["thread", "isolate", &id, "--carry", "--env-dir", "full", "--branch", "fix/typo"];
    assert!(daemon.qcc_in(&root, &full).ok);
    let detached = daemon.wait_for(&id, "environment/detached");
    assert!(detached["message"].as_str().unwrap().contains("exists and is not empty"));
    assert_eq!(std::fs::read_to_string(repo.join("README.md")).unwrap(), "hello\nedited\n");
    assert_eq!(git(&repo, &["branch", "--list", "fix/*"]), "");

    let moved =
        ["thread", "isolate", &id, "--carry", "--env-dir", "tasks/typo", "--branch", "fix/typo"];
    assert!(daemon.qcc_in(&root, &moved).ok);
    daemon.wait_for(&id, "environment/ready");
    let worktree = root.join("tasks/typo");
    assert_eq!(daemon.thread(&id)["cwd"], worktree.to_str().unwrap());
    assert_eq!(git(&worktree, &["branch", "--show-current"]).trim(), "fix/typo");
    assert_eq!(std::fs::read_to_string(worktree.join("README.md")).unwrap(), "hello\nedited\n");
    assert_eq!(git(&repo, &["status", "--porcelain"]), "", "the work left the source");
}

#[test]
fn a_thread_in_a_checkout_of_its_own_reports_what_is_uncommitted_there() {
    let dir = tempfile::tempdir().unwrap();
    let long: String = (1..=20).map(|n| format!("line {n}\n")).collect();
    let repo = repo(
        dir.path(),
        &[("README.md", "hello\n"), ("src/a.txt", "one\n"), ("src/long.txt", &long)],
    );
    write(repo.join("src/a.txt"), "one\ntwo\n");
    write(repo.join("src/long.txt"), &long.replace("line 10\n", "line ten\n"));
    write(repo.join("src/new.txt"), "new\n");
    let daemon = Daemon::start(&dir.path().join("data"));
    let cwd = repo.join("src");
    let output = daemon.qcc(&[
        "thread",
        "spawn",
        "--provider",
        "echo",
        "--json",
        "--cwd",
        cwd.to_str().unwrap(),
    ]);
    assert!(output.ok, "{}", output.stderr);
    let id = output.json()["thread"]["id"].as_str().unwrap().to_owned();

    // Paths read from the top of the checkout, as they do for a worktree.
    let changes = daemon.qcc(&["thread", "changes", &id, "--json"]);
    assert!(changes.ok, "{}", changes.stderr);
    let changes = changes.json();
    assert_eq!(changes["changes"]["base"], "HEAD");
    let files: Vec<_> = changes["changes"]["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| {
            (file["path"].as_str().unwrap().to_owned(), file["kind"].as_str().unwrap().to_owned())
        })
        .collect();
    assert_eq!(
        files,
        [
            ("src/a.txt".to_owned(), "modified".to_owned()),
            ("src/long.txt".to_owned(), "modified".to_owned()),
            ("src/new.txt".to_owned(), "untracked".to_owned())
        ]
    );
    let diff = daemon.qcc(&["thread", "diff", &id]);
    assert!(diff.ok, "{}", diff.stderr);
    assert!(diff.stdout.contains("+two") && diff.stdout.contains("+new"), "{}", diff.stdout);
    // A few lines around the change by default; the whole file with `--full`.
    assert!(!diff.stdout.contains(" line 1\n"), "{}", diff.stdout);
    let full = daemon.qcc(&["thread", "diff", &id, "--full", "src/long.txt", "src/new.txt"]);
    assert!(full.ok, "{}", full.stderr);
    assert!(full.stdout.contains("@@ -1,20 +1,20 @@\n line 1\n"), "{}", full.stdout);
    assert!(full.stdout.contains("-line 10\n+line ten\n") && full.stdout.contains(" line 20\n"));
    assert!(full.stdout.contains("+new\n") && !full.stdout.contains("+two"), "{}", full.stdout);
}

#[test]
fn threads_started_together_each_get_a_worktree_of_the_same_repository() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let repo = repo(dir.path(), &[("README.md", "hello\n")]);
    let daemon = Daemon::start(&data);

    // Spawned side by side, as an orchestrator starts its children: git alone would
    // let the worktrees race for the names of their records in `.git/worktrees`.
    let ids: Vec<String> = std::thread::scope(|scope| {
        let spawns: Vec<_> = (0..12)
            .map(|n| {
                let (daemon, repo) = (&daemon, &repo);
                scope.spawn(move || {
                    let prompt = format!("task {n}");
                    let output = spawn_in_worktree(daemon, repo, &["--prompt", &prompt]);
                    assert!(output.ok, "{}", output.stderr);
                    output.json()["thread"]["id"].as_str().unwrap().to_owned()
                })
            })
            .collect();
        spawns.into_iter().map(|spawn| spawn.join().unwrap()).collect()
    });
    let mut wait = vec!["thread", "wait", "--timeout", "60"];
    wait.extend(ids.iter().map(String::as_str));
    let waited = daemon.qcc(&wait);
    assert!(waited.ok, "{}\n{}", waited.stdout, waited.stderr);
    for id in &ids {
        let worktree = data.join("environments/git-worktree").join(id).join("app");
        assert_eq!(std::fs::read_to_string(worktree.join("README.md")).unwrap(), "hello\n");
        assert_eq!(git(&worktree, &["branch", "--show-current"]).trim(), format!("qcc/{id}"));
    }
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

    // The commit is listed, and the work can be looked at a part at a time.
    let commit = git(&worktree, &["rev-parse", "HEAD"]).trim().to_owned();
    let revisions = &changes["changes"]["revisions"];
    assert_eq!(revisions.as_array().map(Vec::len), Some(1), "{revisions}");
    assert_eq!(
        (revisions[0]["id"].as_str(), revisions[0]["title"].as_str()),
        (Some(&*commit), Some("three"))
    );
    assert!(revisions[0]["time"].as_u64().is_some_and(|ms| ms > 1_000_000_000_000), "{revisions}");
    let paths = |args: &[&str]| -> Vec<String> {
        let mut all = vec!["thread", "changes", &id, "--json"];
        all.extend(args);
        let listed = daemon.qcc(&all);
        assert!(listed.ok, "{}", listed.stderr);
        let files = listed.json()["changes"]["files"].as_array().unwrap().clone();
        files.iter().map(|file| file["path"].as_str().unwrap().to_owned()).collect()
    };
    let uncommitted = [".worktreeinclude", "README.md", "docs/new.md"];
    assert_eq!(paths(&["--uncommitted"]), uncommitted);
    assert_eq!(paths(&["--from", &commit]), uncommitted);
    assert_eq!(paths(&["--to", &commit]), ["src/a.txt"]);
    let one = daemon.qcc(&["thread", "diff", &id, "--to", &commit]);
    assert!(one.stdout.contains("+three\n") && !one.stdout.contains("brand new"), "{}", one.stdout);
    let bad = daemon.qcc(&["thread", "diff", &id, "--to=--output=/tmp/x"]);
    assert!(!bad.ok && bad.stderr.contains("does not name a commit"), "{}", bad.stderr);

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
    assert!(changes.stderr.contains("is not ready"), "{}", changes.stderr);

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
    // A thread in a directory of its own outside git has no changes to tell.
    let outside = dir.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    let plain_thread = daemon.qcc(&[
        "thread",
        "spawn",
        "--provider",
        "echo",
        "--json",
        "--cwd",
        outside.to_str().unwrap(),
    ]);
    let plain_thread = plain_thread.json()["thread"]["id"].as_str().unwrap().to_owned();
    let diff = daemon.qcc(&["thread", "diff", &plain_thread]);
    assert!(!diff.ok);
    assert!(diff.stderr.contains("is not in a git checkout"), "{}", diff.stderr);
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
    let events = daemon.eventually(&id, "ran the second turn", |events| {
        types(events).iter().filter(|kind| **kind == "turn/completed").count() == 2
    });
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
    let events = daemon.eventually(&id, "ran the second turn", |e| messages(e).len() >= 2);
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
    let events = daemon.eventually(&id, "ran the second turn", |e| messages(e).len() >= 2);
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
fn comments_on_the_changes_reach_the_agent_and_it_resolves_them() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path());
    let id = daemon.spawn("first");
    daemon.wait_for(&id, "turn/completed");

    let added = daemon.qcc(&[
        "--json",
        "comment",
        "add",
        "--thread",
        &id,
        "src/lib.rs",
        "12",
        "Name this better",
    ]);
    assert!(added.ok, "{}", added.stderr);
    let comment = added.json_lines()[0][0]["id"].as_str().unwrap().to_owned();
    let old = daemon.qcc(&["comment", "add", "--thread", &id, "--old", "a.txt", "3", "Keep it"]);
    assert!(old.ok, "{}", old.stderr);
    assert!(old.stdout.contains("a.txt:3 (removed line)"), "{}", old.stdout);
    let other = old.stdout.split_whitespace().next().unwrap().to_owned();
    // Empty text and a line 0 are refused.
    assert!(!daemon.qcc(&["comment", "add", "--thread", &id, "a.txt", "1", " "]).ok);
    assert!(!daemon.qcc(&["comment", "add", "--thread", &id, "a.txt", "0", "x"]).ok);

    // The agent is handed the comment in full, with how to resolve it.
    let sent = daemon.qcc(&["thread", "send", &id, "/notes", "--comment", &comment]);
    assert!(sent.ok, "{}", sent.stderr);
    let events = daemon.eventually(&id, "ran the second turn", |e| messages(e).len() >= 2);
    let reply = messages(&events)[1];
    assert!(reply.contains(&format!("{comment} at src/lib.rs:12")), "{reply}");
    assert!(reply.contains("Name this better"), "{reply}");
    assert!(reply.contains("qcc comment resolve"), "{reply}");
    assert!(!reply.contains(&other), "only the comment sent is handed over: {reply}");

    // From inside its session the agent lists and resolves its thread's comments.
    let env = [("QCC_THREAD_ID", id.as_str())];
    let open = daemon.qcc_env(&["comment", "list"], &env);
    assert!(open.stdout.contains(&comment) && open.stdout.contains(&other), "{}", open.stdout);
    let resolved = daemon.qcc_env(&["comment", "resolve", &comment, "--note", "Renamed it"], &env);
    assert!(resolved.ok, "{}", resolved.stderr);
    let open = daemon.qcc_env(&["comment", "list"], &env).stdout;
    assert!(!open.contains(&comment) && open.contains(&other), "{open}");
    let all = daemon.qcc_env(&["comment", "list", "--all"], &env).stdout;
    assert!(all.contains("(resolved)") && all.contains("resolved: Renamed it"), "{all}");
    // A wrong id changes nothing, even beside a right one.
    assert!(!daemon.qcc(&["comment", "resolve", &other, "cmt_nope"]).ok);
    assert!(daemon.qcc_env(&["comment", "list"], &env).stdout.contains(&other));

    assert!(daemon.qcc(&["comment", "reopen", &comment]).ok);
    assert!(daemon.qcc(&["comment", "delete", &other]).ok);
    let open = daemon.qcc_env(&["--json", "comment", "list"], &env).json_lines();
    assert_eq!(open[0].as_array().unwrap().len(), 1, "{open:?}");
    assert_eq!(open[0][0]["id"], comment.as_str());
    assert!(open[0][0].get("resolution").is_none(), "{open:?}");

    // A watching client hears of every change, with all of the thread's comments.
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let (client, mut notifications) =
            qcc_client::Client::connect_raw(&daemon.socket).await.unwrap();
        client.call_raw("thread.watch", serde_json::json!({ "subscription": 3 })).await.unwrap();
        assert!(daemon.qcc(&["comment", "resolve", &comment]).ok);
        let notification = loop {
            let notification = tokio::time::timeout(Duration::from_secs(30), notifications.recv())
                .await
                .expect("the watcher is told about the comment")
                .expect("the connection stays open");
            if notification.method == "comments.updated" {
                break notification;
            }
        };
        assert_eq!(notification.params["subscription"], 3);
        assert_eq!(notification.params["thread_id"], id.as_str());
        assert!(notification.params["comments"][0]["resolved_at"].is_u64());
    });
}

#[test]
fn a_thread_starts_its_conversation_afresh_and_compacts_its_context() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path());
    let id = daemon.spawn("first words");
    daemon.wait_for(&id, "turn/completed");
    let turns = |events: &[Value]| types(events).iter().filter(|t| **t == "turn/completed").count();
    let wait_turns = |count: usize| {
        daemon.eventually(&id, &format!("ended turn {count}"), |events| turns(events) >= count)
    };

    // Compacting is a turn of the agent's own, with no message of the user's.
    let compacted = daemon.qcc(&["thread", "compact", &id, "--follow"]);
    assert!(compacted.ok, "{}", compacted.stderr);
    assert!(compacted.stdout.contains("context compacted:"), "{}", compacted.stdout);
    let events = wait_turns(2);
    let event = events.iter().find(|e| e["type"] == "session/compacted").unwrap();
    assert!(event["compaction"]["tokens_before"].as_u64().unwrap() > 0, "{event}");
    assert_eq!(inputs(&events), ["first words"]);

    // A new conversation: the next message starts a session of its own, and
    // the transcript is that conversation's.
    assert!(daemon.qcc(&["thread", "reset", &id]).ok);
    let thread = daemon.thread(&id);
    assert_eq!(thread["conversations"], 1);
    assert!(thread.get("provider_session").is_none(), "{thread}");
    assert!(daemon.qcc(&["thread", "send", &id, "second words"]).ok);
    let events = wait_turns(3);
    let reset = events.iter().position(|e| e["type"] == "thread/reset").unwrap();
    // The old session ends in the old conversation.
    let ended = events.iter().rposition(|e| e["type"] == "session/ended").unwrap();
    assert!(ended < reset, "{events:#?}");
    let started = events[reset..].iter().find(|e| e["type"] == "session/started").unwrap();
    assert_eq!(started["resumed"], false, "{started}");
    let shown = daemon.qcc(&["thread", "show", &id]).stdout;
    assert!(shown.contains("a new conversation"), "{shown}");
    assert!(shown.contains("first words"), "earlier conversations stay in the timeline: {shown}");
    let waited = daemon.qcc(&["thread", "wait", &id, "--json"]).json();
    assert_eq!(waited["threads"][0]["last_message"], "second words");

    // Nothing to compact in a conversation not started yet.
    assert!(daemon.qcc(&["thread", "reset", &id]).ok);
    let refused = daemon.qcc(&["thread", "compact", &id]);
    assert!(!refused.ok && refused.stderr.contains("no conversation"), "{}", refused.stderr);
}

#[test]
fn the_agent_says_what_its_context_holds() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path());
    let id = daemon.spawn("one two three");
    daemon.wait_for(&id, "turn/completed");
    // Only how full it is until the agent breaks it down.
    let plain = daemon.qcc(&["thread", "context", &id]).stdout;
    assert!(plain.contains("tokens"), "{plain}");

    assert!(daemon.qcc(&["thread", "send", &id, "/context"]).ok);
    let events = daemon
        .eventually(&id, "a context breakdown", |events| types(events).contains(&"context/usage"));
    let event = events.iter().find(|e| e["type"] == "context/usage").unwrap();
    assert_eq!(event["context"]["entries"][0]["name"], "echo-demo", "{event}");

    // The projection keeps the parts, not what each is made of.
    let thread = daemon.thread(&id);
    assert_eq!(thread["context"]["window"], 10_000, "{thread}");
    assert!(thread["context"].get("entries").is_none(), "{thread}");
    let shown = daemon.qcc(&["thread", "context", &id]).stdout;
    assert!(shown.contains("System prompt") && shown.contains("(free)"), "{shown}");

    // A new conversation has an empty context.
    assert!(daemon.qcc(&["thread", "reset", &id]).ok);
    assert!(daemon.thread(&id).get("context").is_none());
}

#[test]
fn uses_of_tools_are_counted_across_threads() {
    use qcc_client::Client;
    use serde_json::json;

    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path());
    let first = daemon.spawn("/tool docs__search");
    let second = daemon.spawn("/tool lint");
    daemon.wait_for(&first, "turn/completed");
    daemon.wait_for(&second, "turn/completed");

    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let result = runtime.block_on(async {
        let client = Client::connect(&daemon.socket).await.unwrap();
        client.call_raw("usage.tools", json!({})).await.unwrap()
    });
    let uses: Vec<(String, String, String, u64)> = result["buckets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|bucket| {
            (
                bucket["thread_id"].as_str().unwrap().to_owned(),
                bucket["kind"].as_str().unwrap().to_owned(),
                bucket["name"].as_str().unwrap().to_owned(),
                bucket["calls"].as_u64().unwrap(),
            )
        })
        .collect();
    assert!(uses.contains(&(first.clone(), "mcp".into(), "docs__search".into(), 1)), "{uses:?}");
    assert!(uses.contains(&(second.clone(), "tool".into(), "lint".into(), 1)), "{uses:?}");
    let mcp = result["buckets"].as_array().unwrap().iter().find(|b| b["kind"] == "mcp").unwrap();
    assert_eq!(mcp["server"], "docs");
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

    let events = daemon.eventually(&id, "ran the queued message", |events| {
        types(events).iter().filter(|kind| **kind == "turn/completed").count() == 2
    });
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
                }, {
                    "plugin": "reviewers",
                    "id": "reviewers",
                    "title": "Reviewers",
                    "placement": "threadPanel",
                    "scope": "allThreads",
                    "providers": [],
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
fn skills_are_switched_for_all_threads_and_for_one() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path());
    let last = |thread: &str| {
        let told = daemon.qcc(&["thread", "wait", thread, "--json"]).json();
        told["threads"][0]["last_message"].as_str().unwrap().to_owned()
    };
    let listed = daemon.qcc(&["skill", "list", "--json"]).json();
    let names: Vec<&str> =
        listed.as_array().unwrap().iter().map(|skill| skill["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["echo-demo", "qcc-orchestration", "qcc-reviewers"]);
    assert_eq!(listed[0]["on_request"], true);

    // A skill offered on request reaches only a thread that asks for it, and one
    // every session gets can be left out.
    let plain = daemon.spawn("/skills");
    let told = last(&plain);
    assert!(told.contains("qcc-orchestration:") && !told.contains("echo-demo"), "{told}");
    let output = daemon.qcc(&[
        "thread",
        "spawn",
        "--provider",
        "echo",
        "--prompt",
        "/skills",
        "--skill",
        "echo-demo",
        "--without-skill",
        "qcc-orchestration",
        "--json",
    ]);
    assert!(output.ok, "{}", output.stderr);
    let chosen = output.json()["thread"]["id"].as_str().unwrap().to_owned();
    let told = last(&chosen);
    assert!(told.contains("echo-demo:") && !told.contains("qcc-orchestration"), "{told}");
    let unknown = daemon.qcc(&["thread", "spawn", "--provider", "echo", "--skill", "nope"]);
    assert!(!unknown.ok && unknown.stderr.contains("no plugin offers"), "{}", unknown.stderr);

    // Turned on in a running session, the skill is told with the next message.
    let switched = daemon.qcc(&["thread", "skills", &plain, "--on", "echo-demo"]);
    assert!(switched.ok, "{}", switched.stderr);
    assert!(
        switched.stdout.contains("echo-demo                on  (on request)"),
        "{}",
        switched.stdout
    );
    assert!(daemon.qcc(&["thread", "send", &plain, "/notes"]).ok);
    let events = daemon.eventually(&plain, "ran the second turn", |e| messages(e).len() >= 2);
    let note = messages(&events)[1].to_owned();
    assert!(note.contains("qcc now offers the skill echo-demo"), "{note}");
    // Told once: the next message carries no note.
    assert!(daemon.qcc(&["thread", "send", &plain, "/notes"]).ok);
    let events = daemon.eventually(&plain, "ran the third turn", |e| messages(e).len() >= 3);
    assert_eq!(messages(&events)[2], "no notes");
    // Turned off, it is gone for the thread's next session, with no note.
    assert!(daemon.qcc(&["thread", "skills", &plain, "--off", "echo-demo"]).ok);
    assert!(daemon.thread(&plain).get("skills_enabled").is_none());

    // Turned off for all, no session gets it, whatever a thread asks.
    assert!(daemon.qcc(&["skill", "disable", "echo-demo"]).ok);
    let output = daemon.qcc(&[
        "thread",
        "spawn",
        "--provider",
        "echo",
        "--prompt",
        "/skills",
        "--skill",
        "echo-demo",
        "--json",
    ]);
    let asked = output.json()["thread"]["id"].as_str().unwrap().to_owned();
    assert!(!last(&asked).contains("echo-demo"));
    assert!(daemon.qcc(&["skill", "list"]).stdout.contains("(on request, off)"));
    assert!(daemon.qcc(&["skill", "enable", "echo-demo"]).ok);

    // The skills an agent finds by itself are shown, as it reports them.
    let own = daemon.qcc(&["provider", "skills", "echo", "--json"]).json();
    assert_eq!(own["skills"][0]["name"], "echo-own");
    assert_eq!(own["skills"][0]["source"], "user");
    let shown = daemon.qcc(&["provider", "skills", "echo"]).stdout;
    assert!(shown.contains("echo-own  (user)"), "{shown}");
    let settings = daemon.qcc(&["settings", "show", "--json"]).json();
    assert!(settings.get("skills").is_none(), "{settings}");
}

#[test]
fn a_plugin_is_told_its_own_settings() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path());
    let ask = |daemon: &Daemon| {
        let id = daemon.spawn("/settings");
        let told = daemon.qcc(&["thread", "wait", &id, "--json"]).json();
        told["threads"][0]["last_message"].as_str().unwrap().to_owned()
    };
    // Unset, a setting is its default; one without a default is not told.
    assert_eq!(ask(&daemon), "greeting=hello\nloud=false");
    let shown = daemon.qcc(&["plugin", "settings", "provider-echo"]).stdout;
    assert!(shown.contains("greeting             hello  (default)"), "{shown}");
    assert!(shown.contains("calm | cheery"), "{shown}");

    for (setting, value) in [("greeting", "hi there"), ("voice", "calm"), ("agent", "echo")] {
        let set = daemon.qcc(&["plugin", "set", "provider-echo", setting, value]);
        assert!(set.ok, "{}", set.stderr);
    }
    assert_eq!(ask(&daemon), "greeting=hi there\nloud=false\nvoice=calm\nagent=echo");

    // What does not fit the setting is refused, and nothing changes.
    for (setting, value, why) in [
        ("voice", "shouty", "one of calm, cheery"),
        ("loud", "yes", "true or false"),
        ("agent", "nobody", "no plugin provides"),
        ("mode", "loose", "unknown permission mode"),
        ("nope", "x", "has no setting"),
    ] {
        let refused = daemon.qcc(&["plugin", "set", "provider-echo", setting, value]);
        assert!(!refused.ok && refused.stderr.contains(why), "{setting}: {}", refused.stderr);
    }
    // Back to the default, and the values survive a restart.
    assert!(daemon.qcc(&["plugin", "set", "provider-echo", "greeting"]).ok);
    daemon.stop();
    let daemon = Daemon::start(dir.path());
    assert_eq!(ask(&daemon), "greeting=hello\nloud=false\nvoice=calm\nagent=echo");
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
        .filter(|c| c["plugin"] == "orchestration")
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
    daemon.eventually(&source, "received the review", |events| {
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

#[test]
fn files_and_pasted_images_are_attached_to_messages() {
    use base64::Engine as _;
    use qcc_client::Client;
    use serde_json::json;

    let dir = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let png = b"\x89PNG\r\n\x1a\nnot really";
    std::fs::write(files.path().join("shot.png"), png).unwrap();
    std::fs::write(files.path().join("notes.md"), "# notes\n").unwrap();
    let daemon = Daemon::start(dir.path());
    let id = daemon.spawn("first");
    daemon.wait_for(&id, "turn/completed");

    let shot = files.path().join("shot.png");
    let notes = files.path().join("notes.md");
    let sent = daemon.qcc(&[
        "thread",
        "send",
        &id,
        "/attachments",
        "--attach",
        shot.to_str().unwrap(),
        "--attach",
        notes.to_str().unwrap(),
    ]);
    assert!(sent.ok, "{}", sent.stderr);
    let events = daemon.eventually(&id, "ran the second turn", |e| messages(e).len() >= 2);
    let reply = messages(&events)[1].to_owned();
    // Echo looks at images: the bytes of the picture came along, the other file by path.
    let lines: Vec<&str> = reply.lines().collect();
    assert!(
        lines[0].starts_with("shot.png (image/png, 18 bytes) image, 18 bytes inline: "),
        "{reply}"
    );
    assert!(lines[1].starts_with("notes.md (text/markdown, 8 bytes) by path: "), "{reply}");
    // qcc's copies, kept with the thread.
    let own = dir.path().join("attachments").join(&id);
    let submitted = events.iter().rfind(|event| event["type"] == "input/submitted").unwrap();
    let attachments = submitted["attachments"].as_array().unwrap();
    assert_eq!(attachments.len(), 2);
    let copy = attachments[0]["path"].as_str().unwrap().to_owned();
    assert!(Path::new(&copy).starts_with(&own), "{copy}");
    assert_eq!(std::fs::read(&copy).unwrap(), png);
    // The CLI shows them with the message.
    let shown = daemon.qcc(&["thread", "events", &id]).stdout;
    assert!(shown.contains("[attached shot.png (image/png, 18 bytes)]"), "{shown}");

    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let client = Client::connect(&daemon.socket).await.unwrap();
        // A client reads them back to show them.
        let read = client
            .call_raw("thread.attachment", json!({"thread_id": id, "path": copy}))
            .await
            .unwrap();
        assert_eq!(read["mime"], "image/png");
        let data = base64::engine::general_purpose::STANDARD.encode(png);
        assert_eq!(read["data"], data.as_str());
        // Nothing but the thread's own attachments.
        let outside = notes.display().to_string();
        let refused =
            client.call_raw("thread.attachment", json!({"thread_id": id, "path": outside})).await;
        assert!(refused.is_err());

        // An empty message is refused; a picture alone is a message.
        let empty = client.call_raw("thread.send", json!({"thread_id": id, "text": " "})).await;
        assert!(empty.is_err());
        let pasted = json!({"data": data, "mime": "image/png"});
        let alone = json!({"thread_id": id, "text": "", "attachments": [pasted]});
        client.call_raw("thread.send", alone).await.unwrap();
        daemon.eventually(&id, "ran the third turn", |e| {
            e.iter().filter(|event| event["type"] == "turn/completed").count() >= 3
        });
        // A pasted image comes as its bytes, named for its type.
        let ask = json!({"thread_id": id, "text": "/attachments", "attachments": [pasted]});
        client.call_raw("thread.send", ask).await.unwrap();
    });
    let events = daemon.eventually(&id, "ran the fourth turn", |e| messages(e).len() >= 3);
    let reply = messages(&events)[2];
    assert!(
        reply.starts_with("pasted.png (image/png, 18 bytes) image, 18 bytes inline: "),
        "{reply}"
    );
}

impl Daemon {
    /// Make an automation that runs the echo agent in `dir`; returns its id.
    fn automation(
        &self,
        dir: &Path,
        name: &str,
        schedule: &str,
        prompt: &str,
        extra: &[&str],
    ) -> String {
        let cwd = dir.to_str().unwrap();
        let mut args = vec![
            "automation",
            "add",
            name,
            "--schedule",
            schedule,
            "--prompt",
            prompt,
            "--provider",
            "echo",
            "--cwd",
            cwd,
            "--json",
        ];
        args.extend_from_slice(extra);
        let output = self.qcc(&args);
        assert!(output.ok, "{}", output.stderr);
        output.json()["automation"]["automation"]["id"].as_str().unwrap().to_owned()
    }

    /// An automation's runs, newest first.
    fn runs(&self, automation: &str) -> Vec<Value> {
        let output = self.qcc(&["automation", "show", automation, "--runs", "100", "--json"]);
        assert!(output.ok, "{}", output.stderr);
        output.json()["runs"].as_array().unwrap().clone()
    }

    /// Poll an automation's runs until `check` holds for them, and return them.
    fn runs_until(
        &self,
        automation: &str,
        what: &str,
        check: impl Fn(&[Value]) -> bool,
    ) -> Vec<Value> {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let runs = self.runs(automation);
            if check(&runs) {
                return runs;
            }
            assert!(Instant::now() < deadline, "never {what}: {runs:#?}");
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

#[test]
fn an_automation_runs_on_schedule_and_skips_while_its_last_run_works() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path());
    // Each run streams for about three seconds; one comes due every two.
    let prompt = format!("/slow {}", words(300));
    let id = daemon.automation(dir.path(), "Nightly check", "every 2s", &prompt, &[]);

    let listed = daemon.qcc(&["automation", "list", "--json"]).json();
    let entry = &listed["automations"][0];
    assert_eq!(entry["automation"]["name"], "Nightly check");
    assert_eq!(entry["automation"]["enabled"], true);
    assert!(entry["next_run_at"].as_u64().is_some());

    let runs = daemon.runs_until(&id, "skipped a run that overlapped", |runs| {
        runs.iter().any(|run| run["outcome"] == "skipped")
    });
    let started: Vec<&Value> = runs.iter().filter(|run| run["outcome"] == "started").collect();
    let first = started.last().expect("a run started before one was skipped");
    assert_eq!(first["trigger"], "schedule");
    let thread = first["thread_id"].as_str().unwrap();
    let skipped = runs.iter().find(|run| run["outcome"] == "skipped").unwrap();
    assert!(skipped["reason"].as_str().unwrap().contains("still at work"), "{skipped:#?}");

    // The thread says which automation started it, and is named after it.
    let shown = daemon.thread(thread);
    assert_eq!(shown["automation"], id.as_str());
    assert_eq!(shown["title"], "Nightly check");
    let human = daemon.qcc(&["thread", "show", thread]);
    assert!(human.stdout.contains(&format!("started   by automation {id}")), "{}", human.stdout);

    // Once the first run is done, the next one starts a thread of its own.
    let runs = daemon.runs_until(&id, "started a second thread", |runs| {
        runs.iter().filter(|run| run["outcome"] == "started").count() >= 2
    });
    let threads: Vec<&str> = runs
        .iter()
        .filter(|run| run["outcome"] == "started")
        .map(|run| run["thread_id"].as_str().unwrap())
        .collect();
    assert_ne!(threads[0], threads[1]);

    // Switched off, it runs no more; its history stays.
    let off = daemon.qcc(&["automation", "disable", "Nightly check", "--json"]).json();
    assert_eq!(off["automation"]["automation"]["enabled"], false);
    assert!(off["automation"]["next_run_at"].is_null());
    let count = daemon.runs(&id).len();
    std::thread::sleep(Duration::from_millis(2500));
    assert_eq!(daemon.runs(&id).len(), count);
}

#[test]
fn an_automation_runs_by_hand_and_can_continue_one_thread() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path());
    let id = daemon.automation(
        dir.path(),
        "Standup",
        "0 9 * * mon-fri",
        "what changed?",
        &["--threads", "continue", "--disabled"],
    );

    // A run by hand goes ahead though the automation is switched off.
    let run = daemon.qcc(&["automation", "run", &id, "--json"]).json();
    assert_eq!(run["run"]["trigger"], "manual");
    assert_eq!(run["run"]["outcome"], "started");
    let thread = run["run"]["thread_id"].as_str().unwrap().to_owned();
    let waited = daemon.qcc(&["thread", "wait", &thread, "--json"]);
    assert!(waited.ok, "{}", waited.stderr);

    // The second run is a message to the same thread.
    let again = daemon.qcc(&["automation", "run", "Standup", "--json"]).json();
    assert_eq!(again["run"]["outcome"], "started");
    assert_eq!(again["run"]["thread_id"], thread.as_str());
    daemon.eventually(&thread, "answered twice", |events| messages(events).len() == 2);
    let shown = daemon.qcc(&["automation", "show", &id, "--json"]).json();
    assert_eq!(shown["automation"]["automation"]["thread_id"], thread.as_str());
    assert_eq!(shown["runs"].as_array().unwrap().len(), 2);

    // Mistakes are refused when the automation is made or changed, not at night.
    let wrong = daemon.qcc(&["automation", "edit", &id, "--schedule", "61 * * * *"]);
    assert!(!wrong.ok);
    assert!(wrong.stderr.contains("out of range"), "{}", wrong.stderr);
    let wrong = daemon.qcc(&[
        "automation",
        "add",
        "Bad",
        "--schedule",
        "every 1h",
        "--prompt",
        "x",
        "--provider",
        "nobody",
    ]);
    assert!(!wrong.ok);
    let wrong = daemon.qcc(&[
        "automation",
        "add",
        "Bad",
        "--schedule",
        "every 1h",
        "--prompt",
        "x",
        "--provider",
        "echo",
        "--cwd",
        "/no/such/dir",
    ]);
    assert!(!wrong.ok);

    let edited =
        daemon.qcc(&["automation", "edit", &id, "--name", "Daily", "--threads", "new", "--json"]);
    assert!(edited.ok, "{}", edited.stderr);
    let edited = edited.json();
    assert_eq!(edited["automation"]["automation"]["name"], "Daily");
    assert!(edited["automation"]["automation"]["thread_id"].is_null());

    // Deleting it leaves its threads be.
    assert!(daemon.qcc(&["automation", "remove", "Daily"]).ok);
    let listed = daemon.qcc(&["automation", "list", "--json"]).json();
    assert_eq!(listed["automations"].as_array().unwrap().len(), 0);
    assert_eq!(daemon.thread(&thread)["automation"], id.as_str());
}

#[test]
fn a_run_missed_while_the_daemon_was_down_is_made_up_for_at_most_once() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path());
    let eager = daemon.automation(dir.path(), "Eager", "every 3s", "catch up", &["--catch-up"]);
    let lazy = daemon.automation(dir.path(), "Lazy", "every 3s", "let it go", &[]);
    daemon.stop();
    // Down long enough to miss two runs of each.
    std::thread::sleep(Duration::from_secs(7));
    let daemon = Daemon::start(dir.path());

    let missed = |runs: &[Value]| runs.iter().any(|run| run["trigger"] == "missed");
    let runs = daemon.runs_until(&eager, "made up for the missed run", missed);
    let made_up: Vec<&Value> = runs.iter().filter(|run| run["trigger"] == "missed").collect();
    assert_eq!(made_up.len(), 1, "{runs:#?}");
    assert_eq!(made_up[0]["outcome"], "started");

    let runs = daemon.runs_until(&lazy, "noted the missed run", missed);
    let noted: Vec<&Value> = runs.iter().filter(|run| run["trigger"] == "missed").collect();
    assert_eq!(noted.len(), 1, "{runs:#?}");
    assert_eq!(noted[0]["outcome"], "skipped");
    assert!(noted[0]["thread_id"].is_null());
}

#[test]
fn usage_limits_are_kept_per_provider_and_survive_a_restart() {
    use qcc_client::Client;
    use serde_json::json;

    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path());

    // Nothing known yet: the CLI asks the agent, without a session. (Only echo's:
    // asking every provider would run the real agents.)
    let read = daemon.qcc(&["provider", "limits", "echo", "--json"]).json();
    let echo = &read["limits"][0];
    assert_eq!(echo["provider"], "echo", "{read}");
    assert_eq!(echo["plan"], "echo");
    assert_eq!(echo["windows"][0]["id"], "five_hour");
    assert_eq!(echo["windows"][0]["used_percent"], 10);
    assert_eq!(echo["windows"][1]["label"], "Weekly");
    assert!(
        echo["windows"][0]["resets_at"].as_u64().unwrap() > echo["updated_at"].as_u64().unwrap()
    );
    // A provider that does not report limits cannot be asked.
    let refused = daemon.qcc(&["provider", "limits", "opencode"]);
    assert!(!refused.ok);
    assert!(refused.stderr.contains("does not report usage limits"), "{}", refused.stderr);

    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let (client, mut notifications) = Client::connect_raw(&daemon.socket).await.unwrap();
        client.call_raw("thread.watch", json!({ "subscription": 3 })).await.unwrap();

        // A session reports the window its turn spent; the others stay as they were,
        // and the thread's timeline never hears of it.
        let id = daemon.spawn("/limits 85");
        let limits = loop {
            let notification = tokio::time::timeout(Duration::from_secs(30), notifications.recv())
                .await
                .expect("the watcher is told about the limits")
                .expect("the connection stays open");
            if notification.method == "limits.updated" {
                assert_eq!(notification.params["subscription"], 3);
                break notification.params["limits"].clone();
            }
        };
        assert_eq!(limits["provider"], "echo");
        assert_eq!(limits["windows"][0]["used_percent"], 85);
        assert_eq!(limits["windows"][0]["status"], "warning");
        assert_eq!(limits["windows"][1]["used_percent"], 20);
        daemon.wait_for(&id, "turn/completed");
        let events = daemon.events(&id);
        assert!(!events.iter().any(|event| event.to_string().contains("used_percent")));
    });

    let shown = daemon.qcc(&["provider", "limits", "echo"]).stdout;
    assert!(shown.starts_with("echo (echo)  as of"), "{shown}");
    assert!(
        shown.contains("5-hour") && shown.contains("85%  resets in 4h 59m  warning"),
        "{shown}"
    );

    daemon.stop();
    let daemon = Daemon::start(dir.path());
    // Known now, so nothing is asked.
    let kept = daemon.qcc(&["provider", "limits", "--json"]).json();
    assert_eq!(kept["limits"][0]["windows"][0]["used_percent"], 85, "{kept}");
}

/// Spawn an echo thread under `parent`; returns its id.
fn spawn_child(daemon: &Daemon, parent: &str, prompt: &str) -> String {
    let args = ["thread", "spawn", "--provider", "echo", "--parent", parent, "--prompt", prompt];
    let output = daemon.qcc(&[&args[..], &["--json"]].concat());
    assert!(output.ok, "{}", output.stderr);
    output.json()["thread"]["id"].as_str().unwrap().to_owned()
}

fn ids(list: &Value) -> Vec<&str> {
    list.as_array()
        .into_iter()
        .flatten()
        .map(|v| v.as_str().unwrap_or_else(|| v["id"].as_str().unwrap()))
        .collect()
}

#[test]
fn archiving_a_thread_takes_every_thread_under_it_and_stops_the_working_ones() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path());
    let root = daemon.spawn("root");
    let child = spawn_child(&daemon, &root, "child");
    let other = spawn_child(&daemon, &root, "other child");
    let grandchild = spawn_child(&daemon, &child, &format!("/slow {}", words(3000)));
    assert!(daemon.qcc(&["thread", "wait", &root, &child, &other]).ok);
    daemon.wait_for(&grandchild, "item/text/delta");

    let archived = daemon.qcc(&["thread", "archive", &root, "--json"]);
    assert!(archived.ok, "{}", archived.stderr);
    let report = archived.json();
    assert_eq!(report["thread"]["archived"], true);
    // Deepest first, so no archived thread ever has live threads under it.
    let gone = ids(&report["archived"]);
    assert_eq!(gone.len(), 3, "{report:#}");
    assert_eq!(gone[0], grandchild);
    assert!(gone.contains(&child.as_str()) && gone.contains(&other.as_str()), "{report:#}");
    assert_eq!(ids(&report["stopped"]), [grandchild.as_str()]);
    assert_eq!(report.get("refused"), None);
    for id in [&root, &child, &other, &grandchild] {
        assert_eq!(daemon.thread(id)["archived"], true, "{id}");
    }
    // The working one was stopped: its turn ended before it was archived.
    let kinds = types(&daemon.events(&grandchild)).join(" ");
    assert!(kinds.ends_with("session/ended thread/archived"), "{kinds}");
    assert!(kinds.contains("turn/completed"), "{kinds}");

    // Nothing new is started under an archived thread.
    let late = daemon.qcc(&["thread", "spawn", "--provider", "echo", "--parent", &root]);
    assert!(!late.ok && late.stderr.contains("is archived"), "{}", late.stderr);

    // In text, one line per archived thread, the one asked for last.
    let root = daemon.spawn("again");
    let child = spawn_child(&daemon, &root, "child");
    assert!(daemon.qcc(&["thread", "wait", &root, &child]).ok);
    let text = daemon.qcc(&["thread", "archive", &root]);
    assert!(text.ok, "{}", text.stderr);
    let lines: Vec<&str> = text.stdout.lines().collect();
    assert_eq!(lines.len(), 2, "{}", text.stdout);
    assert!(lines[0].starts_with(&child) && lines[1].starts_with(&root), "{}", text.stdout);
}

#[test]
fn a_thread_alone_is_archived_without_the_threads_under_it_when_asked() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path());
    let root = daemon.spawn("root");
    let child = spawn_child(&daemon, &root, "child");
    let grandchild = spawn_child(&daemon, &child, "grandchild");
    assert!(daemon.qcc(&["thread", "wait", &root, &child, &grandchild]).ok);

    // As before descendants were archived: the protocol's default.
    let alone = daemon.qcc(&["thread", "archive", &child, "--no-descendants", "--json"]);
    assert!(alone.ok, "{}", alone.stderr);
    assert_eq!(alone.json().as_object().unwrap().keys().collect::<Vec<_>>(), ["thread"]);
    assert_eq!(daemon.thread(&grandchild).get("archived"), None);

    // The tree is looked through an archived thread: the grandchild still goes.
    let archived = daemon.qcc(&["thread", "archive", &root, "--json"]);
    assert!(archived.ok, "{}", archived.stderr);
    assert_eq!(ids(&archived.json()["archived"]), [grandchild.as_str()]);
    assert_eq!(daemon.thread(&grandchild)["archived"], true);
}

#[test]
fn a_working_thread_is_not_archived_and_neither_is_anything_under_it() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path());
    let root = daemon.spawn(&format!("/slow {}", words(3000)));
    let child = spawn_child(&daemon, &root, "child");
    assert!(daemon.qcc(&["thread", "wait", &child]).ok);
    daemon.wait_for(&root, "item/text/delta");

    let refused = daemon.qcc(&["thread", "archive", &root]);
    assert!(!refused.ok);
    assert!(refused.stderr.contains("stop it before archiving it"), "{}", refused.stderr);
    assert_eq!(daemon.thread(&child).get("archived"), None);
    assert!(daemon.qcc(&["thread", "stop", &root]).ok);
}

#[test]
fn a_refusal_under_a_thread_keeps_it_and_its_ancestors_and_lets_the_rest_go() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let repo = repo(dir.path(), &[("README.md", "hello\n")]);
    let daemon = Daemon::start(&data);
    let root = daemon.spawn("root");
    let middle = spawn_child(&daemon, &root, "middle");
    let clean = spawn_child(&daemon, &root, "clean");
    let output = spawn_in_worktree(&daemon, &repo, &["--parent", &middle, "--prompt", "dirty"]);
    assert!(output.ok, "{}", output.stderr);
    let dirty = output.json()["thread"]["id"].as_str().unwrap().to_owned();
    assert!(daemon.qcc(&["thread", "wait", &root, &middle, &clean, &dirty]).ok);
    let worktree = data.join("environments/git-worktree").join(&dirty).join("app");
    write(worktree.join("README.md"), "hello\nworld\n");

    let refused = daemon.qcc(&["thread", "archive", &root, "--json"]);
    assert!(!refused.ok);
    assert!(refused.stderr.contains("uncommitted change"), "{}", refused.stderr);
    assert!(refused.stderr.contains("--force"), "{}", refused.stderr);
    let report = refused.json();
    assert_eq!(report["refused"][0]["thread_id"], dirty.as_str());
    assert_eq!(ids(&report["archived"]), [clean.as_str()]);
    // Above the refusal, from the bottom up.
    assert_eq!(ids(&report["kept"]), [middle.as_str(), root.as_str()]);
    assert_eq!(report["thread"].get("archived"), None);
    assert!(worktree.is_dir());
    assert_eq!(daemon.thread(&clean)["archived"], true);
    // What was kept is let go: it takes messages again.
    assert!(daemon.qcc(&["thread", "send", &middle, "still here"]).ok);
    assert!(daemon.qcc(&["thread", "wait", &middle]).ok);

    let forced = daemon.qcc(&["thread", "archive", &root, "--force", "--json"]);
    assert!(forced.ok, "{}", forced.stderr);
    assert_eq!(ids(&forced.json()["archived"]), [dirty.as_str(), middle.as_str()]);
    assert!(!worktree.exists());
    assert_eq!(daemon.thread(&root)["archived"], true);
}

// ----- reviewers --------------------------------------------------------------------

/// A reviewer profile for the echo agent: echo answers with its prompt, which ends
/// with the profile's words, so the findings object in them is what it reports.
const FINDER: &str = "---\nname: finder\ndescription: Finds one thing\n---\nReport exactly this: \
{\"findings\": [{\"file\": \"a.txt\", \"line\": 2, \"severity\": \"high\", \"text\": \"Name this better\"}]}\n";

/// Poll until `check` gives something, for up to 20 seconds.
fn eventually<T>(what: &str, check: impl Fn() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(found) = check() {
            return found;
        }
        assert!(Instant::now() < deadline, "never {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

impl Daemon {
    fn children(&self, thread: &str) -> Vec<Value> {
        let listed = self.qcc(&["thread", "list", "--parent", thread, "--json"]).json();
        listed["threads"].as_array().unwrap().clone()
    }

    fn comments(&self, thread: &str) -> Vec<Value> {
        let listed = self.qcc_env(&["--json", "comment", "list"], &[("QCC_THREAD_ID", thread)]);
        assert!(listed.ok, "{}", listed.stderr);
        listed.json_lines()[0].as_array().unwrap().clone()
    }

    /// An echo thread working in `dir` that has finished its first turn.
    fn spawn_in_dir(&self, dir: &Path, prompt: &str) -> String {
        let cwd = dir.to_str().unwrap();
        let output = self.qcc(&[
            "thread",
            "spawn",
            "--provider",
            "echo",
            "--cwd",
            cwd,
            "--prompt",
            prompt,
            "--json",
        ]);
        assert!(output.ok, "{}", output.stderr);
        let id = output.json()["thread"]["id"].as_str().unwrap().to_owned();
        assert!(self.qcc(&["thread", "wait", &id]).ok);
        id
    }

    /// Send a message and wait until the turn it starts is over.
    fn turn(&self, thread: &str, text: &str) {
        let before = self.events(thread).iter().filter(|e| e["type"] == "turn/completed").count();
        assert!(self.qcc(&["thread", "send", thread, text]).ok);
        self.eventually(thread, "finished the turn", |events| {
            events.iter().filter(|e| e["type"] == "turn/completed").count() > before
        });
    }
}

#[test]
fn reviewers_review_a_thread_once_its_agent_is_done_and_leave_comments() {
    let dir = tempfile::tempdir().unwrap();
    let repo = repo(dir.path(), &[("a.txt", "one\n"), (".qcc/reviewers/finder.md", FINDER)]);
    let daemon = Daemon::start(&dir.path().join("data"));
    let id = daemon.spawn_in_dir(&repo, "first");

    let profiles = daemon.qcc(&["reviewers", "profiles", "--thread", &id]);
    assert!(profiles.ok, "{}", profiles.stderr);
    assert!(profiles.stdout.contains("finder (project): Finds one thing"), "{}", profiles.stdout);
    assert!(profiles.stdout.contains("general (built in)"), "{}", profiles.stdout);
    let configured = daemon.qcc(&[
        "reviewers",
        "configure",
        "--thread",
        &id,
        "--enabled",
        "on",
        "--profiles",
        "finder",
        "--delay",
        "1s",
    ]);
    assert!(configured.ok, "{}", configured.stderr);
    assert!(configured.stdout.contains("reviews on; profiles: finder"), "{}", configured.stdout);
    // The agent is told how to ask for a review.
    let skills = daemon.thread(&id)["skills_enabled"].clone();
    assert_eq!(skills, serde_json::json!(["qcc-reviewers"]));

    // The agent changes a file and stops, then starts again before the delay is over
    // and works past it: no review while it works.
    write(repo.join("a.txt"), "one\ntwo\n");
    daemon.turn(&id, "hello");
    daemon.turn(&id, &format!("/slow {}", words(150)));
    assert!(daemon.children(&id).is_empty(), "a review started while the agent worked");
    // Then it stays idle: one reviewer looks.
    let critic = eventually("started a reviewer", || daemon.children(&id).first().cloned());
    assert_eq!(critic["parent"], id.as_str());
    assert_eq!(critic["permission_mode"], "ask");
    let critic = critic["id"].as_str().unwrap().to_owned();
    assert!(daemon.qcc(&["thread", "wait", &critic]).ok);

    // Its finding becomes a comment on the line, marked as the reviewer's.
    let comment = eventually("left a comment", || daemon.comments(&id).first().cloned());
    assert_eq!(comment["path"], "a.txt");
    assert_eq!(comment["line"], 2);
    assert_eq!(comment["quote"], "two");
    assert_eq!(comment["text"], "Name this better");
    assert_eq!(comment["author"]["plugin"], "reviewers");
    assert_eq!(comment["author"]["name"], "finder");
    assert_eq!(comment["author"]["thread"], critic.as_str());
    assert_eq!(comment["author"]["severity"], "high");
    // Collect: the agent is not told; the user sends the comment when they want.
    assert!(!messages(&daemon.events(&id)).iter().any(|m| m.contains("qcc reviewers finished")));

    // Nothing changed since: another finished turn is not reviewed again.
    daemon.turn(&id, "again");
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(daemon.children(&id).len(), 1);
    let asked = daemon.qcc(&["reviewers", "request", "--thread", &id]);
    assert!(
        !asked.ok && asked.stderr.contains("nothing changed since review round 1"),
        "{}",
        asked.stderr
    );

    // Past the limit no round starts, even when asked.
    let limited = daemon.qcc(&["reviewers", "configure", "--thread", &id, "--max-rounds", "1"]);
    assert!(limited.ok, "{}", limited.stderr);
    write(repo.join("a.txt"), "one\ntwo\nthree\n");
    let asked = daemon.qcc(&["reviewers", "request", "--thread", &id]);
    assert!(!asked.ok && asked.stderr.contains("limit of 1 review rounds"), "{}", asked.stderr);
    // Counted again, the agent's own request starts one at once.
    assert!(daemon.qcc(&["reviewers", "reset", "--thread", &id]).ok);
    let asked = daemon.qcc_env(&["reviewers", "request"], &[("QCC_THREAD_ID", &id)]);
    assert!(asked.ok, "{}", asked.stderr);
    assert!(asked.stdout.contains("review round 1 started: finder"), "{}", asked.stdout);
    assert_eq!(daemon.children(&id).len(), 2);
}

#[test]
fn reviewers_send_their_findings_to_the_agent_in_one_message() {
    let dir = tempfile::tempdir().unwrap();
    let repo = repo(dir.path(), &[("a.txt", "one\n"), (".qcc/reviewers/finder.md", FINDER)]);
    let daemon = Daemon::start(&dir.path().join("data"));
    let id = daemon.spawn_in_dir(&repo, "first");
    let configured = daemon.qcc(&[
        "reviewers",
        "configure",
        "--thread",
        &id,
        "--enabled",
        "on",
        "--profiles",
        "finder,general",
        "--delivery",
        "auto",
        "--trigger",
        "every-turn",
    ]);
    assert!(configured.ok, "{}", configured.stderr);

    write(repo.join("a.txt"), "one\ntwo\n");
    daemon.turn(&id, "changed it");
    let reviewers = eventually("started both reviewers", || {
        let children = daemon.children(&id);
        (children.len() == 2).then_some(children)
    });
    for reviewer in &reviewers {
        assert!(daemon.qcc(&["thread", "wait", reviewer["id"].as_str().unwrap()]).ok);
    }

    // One message for the round, queued to the agent, which answers it.
    let events = daemon.eventually(&id, "was sent the findings", |events| {
        inputs(events).iter().any(|text| text.starts_with("qcc reviewers finished review round 1"))
    });
    let sent: Vec<&str> = inputs(&events)
        .into_iter()
        .filter(|text| text.starts_with("qcc reviewers finished"))
        .collect();
    assert_eq!(sent.len(), 1);
    let comment =
        daemon.comments(&id).iter().find(|c| c["author"]["name"] == "finder").cloned().unwrap();
    let id_of_comment = comment["id"].as_str().unwrap();
    assert!(
        sent[0].contains(&format!(
            "(high, from finder) [{id_of_comment}] at a.txt:2: Name this better"
        )),
        "{}",
        sent[0]
    );
    assert!(sent[0].contains("qcc comment resolve"), "{}", sent[0]);
    // The echo reviewer of the general profile answered with no findings object:
    // its words go along as one remark.
    assert!(sent[0].contains("from general)"), "{}", sent[0]);
    // The agent's answer to them is a turn of its own, which is not reviewed again.
    assert!(daemon.qcc(&["thread", "wait", &id]).ok);
    write(repo.join("a.txt"), "one\ntwo\nthree\n");
    std::thread::sleep(Duration::from_millis(1000));
    assert_eq!(daemon.children(&id).len(), 2);
}

#[test]
fn reviewers_turned_off_leave_nothing_behind() {
    let dir = tempfile::tempdir().unwrap();
    let repo = repo(dir.path(), &[("a.txt", "one\n"), (".qcc/reviewers/finder.md", FINDER)]);
    let daemon = Daemon::start(&dir.path().join("data"));
    let id = daemon.spawn_in_dir(&repo, "first");
    let configured = daemon.qcc(&[
        "reviewers",
        "configure",
        "--thread",
        &id,
        "--enabled",
        "on",
        "--profiles",
        "finder",
        "--trigger",
        "every-turn",
    ]);
    assert!(configured.ok, "{}", configured.stderr);

    let off = daemon.qcc(&["plugin", "disable", "reviewers"]);
    assert!(off.ok, "{}", off.stderr);
    // No commands, no panel, no skill.
    assert!(!daemon.qcc(&["reviewers", "configure", "--thread", &id]).ok);
    let commands = daemon.qcc(&["command", "list", "--json"]).stdout;
    assert!(!commands.contains("\"reviewers\""), "{commands}");
    let skills = daemon.qcc(&["skill", "list", "--json"]).stdout;
    assert!(!skills.contains("qcc-reviewers"), "{skills}");
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let views = runtime.block_on(async {
        let client = qcc_client::Client::connect(&daemon.socket).await.unwrap();
        client.call_raw("ui.contributions", serde_json::json!({})).await.unwrap()
    });
    assert!(!views.to_string().contains("reviewers"), "{views}");
    // No trigger: a finished turn with changes starts no reviewer.
    write(repo.join("a.txt"), "one\ntwo\n");
    daemon.turn(&id, "changed it");
    std::thread::sleep(Duration::from_millis(1000));
    assert!(daemon.children(&id).is_empty());

    // On again, the thread's set-up is still there and the next turn is reviewed.
    assert!(daemon.qcc(&["plugin", "enable", "reviewers"]).ok);
    let status = daemon.qcc(&["reviewers", "configure", "--thread", &id]);
    assert!(status.stdout.contains("reviews on; profiles: finder"), "{}", status.stdout);
    daemon.turn(&id, "once more");
    eventually("started a reviewer", || daemon.children(&id).first().cloned());
}

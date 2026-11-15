use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use qcc_plugin_host::services::{
    ProcessBackend, ProcessHandle, ProcessOutput, ProcessSink, SpawnOptions,
};
use serde_json::Value;

/// One step of a recorded process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptStep {
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
    /// Pause until the plugin writes anything to stdin.
    AwaitStdin,
    /// Pause until the plugin writes a line with this [`message_key`]. Other writes
    /// are recorded and do not resume the script, so a bridge may say more than the
    /// recorded client did.
    AwaitMessage(String),
    Exit(Option<i32>),
}

impl ScriptStep {
    /// A stdout line; the newline is added.
    pub fn line(text: &str) -> Self {
        Self::Stdout(format!("{text}\n").into_bytes())
    }
}

/// What kind of message a JSON line is, ignoring its payload: the method of a
/// JSON-RPC request or notification, the id of a response, or the `type` of a
/// stream-json message (with the control request's subtype).
pub fn message_key(line: &str) -> Option<String> {
    let value: Value = serde_json::from_str(line).ok()?;
    if let Some(method) = value.get("method").and_then(Value::as_str) {
        return Some(format!("method:{method}"));
    }
    if let Some(kind) = value.get("type").and_then(Value::as_str) {
        let subtype = value.pointer("/request/subtype").and_then(Value::as_str);
        return Some(match subtype {
            Some(subtype) => format!("type:{kind}:{subtype}"),
            None => format!("type:{kind}"),
        });
    }
    value.get("id").map(|id| format!("response:{id}"))
}

/// Turn a recording into a script. A recording is NDJSON, one
/// `{"dir": "in" | "out" | "err", "line": "…"}` per line crossing the agent's stdio:
/// `out` and `err` lines are replayed, and each `in` line pauses the replay until the
/// bridge under test writes a message of the same kind.
pub fn script_from_recording(recording: &str) -> Vec<ScriptStep> {
    recording
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|entry| {
            let entry: Value = serde_json::from_str(entry).expect("recording entry is JSON");
            let line = entry["line"].as_str().expect("recording entry has a line");
            match entry["dir"].as_str().expect("recording entry has a dir") {
                "out" => ScriptStep::line(line),
                "err" => ScriptStep::Stderr(format!("{line}\n").into_bytes()),
                "in" => match message_key(line) {
                    Some(key) => ScriptStep::AwaitMessage(key),
                    None => ScriptStep::AwaitStdin,
                },
                other => panic!("unknown direction {other:?} in recording"),
            }
        })
        .collect()
}

#[derive(Default)]
struct Recording {
    spawns: Vec<SpawnOptions>,
    stdin: Vec<u8>,
    killed: usize,
}

/// A [`ProcessBackend`] that replays recorded traffic instead of running anything.
///
/// Each spawn takes the next script. Output steps play immediately, in order, on the
/// caller's thread, so a replay is deterministic.
#[derive(Default)]
pub struct ScriptedProcesses {
    scripts: Mutex<VecDeque<Vec<ScriptStep>>>,
    recording: Arc<Mutex<Recording>>,
}

impl ScriptedProcesses {
    pub fn new(scripts: impl IntoIterator<Item = Vec<ScriptStep>>) -> Arc<Self> {
        Arc::new(Self {
            scripts: Mutex::new(scripts.into_iter().collect()),
            recording: Arc::default(),
        })
    }

    /// One process per recording, in spawn order. See [`script_from_recording`].
    pub fn from_recordings<'a>(recordings: impl IntoIterator<Item = &'a str>) -> Arc<Self> {
        Self::new(recordings.into_iter().map(script_from_recording))
    }

    /// Every spawn the plugin asked for, in order.
    pub fn spawns(&self) -> Vec<SpawnOptions> {
        self.recording.lock().unwrap().spawns.clone()
    }

    /// Everything the plugin wrote to stdin of any process.
    pub fn stdin(&self) -> String {
        String::from_utf8_lossy(&self.recording.lock().unwrap().stdin).into_owned()
    }

    /// The JSON messages the plugin wrote to stdin, in order.
    pub fn stdin_messages(&self) -> Vec<Value> {
        self.stdin().lines().filter_map(|line| serde_json::from_str(line).ok()).collect()
    }

    pub fn kills(&self) -> usize {
        self.recording.lock().unwrap().killed
    }

    /// Scripts no spawn has taken yet.
    pub fn unused_scripts(&self) -> usize {
        self.scripts.lock().unwrap().len()
    }
}

impl ProcessBackend for ScriptedProcesses {
    fn spawn(
        &self,
        options: SpawnOptions,
        sink: ProcessSink,
    ) -> Result<Box<dyn ProcessHandle>, String> {
        let script = self
            .scripts
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| format!("no script left for {:?}", options.program))?;
        self.recording.lock().unwrap().spawns.push(options);
        let mut process = ScriptedProcess {
            steps: script.into(),
            sink,
            recording: self.recording.clone(),
            written: Vec::new(),
            exited: false,
        };
        process.play();
        Ok(Box::new(process))
    }
}

struct ScriptedProcess {
    steps: VecDeque<ScriptStep>,
    sink: ProcessSink,
    recording: Arc<Mutex<Recording>>,
    /// Keys of written messages no pause has consumed yet.
    written: Vec<String>,
    exited: bool,
}

impl ScriptedProcess {
    /// Play steps up to the next pause that nothing written so far satisfies.
    fn play(&mut self) {
        while let Some(step) = self.steps.front() {
            match step {
                ScriptStep::AwaitStdin => return,
                ScriptStep::AwaitMessage(key) => {
                    let Some(index) = self.written.iter().position(|written| written == key) else {
                        return;
                    };
                    self.written.remove(index);
                    self.steps.pop_front();
                }
                ScriptStep::Stdout(_) | ScriptStep::Stderr(_) | ScriptStep::Exit(_) => {
                    match self.steps.pop_front() {
                        Some(ScriptStep::Stdout(data)) => (self.sink)(ProcessOutput::Stdout(data)),
                        Some(ScriptStep::Stderr(data)) => (self.sink)(ProcessOutput::Stderr(data)),
                        Some(ScriptStep::Exit(code)) => {
                            self.exited = true;
                            self.steps.clear();
                            (self.sink)(ProcessOutput::Exit(code));
                        }
                        _ => unreachable!("matched above"),
                    }
                }
            }
        }
    }
}

impl ProcessHandle for ScriptedProcess {
    fn write_stdin(&mut self, data: &[u8]) -> Result<(), String> {
        if self.exited {
            return Err("process has exited".into());
        }
        self.recording.lock().unwrap().stdin.extend_from_slice(data);
        if matches!(self.steps.front(), Some(ScriptStep::AwaitStdin)) {
            self.steps.pop_front();
        }
        for line in String::from_utf8_lossy(data).lines() {
            self.written.extend(message_key(line));
        }
        self.play();
        Ok(())
    }

    fn close_stdin(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn kill(&mut self) -> Result<(), String> {
        if !self.exited {
            self.exited = true;
            self.steps.clear();
            self.recording.lock().unwrap().killed += 1;
            (self.sink)(ProcessOutput::Exit(None));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_ignore_payloads() {
        let key = |line| message_key(line).unwrap();
        assert_eq!(
            key(r#"{"jsonrpc":"2.0","id":7,"method":"turn/start","params":{}}"#),
            "method:turn/start"
        );
        assert_eq!(key(r#"{"id":0,"result":{"decision":"accept"}}"#), "response:0");
        assert_eq!(key(r#"{"type":"user","message":{}}"#), "type:user");
        assert_eq!(
            key(r#"{"type":"control_request","request_id":"x","request":{"subtype":"interrupt"}}"#),
            "type:control_request:interrupt"
        );
        assert_eq!(message_key("plain text"), None);
    }

    #[test]
    fn replay_pauses_for_matching_messages_only() {
        let recording = r#"
            {"dir":"in","line":"{\"id\":1,\"method\":\"initialize\"}"}
            {"dir":"out","line":"{\"id\":1,\"result\":{}}"}
            {"dir":"in","line":"{\"method\":\"initialized\"}"}
            {"dir":"in","line":"{\"id\":2,\"method\":\"thread/start\"}"}
            {"dir":"err","line":"warming up"}
            {"dir":"out","line":"{\"id\":2,\"result\":{}}"}
        "#;
        let processes = ScriptedProcesses::from_recordings([recording]);
        let (tx, rx) = std::sync::mpsc::channel();
        let sink: ProcessSink = Arc::new(move |output| tx.send(output).unwrap());
        let options =
            SpawnOptions { program: "agent".into(), args: vec![], env: vec![], cwd: None };
        let mut process = processes.spawn(options, sink).unwrap();
        let drain = || rx.try_iter().count();

        assert_eq!(drain(), 0, "nothing plays before the first request");
        process.write_stdin(b"{\"id\":9,\"method\":\"extra\"}\n").unwrap();
        assert_eq!(drain(), 0, "an unrecorded message does not resume the replay");
        process.write_stdin(b"{\"id\":1,\"method\":\"initialize\"}\n").unwrap();
        assert_eq!(drain(), 1);
        // Written in the opposite order of the recording: both are still consumed.
        process.write_stdin(b"{\"id\":2,\"method\":\"thread/start\"}\n").unwrap();
        assert_eq!(drain(), 0);
        process.write_stdin(b"{\"method\":\"initialized\"}\n").unwrap();
        assert_eq!(drain(), 2);
        assert_eq!(processes.stdin_messages().len(), 4);

        process.kill().unwrap();
        assert_eq!(rx.try_iter().collect::<Vec<_>>(), [ProcessOutput::Exit(None)]);
        assert_eq!(processes.kills(), 1);
    }
}

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use qcc_plugin_host::services::{
    ProcessBackend, ProcessHandle, ProcessOutput, ProcessSink, SpawnOptions,
};

/// One step of a recorded process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptStep {
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
    /// Pause until the plugin writes to stdin (one write resumes one pause).
    AwaitStdin,
    Exit(Option<i32>),
}

impl ScriptStep {
    /// A stdout line; the newline is added.
    pub fn line(text: &str) -> Self {
        Self::Stdout(format!("{text}\n").into_bytes())
    }
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

    /// Every spawn the plugin asked for, in order.
    pub fn spawns(&self) -> Vec<SpawnOptions> {
        self.recording.lock().unwrap().spawns.clone()
    }

    /// Everything the plugin wrote to stdin of any process.
    pub fn stdin(&self) -> String {
        String::from_utf8_lossy(&self.recording.lock().unwrap().stdin).into_owned()
    }

    pub fn kills(&self) -> usize {
        self.recording.lock().unwrap().killed
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
    exited: bool,
}

impl ScriptedProcess {
    /// Play steps up to the next pause.
    fn play(&mut self) {
        while let Some(step) = self.steps.pop_front() {
            match step {
                ScriptStep::Stdout(data) => (self.sink)(ProcessOutput::Stdout(data)),
                ScriptStep::Stderr(data) => (self.sink)(ProcessOutput::Stderr(data)),
                ScriptStep::AwaitStdin => return,
                ScriptStep::Exit(code) => {
                    self.exited = true;
                    self.steps.clear();
                    (self.sink)(ProcessOutput::Exit(code));
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

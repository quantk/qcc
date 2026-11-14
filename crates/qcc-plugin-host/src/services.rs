//! The outside world as a plugin sees it. Everything is a trait so tests and the
//! conformance kit can substitute a scripted world.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use qcc_domain::{Delta, ThreadId};

/// Receives the deltas a plugin emits: `(plugin id, thread, deltas)`.
pub type DeltaSink = Arc<dyn Fn(&str, ThreadId, Vec<Delta>) + Send + Sync>;

/// Told when a plugin traps: `(plugin id, message)`. The plugin handles nothing after.
pub type CrashSink = Arc<dyn Fn(&str, &str) + Send + Sync>;

/// What one plugin instance is wired to.
pub struct Services {
    pub deltas: DeltaSink,
    pub kv: Box<dyn KvStore>,
    pub processes: Arc<dyn ProcessBackend>,
    pub on_crash: CrashSink,
}

impl Services {
    /// Deltas go to `deltas`; the key-value store lives in memory, processes are real.
    pub fn new(deltas: DeltaSink) -> Self {
        Self {
            deltas,
            kv: Box::new(MemoryKv::default()),
            processes: Arc::new(OsProcesses),
            on_crash: Arc::new(|_, _| {}),
        }
    }
}

// ----- key-value store ------------------------------------------------------------

pub const KV_MAX_KEY_BYTES: usize = 120;
pub const KV_MAX_VALUE_BYTES: usize = 1024 * 1024;

/// A plugin's private store.
pub trait KvStore: Send {
    fn get(&mut self, key: &str) -> Result<Option<Vec<u8>>, String>;
    fn put(&mut self, key: &str, value: &[u8]) -> Result<(), String>;
    fn remove(&mut self, key: &str) -> Result<(), String>;
    fn keys(&mut self, prefix: &str) -> Result<Vec<String>, String>;
}

fn check_kv(key: &str, value_len: usize) -> Result<(), String> {
    if key.is_empty() || key.len() > KV_MAX_KEY_BYTES {
        return Err(format!("key must be 1..={KV_MAX_KEY_BYTES} bytes"));
    }
    if value_len > KV_MAX_VALUE_BYTES {
        return Err(format!("value exceeds {KV_MAX_VALUE_BYTES} bytes"));
    }
    Ok(())
}

#[derive(Default)]
pub struct MemoryKv {
    entries: BTreeMap<String, Vec<u8>>,
}

impl KvStore for MemoryKv {
    fn get(&mut self, key: &str) -> Result<Option<Vec<u8>>, String> {
        Ok(self.entries.get(key).cloned())
    }

    fn put(&mut self, key: &str, value: &[u8]) -> Result<(), String> {
        check_kv(key, value.len())?;
        self.entries.insert(key.to_owned(), value.to_vec());
        Ok(())
    }

    fn remove(&mut self, key: &str) -> Result<(), String> {
        self.entries.remove(key);
        Ok(())
    }

    fn keys(&mut self, prefix: &str) -> Result<Vec<String>, String> {
        Ok(self
            .entries
            .keys()
            .filter(|key| key.starts_with(prefix))
            .cloned()
            .collect())
    }
}

/// One file per key in a directory. File names are the hex of the key, so any key is
/// a safe file name.
pub struct DirKv {
    dir: PathBuf,
}

impl DirKv {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn path(&self, key: &str) -> PathBuf {
        let name: String = key.bytes().map(|byte| format!("{byte:02x}")).collect();
        self.dir.join(name)
    }
}

fn unhex(name: &str) -> Option<String> {
    if !name.len().is_multiple_of(2) {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..name.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(name.get(i..i + 2)?, 16).ok())
        .collect();
    String::from_utf8(bytes?).ok()
}

impl KvStore for DirKv {
    fn get(&mut self, key: &str) -> Result<Option<Vec<u8>>, String> {
        match std::fs::read(self.path(key)) {
            Ok(value) => Ok(Some(value)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    fn put(&mut self, key: &str, value: &[u8]) -> Result<(), String> {
        check_kv(key, value.len())?;
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        // Write-then-rename so a crash never leaves a torn value.
        let path = self.path(key);
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, value).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
    }

    fn remove(&mut self, key: &str) -> Result<(), String> {
        match std::fs::remove_file(self.path(key)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }

    fn keys(&mut self, prefix: &str) -> Result<Vec<String>, String> {
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.to_string()),
        };
        let mut keys: Vec<String> = entries
            .filter_map(|entry| unhex(entry.ok()?.file_name().to_str()?))
            .filter(|key| key.starts_with(prefix))
            .collect();
        keys.sort();
        Ok(keys)
    }
}

// ----- processes ------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnOptions {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessOutput {
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
    /// Sent last, after all output. `None` when a signal ended the process.
    Exit(Option<i32>),
}

/// Where a process reports to. Called from backend threads.
pub type ProcessSink = Arc<dyn Fn(ProcessOutput) + Send + Sync>;

pub trait ProcessBackend: Send + Sync {
    fn spawn(
        &self,
        options: SpawnOptions,
        sink: ProcessSink,
    ) -> Result<Box<dyn ProcessHandle>, String>;
}

pub trait ProcessHandle: Send {
    fn write_stdin(&mut self, data: &[u8]) -> Result<(), String>;
    fn close_stdin(&mut self) -> Result<(), String>;
    fn kill(&mut self) -> Result<(), String>;
}

/// Variables of the daemon's environment a child may see. Everything else — secrets
/// included — has to be passed explicitly by the plugin.
const INHERITED_ENV: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "TERM",
    "TMPDIR",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_CACHE_HOME",
    "XDG_STATE_HOME",
    "XDG_RUNTIME_DIR",
    "SSL_CERT_FILE",
    "NIX_SSL_CERT_FILE",
];

/// Real operating-system processes.
pub struct OsProcesses;

impl ProcessBackend for OsProcesses {
    fn spawn(
        &self,
        options: SpawnOptions,
        sink: ProcessSink,
    ) -> Result<Box<dyn ProcessHandle>, String> {
        let mut command = Command::new(&options.program);
        command.args(&options.args).env_clear();
        for name in INHERITED_ENV {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        command.envs(options.env.iter().map(|(name, value)| (name, value)));
        if let Some(cwd) = &options.cwd {
            command.current_dir(cwd);
        }
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|error| format!("cannot run {:?}: {error}", options.program))?;

        let stdin = child.stdin.take();
        let stdout = child.stdout.take().expect("stdout is piped");
        let stderr = child.stderr.take().expect("stderr is piped");
        let child = Arc::new(Mutex::new(child));

        let out = pump(stdout, sink.clone(), ProcessOutput::Stdout);
        let err = pump(stderr, sink.clone(), ProcessOutput::Stderr);
        let waiter = child.clone();
        std::thread::spawn(move || {
            // Output first: the exit event is the last thing a process says.
            let _ = out.join();
            let _ = err.join();
            let code = loop {
                // Poll instead of blocking in `wait` so `kill` can take the lock.
                match waiter.lock().unwrap_or_else(|p| p.into_inner()).try_wait() {
                    Ok(Some(status)) => break status.code(),
                    Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                    Err(_) => break None,
                }
            };
            sink(ProcessOutput::Exit(code));
        });

        Ok(Box::new(OsProcess { child, stdin }))
    }
}

fn pump(
    mut stream: impl Read + Send + 'static,
    sink: ProcessSink,
    wrap: fn(Vec<u8>) -> ProcessOutput,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut buffer = [0u8; 16 * 1024];
        loop {
            match stream.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => sink(wrap(buffer[..read].to_vec())),
            }
        }
    })
}

struct OsProcess {
    child: Arc<Mutex<Child>>,
    stdin: Option<ChildStdin>,
}

impl ProcessHandle for OsProcess {
    fn write_stdin(&mut self, data: &[u8]) -> Result<(), String> {
        let stdin = self.stdin.as_mut().ok_or("stdin is closed")?;
        stdin
            .write_all(data)
            .and_then(|()| stdin.flush())
            .map_err(|error| error.to_string())
    }

    fn close_stdin(&mut self) -> Result<(), String> {
        self.stdin = None;
        Ok(())
    }

    fn kill(&mut self) -> Result<(), String> {
        self.stdin = None;
        self.child
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .kill()
            .map_err(|error| error.to_string())
    }
}

impl Drop for OsProcess {
    fn drop(&mut self) {
        // A plugin that goes away takes its processes with it.
        let _ = self.kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exercise(kv: &mut dyn KvStore) {
        assert_eq!(kv.get("a/b").unwrap(), None);
        kv.put("a/b", b"1").unwrap();
        kv.put("a/c", b"2").unwrap();
        kv.put("z", b"").unwrap();
        kv.put("a/b", b"3").unwrap();
        assert_eq!(kv.get("a/b").unwrap().as_deref(), Some(&b"3"[..]));
        assert_eq!(kv.keys("a/").unwrap(), ["a/b", "a/c"]);
        assert_eq!(kv.keys("").unwrap().len(), 3);
        kv.remove("a/b").unwrap();
        kv.remove("a/b").unwrap();
        assert_eq!(kv.keys("a/").unwrap(), ["a/c"]);
        assert!(kv.put("", b"x").is_err());
        assert!(kv.put(&"k".repeat(KV_MAX_KEY_BYTES + 1), b"x").is_err());
        assert!(kv.put("big", &vec![0; KV_MAX_VALUE_BYTES + 1]).is_err());
    }

    #[test]
    fn memory_kv() {
        exercise(&mut MemoryKv::default());
    }

    #[test]
    fn dir_kv_persists() {
        let dir = tempfile::tempdir().unwrap();
        exercise(&mut DirKv::new(dir.path().join("kv")));
        let mut reopened = DirKv::new(dir.path().join("kv"));
        assert_eq!(reopened.get("a/c").unwrap().as_deref(), Some(&b"2"[..]));
        assert_eq!(reopened.keys("").unwrap(), ["a/c", "z"]);
    }

    #[test]
    fn os_process_streams_output_then_exit() {
        let (tx, rx) = std::sync::mpsc::channel();
        let sink: ProcessSink = Arc::new(move |output| {
            let _ = tx.send(output);
        });
        let mut process = OsProcesses
            .spawn(
                SpawnOptions {
                    program: "sh".into(),
                    args: vec![
                        "-c".into(),
                        "read line; echo \"got $line $SECRET $QCC\"; exit 3".into(),
                    ],
                    env: vec![("QCC".into(), "yes".into())],
                    cwd: None,
                },
                sink,
            )
            .unwrap();
        process.write_stdin(b"hello\n").unwrap();
        let events: Vec<_> = rx
            .iter()
            .take_while(|e| !matches!(e, ProcessOutput::Exit(_)))
            .collect();
        // The daemon's own environment does not leak: `$SECRET` is empty even if set.
        assert_eq!(
            events,
            [ProcessOutput::Stdout(b"got hello  yes\n".to_vec())]
        );
        process.close_stdin().unwrap();
        assert!(process.write_stdin(b"x").is_err());

        assert!(
            OsProcesses
                .spawn(
                    SpawnOptions {
                        program: "qcc-no-such-program".into(),
                        args: vec![],
                        env: vec![],
                        cwd: None
                    },
                    Arc::new(|_| {}),
                )
                .is_err()
        );
    }

    #[test]
    fn os_process_kill() {
        let (tx, rx) = std::sync::mpsc::channel();
        let sink: ProcessSink = Arc::new(move |output| {
            let _ = tx.send(output);
        });
        let mut process = OsProcesses
            .spawn(
                SpawnOptions {
                    program: "sleep".into(),
                    args: vec!["30".into()],
                    env: vec![],
                    cwd: None,
                },
                sink,
            )
            .unwrap();
        process.kill().unwrap();
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            ProcessOutput::Exit(None)
        );
    }
}

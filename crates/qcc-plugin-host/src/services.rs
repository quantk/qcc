//! The outside world as a plugin sees it. Everything is a trait so tests and the
//! conformance kit can substitute a scripted world.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use qcc_domain::{Delta, ThreadId};

use crate::api::{ThreadsAnswer, ThreadsCall};

/// Receives the deltas a plugin emits: `(plugin id, thread, deltas)`.
pub type DeltaSink = Arc<dyn Fn(&str, ThreadId, Vec<Delta>) + Send + Sync>;

/// Told when a plugin traps: `(plugin id, message)`. The plugin handles nothing after.
pub type CrashSink = Arc<dyn Fn(&str, &str) + Send + Sync>;

/// Carries out what a plugin asks of threads: `(plugin id, call)`. Called on the
/// plugin's own thread, which waits for the answer.
pub type ThreadsBackend =
    Arc<dyn Fn(&str, ThreadsCall) -> Result<ThreadsAnswer, String> + Send + Sync>;

/// What one plugin instance is wired to.
pub struct Services {
    pub deltas: DeltaSink,
    pub kv: Box<dyn KvStore>,
    pub processes: Arc<dyn ProcessBackend>,
    pub threads: ThreadsBackend,
    pub on_crash: CrashSink,
}

impl Services {
    /// Deltas go to `deltas`; the key-value store lives in memory, processes are real.
    pub fn new(deltas: DeltaSink) -> Self {
        Self {
            deltas,
            kv: Box::new(MemoryKv::default()),
            processes: Arc::new(OsProcesses),
            threads: Arc::new(|_, _| Err("no daemon is attached to answer for threads".into())),
            on_crash: Arc::new(|_, _| {}),
        }
    }
}

// ----- key-value store ------------------------------------------------------------

pub const KV_MAX_KEY_BYTES: usize = 120;
pub const KV_MAX_VALUE_BYTES: usize = 1024 * 1024;
/// What one plugin may keep in its store altogether.
pub const KV_MAX_TOTAL_BYTES: usize = 64 * 1024 * 1024;
pub const KV_MAX_KEYS: usize = 10_000;

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

/// How much of its quota a store uses.
#[derive(Debug, Clone, Copy, Default)]
struct Usage {
    keys: usize,
    bytes: usize,
}

impl Usage {
    /// Account for `key` going from `old` bytes (absent: `None`) to `new`, or refuse
    /// when that would exceed the quota.
    fn replace(&mut self, old: Option<usize>, new: usize) -> Result<(), String> {
        let keys = self.keys + usize::from(old.is_none());
        let bytes = self.bytes - old.unwrap_or(0).min(self.bytes) + new;
        if keys > KV_MAX_KEYS {
            return Err(format!("the store holds {KV_MAX_KEYS} keys already"));
        }
        if bytes > KV_MAX_TOTAL_BYTES {
            return Err(format!("the store would exceed {KV_MAX_TOTAL_BYTES} bytes"));
        }
        *self = Self { keys, bytes };
        Ok(())
    }

    fn remove(&mut self, old: usize) {
        self.keys = self.keys.saturating_sub(1);
        self.bytes = self.bytes.saturating_sub(old);
    }
}

#[derive(Default)]
pub struct MemoryKv {
    entries: BTreeMap<String, Vec<u8>>,
    usage: Usage,
}

impl KvStore for MemoryKv {
    fn get(&mut self, key: &str) -> Result<Option<Vec<u8>>, String> {
        Ok(self.entries.get(key).cloned())
    }

    fn put(&mut self, key: &str, value: &[u8]) -> Result<(), String> {
        check_kv(key, value.len())?;
        self.usage.replace(self.entries.get(key).map(Vec::len), value.len())?;
        self.entries.insert(key.to_owned(), value.to_vec());
        Ok(())
    }

    fn remove(&mut self, key: &str) -> Result<(), String> {
        if let Some(old) = self.entries.remove(key) {
            self.usage.remove(old.len());
        }
        Ok(())
    }

    fn keys(&mut self, prefix: &str) -> Result<Vec<String>, String> {
        Ok(self.entries.keys().filter(|key| key.starts_with(prefix)).cloned().collect())
    }
}

/// One file per key in a directory. File names are the hex of the key, so any key is
/// a safe file name.
pub struct DirKv {
    dir: PathBuf,
    /// Counted from the directory on the first write, then kept up to date.
    usage: Option<Usage>,
}

impl DirKv {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into(), usage: None }
    }

    fn usage(&mut self) -> Result<&mut Usage, String> {
        if self.usage.is_none() {
            let mut usage = Usage::default();
            match std::fs::read_dir(&self.dir) {
                Ok(entries) => {
                    for entry in entries.flatten() {
                        let is_key = entry.file_name().to_str().and_then(unhex).is_some();
                        if let (true, Ok(meta)) = (is_key, entry.metadata()) {
                            usage.keys += 1;
                            usage.bytes += usize::try_from(meta.len()).unwrap_or(usize::MAX);
                        }
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.to_string()),
            }
            self.usage = Some(usage);
        }
        Ok(self.usage.get_or_insert_default())
    }

    fn write(&self, key: &str, value: &[u8]) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        // Write-then-rename so a crash never leaves a torn value.
        let path = self.path(key);
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, value).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
    }

    /// The size of the value stored under `key`, if there is one.
    fn stored(&self, key: &str) -> Option<usize> {
        let len = std::fs::metadata(self.path(key)).ok()?.len();
        Some(usize::try_from(len).unwrap_or(usize::MAX))
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
        let old = self.stored(key);
        let before = *self.usage()?;
        self.usage()?.replace(old, value.len())?;
        let written = self.write(key, value);
        if written.is_err() {
            self.usage = Some(before);
        }
        written
    }

    fn remove(&mut self, key: &str) -> Result<(), String> {
        let old = self.stored(key);
        match std::fs::remove_file(self.path(key)) {
            Ok(()) => {
                if let (Some(old), Some(usage)) = (old, self.usage.as_mut()) {
                    usage.remove(old);
                }
                Ok(())
            }
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
        let program = resolve_program(&options.program)?;
        let mut command = Command::new(program);
        command.args(&options.args).env_clear();
        for name in INHERITED_ENV {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        for (name, value) in &options.env {
            if loader_variable(name) {
                return Err(format!("{name} cannot be set for a child process"));
            }
            command.env(name, value);
        }
        if let Some(cwd) = &options.cwd {
            command.current_dir(cwd);
        }
        command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
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

/// Where the program a plugin names lives, looked up on the daemon's own `PATH`.
///
/// The permission names a program, and that is the program that runs: left to
/// `Command`, a bare name would be looked up on the `PATH` the plugin passes to the
/// child, and any directory there could provide a program of that name.
fn resolve_program(program: &str) -> Result<PathBuf, String> {
    if program.contains('/') {
        return Ok(PathBuf::from(program));
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .map(|dir| dir.join(program))
        .find(|candidate| is_executable(candidate))
        .ok_or_else(|| format!("cannot run {program:?}: not found on PATH"))
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata().is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

/// Variables that make the dynamic loader run code of the plugin's choosing in a
/// program it was only allowed to start.
fn loader_variable(name: &str) -> bool {
    name.starts_with("LD_") || name.starts_with("DYLD_")
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
        stdin.write_all(data).and_then(|()| stdin.flush()).map_err(|error| error.to_string())
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

    fn quota(kv: &mut dyn KvStore) {
        let value = vec![0; KV_MAX_VALUE_BYTES];
        let fits = KV_MAX_TOTAL_BYTES / KV_MAX_VALUE_BYTES;
        for n in 0..fits {
            kv.put(&format!("k{n}"), &value).unwrap();
        }
        assert!(kv.put("one-more", &value).is_err(), "the quota let a value through");
        // Replacing a value counts only the difference; removing frees its share.
        kv.put("k0", &value).unwrap();
        kv.remove("k0").unwrap();
        kv.put("one-more", &value).unwrap();
    }

    #[test]
    fn stores_have_a_quota() {
        quota(&mut MemoryKv::default());
        let dir = tempfile::tempdir().unwrap();
        quota(&mut DirKv::new(dir.path().join("kv")));
        // A reopened store counts what is already there.
        let mut reopened = DirKv::new(dir.path().join("kv"));
        assert!(reopened.put("again", &[0; KV_MAX_VALUE_BYTES]).is_err());
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
        let events: Vec<_> =
            rx.iter().take_while(|e| !matches!(e, ProcessOutput::Exit(_))).collect();
        // The daemon's own environment does not leak: `$SECRET` is empty even if set.
        assert_eq!(events, [ProcessOutput::Stdout(b"got hello  yes\n".to_vec())]);
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
    fn a_plugin_cannot_choose_which_program_a_name_means() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("echo");
        std::fs::write(&fake, "#!/bin/sh\necho fake\n").unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();

        let (tx, rx) = std::sync::mpsc::channel();
        let sink: ProcessSink = Arc::new(move |output| {
            let _ = tx.send(output);
        });
        let path = dir.path().to_str().unwrap().to_owned();
        let _process = OsProcesses
            .spawn(
                SpawnOptions {
                    program: "echo".into(),
                    args: vec!["real".into()],
                    env: vec![("PATH".into(), path)],
                    cwd: None,
                },
                sink,
            )
            .unwrap();
        let events: Vec<_> =
            rx.iter().take_while(|e| !matches!(e, ProcessOutput::Exit(_))).collect();
        assert_eq!(events, [ProcessOutput::Stdout(b"real\n".to_vec())]);

        let preload = OsProcesses.spawn(
            SpawnOptions {
                program: "echo".into(),
                args: vec![],
                env: vec![("LD_PRELOAD".into(), "/tmp/evil.so".into())],
                cwd: None,
            },
            Arc::new(|_| {}),
        );
        assert!(preload.is_err());
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
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), ProcessOutput::Exit(None));
    }
}

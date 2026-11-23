//! Opening a thread's directory in the user's editor.
//!
//! The editor is the command the user set in the app's settings, else the first of
//! the common GUI editors found, else whatever the system opens a directory with.
//!
//! An app started from a desktop menu does not get the PATH of the user's shell: on
//! most systems it gets the session's, which often lacks `~/.local/bin`, a Nix
//! profile or Homebrew. So programs are looked up in the app's own PATH, then in the
//! PATH a login shell sets up, then in the usual places, and the editor is started
//! with all of them in its PATH, since editors' launchers are often scripts that run
//! other programs.

use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::sync::mpsc;
use std::time::Duration;

/// GUI editors tried, in this order, when the user did not name one. Terminal editors
/// are left out: without a terminal they would have nowhere to show.
const CANDIDATES: &[&str] = &[
    "code",
    "cursor",
    "zed",
    // Zed's program on NixOS and some other distributions.
    "zeditor",
    "windsurf",
    "codium",
    "subl",
    "idea",
    "idea-ultimate",
    "idea-community",
];

/// How a word of the editor's command stands for the directory.
const PATH_PLACEHOLDER: &str = "{path}";
/// A login shell that does not answer by then (it waits for input, say) is given up.
const SHELL_TIMEOUT: Duration = Duration::from_secs(3);
/// An editor that fails at once, because of a bad flag for instance, does it within
/// this; one that is still running then is taken to have started.
const EARLY_EXIT: Duration = Duration::from_millis(1500);
/// How much of what a failed editor printed is shown.
const STDERR_TAIL: usize = 600;

/// What opened the directory, for the app to tell the user.
pub type Opened = String;

/// Open `dir` with `command` (the user's setting; empty or absent: find an editor).
pub fn open(dir: &Path, command: Option<&str>) -> Result<Opened, String> {
    if !dir.is_dir() {
        return Err(format!("{} does not exist", dir.display()));
    }
    let dirs = search_path();
    let command = command.map(str::trim).filter(|command| !command.is_empty());
    let (program, words) = match command {
        Some(command) => {
            let mut words = split_command(command)?;
            if words.is_empty() {
                return Err("the editor command is empty".into());
            }
            let name = words.remove(0);
            let program = find_program(&name, dirs).ok_or_else(|| {
                format!("cannot find `{name}`: it is not in PATH nor in the usual places")
            })?;
            (program, words)
        }
        None => match detect_in(dirs) {
            Some(program) => (program, Vec::new()),
            None => {
                tauri_plugin_opener::open_path(dir, None::<&str>)
                    .map_err(|error| format!("cannot open {}: {error}", dir.display()))?;
                return Ok("the system's default app".into());
            }
        },
    };
    let name = program.file_name().unwrap_or(program.as_os_str()).to_string_lossy().into_owned();
    spawn(&program, &arguments(&words, dir), dir, dirs)?;
    Ok(name)
}

/// The editor used when the user names none, as its full path.
pub fn detect() -> Option<PathBuf> {
    detect_in(search_path())
}

fn detect_in(dirs: &[PathBuf]) -> Option<PathBuf> {
    CANDIDATES.iter().find_map(|name| find_program(name, dirs))
}

/// Start the editor apart from the app, and report it if it fails at once.
fn spawn(program: &Path, args: &[OsString], dir: &Path, dirs: &[PathBuf]) -> Result<(), String> {
    let mut command = Command::new(program);
    command.args(args).current_dir(dir).stdin(Stdio::null()).stdout(Stdio::null());
    command.stderr(Stdio::piped());
    if let Ok(path) = std::env::join_paths(dirs) {
        command.env("PATH", path);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Its own process group: the editor outlives the app and its signals.
        command.process_group(0);
    }
    let mut child =
        command.spawn().map_err(|error| format!("cannot start {}: {error}", program.display()))?;
    let stderr = child.stderr.take();
    let (sender, receiver) = mpsc::channel();
    // The child is waited for (and its output drained) for as long as it runs, so it
    // leaves no zombie and never blocks on a full pipe.
    std::thread::spawn(move || {
        let mut tail = Vec::new();
        if let Some(mut stderr) = stderr {
            let mut chunk = [0u8; 4096];
            while let Ok(read) = stderr.read(&mut chunk) {
                if read == 0 {
                    break;
                }
                tail.extend_from_slice(&chunk[..read]);
                if tail.len() > STDERR_TAIL {
                    tail.drain(..tail.len() - STDERR_TAIL);
                }
            }
        }
        let status = child.wait();
        let _ = sender.send((status, tail));
    });
    match receiver.recv_timeout(EARLY_EXIT) {
        Ok((Ok(status), tail)) if !status.success() => {
            let said = String::from_utf8_lossy(&tail).trim().to_owned();
            let mut message = format!("{} exited with {status}", program.display());
            if !said.is_empty() {
                message.push_str(": ");
                message.push_str(&said);
            }
            Err(message)
        }
        _ => Ok(()),
    }
}

/// The words of a command, as a shell would split it: spaces part words, quotes
/// keep them together, a backslash outside single quotes takes the next character.
fn split_command(command: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    let mut chars = command.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(open), c) if c == open => quote = None,
            (Some('"') | None, '\\') => {
                word.push(chars.next().ok_or("the editor command ends with a backslash")?);
                in_word = true;
            }
            (Some(_), c) => word.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                in_word = true;
            }
            (None, c) if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            (None, c) => {
                word.push(c);
                in_word = true;
            }
        }
    }
    if quote.is_some() {
        return Err("the editor command has an unclosed quote".into());
    }
    if in_word {
        words.push(word);
    }
    Ok(words)
}

/// The editor's arguments: `{path}` in a word stands for the directory; a command
/// that does not mention it gets the directory as its last argument.
fn arguments(words: &[String], dir: &Path) -> Vec<OsString> {
    if !words.iter().any(|word| word.contains(PATH_PLACEHOLDER)) {
        let mut args: Vec<OsString> = words.iter().map(OsString::from).collect();
        args.push(dir.as_os_str().to_owned());
        return args;
    }
    words
        .iter()
        .map(|word| {
            if word == PATH_PLACEHOLDER {
                // Whole, so a path that is not UTF-8 passes intact.
                dir.as_os_str().to_owned()
            } else {
                OsString::from(word.replace(PATH_PLACEHOLDER, &dir.to_string_lossy()))
            }
        })
        .collect()
}

/// Where `name` is: as given when it is a path, else the first executable of that
/// name in `dirs`.
fn find_program(name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    let given = Path::new(name);
    if given.components().count() > 1 || given.is_absolute() {
        return is_executable(given).then(|| given.to_owned());
    }
    dirs.iter().flat_map(|dir| executables(dir, name)).find(|path| is_executable(path))
}

#[cfg(windows)]
fn executables(dir: &Path, name: &str) -> Vec<PathBuf> {
    let extensions = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
    std::iter::once(dir.join(name))
        .chain(
            extensions
                .split(';')
                .filter(|ext| !ext.is_empty())
                .map(|ext| dir.join(format!("{name}{ext}"))),
        )
        .collect()
}

#[cfg(not(windows))]
fn executables(dir: &Path, name: &str) -> Vec<PathBuf> {
    vec![dir.join(name)]
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata().is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// The directories programs are looked up in, found once: the app's PATH, the login
/// shell's and the usual places, without repeats.
fn search_path() -> &'static [PathBuf] {
    static DIRS: OnceLock<Vec<PathBuf>> = OnceLock::new();
    DIRS.get_or_init(|| {
        let own = std::env::var_os("PATH").map(|path| std::env::split_paths(&path).collect());
        merge([own.unwrap_or_default(), login_shell_path(), usual_dirs()])
    })
}

fn merge(lists: impl IntoIterator<Item = Vec<PathBuf>>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    for dir in lists.into_iter().flatten() {
        if !dir.as_os_str().is_empty() && !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    dirs
}

/// Where editors' launchers are put when the session's PATH does not say.
fn usual_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let user = std::env::var_os("USER");
    let mut dirs = Vec::new();
    if let Some(home) = &home {
        dirs.push(home.join(".local/bin"));
        dirs.push(home.join(".nix-profile/bin"));
        dirs.push(home.join("bin"));
    }
    if let Some(user) = user {
        dirs.push(Path::new("/etc/profiles/per-user").join(user).join("bin"));
    }
    for dir in [
        "/run/current-system/sw/bin",
        "/nix/var/nix/profiles/default/bin",
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/usr/bin",
        "/snap/bin",
        "/var/lib/flatpak/exports/bin",
    ] {
        dirs.push(PathBuf::from(dir));
    }
    if let Some(home) = &home {
        dirs.push(home.join(".local/share/flatpak/exports/bin"));
    }
    dirs
}

/// The PATH the user's login shell sets up, or nothing when it cannot be asked.
#[cfg(unix)]
fn login_shell_path() -> Vec<PathBuf> {
    const MARK: &str = "__qcc_path__";
    let shell = std::env::var_os("SHELL").unwrap_or_else(|| "/bin/sh".into());
    // Marked, so whatever the profile prints on its own is not taken for the PATH.
    // Quoted, fish joins its PATH with colons too.
    let script = format!("printf '{MARK}%s{MARK}' \"$PATH\"");
    let Ok(mut child) = Command::new(&shell)
        .args(["-l", "-c", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return Vec::new();
    };
    let stdout = child.stdout.take();
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut output = String::new();
        if let Some(mut stdout) = stdout {
            let _ = stdout.read_to_string(&mut output);
        }
        let _ = sender.send(output);
    });
    let output = receiver.recv_timeout(SHELL_TIMEOUT);
    // Whatever it is doing still, it is not needed any more.
    let _ = child.kill();
    let _ = child.wait();
    let Ok(output) = output else {
        return Vec::new();
    };
    marked(&output, MARK).map(parse_path).unwrap_or_default()
}

#[cfg(not(unix))]
fn login_shell_path() -> Vec<PathBuf> {
    Vec::new()
}

fn marked<'a>(output: &'a str, mark: &str) -> Option<&'a str> {
    let start = output.find(mark)? + mark.len();
    let length = output[start..].find(mark)?;
    Some(&output[start..start + length])
}

fn parse_path(path: &str) -> Vec<PathBuf> {
    std::env::split_paths(path).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(command: &str) -> Vec<String> {
        split_command(command).unwrap()
    }

    #[test]
    fn splits_a_command_as_a_shell_does() {
        assert_eq!(words("code"), ["code"]);
        assert_eq!(words("  idea  --new-window "), ["idea", "--new-window"]);
        assert_eq!(words(r#""/opt/My Editor/bin/ed" -n"#), ["/opt/My Editor/bin/ed", "-n"]);
        assert_eq!(words(r"my\ editor 'a\b' x"), ["my editor", r"a\b", "x"]);
        assert_eq!(words(r#"ed "" --flag"#), ["ed", "", "--flag"]);
        assert_eq!(words(""), Vec::<String>::new());
    }

    #[test]
    fn refuses_a_broken_command() {
        assert!(split_command("code 'open").is_err());
        assert!(split_command(r"code \").is_err());
    }

    #[test]
    fn passes_the_directory_last_or_in_place_of_the_placeholder() {
        let dir = Path::new("/work/my repo");
        assert_eq!(arguments(&[], dir), [OsString::from("/work/my repo")]);
        assert_eq!(arguments(&words("-n"), dir), ["-n", "/work/my repo"]);
        assert_eq!(arguments(&words("{path} -n"), dir), ["/work/my repo", "-n"]);
        assert_eq!(arguments(&words("--folder={path}"), dir), ["--folder=/work/my repo"]);
    }

    #[cfg(unix)]
    #[test]
    fn finds_executables_in_order_and_skips_other_files() {
        use std::os::unix::fs::PermissionsExt;
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let plain = first.path().join("zed");
        std::fs::write(&plain, "").unwrap();
        let program = second.path().join("zed");
        std::fs::write(&program, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let dirs = [first.path().to_owned(), second.path().to_owned()];

        assert_eq!(find_program("zed", &dirs), Some(program.clone()));
        assert_eq!(find_program(program.to_str().unwrap(), &[]), Some(program.clone()));
        assert_eq!(find_program(plain.to_str().unwrap(), &dirs), None);
        assert_eq!(find_program("code", &dirs), None);
        assert_eq!(detect_in(&dirs), Some(program));
    }

    #[test]
    fn reads_the_marked_path_and_merges_without_repeats() {
        assert_eq!(marked("motd\n__m__/a:/b__m__", "__m__"), Some("/a:/b"));
        assert_eq!(marked("no mark", "__m__"), None);
        let merged = merge([parse_path("/a:/b"), parse_path("/b::/c"), vec![PathBuf::from("/a")]]);
        assert_eq!(merged, [PathBuf::from("/a"), PathBuf::from("/b"), PathBuf::from("/c")]);
    }

    #[cfg(unix)]
    #[test]
    fn reports_an_editor_that_fails_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let error = spawn(
            Path::new("/bin/sh"),
            &[OsString::from("-c"), OsString::from("echo bad flag >&2; exit 3")],
            dir.path(),
            &[],
        )
        .unwrap_err();
        assert!(error.contains("bad flag"), "{error}");
        assert!(
            open(&dir.path().join("gone"), Some("code")).unwrap_err().contains("does not exist")
        );
    }
}

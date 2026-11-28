//! The `files` import: plugins read text files where a thread works and in their
//! own data folder, and nowhere else.

use std::io::Read;
use std::path::{Component, Path, PathBuf};

use qcc_plugin_host::FileRoot;
use qcc_protocol::methods::{ThreadPathsParams, ThreadPathsResult};

use super::{Core, Responder, thread_not_found};
use crate::paths::{DEFAULT_LIMIT, MAX_LIMIT};

/// The largest file a plugin reads whole.
const MAX_FILE_BYTES: u64 = 1024 * 1024;

impl Core {
    /// `thread.paths`: matched off the core task, since a scan can take a while.
    /// A working directory that is not there yet (its environment is being
    /// prepared) has no paths.
    pub(super) fn thread_paths(&self, p: ThreadPathsParams, responder: Responder) {
        let Some(thread) = self.supervisor.thread(&p.thread_id) else {
            return responder.reply::<ThreadPathsResult>(Err(thread_not_found(&p.thread_id)));
        };
        let root = PathBuf::from(&thread.cwd);
        let limit = p.limit.map_or(DEFAULT_LIMIT, |limit| (limit as usize).min(MAX_LIMIT));
        let index = self.paths.clone();
        tokio::task::spawn_blocking(move || {
            let (paths, incomplete) = if root.is_dir() {
                index.search(&root, &p.query, limit)
            } else {
                (Vec::new(), false)
            };
            responder.reply(Ok(ThreadPathsResult { paths, incomplete }));
        });
    }

    /// The folder a plugin's `files` call counts from.
    fn file_root(&self, plugin: &str, root: &FileRoot) -> Result<PathBuf, String> {
        match root {
            FileRoot::Own => Ok(self.config.plugin_data_dir(plugin)),
            FileRoot::Thread(id) => self
                .supervisor
                .thread(id)
                .map(|thread| PathBuf::from(&thread.cwd))
                .ok_or_else(|| format!("thread {id} does not exist")),
            FileRoot::WorkingDir(dir) => {
                let wanted = Path::new(dir);
                let wanted_real = wanted.canonicalize().ok();
                let known = self.supervisor.threads().any(|thread| {
                    let cwd = Path::new(&thread.cwd);
                    cwd == wanted
                        || (wanted_real.is_some() && cwd.canonicalize().ok() == wanted_real)
                });
                if known {
                    Ok(wanted.to_path_buf())
                } else {
                    Err(format!("{dir:?} is not a folder a thread works in"))
                }
            }
            // Which folders under it may be read, the host checked against the
            // plugin's `home-files` permission.
            FileRoot::Home => std::env::var_os("HOME")
                .filter(|home| !home.is_empty())
                .map(PathBuf::from)
                .ok_or_else(|| "the home folder is not known".to_owned()),
        }
    }

    pub(super) fn plugin_read_file(
        &self,
        plugin: &str,
        root: &FileRoot,
        path: &str,
    ) -> Result<Option<String>, String> {
        let Some(path) = inside(&self.file_root(plugin, root)?, path)? else {
            return Ok(None);
        };
        if !path.is_file() {
            return Ok(None);
        }
        let file =
            std::fs::File::open(&path).map_err(|error| format!("cannot read {path:?}: {error}"))?;
        let mut bytes = Vec::new();
        file.take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("cannot read {path:?}: {error}"))?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(format!("{path:?} is larger than {MAX_FILE_BYTES} bytes"));
        }
        String::from_utf8(bytes).map(Some).map_err(|_| format!("{path:?} is not text"))
    }

    pub(super) fn plugin_list_files(
        &self,
        plugin: &str,
        root: &FileRoot,
        dir: &str,
    ) -> Result<Vec<String>, String> {
        let Some(dir) = inside(&self.file_root(plugin, root)?, dir)? else {
            return Ok(Vec::new());
        };
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return Ok(Vec::new());
        };
        let mut names: Vec<String> =
            entries.flatten().filter_map(|entry| entry.file_name().into_string().ok()).collect();
        names.sort();
        Ok(names)
    }
}

/// `relative` under `root`, if it is there. A path that climbs out, by `..`, by
/// being absolute or through a link, is refused; one that does not exist is `None`.
fn inside(root: &Path, relative: &str) -> Result<Option<PathBuf>, String> {
    let relative = Path::new(relative);
    let plain =
        relative.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir));
    if !plain {
        return Err(format!("{relative:?} must be a relative path that stays inside its folder"));
    }
    let (Ok(root), Ok(path)) = (root.canonicalize(), root.join(relative).canonicalize()) else {
        return Ok(None);
    };
    if !path.starts_with(&root) {
        return Err(format!("{relative:?} leads out of its folder"));
    }
    Ok(Some(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_stays_inside_its_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir_all(root.join(".qcc/reviewers")).unwrap();
        std::fs::write(root.join(".qcc/reviewers/a.md"), "a").unwrap();
        std::fs::write(dir.path().join("secret"), "s").unwrap();

        let found = inside(&root, ".qcc/reviewers/a.md").unwrap().unwrap();
        assert!(found.ends_with(".qcc/reviewers/a.md"));
        assert_eq!(inside(&root, "missing.md").unwrap(), None);
        assert!(inside(&root, "../secret").is_err());
        assert!(inside(&root, "/etc/passwd").is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.path().join("secret"), root.join("link")).unwrap();
            assert!(inside(&root, "link").is_err(), "a link out of the root is refused");
        }
    }
}

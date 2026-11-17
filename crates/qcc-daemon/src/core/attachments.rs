//! Files attached to messages: qcc's copies of them, kept per thread under the data
//! directory, so a message reads the same later and an agent can always open them.

use std::path::{Path, PathBuf};

use base64::Engine as _;
use qcc_domain::{Attachment, ThreadId, guess_mime};
use qcc_protocol::jsonrpc::{RpcError, codes};
use qcc_protocol::methods::{NewAttachment, ThreadAttachmentParams, ThreadAttachmentResult};

use super::{Core, Responder, thread_not_found};

/// The largest file qcc takes as an attachment. Agents read files, not archives:
/// anything bigger is better pointed to by its path in the message.
pub(super) const MAX_ATTACHMENT_BYTES: u64 = 50 * 1024 * 1024;

fn invalid(message: impl Into<String>) -> RpcError {
    RpcError::new(codes::INVALID_PARAMS, message)
}

/// A file name that stays one path component and reads well. Absent or unusable:
/// `fallback`.
fn file_name(name: Option<&str>, fallback: &str) -> String {
    let name = name.map(|name| name.rsplit(['/', '\\']).next().unwrap_or(name).trim());
    match name {
        Some(name) if !name.is_empty() && name != "." && name != ".." => {
            name.chars().map(|c| if c.is_control() { '_' } else { c }).collect()
        }
        _ => fallback.to_owned(),
    }
}

/// What a pasted file is called when its client did not name it.
fn pasted_name(mime: Option<&str>) -> String {
    let extension = match mime {
        Some("image/png") => "png",
        Some("image/jpeg") => "jpg",
        Some("image/gif") => "gif",
        Some("image/webp") => "webp",
        Some(mime) if mime.starts_with("text/") => "txt",
        _ => "bin",
    };
    format!("pasted.{extension}")
}

/// Where the files attached to a thread's messages are kept.
fn thread_dir(root: &Path, thread: &ThreadId) -> PathBuf {
    root.join(thread.as_str())
}

/// Store one attachment for `thread` under `root`, or find the copy already there.
pub(super) fn store(
    root: &Path,
    thread: &ThreadId,
    new: NewAttachment,
) -> Result<Attachment, RpcError> {
    let NewAttachment { path, data, name, mime } = new;
    let mime = mime.filter(|mime| mime.contains('/') && !mime.trim().is_empty());
    let (bytes, name) = match (path, data) {
        (Some(path), None) => {
            let source = PathBuf::from(&path);
            if !source.is_absolute() {
                return Err(invalid(format!("attachment path {path:?} is not absolute")));
            }
            let metadata = std::fs::metadata(&source)
                .map_err(|error| invalid(format!("cannot attach {path}: {error}")))?;
            if !metadata.is_file() {
                return Err(invalid(format!("cannot attach {path}: not a file")));
            }
            if metadata.len() > MAX_ATTACHMENT_BYTES {
                return Err(too_big(&path));
            }
            let fallback = file_name(Some(&path), "attachment");
            let name = file_name(name.as_deref(), &fallback);
            // A copy qcc already keeps for this thread is used as it is.
            if let Ok(kept) = source.canonicalize()
                && let Ok(own) = thread_dir(root, thread).canonicalize()
                && kept.starts_with(&own)
            {
                let head = read_head(&kept);
                let mime = mime.unwrap_or_else(|| guess_mime(&name, &head).to_owned());
                let path = kept.display().to_string();
                return Ok(Attachment { name, mime, path, size: metadata.len() });
            }
            let bytes = std::fs::read(&source)
                .map_err(|error| invalid(format!("cannot attach {path}: {error}")))?;
            (bytes, name)
        }
        (None, Some(data)) => {
            if data.len() as u64 > MAX_ATTACHMENT_BYTES / 3 * 4 + 4 {
                return Err(too_big(name.as_deref().unwrap_or("the pasted file")));
            }
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data.trim())
                .map_err(|error| invalid(format!("attachment data is not base64: {error}")))?;
            let name = file_name(name.as_deref(), &pasted_name(mime.as_deref()));
            (bytes, name)
        }
        _ => return Err(invalid("an attachment gives either `path` or `data`")),
    };
    let mime = mime.unwrap_or_else(|| guess_mime(&name, &bytes).to_owned());
    // Each file in a folder of its own keeps its name without clashing.
    let dir = thread_dir(root, thread).join(uuid::Uuid::now_v7().simple().to_string());
    std::fs::create_dir_all(&dir).map_err(|error| store_failed(&dir, error))?;
    let target = dir.join(&name);
    std::fs::write(&target, &bytes).map_err(|error| store_failed(&target, error))?;
    Ok(Attachment { name, mime, path: target.display().to_string(), size: bytes.len() as u64 })
}

fn read_head(path: &Path) -> Vec<u8> {
    use std::io::Read;
    let mut head = Vec::with_capacity(16);
    if let Ok(file) = std::fs::File::open(path) {
        let _ = file.take(16).read_to_end(&mut head);
    }
    head
}

fn too_big(what: &str) -> RpcError {
    invalid(format!(
        "{what} is larger than {} MiB; mention its path in the message instead",
        MAX_ATTACHMENT_BYTES / 1024 / 1024
    ))
}

fn store_failed(path: &Path, error: std::io::Error) -> RpcError {
    RpcError::new(codes::INTERNAL_ERROR, format!("cannot keep {}: {error}", path.display()))
}

impl Core {
    /// Copy the files of a message where the thread keeps them.
    pub(super) fn store_attachments(
        &self,
        thread: &ThreadId,
        attachments: Vec<NewAttachment>,
    ) -> Result<Vec<Attachment>, RpcError> {
        let root = self.config.attachments_dir();
        attachments.into_iter().map(|new| store(&root, thread, new)).collect()
    }

    /// Read one of a thread's attachments off the core task: images can be large.
    pub(super) fn thread_attachment(&self, p: ThreadAttachmentParams, responder: Responder) {
        let ThreadAttachmentParams { thread_id, path } = p;
        if self.supervisor.thread(&thread_id).is_none() {
            responder.reply::<ThreadAttachmentResult>(Err(thread_not_found(&thread_id)));
            return;
        }
        let dir = thread_dir(&self.config.attachments_dir(), &thread_id);
        tokio::task::spawn_blocking(move || responder.reply(read(&dir, &path)));
    }
}

/// A file of `dir` and nothing outside it, whatever the path says.
fn read(dir: &Path, path: &str) -> Result<ThreadAttachmentResult, RpcError> {
    let not_found = || invalid(format!("{path} is not an attachment of the thread"));
    let file = Path::new(path).canonicalize().map_err(|_| not_found())?;
    let dir = dir.canonicalize().map_err(|_| not_found())?;
    if !file.starts_with(&dir) {
        return Err(not_found());
    }
    let data = std::fs::read(&file).map_err(|_| not_found())?;
    let name = file.file_name().map(|name| name.to_string_lossy()).unwrap_or_default();
    Ok(ThreadAttachmentResult {
        mime: guess_mime(&name, &data).to_owned(),
        data: base64::engine::general_purpose::STANDARD.encode(data),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(bytes: &[u8]) -> NewAttachment {
        NewAttachment {
            data: Some(base64::engine::general_purpose::STANDARD.encode(bytes)),
            ..NewAttachment::default()
        }
    }

    #[test]
    fn pasted_images_are_named_and_typed_from_their_bytes() {
        let root = tempfile::tempdir().unwrap();
        let thread = ThreadId("thr_a".into());
        let png = b"\x89PNG\r\n\x1a\nrest";
        let stored = store(root.path(), &thread, data(png)).unwrap();
        assert_eq!(stored.name, "pasted.bin");
        assert_eq!(stored.mime, "image/png");
        assert_eq!(stored.size, png.len() as u64);
        assert_eq!(std::fs::read(&stored.path).unwrap(), png);
        assert!(Path::new(&stored.path).starts_with(root.path().join("thr_a")));

        let named = NewAttachment {
            name: Some("../../etc/shot.png".into()),
            mime: Some("image/png".into()),
            ..data(png)
        };
        let stored = store(root.path(), &thread, named).unwrap();
        assert_eq!(stored.name, "shot.png");
        assert!(Path::new(&stored.path).starts_with(root.path().join("thr_a")));
    }

    #[test]
    fn files_are_copied_once() {
        let root = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        let file = source.path().join("notes.md");
        std::fs::write(&file, "# notes").unwrap();
        let thread = ThreadId("thr_b".into());
        let path = Some(file.display().to_string());
        let stored =
            store(root.path(), &thread, NewAttachment { path, ..NewAttachment::default() })
                .unwrap();
        assert_eq!((stored.name.as_str(), stored.mime.as_str()), ("notes.md", "text/markdown"));
        assert_ne!(stored.path, file.display().to_string());

        // Sent again from the thread's own copy: the same file.
        let again = NewAttachment { path: Some(stored.path.clone()), ..NewAttachment::default() };
        let again = store(root.path(), &thread, again).unwrap();
        assert_eq!(
            again.path,
            Path::new(&stored.path).canonicalize().unwrap().display().to_string()
        );
        assert_eq!(std::fs::read_dir(root.path().join("thr_b")).unwrap().count(), 1);
    }

    #[test]
    fn bad_attachments_are_refused() {
        let root = tempfile::tempdir().unwrap();
        let thread = ThreadId("thr_c".into());
        let neither = NewAttachment::default();
        assert!(store(root.path(), &thread, neither).is_err());
        let relative = NewAttachment { path: Some("x.png".into()), ..NewAttachment::default() };
        assert!(store(root.path(), &thread, relative).is_err());
        let garbage = NewAttachment { data: Some("@@@".into()), ..NewAttachment::default() };
        assert!(store(root.path(), &thread, garbage).is_err());
    }

    #[test]
    fn only_the_threads_own_files_are_read() {
        let root = tempfile::tempdir().unwrap();
        let thread = ThreadId("thr_d".into());
        let stored = store(root.path(), &thread, data(b"hello")).unwrap();
        let dir = thread_dir(root.path(), &thread);
        assert!(read(&dir, &stored.path).is_ok());
        let outside = root.path().join("secret");
        std::fs::write(&outside, "x").unwrap();
        assert!(read(&dir, &outside.display().to_string()).is_err());
        let sneaky =
            format!("{}/../../secret", Path::new(&stored.path).parent().unwrap().display());
        assert!(read(&dir, &sneaky).is_err());
    }
}

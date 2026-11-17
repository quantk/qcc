//! Files the user attaches to a message.

use serde::{Deserialize, Serialize};

/// A file attached to a user message. qcc keeps its own copy, so the message reads
/// the same later whatever happens to the file it came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct Attachment {
    /// The file name as the user knows it.
    pub name: String,
    /// Media type, like `image/png`.
    pub mime: String,
    /// Absolute path of qcc's copy.
    pub path: String,
    /// Bytes.
    pub size: u64,
}

impl Attachment {
    /// An image an agent can look at, as opposed to a file it has to read.
    pub fn is_image(&self) -> bool {
        is_image_mime(&self.mime)
    }
}

/// Image types agents take as pictures. Other `image/*` types (SVG, TIFF, …) are
/// files to them.
pub fn is_image_mime(mime: &str) -> bool {
    matches!(mime, "image/png" | "image/jpeg" | "image/gif" | "image/webp")
}

/// The media type of a file, from its leading bytes for the image types agents take
/// and from its name otherwise. `application/octet-stream` when neither tells.
pub fn guess_mime(name: &str, head: &[u8]) -> &'static str {
    const SIGNATURES: [(&[u8], &str); 5] = [
        (b"\x89PNG\r\n\x1a\n", "image/png"),
        (b"\xff\xd8\xff", "image/jpeg"),
        (b"GIF87a", "image/gif"),
        (b"GIF89a", "image/gif"),
        (b"%PDF-", "application/pdf"),
    ];
    if let Some((_, mime)) = SIGNATURES.iter().find(|(magic, _)| head.starts_with(magic)) {
        return mime;
    }
    if head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WEBP" {
        return "image/webp";
    }
    let extension = name.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase());
    match extension.as_deref() {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("pdf") => "application/pdf",
        Some("json") => "application/json",
        Some("md" | "markdown") => "text/markdown",
        Some("html" | "htm") => "text/html",
        Some("csv") => "text/csv",
        Some(
            "txt" | "log" | "rs" | "ts" | "tsx" | "js" | "jsx" | "py" | "go" | "c" | "h" | "cpp"
            | "java" | "toml" | "yaml" | "yml" | "sh" | "css" | "sql" | "xml",
        ) => "text/plain",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_wins_over_the_name() {
        assert_eq!(guess_mime("shot.jpg", b"\x89PNG\r\n\x1a\n...."), "image/png");
        assert_eq!(guess_mime("x", b"RIFF\0\0\0\0WEBPVP8 "), "image/webp");
        assert_eq!(guess_mime("notes.MD", b"# hi"), "text/markdown");
        assert_eq!(guess_mime("blob", b"\0\x01"), "application/octet-stream");
    }

    #[test]
    fn only_raster_images_are_pictures() {
        assert!(is_image_mime("image/png"));
        assert!(!is_image_mime("image/svg+xml"));
        assert!(!is_image_mime("text/plain"));
    }
}

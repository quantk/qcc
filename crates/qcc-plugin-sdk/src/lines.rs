//! Splitting a byte stream into lines.

/// Reassembles lines from the arbitrary chunks a process's stdout arrives in.
#[derive(Debug, Default)]
pub struct LineBuffer {
    pending: Vec<u8>,
}

impl LineBuffer {
    /// Add a chunk and return the lines it completed, without their terminators.
    /// Invalid UTF-8 is replaced rather than dropped; empty lines are skipped.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        // What is pending already holds no newline: only the new chunk is searched,
        // so a long line costs its length once, not once per chunk.
        let mut searched = self.pending.len();
        self.pending.extend_from_slice(chunk);
        let mut lines = Vec::new();
        let mut start = 0;
        while let Some(offset) = self.pending[searched..].iter().position(|byte| *byte == b'\n') {
            let end = searched + offset;
            let line = String::from_utf8_lossy(&self.pending[start..end]);
            let line = line.trim_end_matches('\r');
            if !line.trim().is_empty() {
                lines.push(line.to_owned());
            }
            start = end + 1;
            searched = start;
        }
        self.pending.drain(..start);
        lines
    }

    /// What is left after the last newline, for a stream that ended mid-line.
    pub fn finish(&mut self) -> Option<String> {
        let rest = String::from_utf8_lossy(&std::mem::take(&mut self.pending)).trim().to_owned();
        (!rest.is_empty()).then_some(rest)
    }
}

/// Keeps the tail of a stream: enough of stderr to explain why a process died.
#[derive(Debug)]
pub struct Tail {
    limit: usize,
    text: String,
    /// The start of a character whose remaining bytes come in the next chunk.
    split: Vec<u8>,
}

impl Tail {
    pub fn new(limit: usize) -> Self {
        Self { limit, text: String::new(), split: Vec::new() }
    }

    pub fn push(&mut self, chunk: &[u8]) {
        self.split.extend_from_slice(chunk);
        let complete = self.split.len() - incomplete_suffix(&self.split);
        self.text.push_str(&String::from_utf8_lossy(&self.split[..complete]));
        self.split.drain(..complete);
        if self.text.len() > self.limit {
            let mut cut = self.text.len() - self.limit;
            while !self.text.is_char_boundary(cut) {
                cut += 1;
            }
            self.text.drain(..cut);
        }
    }

    /// All of the tail that is kept.
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// The last non-empty line, which is usually the error.
    pub fn last_line(&self) -> Option<&str> {
        self.text.lines().rev().map(str::trim).find(|line| !line.is_empty())
    }
}

/// How many trailing bytes start a UTF-8 character that the chunk cut off.
fn incomplete_suffix(bytes: &[u8]) -> usize {
    for len in 1..=bytes.len().min(3) {
        let byte = bytes[bytes.len() - len];
        if byte & 0xC0 != 0x80 {
            let needed = match byte {
                0xC0..=0xDF => 2,
                0xE0..=0xEF => 3,
                0xF0..=0xF7 => 4,
                _ => 1,
            };
            return if needed > len { len } else { 0 };
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_survive_any_chunking() {
        let stream = "{\"a\":1}\r\n\n{\"b\":\"д\"}\n{\"c\":3}";
        for size in 1..stream.len() {
            let mut buffer = LineBuffer::default();
            let mut lines = Vec::new();
            for chunk in stream.as_bytes().chunks(size) {
                lines.extend(buffer.push(chunk));
            }
            assert_eq!(lines, ["{\"a\":1}", "{\"b\":\"д\"}"], "chunk size {size}");
            assert_eq!(buffer.finish().as_deref(), Some("{\"c\":3}"));
            assert_eq!(buffer.finish(), None);
        }
    }

    #[test]
    fn tail_keeps_the_end() {
        let mut tail = Tail::new(16);
        tail.push("first line\nвторая строка\n".as_bytes());
        tail.push(b"error: boom\n\n");
        assert_eq!(tail.last_line(), Some("error: boom"));
        assert_eq!(Tail::new(8).last_line(), None);
    }

    #[test]
    fn tail_keeps_characters_split_between_chunks() {
        let text = "ошибка: 💥";
        for size in 1..text.len() {
            let mut tail = Tail::new(64);
            for chunk in text.as_bytes().chunks(size) {
                tail.push(chunk);
            }
            assert_eq!(tail.as_str(), text, "chunk size {size}");
        }
    }
}

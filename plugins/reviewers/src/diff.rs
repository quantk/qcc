//! Reading a unified diff as git writes it: one section per file, a fingerprint of
//! the whole and of each file, and the text of a line by its number.

use std::collections::BTreeMap;

/// The part of a diff about one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiff<'a> {
    /// The file as it is now; as it was, for a deleted one.
    pub path: String,
    pub text: &'a str,
}

/// FNV-1a: stable across runs and builds, which is all a fingerprint needs.
pub fn fingerprint(text: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// The sections of a diff, one per `diff --git` header, in order.
pub fn files(diff: &str) -> Vec<FileDiff<'_>> {
    let mut starts: Vec<usize> = Vec::new();
    let mut offset = 0;
    for line in diff.split_inclusive('\n') {
        if line.starts_with("diff --git ") {
            starts.push(offset);
        }
        offset += line.len();
    }
    let mut sections = Vec::new();
    for (index, start) in starts.iter().enumerate() {
        let end = starts.get(index + 1).copied().unwrap_or(diff.len());
        let text = &diff[*start..end];
        sections.push(FileDiff { path: path_of(text), text });
    }
    sections
}

/// The file a section is about: `+++ b/…` unless it was deleted, then `--- a/…`,
/// and the header for a section without either (a binary file, a mode change).
fn path_of(section: &str) -> String {
    let strip = |name: &str| {
        let name = name.trim_end_matches(['\r', '\t']);
        name.strip_prefix("a/").or_else(|| name.strip_prefix("b/")).unwrap_or(name).to_owned()
    };
    let mut old = None;
    for line in section.lines() {
        if let Some(name) = line.strip_prefix("+++ ") {
            if name != "/dev/null" {
                return strip(name);
            }
        } else if let Some(name) = line.strip_prefix("--- ")
            && name != "/dev/null"
        {
            old = Some(strip(name));
        } else if line.starts_with("@@") {
            break;
        }
    }
    if let Some(old) = old {
        return old;
    }
    let header = section.lines().next().unwrap_or_default();
    header.rsplit(" b/").next().unwrap_or(header).to_owned()
}

/// Each file's fingerprint, by path.
pub fn file_prints(diff: &str) -> BTreeMap<String, String> {
    files(diff).into_iter().map(|file| (file.path, fingerprint(file.text))).collect()
}

/// The files whose part of the diff is not what it was: new, changed or gone.
pub fn changed_since(
    before: &BTreeMap<String, String>,
    now: &BTreeMap<String, String>,
) -> Vec<String> {
    let mut changed: Vec<String> = now
        .iter()
        .filter(|(path, print)| before.get(*path) != Some(print))
        .map(|(path, _)| path.clone())
        .collect();
    changed.extend(before.keys().filter(|path| !now.contains_key(*path)).cloned());
    changed.sort();
    changed
}

/// The text of `line` of `path` as it is now, if the diff shows that line.
pub fn line_text(diff: &str, path: &str, line: u32) -> Option<String> {
    let file = files(diff).into_iter().find(|file| file.path == path)?;
    let mut current = 0u32;
    let mut in_hunk = false;
    for text in file.text.lines() {
        if let Some(header) = text.strip_prefix("@@ ") {
            // `@@ -a,b +c,d @@`: the new side starts at c.
            let start = header.split_whitespace().find_map(|part| part.strip_prefix('+'))?;
            current = start.split(',').next()?.parse().ok()?;
            in_hunk = true;
            continue;
        }
        if !in_hunk {
            continue;
        }
        match text.as_bytes().first() {
            Some(b'+') | Some(b' ') => {
                if current == line {
                    return Some(text[1..].to_owned());
                }
                current += 1;
            }
            Some(b'-') | Some(b'\\') => {}
            _ => {
                if current == line {
                    return Some(String::new());
                }
                current += 1;
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIFF: &str = r#"diff --git a/src/a.rs b/src/a.rs
index 1..2 100644
--- a/src/a.rs
+++ b/src/a.rs
@@ -1,3 +1,4 @@
 fn a() {
-    old();
+    new();
+    more();
 }
diff --git a/gone.txt b/gone.txt
deleted file mode 100644
--- a/gone.txt
+++ /dev/null
@@ -1 +0,0 @@
-bye
diff --git a/new.txt b/new.txt
new file mode 100644
--- /dev/null
+++ b/new.txt
@@ -0,0 +1 @@
+hello
"#;

    #[test]
    fn a_diff_splits_into_files() {
        let files = files(DIFF);
        let paths: Vec<&str> = files.iter().map(|file| file.path.as_str()).collect();
        assert_eq!(paths, ["src/a.rs", "gone.txt", "new.txt"]);
        assert!(files[0].text.ends_with(" }\n"));
        assert_eq!(files.iter().map(|f| f.text.len()).sum::<usize>(), DIFF.len());
    }

    #[test]
    fn lines_are_found_by_their_new_number() {
        assert_eq!(line_text(DIFF, "src/a.rs", 2).as_deref(), Some("    new();"));
        assert_eq!(line_text(DIFF, "src/a.rs", 4).as_deref(), Some("}"));
        assert_eq!(line_text(DIFF, "src/a.rs", 9), None);
        assert_eq!(line_text(DIFF, "new.txt", 1).as_deref(), Some("hello"));
        assert_eq!(line_text(DIFF, "nope.rs", 1), None);
    }

    #[test]
    fn changed_files_are_told_by_their_prints() {
        let before = file_prints(DIFF);
        let edited = DIFF.replace("+hello", "+hello there");
        let now = file_prints(&edited);
        assert_eq!(changed_since(&before, &now), ["new.txt"]);
        let fewer = file_prints(&DIFF[..DIFF.find("diff --git a/gone").unwrap()]);
        assert_eq!(changed_since(&before, &fewer), ["gone.txt", "new.txt"]);
        assert_eq!(fingerprint(DIFF), fingerprint(DIFF));
        assert_ne!(fingerprint(DIFF), fingerprint(&edited));
    }
}

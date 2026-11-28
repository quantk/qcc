//! Finding files and folders in a thread's working directory by a fuzzy query:
//! what the app's composer offers after `@`.
//!
//! A scan walks the folder once, as git sees it (`.gitignore`, `.git/info/exclude`
//! and the global excludes are honoured, even outside a repository), and is kept
//! for a few seconds, so a query per keystroke only matches against the list.
//! Matching is fzf's kind: the query's characters in order, scored higher where
//! they run together, start a word or a path segment, or fall in the file's name.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use qcc_protocol::methods::PathMatch;

/// How long a scan is good for: long enough to cover a burst of typing, short
/// enough that a file the agent just made shows up soon.
const FRESH_FOR: Duration = Duration::from_secs(5);

/// A scan stops after this many entries, so a thread working in a home folder
/// does not walk the whole disk.
const MAX_ENTRIES: usize = 200_000;

/// A scan also stops after this long.
const MAX_SCAN_TIME: Duration = Duration::from_secs(3);

/// A folder whose walk takes this long is rescanned in the background.
const SLOW_SCAN: Duration = Duration::from_millis(200);

/// Scans kept at once, for the folders asked about most recently.
const MAX_ROOTS: usize = 8;

/// Folders skipped even where no `.gitignore` names them: they are never what a
/// user means to point an agent at, and some are huge.
const SKIPPED_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    ".jj",
    "node_modules",
    "target",
    ".direnv",
    ".venv",
    "__pycache__",
    ".mypy_cache",
    ".pytest_cache",
    ".gradle",
    ".next",
    ".turbo",
    ".cache",
];

pub const DEFAULT_LIMIT: usize = 50;
pub const MAX_LIMIT: usize = 500;

/// One file or folder found by a scan.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    /// Relative to the root, `/`-separated.
    path: String,
    dir: bool,
}

/// What one walk of a folder found.
#[derive(Debug)]
struct Scan {
    /// When the walk ended.
    at: Instant,
    /// How long it took.
    took: Duration,
    /// Sorted for an empty query: shallow first, then by path.
    entries: Vec<Entry>,
    incomplete: bool,
}

/// One folder's latest scan. Its own lock makes concurrent queries about the same
/// folder wait for a single walk instead of each starting one.
type Slot = Arc<Mutex<SlotState>>;

#[derive(Default)]
struct SlotState {
    scan: Option<Arc<Scan>>,
    /// A walk to replace a stale scan runs in the background.
    refreshing: bool,
}

/// The scans of the folders asked about lately. Cheap to clone; each clone sees
/// the same scans.
#[derive(Clone, Default)]
pub struct PathIndex {
    slots: Arc<Mutex<HashMap<PathBuf, (Instant, Slot)>>>,
}

impl PathIndex {
    /// The best matches of `query` under `root`, at most `limit`, and whether the
    /// scan had to stop early. Blocking: it may walk the folder.
    pub fn search(&self, root: &Path, query: &str, limit: usize) -> (Vec<PathMatch>, bool) {
        let scan = self.scan(root);
        (best_matches(&scan.entries, query, limit), scan.incomplete)
    }

    fn scan(&self, root: &Path) -> Arc<Scan> {
        let slot = {
            let mut slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
            let now = Instant::now();
            let slot = slots.entry(root.to_path_buf()).or_insert_with(|| (now, Slot::default()));
            slot.0 = now;
            let slot = slot.1.clone();
            while slots.len() > MAX_ROOTS {
                let Some(oldest) =
                    slots.iter().min_by_key(|(_, (used, _))| *used).map(|(path, _)| path.clone())
                else {
                    break;
                };
                slots.remove(&oldest);
            }
            slot
        };
        let mut held = slot.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(scan) = held.scan.clone() {
            if scan.at.elapsed() < FRESH_FOR {
                return scan;
            }
            // A folder slow to walk is answered from the stale scan while a new
            // one runs, so typing is not held up by it every few seconds.
            if scan.took >= SLOW_SCAN {
                if !held.refreshing {
                    held.refreshing = true;
                    let (slot, root) = (slot.clone(), root.to_path_buf());
                    std::thread::spawn(move || {
                        let fresh = Arc::new(walk(&root));
                        let mut held = slot.lock().unwrap_or_else(|e| e.into_inner());
                        held.scan = Some(fresh);
                        held.refreshing = false;
                    });
                }
                return scan;
            }
        }
        let scan = Arc::new(walk(root));
        held.scan = Some(scan.clone());
        scan
    }
}

/// Walk `root` as git sees it, without the folders in [`SKIPPED_DIRS`].
fn walk(root: &Path) -> Scan {
    let started = Instant::now();
    let mut entries = Vec::new();
    let mut incomplete = false;
    let walker = ignore::WalkBuilder::new(root)
        // Dotfiles such as `.github/` or `.claude/` are worth pointing at; what
        // git ignores and the folders below are not.
        .hidden(false)
        .require_git(false)
        .follow_links(false)
        .filter_entry(|entry| {
            let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
            !(is_dir && entry.file_name().to_str().is_some_and(|name| SKIPPED_DIRS.contains(&name)))
        })
        .build();
    for entry in walker.flatten() {
        if entry.depth() == 0 {
            continue;
        }
        let Ok(relative) = entry.path().strip_prefix(root) else { continue };
        let Some(relative) = relative.to_str() else { continue };
        if entries.len() == MAX_ENTRIES || started.elapsed() > MAX_SCAN_TIME {
            incomplete = true;
            break;
        }
        let path = if std::path::MAIN_SEPARATOR == '/' {
            relative.to_owned()
        } else {
            relative.replace(std::path::MAIN_SEPARATOR, "/")
        };
        let dir = entry.file_type().is_some_and(|t| t.is_dir());
        entries.push(Entry { path, dir });
    }
    entries.sort_by(|a, b| depth(&a.path).cmp(&depth(&b.path)).then_with(|| a.path.cmp(&b.path)));
    Scan { at: Instant::now(), took: started.elapsed(), entries, incomplete }
}

fn depth(path: &str) -> usize {
    path.bytes().filter(|&b| b == b'/').count()
}

/// The `limit` entries that match `query` best, best first. An empty query takes
/// the entries in their order: shallow first.
fn best_matches(entries: &[Entry], query: &str, limit: usize) -> Vec<PathMatch> {
    let query: Vec<char> = query.chars().filter(|c| !c.is_whitespace()).collect();
    if query.is_empty() {
        return entries
            .iter()
            .take(limit)
            .map(|e| PathMatch { path: e.path.clone(), dir: e.dir, positions: Vec::new() })
            .collect();
    }
    let mut scored: Vec<(i32, &Entry, Vec<u32>)> = entries
        .iter()
        .filter_map(|entry| fuzzy_match(&query, &entry.path).map(|(s, p)| (s, entry, p)))
        .collect();
    scored.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.path.len().cmp(&b.1.path.len()))
            .then_with(|| a.1.path.cmp(&b.1.path))
    });
    scored
        .into_iter()
        .take(limit)
        .map(|(_, e, positions)| PathMatch { path: e.path.clone(), dir: e.dir, positions })
        .collect()
}

const SCORE_MATCH: i32 = 16;
const GAP_START: i32 = -3;
const GAP_EXTENSION: i32 = -1;
/// After `/`, or at the start of the path.
const BONUS_SEGMENT: i32 = 10;
/// After `-`, `_`, `.` or a space.
const BONUS_WORD: i32 = 8;
/// A capital after a small letter, or a digit after a letter.
const BONUS_CAMEL: i32 = 7;
/// Each character of a run after the first gets at least this.
const BONUS_CONSECUTIVE: i32 = 4;
/// A character matched in the last segment, the file's (or folder's) own name.
const BONUS_NAME: i32 = 3;
/// The first query character's bonus counts this many times.
const FIRST_MULTIPLIER: i32 = 2;

/// Score `query` against `text` as fzf does, and the character positions of the
/// best alignment; `None` when the query's characters are not all in `text` in
/// order. Smart case: a query with a capital letter matches case-sensitively.
pub fn fuzzy_match(query: &[char], text: &str) -> Option<(i32, Vec<u32>)> {
    let sensitive = query.iter().any(|c| c.is_uppercase());
    let fold = |c: char| if sensitive { c } else { lower(c) };
    // A quick pass before the full one: most entries fail it.
    let mut wanted = query.iter().copied().map(fold).peekable();
    for c in text.chars() {
        if wanted.peek() == Some(&fold(c)) {
            wanted.next();
        }
    }
    if wanted.peek().is_some() {
        return None;
    }

    let chars: Vec<char> = text.chars().collect();
    let (n, m) = (chars.len(), query.len());
    let name_start = chars.iter().rposition(|&c| c == '/').map_or(0, |i| i + 1);
    let bonus: Vec<i32> = (0..n)
        .map(|j| {
            let own = if j >= name_start { BONUS_NAME } else { 0 };
            own + boundary(j.checked_sub(1).map(|p| chars[p]), chars[j])
        })
        .collect();
    let folded: Vec<char> = chars.iter().copied().map(fold).collect();
    let query: Vec<char> = query.iter().copied().map(fold).collect();

    // `score[i][j]`: the best score of `query[..=i]` with `query[i]` at `j`;
    // `from[i][j]`: where `query[i - 1]` sits in that alignment; `chunk[i][j]`:
    // the bonus of the first character of the run `j` ends, which the rest of
    // the run shares, as in fzf: `inner` matches `inner.rs` as a whole word.
    const NONE: i32 = i32::MIN / 2;
    let mut score = vec![NONE; m * n];
    let mut from = vec![0u32; m * n];
    let mut chunk = vec![0i32; m * n];
    for i in 0..m {
        // The best alignment of `query[..i]` ending before a gap of at least one
        // character up to `j`, already charged for the gap, and where it ended.
        let mut gap = (NONE, 0usize);
        for j in i..n {
            if i > 0 && j >= 2 {
                let opened = score[(i - 1) * n + j - 2] + GAP_START;
                let extended = gap.0 + GAP_EXTENSION;
                gap = if opened >= extended { (opened, j - 2) } else { (extended, gap.1) };
            }
            if folded[j] != query[i] {
                continue;
            }
            let cell = i * n + j;
            if i == 0 {
                score[cell] = SCORE_MATCH + bonus[j] * FIRST_MULTIPLIER;
                chunk[cell] = bonus[j];
                continue;
            }
            let before = (i - 1) * n + j - 1;
            let run_bonus = bonus[j].max(chunk[before]).max(BONUS_CONSECUTIVE);
            let run = (score[before] > NONE).then(|| score[before] + run_bonus);
            let jump = (gap.0 > NONE).then(|| gap.0 + bonus[j]);
            let best = match (run, jump) {
                (Some(r), Some(g)) if g > r => Some((g, gap.1, bonus[j])),
                (Some(r), _) => Some((r, j - 1, chunk[before].max(bonus[j]))),
                (None, Some(g)) => Some((g, gap.1, bonus[j])),
                (None, None) => None,
            };
            if let Some((best, previous, first)) = best {
                score[cell] = SCORE_MATCH + best;
                from[cell] = previous as u32;
                chunk[cell] = first;
            }
        }
    }

    let last = (m - 1) * n;
    let (end, best) = (0..n)
        .map(|j| (j, score[last + j]))
        .filter(|&(_, s)| s > NONE)
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))?;
    let mut positions = vec![0u32; m];
    let mut j = end;
    for i in (0..m).rev() {
        positions[i] = j as u32;
        j = from[i * n + j] as usize;
    }
    Some((best, positions))
}

fn lower(c: char) -> char {
    if c.is_ascii() { c.to_ascii_lowercase() } else { c.to_lowercase().next().unwrap_or(c) }
}

/// The bonus for a match at `current`, by the character before it.
fn boundary(previous: Option<char>, current: char) -> i32 {
    match previous {
        None | Some('/') => BONUS_SEGMENT,
        Some('-' | '_' | '.' | ' ') => BONUS_WORD,
        Some(p) if p.is_lowercase() && current.is_uppercase() => BONUS_CAMEL,
        Some(p) if !p.is_ascii_digit() && current.is_ascii_digit() => BONUS_CAMEL,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    fn score(query: &str, text: &str) -> Option<i32> {
        fuzzy_match(&chars(query), text).map(|(s, _)| s)
    }

    fn ranked(query: &str, paths: &[&str]) -> Vec<String> {
        let entries: Vec<Entry> =
            paths.iter().map(|p| Entry { path: (*p).to_owned(), dir: false }).collect();
        best_matches(&entries, query, 10).into_iter().map(|m| m.path).collect()
    }

    #[test]
    fn matches_a_subsequence_and_nothing_else() {
        assert!(score("cmp", "src/Composer.tsx").is_some());
        assert!(score("cmpz", "src/Composer.tsx").is_none());
        assert!(score("rc", "src").is_some());
        assert!(score("cr", "src").is_none());
    }

    #[test]
    fn positions_point_at_the_best_alignment() {
        // `main` as a whole word in the name, not its letters strewn over the path.
        let (_, positions) = fuzzy_match(&chars("main"), "my/app/inner/main.rs").unwrap();
        assert_eq!(positions, [13, 14, 15, 16]);
        let (_, positions) = fuzzy_match(&chars("ab"), "a/xb/b").unwrap();
        assert_eq!(positions, [0, 5], "the segment start beats the nearer middle");
    }

    #[test]
    fn smart_case() {
        assert!(score("comp", "Composer.tsx").is_some());
        assert!(score("Comp", "Composer.tsx").is_some());
        assert!(score("Comp", "composer.tsx").is_none());
    }

    #[test]
    fn prefers_the_file_name_segment_starts_and_runs() {
        assert_eq!(
            ranked("lib", &["crates/lib-old/src/main.rs", "src/lib.rs", "docs/library/x.md"]),
            ["src/lib.rs", "docs/library/x.md", "crates/lib-old/src/main.rs"]
        );
        assert_eq!(
            ranked("comp", &["src/ui/c/o/m/p.ts", "src/modules/timeline/Composer.tsx"]),
            ["src/modules/timeline/Composer.tsx", "src/ui/c/o/m/p.ts"]
        );
        assert_eq!(
            ranked("tc", &["src/timeline/composer.ts", "src/stack.ts"]),
            ["src/timeline/composer.ts", "src/stack.ts"],
            "two segment starts beat a run in the middle"
        );
    }

    #[test]
    fn ties_go_to_the_shorter_path() {
        assert_eq!(ranked("a", &["b/a/c", "a"]), ["a", "b/a/c"]);
    }

    #[test]
    fn an_empty_query_lists_in_order() {
        assert_eq!(ranked("", &["a", "b/c"]), ["a", "b/c"]);
    }

    #[test]
    fn scans_as_git_sees_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for path in [
            "src/main.rs",
            "src/deep/inner.rs",
            "node_modules/pkg/index.js",
            "target/debug/qcc",
            "build/out.txt",
            ".git/HEAD",
            ".github/workflows/ci.yml",
            "notes.log",
        ] {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "").unwrap();
        }
        std::fs::write(root.join(".gitignore"), "build/\n*.log\n").unwrap();

        let index = PathIndex::default();
        let (all, incomplete) = index.search(root, "", 100);
        assert!(!incomplete);
        let all: Vec<_> =
            all.iter().map(|m| format!("{}{}", m.path, if m.dir { "/" } else { "" })).collect();
        assert_eq!(
            all,
            [
                ".github/",
                ".gitignore",
                "src/",
                ".github/workflows/",
                "src/deep/",
                "src/main.rs",
                ".github/workflows/ci.yml",
                "src/deep/inner.rs",
            ]
        );

        let (found, _) = index.search(root, "inner", 10);
        assert_eq!(found[0].path, "src/deep/inner.rs");
        assert_eq!(found[0].positions, [9, 10, 11, 12, 13]);

        // Kept for a while: a new file shows up once the scan is stale.
        std::fs::write(root.join("src/fresh.rs"), "").unwrap();
        assert!(index.search(root, "fresh", 10).0.is_empty());
    }
}

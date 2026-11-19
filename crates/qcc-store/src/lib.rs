//! SQLite persistence.
//!
//! The journal (`events`) is append-only and is the source of truth. The `threads`
//! table is a projection maintained in the same transaction as each append by folding
//! the events with [`Thread::apply`] — the very fold the runtime uses, so the stored
//! row cannot drift from what the engine holds.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use qcc_domain::{
    Automation, AutomationId, AutomationRun, CommentId, EventPayload, Item, ItemShape, LineComment,
    Project, ProjectId, ProviderLimits, Role, Thread, ThreadEvent, ThreadId, ThreadUsage,
    ToolBucket, ToolUse, TranscriptEntry, UsageBucket,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("corrupt record: {0}")]
    Corrupt(serde_json::Error),
    #[error("cannot encode a record: {0}")]
    Encode(serde_json::Error),
    #[error("thread {thread}: event seq {got} does not follow {last}")]
    OutOfOrder { thread: ThreadId, last: u64, got: u64 },
    #[error("thread {0}: the first event must be thread/created")]
    MissingThread(ThreadId),
    #[error("database schema version {found} is newer than this build supports ({supported})")]
    SchemaTooNew { found: u32, supported: u32 },
}

pub type Result<T> = std::result::Result<T, StoreError>;

/// Ordered schema migrations; `PRAGMA user_version` is the count applied.
const MIGRATIONS: &[&str] = &[
    "
    CREATE TABLE threads (
        id          TEXT PRIMARY KEY,
        created_at  INTEGER NOT NULL,
        data        TEXT NOT NULL
    ) STRICT;

    CREATE TABLE events (
        thread_id   TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
        seq         INTEGER NOT NULL,
        ts          INTEGER NOT NULL,
        payload     TEXT NOT NULL,
        PRIMARY KEY (thread_id, seq)
    ) STRICT, WITHOUT ROWID;
",
    "
    CREATE TABLE projects (
        id          TEXT PRIMARY KEY,
        created_at  INTEGER NOT NULL,
        data        TEXT NOT NULL
    ) STRICT;
",
    "
    CREATE TABLE meta (
        key         TEXT PRIMARY KEY,
        value       INTEGER NOT NULL
    ) STRICT;
",
    "
    CREATE TABLE comments (
        id          TEXT PRIMARY KEY,
        thread_id   TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
        created_at  INTEGER NOT NULL,
        data        TEXT NOT NULL
    ) STRICT;

    CREATE INDEX comments_by_thread ON comments (thread_id, created_at);
",
    "
    CREATE TABLE automations (
        id          TEXT PRIMARY KEY,
        created_at  INTEGER NOT NULL,
        data        TEXT NOT NULL
    ) STRICT;

    CREATE TABLE automation_runs (
        id              INTEGER PRIMARY KEY,
        automation_id   TEXT NOT NULL REFERENCES automations(id) ON DELETE CASCADE,
        data            TEXT NOT NULL
    ) STRICT;

    CREATE INDEX automation_runs_by_automation ON automation_runs (automation_id, id);
",
    "
    CREATE TABLE provider_limits (
        provider    TEXT PRIMARY KEY,
        data        TEXT NOT NULL
    ) STRICT;
",
];

/// How many runs of each automation the history keeps; older ones are dropped.
pub const AUTOMATION_RUNS_KEPT: usize = 100;

/// The version of what [`Thread::apply`] folds into a projection. Raise it when the
/// fold learns something new from old events: on open, every stored projection is
/// then folded again from the journal, so threads made before carry it too.
const PROJECTION_VERSION: i64 = 1;

/// The span of one bucket of [`Store::usage_history`]: an hour, so a client can
/// group them into days of its own time zone.
pub const USAGE_BUCKET_MS: u64 = 3_600_000;

pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let version: u32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        let supported = MIGRATIONS.len() as u32;
        if version > supported {
            return Err(StoreError::SchemaTooNew { found: version, supported });
        }
        for (index, migration) in MIGRATIONS.iter().enumerate().skip(version as usize) {
            let tx = conn.transaction()?;
            tx.execute_batch(migration)?;
            tx.pragma_update(None, "user_version", index as u32 + 1)?;
            tx.commit()?;
        }
        let projected: i64 = conn
            .query_row("SELECT value FROM meta WHERE key = 'projection'", [], |row| row.get(0))
            .optional()?
            .unwrap_or(0);
        if projected < PROJECTION_VERSION {
            let tx = conn.transaction()?;
            rebuild_projections(&tx)?;
            tx.execute(
                "INSERT OR REPLACE INTO meta (key, value) VALUES ('projection', ?1)",
                [PROJECTION_VERSION],
            )?;
            tx.commit()?;
        }
        Ok(Self { conn: Mutex::new(conn) })
    }

    /// What every thread spent and how its turns went, hour by hour, oldest first;
    /// hours with nothing in them are left out. Folded from the journal with the
    /// same [`ThreadUsage`] fold as the projection, so the hours of a thread add up
    /// to its projection's numbers.
    pub fn usage_history(&self) -> Result<Vec<UsageBucket>> {
        let conn = self.lock();
        let mut statement = conn.prepare(
            "SELECT thread_id, ts, payload FROM events \
             WHERE json_extract(payload, '$.type') IN \
                   ('usage/updated', 'turn/started', 'turn/completed', 'thread/reset') \
             ORDER BY thread_id, seq",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?))
        })?;
        let mut buckets: Vec<UsageBucket> = Vec::new();
        let mut thread: Option<(String, ThreadUsage, Option<u64>)> = None;
        for row in rows {
            let (thread_id, ts, payload) = row?;
            let ts = ts.cast_unsigned();
            if thread.as_ref().is_none_or(|(id, ..)| *id != thread_id) {
                thread = Some((thread_id.clone(), ThreadUsage::default(), None));
            }
            let Some((_, usage, turn_started)) = thread.as_mut() else { continue };
            let before = *usage;
            match decode::<EventPayload>(&payload)? {
                EventPayload::UsageUpdated { usage: report } => {
                    usage.report(&report);
                }
                EventPayload::TurnStarted { .. } => *turn_started = Some(ts),
                EventPayload::TurnCompleted { status, .. } => {
                    usage.turn_ended(status, turn_started.take(), ts);
                }
                EventPayload::ConversationReset => usage.reset(),
                _ => {}
            }
            let change =
                UsageBucket::between(&thread_id, ts - ts % USAGE_BUCKET_MS, &before, usage);
            if change.is_empty() {
                continue;
            }
            match buckets.last_mut() {
                Some(last) if last.thread_id.as_str() == thread_id && last.hour == change.hour => {
                    last.add(&change);
                }
                _ => buckets.push(change),
            }
        }
        buckets.sort_by_key(|bucket| bucket.hour);
        Ok(buckets)
    }

    /// What every thread used — commands, tools, MCP tools, skills — hour by hour
    /// of when each use ended, oldest first. A use still running is not counted.
    pub fn tool_history(&self) -> Result<Vec<ToolBucket>> {
        let conn = self.lock();
        let mut statement = conn.prepare(
            "SELECT thread_id, ts, payload FROM events \
             WHERE json_extract(payload, '$.type') IN ('item/started', 'item/completed') \
               AND json_extract(payload, '$.item.type') \
                   NOT IN ('agentMessage', 'reasoning', 'plan', 'extension') \
             ORDER BY thread_id, seq",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?))
        })?;
        let mut buckets: HashMap<(String, u64, ToolUse), ToolBucket> = HashMap::new();
        // When each item of the thread being read started.
        let mut started: HashMap<String, u64> = HashMap::new();
        let mut thread = String::new();
        for row in rows {
            let (thread_id, ts, payload) = row?;
            let ts = ts.cast_unsigned();
            if thread_id != thread {
                started.clear();
                thread.clone_from(&thread_id);
            }
            match decode::<EventPayload>(&payload)? {
                EventPayload::ItemStarted { item, .. } => {
                    started.insert(item.id.as_str().to_owned(), ts);
                }
                EventPayload::ItemCompleted { item, .. } => {
                    let Some(tool) = ToolUse::of(&item) else { continue };
                    let duration =
                        started.remove(item.id.as_str()).map(|start| ts.saturating_sub(start));
                    let hour = ts - ts % USAGE_BUCKET_MS;
                    buckets
                        .entry((thread_id.clone(), hour, tool.clone()))
                        .or_insert_with(|| {
                            ToolBucket::new(ThreadId::new(thread_id.as_str()), hour, tool)
                        })
                        .count(&item, duration);
                }
                _ => {}
            }
        }
        let mut buckets: Vec<ToolBucket> = buckets.into_values().collect();
        buckets.sort_by(|a, b| {
            (a.hour, a.thread_id.as_str(), a.kind, &a.name, &a.server).cmp(&(
                b.hour,
                b.thread_id.as_str(),
                b.kind,
                &b.name,
                &b.server,
            ))
        });
        Ok(buckets)
    }

    /// Append events and update the projections atomically: either the whole batch
    /// is stored or none of it.
    pub fn append(&self, events: &[ThreadEvent]) -> Result<()> {
        if events.is_empty() {
            return Ok(());
        }
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        // Events of one thread are contiguous in practice; cache the last projection.
        let mut current: Option<Thread> = None;
        for event in events {
            if current.as_ref().is_some_and(|thread| thread.id != event.thread_id)
                && let Some(thread) = current.take()
            {
                save_thread(&tx, &thread)?;
            }
            let mut thread = match current.take() {
                Some(thread) => thread,
                None => match load_thread(&tx, &event.thread_id)? {
                    Some(thread) => thread,
                    None => match &event.payload {
                        EventPayload::ThreadCreated { thread } => {
                            let mut thread = thread.as_ref().clone();
                            thread.last_seq = 0;
                            insert_thread(&tx, &thread)?;
                            thread
                        }
                        _ => return Err(StoreError::MissingThread(event.thread_id.clone())),
                    },
                },
            };
            if event.seq != thread.last_seq + 1 {
                return Err(StoreError::OutOfOrder {
                    thread: event.thread_id.clone(),
                    last: thread.last_seq,
                    got: event.seq,
                });
            }
            tx.execute(
                "INSERT INTO events (thread_id, seq, ts, payload) VALUES (?1, ?2, ?3, ?4)",
                params![
                    event.thread_id.as_str(),
                    event.seq.cast_signed(),
                    event.ts.cast_signed(),
                    encode(&event.payload)?
                ],
            )?;
            thread.apply(event);
            current = Some(thread);
        }
        if let Some(thread) = current {
            save_thread(&tx, &thread)?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn thread(&self, id: &ThreadId) -> Result<Option<Thread>> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        load_thread(&tx, id)
    }

    /// All threads, oldest first.
    pub fn threads(&self) -> Result<Vec<Thread>> {
        let conn = self.lock();
        let mut statement = conn.prepare("SELECT data FROM threads ORDER BY created_at, id")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        let mut threads = Vec::new();
        for data in rows {
            threads.push(decode(&data?)?);
        }
        Ok(threads)
    }

    /// Events of a thread with `seq > after`, in order.
    pub fn events(&self, id: &ThreadId, after: u64) -> Result<Vec<ThreadEvent>> {
        let conn = self.lock();
        let mut statement = conn.prepare(
            "SELECT seq, ts, payload FROM events WHERE thread_id = ?1 AND seq > ?2 ORDER BY seq",
        )?;
        let rows = statement.query_map(
            // Past `i64::MAX` nothing can follow; wrapping would return everything.
            params![id.as_str(), i64::try_from(after).unwrap_or(i64::MAX)],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?)),
        )?;
        let mut events = Vec::new();
        for row in rows {
            let (seq, ts, payload) = row?;
            events.push(ThreadEvent {
                thread_id: id.clone(),
                seq: seq.cast_unsigned(),
                ts: ts.cast_unsigned(),
                payload: decode(&payload)?,
            });
        }
        Ok(events)
    }

    /// The conversation of a thread since it was last started afresh: messages handed
    /// to the agent and the agent's settled replies, oldest first. With
    /// `last_agent_only`, just the latest reply.
    pub fn transcript(&self, id: &ThreadId, last_agent_only: bool) -> Result<Vec<TranscriptEntry>> {
        const REPLY: &str = "json_extract(payload, '$.type') = 'item/completed' \
                             AND json_extract(payload, '$.item.type') = 'agentMessage'";
        let sql = if last_agent_only {
            format!(
                "SELECT payload FROM events WHERE thread_id = ?1 AND {REPLY} \
                 ORDER BY seq DESC LIMIT 1"
            )
        } else {
            // The conversation since the user last started it afresh.
            format!(
                "SELECT payload FROM events WHERE thread_id = ?1 AND \
                 (json_extract(payload, '$.type') = 'input/submitted' OR ({REPLY})) AND \
                 seq > (SELECT COALESCE(MAX(seq), 0) FROM events WHERE thread_id = ?1 AND \
                        json_extract(payload, '$.type') = 'thread/reset') \
                 ORDER BY seq"
            )
        };
        let conn = self.lock();
        let mut statement = conn.prepare(&sql)?;
        let rows = statement.query_map([id.as_str()], |row| row.get::<_, String>(0))?;
        let mut entries = Vec::new();
        for row in rows {
            let entry = match decode(&row?)? {
                EventPayload::InputSubmitted { text, .. } => {
                    TranscriptEntry { role: Role::User, text }
                }
                EventPayload::ItemCompleted {
                    item: Item { shape: ItemShape::AgentMessage { text }, .. },
                    ..
                } => TranscriptEntry { role: Role::Agent, text },
                _ => continue,
            };
            entries.push(entry);
        }
        Ok(entries)
    }

    /// Every project, oldest first. Projects are settings, not history: a row is
    /// replaced when the project changes.
    pub fn projects(&self) -> Result<Vec<Project>> {
        let conn = self.lock();
        let mut statement = conn.prepare("SELECT data FROM projects ORDER BY created_at, id")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        let mut projects = Vec::new();
        for data in rows {
            projects.push(decode(&data?)?);
        }
        Ok(projects)
    }

    pub fn save_project(&self, project: &Project) -> Result<()> {
        self.lock().execute(
            "INSERT INTO projects (id, created_at, data) VALUES (?1, ?2, ?3) \
             ON CONFLICT (id) DO UPDATE SET data = excluded.data",
            params![project.id.as_str(), project.created_at.cast_signed(), encode(project)?],
        )?;
        Ok(())
    }

    pub fn delete_project(&self, id: &ProjectId) -> Result<()> {
        self.lock().execute("DELETE FROM projects WHERE id = ?1", [id.as_str()])?;
        Ok(())
    }

    /// The comments left on a thread's changes, oldest first. Like projects they are
    /// not history: a row is replaced when the comment is resolved.
    pub fn comments(&self, thread: &ThreadId) -> Result<Vec<LineComment>> {
        let conn = self.lock();
        let mut statement =
            conn.prepare("SELECT data FROM comments WHERE thread_id = ?1 ORDER BY created_at, id")?;
        let rows = statement.query_map([thread.as_str()], |row| row.get::<_, String>(0))?;
        let mut comments = Vec::new();
        for data in rows {
            comments.push(decode(&data?)?);
        }
        Ok(comments)
    }

    pub fn comment(&self, id: &CommentId) -> Result<Option<LineComment>> {
        let data: Option<String> = self
            .lock()
            .query_row("SELECT data FROM comments WHERE id = ?1", [id.as_str()], |row| row.get(0))
            .optional()?;
        data.map(|data| decode(&data)).transpose()
    }

    pub fn save_comment(&self, comment: &LineComment) -> Result<()> {
        self.lock().execute(
            "INSERT INTO comments (id, thread_id, created_at, data) VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT (id) DO UPDATE SET data = excluded.data",
            params![
                comment.id.as_str(),
                comment.thread_id.as_str(),
                comment.created_at.cast_signed(),
                encode(comment)?
            ],
        )?;
        Ok(())
    }

    pub fn delete_comment(&self, id: &CommentId) -> Result<()> {
        self.lock().execute("DELETE FROM comments WHERE id = ?1", [id.as_str()])?;
        Ok(())
    }

    /// The latest usage limits each provider reported, by provider id. Not history:
    /// a provider's row is replaced by its next report.
    pub fn provider_limits(&self) -> Result<Vec<ProviderLimits>> {
        let conn = self.lock();
        let mut statement = conn.prepare("SELECT data FROM provider_limits ORDER BY provider")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        let mut limits = Vec::new();
        for data in rows {
            limits.push(decode(&data?)?);
        }
        Ok(limits)
    }

    pub fn save_provider_limits(&self, limits: &ProviderLimits) -> Result<()> {
        self.lock().execute(
            "INSERT INTO provider_limits (provider, data) VALUES (?1, ?2) \
             ON CONFLICT (provider) DO UPDATE SET data = excluded.data",
            params![limits.provider.as_str(), encode(limits)?],
        )?;
        Ok(())
    }

    /// Every automation, oldest first. Like projects they are settings, not
    /// history: a row is replaced when the automation changes.
    pub fn automations(&self) -> Result<Vec<Automation>> {
        let conn = self.lock();
        let mut statement = conn.prepare("SELECT data FROM automations ORDER BY created_at, id")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        let mut automations = Vec::new();
        for data in rows {
            automations.push(decode(&data?)?);
        }
        Ok(automations)
    }

    pub fn save_automation(&self, automation: &Automation) -> Result<()> {
        self.lock().execute(
            "INSERT INTO automations (id, created_at, data) VALUES (?1, ?2, ?3) \
             ON CONFLICT (id) DO UPDATE SET data = excluded.data",
            params![
                automation.id.as_str(),
                automation.created_at.cast_signed(),
                encode(automation)?
            ],
        )?;
        Ok(())
    }

    /// Delete an automation and its runs.
    pub fn delete_automation(&self, id: &AutomationId) -> Result<()> {
        self.lock().execute("DELETE FROM automations WHERE id = ?1", [id.as_str()])?;
        Ok(())
    }

    /// Record a run, and forget the oldest of the automation's runs past
    /// [`AUTOMATION_RUNS_KEPT`].
    pub fn add_automation_run(&self, run: &AutomationRun) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO automation_runs (automation_id, data) VALUES (?1, ?2)",
            params![run.automation_id.as_str(), encode(run)?],
        )?;
        tx.execute(
            "DELETE FROM automation_runs WHERE automation_id = ?1 AND id NOT IN \
             (SELECT id FROM automation_runs WHERE automation_id = ?1 ORDER BY id DESC LIMIT ?2)",
            params![run.automation_id.as_str(), AUTOMATION_RUNS_KEPT as i64],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// An automation's latest runs, newest first.
    pub fn automation_runs(&self, id: &AutomationId, limit: usize) -> Result<Vec<AutomationRun>> {
        let conn = self.lock();
        let mut statement = conn.prepare(
            "SELECT data FROM automation_runs WHERE automation_id = ?1 ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = statement
            .query_map(params![id.as_str(), limit as i64], |row| row.get::<_, String>(0))?;
        let mut runs = Vec::new();
        for data in rows {
            runs.push(decode(&data?)?);
        }
        Ok(runs)
    }

    fn lock(&self) -> MutexGuard<'_, Connection> {
        // A panic while holding the lock cannot leave a transaction half-applied:
        // rusqlite rolls back on drop.
        self.conn.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn encode(value: &impl serde::Serialize) -> Result<String> {
    serde_json::to_string(value).map_err(StoreError::Encode)
}

fn decode<T: serde::de::DeserializeOwned>(text: &str) -> Result<T> {
    serde_json::from_str(text).map_err(StoreError::Corrupt)
}

/// Fold every thread's projection again from its journal.
fn rebuild_projections(tx: &Transaction<'_>) -> Result<()> {
    let ids: Vec<String> = {
        let mut statement = tx.prepare("SELECT id FROM threads")?;
        statement.query_map([], |row| row.get(0))?.collect::<rusqlite::Result<_>>()?
    };
    let mut statement =
        tx.prepare("SELECT seq, ts, payload FROM events WHERE thread_id = ?1 ORDER BY seq")?;
    for id in ids {
        let thread_id = ThreadId::new(id.as_str());
        let mut thread: Option<Thread> = None;
        let rows = statement.query_map([id.as_str()], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?))
        })?;
        for row in rows {
            let (seq, ts, payload) = row?;
            let event = ThreadEvent {
                thread_id: thread_id.clone(),
                seq: seq.cast_unsigned(),
                ts: ts.cast_unsigned(),
                payload: decode(&payload)?,
            };
            if thread.is_none()
                && let EventPayload::ThreadCreated { thread: created } = &event.payload
            {
                let mut created = created.as_ref().clone();
                created.last_seq = 0;
                thread = Some(created);
            }
            // A journal that does not start with its creation keeps the stored projection.
            let Some(thread) = thread.as_mut() else { break };
            thread.apply(&event);
        }
        if let Some(thread) = thread {
            save_thread(tx, &thread)?;
        }
    }
    Ok(())
}

fn load_thread(tx: &Transaction<'_>, id: &ThreadId) -> Result<Option<Thread>> {
    let data: Option<String> = tx
        .query_row("SELECT data FROM threads WHERE id = ?1", [id.as_str()], |row| row.get(0))
        .optional()?;
    Ok(match data {
        Some(data) => Some(decode(&data)?),
        None => None,
    })
}

fn insert_thread(tx: &Transaction<'_>, thread: &Thread) -> Result<()> {
    tx.execute(
        "INSERT INTO threads (id, created_at, data) VALUES (?1, ?2, ?3)",
        params![thread.id.as_str(), thread.created_at.cast_signed(), encode(thread)?],
    )?;
    Ok(())
}

fn save_thread(tx: &Transaction<'_>, thread: &Thread) -> Result<()> {
    tx.execute(
        "UPDATE threads SET data = ?2 WHERE id = ?1",
        params![thread.id.as_str(), encode(thread)?],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use qcc_domain::{Delta, ItemKey, ProjectIcon, TextChannel, ThreadStatus, TurnStatus};
    use qcc_runtime::{NewThread, SequentialMint, Supervisor};

    use super::*;

    #[test]
    fn projects_are_saved_replaced_and_deleted() {
        let store = Store::open_in_memory().unwrap();
        let mut project = Project {
            id: ProjectId::new("p"),
            name: "App".into(),
            folders: vec!["/src/app".into()],
            created_at: 5,
            icon: None,
        };
        store.save_project(&project).unwrap();
        project.folders.push("/src/api".into());
        project.icon = Some(ProjectIcon::Emoji { emoji: "🛒".into() });
        store.save_project(&project).unwrap();
        assert_eq!(store.projects().unwrap(), [project.clone()]);
        store.delete_project(&project.id).unwrap();
        assert!(store.projects().unwrap().is_empty());
    }

    #[test]
    fn provider_limits_are_kept_per_provider_and_replaced() {
        use qcc_domain::{LimitWindow, RateLimits};
        let store = Store::open_in_memory().unwrap();
        let window = |used| LimitWindow {
            id: "five_hour".into(),
            label: "5-hour".into(),
            scope: None,
            used_percent: used,
            resets_at: Some(1_000),
            window_minutes: Some(300),
            status: None,
        };
        let report = |used| RateLimits { plan: None, windows: vec![window(used)], complete: true };
        let codex = ProviderLimits::new("codex".into(), report(4), 1);
        let mut claude = ProviderLimits::new("claude-code".into(), report(10), 1);
        store.save_provider_limits(&codex).unwrap();
        store.save_provider_limits(&claude).unwrap();
        claude.merge(report(42), 2);
        store.save_provider_limits(&claude).unwrap();
        assert_eq!(store.provider_limits().unwrap(), [claude, codex]);
    }

    #[test]
    fn automations_keep_their_latest_runs_and_take_them_along() {
        use qcc_domain::{
            AutomationOutcome, AutomationOverlap, AutomationSpec, AutomationThreadMode,
            AutomationTrigger, ProviderId,
        };
        let store = Store::open_in_memory().unwrap();
        let mut automation = Automation {
            id: AutomationId::new("a"),
            spec: AutomationSpec {
                name: "Nightly".into(),
                schedule: "@daily".into(),
                prompt: "check".into(),
                provider: ProviderId::new("echo"),
                cwd: "/src".into(),
                environment: None,
                model: None,
                effort: None,
                permission_mode: None,
                thread_mode: AutomationThreadMode::New,
                overlap: AutomationOverlap::Skip,
                catch_up: false,
                enabled: true,
            },
            created_at: 1,
            thread_id: None,
            anchor: 1,
        };
        store.save_automation(&automation).unwrap();
        automation.spec.enabled = false;
        store.save_automation(&automation).unwrap();
        assert_eq!(store.automations().unwrap(), [automation.clone()]);

        let run = |at: u64| AutomationRun {
            automation_id: automation.id.clone(),
            at,
            scheduled_for: Some(at),
            trigger: AutomationTrigger::Schedule,
            outcome: AutomationOutcome::Skipped,
            thread_id: None,
            reason: None,
        };
        let total = AUTOMATION_RUNS_KEPT as u64 + 5;
        for at in 0..total {
            store.add_automation_run(&run(at)).unwrap();
        }
        let runs = store.automation_runs(&automation.id, 1000).unwrap();
        assert_eq!(runs.len(), AUTOMATION_RUNS_KEPT);
        assert_eq!(runs[0].at, total - 1, "newest first");
        assert_eq!(store.automation_runs(&automation.id, 2).unwrap().len(), 2);

        store.delete_automation(&automation.id).unwrap();
        assert!(store.automations().unwrap().is_empty());
        assert!(store.automation_runs(&automation.id, 1000).unwrap().is_empty());
    }

    #[test]
    fn comments_are_kept_per_thread_and_go_with_it() {
        let store = Store::open_in_memory().unwrap();
        let mut supervisor = Supervisor::new(|| Box::new(SequentialMint::default()));
        store.append(&scenario(&mut supervisor)).unwrap();
        let comment = |id: &str, thread: &str, created_at: u64| LineComment {
            id: CommentId::new(id),
            thread_id: ThreadId::new(thread),
            path: "src/lib.rs".into(),
            line: 3,
            side: qcc_domain::DiffSide::New,
            quote: Some("let x = 1;".into()),
            text: "name it".into(),
            created_at,
            resolved_at: None,
            resolution: None,
            author: None,
        };
        let mut first = comment("c1", "a", 10);
        store.save_comment(&comment("c2", "a", 20)).unwrap();
        store.save_comment(&first).unwrap();
        store.save_comment(&comment("c3", "b", 5)).unwrap();
        first.resolved_at = Some(30);
        first.resolution = Some("renamed".into());
        store.save_comment(&first).unwrap();

        let ids = |thread: &str| -> Vec<String> {
            let comments = store.comments(&thread.into()).unwrap();
            comments.into_iter().map(|c| c.id.0).collect()
        };
        assert_eq!(ids("a"), ["c1", "c2"]);
        assert_eq!(store.comment(&"c1".into()).unwrap(), Some(first));
        store.delete_comment(&"c2".into()).unwrap();
        assert_eq!(ids("a"), ["c1"]);
        assert_eq!(store.comment(&"zzz".into()).unwrap(), None);
        // A comment cannot outlive its thread, nor be left on one that is not there.
        store.lock().execute("DELETE FROM threads WHERE id = 'b'", []).unwrap();
        assert!(ids("b").is_empty());
        assert!(store.save_comment(&comment("c4", "nope", 1)).is_err());
    }

    fn new_thread(id: &str) -> NewThread {
        NewThread {
            id: id.into(),
            provider: "echo".into(),
            cwd: "/work".into(),
            environment: None,
            model: None,
            effort: None,
            parent: None,
            project: None,
            permission_mode: None,
            skills_enabled: Vec::new(),
            skills_disabled: Vec::new(),
            started_by: None,
            hidden: false,
            automation: None,
        }
    }

    /// Drive two threads through the runtime, interleaved, and return their journal.
    fn scenario(supervisor: &mut Supervisor) -> Vec<ThreadEvent> {
        let mut journal = supervisor.create(new_thread("a"), 1).unwrap();
        journal.extend(supervisor.create(new_thread("b"), 2).unwrap());
        for (id, ts) in [("a", 10), ("b", 20)] {
            let engine = supervisor.engine(&id.into()).unwrap();
            journal.extend(engine.submit_input(format!("hello from {id}").into(), ts));
            journal.extend(engine.session_started(Some(format!("sess-{id}")), false, ts + 1));
            journal.extend(engine.apply_deltas(
                [
                    Delta::TurnOpen,
                    Delta::ItemTextDelta {
                        key: ItemKey::Stream("assistant".into()),
                        channel: TextChannel::AgentMessage,
                        text: "hi".into(),
                    },
                ],
                ts + 2,
            ));
        }
        let engine = supervisor.engine(&"a".into()).unwrap();
        journal.extend(engine.apply_deltas(
            [Delta::TurnBoundary {
                status: TurnStatus::Completed,
                error: None,
                claim_if_idle: false,
            }],
            30,
        ));
        journal
    }

    #[test]
    fn journal_and_projection_roundtrip() {
        let store = Store::open_in_memory().unwrap();
        let mut supervisor = Supervisor::new(|| Box::new(SequentialMint::default()));
        let journal = scenario(&mut supervisor);
        // Persist in the batches a daemon would: split anywhere.
        for batch in journal.chunks(3) {
            store.append(batch).unwrap();
        }

        let threads = store.threads().unwrap();
        assert_eq!(threads.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        for thread in &threads {
            assert_eq!(Some(thread), supervisor.thread(&thread.id), "projection drifted");
            assert_eq!(store.thread(&thread.id).unwrap().as_ref(), Some(thread));
        }
        assert_eq!(threads[0].status, ThreadStatus::Idle);
        assert_eq!(threads[1].status, ThreadStatus::Active);
        assert_eq!(threads[1].title.as_deref(), Some("hello from b"));

        let a: Vec<_> = journal.iter().filter(|e| e.thread_id.as_str() == "a").cloned().collect();
        assert_eq!(store.events(&"a".into(), 0).unwrap(), a);
        assert_eq!(store.events(&"a".into(), 4).unwrap(), a[4..]);
        assert_eq!(store.events(&"a".into(), 999).unwrap(), vec![]);
        assert_eq!(store.thread(&"zzz".into()).unwrap(), None);

        // `a` settled its reply; `b` is still streaming one.
        let user = |text: &str| TranscriptEntry { role: Role::User, text: text.into() };
        let agent = |text: &str| TranscriptEntry { role: Role::Agent, text: text.into() };
        assert_eq!(
            store.transcript(&"a".into(), false).unwrap(),
            [user("hello from a"), agent("hi")]
        );
        assert_eq!(store.transcript(&"a".into(), true).unwrap(), [agent("hi")]);
        assert_eq!(store.transcript(&"b".into(), false).unwrap(), [user("hello from b")]);
        assert_eq!(store.transcript(&"b".into(), true).unwrap(), []);
    }

    #[test]
    fn rejects_gaps_and_orphans_atomically() {
        let store = Store::open_in_memory().unwrap();
        let mut supervisor = Supervisor::new(|| Box::new(SequentialMint::default()));
        let journal = scenario(&mut supervisor);
        let a: Vec<_> = journal.iter().filter(|e| e.thread_id.as_str() == "a").cloned().collect();

        assert!(matches!(store.append(&a[1..]), Err(StoreError::MissingThread(_))));

        let mut gap = a.clone();
        gap.remove(2);
        assert!(matches!(store.append(&gap), Err(StoreError::OutOfOrder { last: 2, got: 4, .. })));
        // Nothing of the rejected batch was kept.
        assert_eq!(store.threads().unwrap(), vec![]);

        store.append(&a).unwrap();
        assert!(matches!(store.append(&a[a.len() - 1..]), Err(StoreError::OutOfOrder { .. })));
        assert_eq!(store.events(&"a".into(), 0).unwrap().len(), a.len());
    }

    #[test]
    fn survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("qcc.db");
        let mut supervisor = Supervisor::new(|| Box::new(SequentialMint::default()));
        let journal = scenario(&mut supervisor);
        Store::open(&path).unwrap().append(&journal).unwrap();

        let store = Store::open(&path).unwrap();
        assert_eq!(store.threads().unwrap().len(), 2);
        assert_eq!(store.thread(&"b".into()).unwrap().as_ref(), supervisor.thread(&"b".into()));
        assert_eq!(store.events(&"b".into(), 0).unwrap().len(), 7);
        // Nothing follows a position past the end, however far past.
        assert_eq!(store.events(&"b".into(), u64::MAX).unwrap().len(), 0);
    }

    /// A turn of `a` that spends tokens, an hour after the scenario.
    fn spending(supervisor: &mut Supervisor, ts: u64) -> Vec<ThreadEvent> {
        let engine = supervisor.engine(&"a".into()).unwrap();
        let total = qcc_domain::TokenUsage {
            input_tokens: 100,
            output_tokens: 7,
            cached_input_tokens: None,
        };
        let mut events = engine.submit_input("more".to_owned().into(), ts);
        events.extend(engine.apply_deltas([Delta::TurnOpen], ts));
        events.extend(engine.apply_deltas(
            [
                Delta::Usage {
                    usage: qcc_domain::UsageSnapshot {
                        total,
                        last: total,
                        model_context_window: Some(1000),
                        context_used: Some(100),
                    },
                },
                Delta::TurnBoundary {
                    status: TurnStatus::Failed,
                    error: None,
                    claim_if_idle: false,
                },
            ],
            ts + 500,
        ));
        events
    }

    #[test]
    fn spending_is_projected_and_kept_by_the_hour() {
        let store = Store::open_in_memory().unwrap();
        let mut supervisor = Supervisor::new(|| Box::new(SequentialMint::default()));
        store.append(&scenario(&mut supervisor)).unwrap();
        store.append(&spending(&mut supervisor, USAGE_BUCKET_MS + 40)).unwrap();

        let a = store.thread(&"a".into()).unwrap().unwrap();
        assert_eq!(Some(&a), supervisor.thread(&"a".into()), "projection drifted");
        assert_eq!(a.usage.spent.input_tokens, 100);
        assert_eq!((a.usage.context_used, a.usage.context_window), (Some(100), Some(1000)));
        assert_eq!((a.usage.turns_completed, a.usage.turns_failed), (1, 1));

        let history = store.usage_history().unwrap();
        // The scenario's turn in the first hour, the spending in the second; `b` is
        // still in its first turn.
        let hours: Vec<_> = history.iter().map(|b| (b.thread_id.as_str(), b.hour)).collect();
        assert_eq!(hours, [("a", 0), ("a", USAGE_BUCKET_MS)]);
        assert_eq!(history[1].spent.input_tokens, 100);
        assert_eq!((history[1].turns_failed, history[1].working_ms), (1, 500));
    }

    #[test]
    fn uses_of_tools_are_counted_by_the_hour_they_ended() {
        use qcc_domain::{ItemShape, ItemStatus, ToolKind};
        let store = Store::open_in_memory().unwrap();
        let mut supervisor = Supervisor::new(|| Box::new(SequentialMint::default()));
        store.append(&scenario(&mut supervisor)).unwrap();
        let engine = supervisor.engine(&"b".into()).unwrap();
        let tool = |name: &str, server: Option<&str>| ItemShape::Tool {
            name: name.into(),
            input: serde_json::Value::Null,
            output: None,
            mcp_server: server.map(Into::into),
            skill: None,
        };
        let key = |name: &str| ItemKey::Provider(name.into());
        let mut events = engine.apply_deltas(
            [
                Delta::ItemOpen { key: key("1"), shape: tool("search", Some("docs")) },
                Delta::ItemOpen { key: key("2"), shape: tool("search", Some("docs")) },
            ],
            1_000,
        );
        events.extend(engine.apply_deltas(
            [Delta::ItemClose {
                key: key("1"),
                shape: tool("search", Some("docs")),
                status: ItemStatus::Completed,
            }],
            1_250,
        ));
        events.extend(engine.apply_deltas(
            [Delta::ItemClose {
                key: key("2"),
                shape: tool("search", Some("docs")),
                status: ItemStatus::Failed,
            }],
            USAGE_BUCKET_MS + 1_000,
        ));
        store.append(&events).unwrap();

        let history = store.tool_history().unwrap();
        let mcp: Vec<_> = history.iter().filter(|b| b.kind == ToolKind::Mcp).collect();
        assert_eq!(mcp.len(), 2, "{history:#?}");
        assert_eq!((mcp[0].hour, mcp[0].calls, mcp[0].failed, mcp[0].duration_ms), (0, 1, 0, 250));
        assert_eq!(mcp[0].server.as_deref(), Some("docs"));
        assert_eq!((mcp[1].hour, mcp[1].calls, mcp[1].failed), (USAGE_BUCKET_MS, 1, 1));
        assert_eq!(mcp[1].duration_ms, USAGE_BUCKET_MS);
    }

    #[test]
    fn projections_made_by_an_older_fold_are_folded_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("qcc.db");
        let mut supervisor = Supervisor::new(|| Box::new(SequentialMint::default()));
        let store = Store::open(&path).unwrap();
        store.append(&scenario(&mut supervisor)).unwrap();
        store.append(&spending(&mut supervisor, 100)).unwrap();
        // As an older build left it: a projection without usage, and no version.
        {
            let mut conn = store.lock();
            let tx = conn.transaction().unwrap();
            let mut stale = load_thread(&tx, &"a".into()).unwrap().unwrap();
            stale.usage = ThreadUsage::default();
            save_thread(&tx, &stale).unwrap();
            tx.execute("DELETE FROM meta", []).unwrap();
            tx.commit().unwrap();
        }
        drop(store);

        let store = Store::open(&path).unwrap();
        for id in ["a", "b"] {
            assert_eq!(store.thread(&id.into()).unwrap().as_ref(), supervisor.thread(&id.into()));
        }
    }
}

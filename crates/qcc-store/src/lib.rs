//! SQLite persistence.
//!
//! The journal (`events`) is append-only and is the source of truth. The `threads`
//! table is a projection maintained in the same transaction as each append by folding
//! the events with [`Thread::apply`] — the very fold the runtime uses, so the stored
//! row cannot drift from what the engine holds.

use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use qcc_domain::{EventPayload, Thread, ThreadEvent, ThreadId};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("corrupt record: {0}")]
    Corrupt(#[from] serde_json::Error),
    #[error("thread {thread}: event seq {got} does not follow {last}")]
    OutOfOrder { thread: ThreadId, last: u64, got: u64 },
    #[error("thread {0}: the first event must be thread/created")]
    MissingThread(ThreadId),
    #[error("database schema version {found} is newer than this build supports ({supported})")]
    SchemaTooNew { found: u32, supported: u32 },
}

pub type Result<T> = std::result::Result<T, StoreError>;

/// Ordered schema migrations; `PRAGMA user_version` is the count applied.
const MIGRATIONS: &[&str] = &["
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
"];

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
        Ok(Self { conn: Mutex::new(conn) })
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
                            let mut thread = thread.clone();
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
                    event.seq as i64,
                    event.ts as i64,
                    serde_json::to_string(&event.payload)?
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
            threads.push(serde_json::from_str(&data?)?);
        }
        Ok(threads)
    }

    /// Events of a thread with `seq > after`, in order.
    pub fn events(&self, id: &ThreadId, after: u64) -> Result<Vec<ThreadEvent>> {
        let conn = self.lock();
        let mut statement = conn.prepare(
            "SELECT seq, ts, payload FROM events WHERE thread_id = ?1 AND seq > ?2 ORDER BY seq",
        )?;
        let rows = statement.query_map(params![id.as_str(), after as i64], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?))
        })?;
        let mut events = Vec::new();
        for row in rows {
            let (seq, ts, payload) = row?;
            events.push(ThreadEvent {
                thread_id: id.clone(),
                seq: seq as u64,
                ts: ts as u64,
                payload: serde_json::from_str(&payload)?,
            });
        }
        Ok(events)
    }

    fn lock(&self) -> MutexGuard<'_, Connection> {
        // A panic while holding the lock cannot leave a transaction half-applied:
        // rusqlite rolls back on drop.
        self.conn.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn load_thread(tx: &Transaction<'_>, id: &ThreadId) -> Result<Option<Thread>> {
    let data: Option<String> = tx
        .query_row("SELECT data FROM threads WHERE id = ?1", [id.as_str()], |row| row.get(0))
        .optional()?;
    Ok(match data {
        Some(data) => Some(serde_json::from_str(&data)?),
        None => None,
    })
}

fn insert_thread(tx: &Transaction<'_>, thread: &Thread) -> Result<()> {
    tx.execute(
        "INSERT INTO threads (id, created_at, data) VALUES (?1, ?2, ?3)",
        params![thread.id.as_str(), thread.created_at as i64, serde_json::to_string(thread)?],
    )?;
    Ok(())
}

fn save_thread(tx: &Transaction<'_>, thread: &Thread) -> Result<()> {
    tx.execute(
        "UPDATE threads SET data = ?2 WHERE id = ?1",
        params![thread.id.as_str(), serde_json::to_string(thread)?],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use qcc_domain::{Delta, ItemKey, TextChannel, ThreadStatus, TurnStatus};
    use qcc_runtime::{NewThread, SequentialMint, Supervisor};

    use super::*;

    fn new_thread(id: &str) -> NewThread {
        NewThread {
            id: id.into(),
            provider: "echo".into(),
            cwd: "/work".into(),
            model: None,
            parent: None,
            permission_mode: None,
        }
    }

    /// Drive two threads through the runtime, interleaved, and return their journal.
    fn scenario(supervisor: &mut Supervisor) -> Vec<ThreadEvent> {
        let mut journal = supervisor.create(new_thread("a"), 1).unwrap();
        journal.extend(supervisor.create(new_thread("b"), 2).unwrap());
        for (id, ts) in [("a", 10), ("b", 20)] {
            let engine = supervisor.engine(&id.into()).unwrap();
            journal.extend(engine.submit_input(format!("hello from {id}"), ts));
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
    }
}

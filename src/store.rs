use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::event::{Event, LogEntry, StoredEvent, StoredLog};

pub type SharedEventStore = Arc<Mutex<EventStore>>;

#[derive(Debug, Clone, Serialize)]
pub struct InboxItem {
    pub event: StoredEvent,
    pub attempts: u32,
}

#[derive(Debug, Clone, Default)]
pub struct EventTailQuery {
    pub event_types: Vec<String>,
    pub event_type_prefix: Option<String>,
    pub correlation_id: Option<Uuid>,
    pub after_sequence: i64,
    pub limit: usize,
}

#[derive(Debug, Clone, Default)]
pub struct StoreEventQuery {
    pub event_type: Option<String>,
    pub event_type_prefix: Option<String>,
    pub source: Option<String>,
    pub event_id: Option<Uuid>,
    pub causation_id: Option<Uuid>,
    pub correlation_id: Option<Uuid>,
    pub before_sequence: Option<i64>,
    pub after_sequence: Option<i64>,
    pub occurred_after: Option<DateTime<Utc>>,
    pub occurred_before: Option<DateTime<Utc>>,
    pub payload_contains: Option<String>,
    pub limit: usize,
}

#[derive(Debug, Clone, Default)]
pub struct StoreLogQuery {
    pub level: Option<String>,
    pub category: Option<String>,
    pub name: Option<String>,
    pub name_prefix: Option<String>,
    pub dispatch_id: Option<Uuid>,
    pub event_id: Option<Uuid>,
    pub correlation_id: Option<Uuid>,
    pub action_group_id: Option<String>,
    pub action_id: Option<String>,
    pub tool_call_id: Option<String>,
    pub before_sequence: Option<i64>,
    pub after_sequence: Option<i64>,
    pub occurred_after: Option<DateTime<Utc>>,
    pub occurred_before: Option<DateTime<Utc>>,
    pub payload_contains: Option<String>,
    pub limit: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelUsageStats {
    pub provider: String,
    pub model: String,
    pub invocations: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub total_tokens: u64,
    pub estimated_cost_usd: Option<f64>,
    pub priced_invocations: u64,
    pub last_invocation_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EventTypeUsageStats {
    pub event_type: String,
    pub invocations: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub average_duration_ms: Option<f64>,
    pub last_invocation_at: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ToolUsageStats {
    pub tool: String,
    pub advertised_invocations: u64,
    pub dispatches_advertised: u64,
    pub calls: u64,
    pub dispatches_used: u64,
    pub succeeded: u64,
    pub failed: u64,
    pub estimated_schema_tokens: u64,
    pub average_duration_ms: Option<f64>,
    pub last_advertised_at: Option<String>,
    pub last_called_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UsageStats {
    pub invocations: u64,
    pub failed_invocations: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub total_tokens: u64,
    pub estimated_cost_usd: Option<f64>,
    pub priced_invocations: u64,
    pub models: Vec<ModelUsageStats>,
    pub event_types: Vec<EventTypeUsageStats>,
    pub tools: Vec<ToolUsageStats>,
}

#[derive(Debug, Deserialize, Serialize)]
struct FilesystemRootGrant {
    path: String,
    identity: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProcessExecutableGrant {
    pub alias: String,
    pub path: String,
    pub identity: Option<String>,
    pub sha256: String,
}

#[cfg(unix)]
fn filesystem_identity(metadata: &std::fs::Metadata) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    Some(format!("{}:{}", metadata.dev(), metadata.ino()))
}

#[cfg(not(unix))]
fn filesystem_identity(_metadata: &std::fs::Metadata) -> Option<String> {
    None
}

pub struct EventStore {
    connection: Connection,
}

const SCHEMA_VERSION: i64 = 2;

impl EventStore {
    pub fn open(path: &str) -> Result<Self> {
        let connection = Connection::open(path)
            .with_context(|| format!("failed to open event store at {path}"))?;
        let schema_version =
            connection.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))?;
        let has_existing_schema = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name IN ('events', 'logs'))",
            [],
            |row| row.get::<_, bool>(0),
        )?;
        if schema_version == 0 && has_existing_schema {
            anyhow::bail!(
                "database predates the durable event engine; configure HABIBI_DB to a new empty database (no migration or backfill is supported)"
            );
        }
        if schema_version != 0 && schema_version != SCHEMA_VERSION {
            anyhow::bail!(
                "unsupported database schema version {schema_version}; expected {SCHEMA_VERSION}"
            );
        }
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;

             CREATE TABLE IF NOT EXISTS events (
                 sequence        INTEGER PRIMARY KEY AUTOINCREMENT,
                 id              TEXT NOT NULL UNIQUE,
                 event_type      TEXT NOT NULL,
                 source          TEXT NOT NULL,
                 occurred_at     TEXT NOT NULL,
                 causation_id    TEXT,
                 correlation_id  TEXT NOT NULL,
                 payload         TEXT NOT NULL
             );

             CREATE INDEX IF NOT EXISTS events_by_type_sequence
                 ON events(event_type, sequence);
             CREATE INDEX IF NOT EXISTS events_by_correlation_sequence
                 ON events(correlation_id, sequence);
             CREATE INDEX IF NOT EXISTS events_by_causation_sequence
                 ON events(causation_id, sequence);

             CREATE TABLE IF NOT EXISTS event_inbox (
                 event_id     TEXT PRIMARY KEY REFERENCES events(id),
                 status       TEXT NOT NULL CHECK(status IN ('pending', 'processing', 'completed', 'failed')),
                 available_at TEXT NOT NULL,
                 claimed_at   TEXT,
                 claimed_by   TEXT,
                 completed_at TEXT,
                 attempts     INTEGER NOT NULL DEFAULT 0,
                 last_error   TEXT
             );
             CREATE INDEX IF NOT EXISTS event_inbox_by_status_available
                 ON event_inbox(status, available_at, event_id);

             CREATE TABLE IF NOT EXISTS engine_owner (
                 singleton  INTEGER PRIMARY KEY CHECK(singleton = 1),
                 owner_id   TEXT NOT NULL,
                 expires_at TEXT NOT NULL
             );

             CREATE TABLE IF NOT EXISTS engine_dispatch_outcomes (
                 event_id TEXT PRIMARY KEY REFERENCES events(id),
                 outcome  TEXT NOT NULL
             );

             CREATE TABLE IF NOT EXISTS engine_validation_retries (
                 event_id TEXT PRIMARY KEY REFERENCES events(id),
                 state    TEXT NOT NULL
             );

             CREATE TABLE IF NOT EXISTS engine_action_groups (
                 event_id           TEXT PRIMARY KEY REFERENCES events(id),
                 state              TEXT NOT NULL,
                 status             TEXT NOT NULL CHECK(status IN ('processing', 'completed')),
                 completed_event_id TEXT
             );
             CREATE TABLE IF NOT EXISTS engine_action_results (
                 event_id       TEXT NOT NULL REFERENCES engine_action_groups(event_id),
                 call_index     INTEGER NOT NULL,
                 result_event_id TEXT NOT NULL REFERENCES events(id),
                 PRIMARY KEY (event_id, call_index)
             );

             CREATE TABLE IF NOT EXISTS logs (
                 sequence         INTEGER PRIMARY KEY AUTOINCREMENT,
                 id               TEXT NOT NULL UNIQUE,
                 occurred_at      TEXT NOT NULL,
                 level            TEXT NOT NULL,
                 category         TEXT NOT NULL,
                 name             TEXT NOT NULL,
                 dispatch_id      TEXT NOT NULL,
                 event_id TEXT,
                 correlation_id   TEXT NOT NULL,
                 action_group_id         TEXT,
                 action_id        TEXT,
                 tool_call_id     TEXT,
                 span_id          TEXT,
                 parent_span_id   TEXT,
                 payload          TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS logs_by_name_sequence ON logs(name, sequence);
             CREATE INDEX IF NOT EXISTS logs_by_dispatch_sequence ON logs(dispatch_id, sequence);
             CREATE INDEX IF NOT EXISTS logs_by_correlation_sequence ON logs(correlation_id, sequence);
             CREATE INDEX IF NOT EXISTS logs_by_trigger_sequence ON logs(event_id, sequence);

             CREATE TABLE IF NOT EXISTS extension_kv (
                 extension_id TEXT NOT NULL,
                 key          TEXT NOT NULL,
                 value        TEXT NOT NULL,
                 updated_at   TEXT NOT NULL,
                 PRIMARY KEY (extension_id, key)
             );

             CREATE TABLE IF NOT EXISTS extension_settings (
                 extension_id TEXT PRIMARY KEY,
                 enabled      INTEGER NOT NULL,
                 updated_at   TEXT NOT NULL
             );

             CREATE TABLE IF NOT EXISTS extension_grants (
                 extension_id     TEXT PRIMARY KEY,
                 filesystem_roots TEXT NOT NULL,
                 updated_at       TEXT NOT NULL
             );

             CREATE TABLE IF NOT EXISTS extension_process_grants (
                 extension_id TEXT PRIMARY KEY,
                 executables  TEXT NOT NULL,
                 updated_at   TEXT NOT NULL
             );

             CREATE TABLE IF NOT EXISTS event_links (
                 link_id          TEXT PRIMARY KEY,
                 from_event_id    TEXT NOT NULL,
                 to_event_id      TEXT NOT NULL,
                 relation         TEXT NOT NULL,
                 description      TEXT,
                 bidirectional    INTEGER NOT NULL,
                 active           INTEGER NOT NULL,
                 created_event_id TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS event_links_from ON event_links(from_event_id, relation);
             CREATE INDEX IF NOT EXISTS event_links_to ON event_links(to_event_id, relation);

             CREATE TABLE IF NOT EXISTS event_acceptances (
                 source          TEXT NOT NULL,
                 event_type      TEXT NOT NULL,
                 idempotency_key TEXT NOT NULL,
                 event_id        TEXT NOT NULL REFERENCES events(id),
                 response_json   TEXT,
                 PRIMARY KEY (source, event_type, idempotency_key)
             );

             PRAGMA user_version = 2;"
        )?;

        Ok(Self { connection })
    }

    pub fn shared(self) -> SharedEventStore {
        Arc::new(Mutex::new(self))
    }

    #[cfg(test)]
    pub fn append(&self, event: &Event) -> Result<i64> {
        self.insert_event(&self.connection, event)
    }

    pub fn append_and_enqueue(&self, event: &Event) -> Result<i64> {
        let transaction = self.connection.unchecked_transaction()?;
        let sequence = self.insert_event(&transaction, event)?;
        transaction.execute(
            "INSERT INTO event_inbox (event_id, status, available_at)
             VALUES (?1, 'pending', ?2)",
            params![event.id.to_string(), Utc::now().to_rfc3339()],
        )?;
        transaction.commit()?;
        Ok(sequence)
    }

    pub fn append_and_enqueue_idempotent(
        &self,
        event: &Event,
        idempotency_key: &str,
        response_json: Option<&serde_json::Value>,
    ) -> Result<(i64, Event, Option<serde_json::Value>, bool)> {
        anyhow::ensure!(
            !idempotency_key.is_empty() && idempotency_key.len() <= 200,
            "idempotency_key must be between 1 and 200 bytes"
        );
        let transaction = self.connection.unchecked_transaction()?;
        let existing = transaction.query_row(
            "SELECT acceptance.event_id, acceptance.response_json, event.sequence
             FROM event_acceptances acceptance JOIN events event ON event.id = acceptance.event_id
             WHERE acceptance.source = ?1 AND acceptance.event_type = ?2 AND acceptance.idempotency_key = ?3",
            params![event.source, event.event_type, idempotency_key],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, i64>(2)?)),
        ).optional()?;
        if let Some((event_id, response_json, sequence)) = existing {
            transaction.rollback()?;
            let stored = self
                .get_event(Some(&event_id), None)?
                .context("accepted event is missing")?;
            return Ok((
                sequence,
                stored.event,
                response_json
                    .map(|value| serde_json::from_str(&value))
                    .transpose()?,
                true,
            ));
        }
        let sequence = self.insert_event(&transaction, event)?;
        transaction.execute(
            "INSERT INTO event_inbox (event_id, status, available_at) VALUES (?1, 'pending', ?2)",
            params![event.id.to_string(), Utc::now().to_rfc3339()],
        )?;
        transaction.execute(
            "INSERT INTO event_acceptances (source, event_type, idempotency_key, event_id, response_json)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![event.source, event.event_type, idempotency_key, event.id.to_string(),
                response_json.map(serde_json::to_string).transpose()?],
        )?;
        transaction.commit()?;
        Ok((sequence, event.clone(), response_json.cloned(), false))
    }

    fn insert_event(&self, connection: &Connection, event: &Event) -> Result<i64> {
        connection.execute(
            "INSERT INTO events (
                id, event_type, source, occurred_at,
                causation_id, correlation_id, payload
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                event.id.to_string(),
                event.event_type,
                event.source,
                event.occurred_at.to_rfc3339(),
                event.causation_id.map(|id| id.to_string()),
                event.correlation_id.to_string(),
                serde_json::to_string(&event.payload)?,
            ],
        )?;
        let sequence = connection.last_insert_rowid();
        if event.event_type == "event.link.created" {
            self.project_link_on(connection, event)?;
        } else if event.event_type == "event.link.removed"
            && let Some(link_id) = event
                .payload
                .get("link_id")
                .and_then(serde_json::Value::as_str)
        {
            connection.execute(
                "UPDATE event_links SET active = 0 WHERE link_id = ?1",
                [link_id],
            )?;
        }
        Ok(sequence)
    }

    pub fn acquire_engine_owner(&self, owner_id: Uuid) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        let now = Utc::now().to_rfc3339();
        transaction.execute(
            "DELETE FROM engine_owner WHERE singleton = 1 AND expires_at <= ?1",
            [&now],
        )?;
        let inserted = transaction.execute(
            "INSERT OR IGNORE INTO engine_owner (singleton, owner_id, expires_at) VALUES (1, ?1, ?2)",
            params![owner_id.to_string(), (Utc::now() + chrono::Duration::seconds(10)).to_rfc3339()],
        )?;
        anyhow::ensure!(inserted == 1, "another Habibi engine owns this database");
        transaction.commit()?;
        Ok(())
    }

    pub fn renew_engine_owner(&self, owner_id: Uuid) -> Result<()> {
        let changed = self.connection.execute(
            "UPDATE engine_owner SET expires_at = ?2 WHERE singleton = 1 AND owner_id = ?1",
            params![
                owner_id.to_string(),
                (Utc::now() + chrono::Duration::seconds(10)).to_rfc3339()
            ],
        )?;
        anyhow::ensure!(changed == 1, "engine database ownership was lost");
        Ok(())
    }

    pub fn release_engine_owner(&self, owner_id: Uuid) -> Result<()> {
        self.connection.execute(
            "DELETE FROM engine_owner WHERE singleton = 1 AND owner_id = ?1",
            [owner_id.to_string()],
        )?;
        Ok(())
    }

    pub fn save_dispatch_outcome(
        &self,
        event_id: Uuid,
        outcome: &serde_json::Value,
        completed_log: &LogEntry,
    ) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute(
            "INSERT INTO engine_dispatch_outcomes (event_id, outcome) VALUES (?1, ?2)",
            params![event_id.to_string(), serde_json::to_string(outcome)?],
        )?;
        transaction.execute(
            "DELETE FROM engine_validation_retries WHERE event_id = ?1",
            [event_id.to_string()],
        )?;
        self.insert_log(&transaction, completed_log)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn save_terminal_dispatch_outcome(
        &self,
        event_id: Uuid,
        outcome: &serde_json::Value,
        completed_log: &LogEntry,
        terminal_log: &LogEntry,
    ) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute(
            "INSERT INTO engine_dispatch_outcomes (event_id, outcome) VALUES (?1, ?2)",
            params![event_id.to_string(), serde_json::to_string(outcome)?],
        )?;
        transaction.execute(
            "DELETE FROM engine_validation_retries WHERE event_id = ?1",
            [event_id.to_string()],
        )?;
        self.insert_log(&transaction, completed_log)?;
        self.insert_log(&transaction, terminal_log)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn dispatch_outcome(&self, event_id: Uuid) -> Result<Option<serde_json::Value>> {
        self.connection
            .query_row(
                "SELECT outcome FROM engine_dispatch_outcomes WHERE event_id = ?1",
                [event_id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(|value| serde_json::from_str(&value).map_err(Into::into))
            .transpose()
    }

    pub fn save_validation_retry(
        &self,
        event_id: Uuid,
        state: &serde_json::Value,
        completed_log: &LogEntry,
        validation_log: &LogEntry,
    ) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute(
            "INSERT INTO engine_validation_retries (event_id, state) VALUES (?1, ?2)
             ON CONFLICT(event_id) DO UPDATE SET state = excluded.state",
            params![event_id.to_string(), serde_json::to_string(state)?],
        )?;
        self.insert_log(&transaction, completed_log)?;
        self.insert_log(&transaction, validation_log)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn validation_retry(&self, event_id: Uuid) -> Result<Option<serde_json::Value>> {
        self.connection
            .query_row(
                "SELECT state FROM engine_validation_retries WHERE event_id = ?1",
                [event_id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(|value| serde_json::from_str(&value).map_err(Into::into))
            .transpose()
    }

    pub fn create_action_group(
        &self,
        event_id: Uuid,
        state: &serde_json::Value,
        requested_events: &[Event],
    ) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute(
            "INSERT INTO engine_action_groups (event_id, state, status) VALUES (?1, ?2, 'processing')",
            params![event_id.to_string(), serde_json::to_string(state)?],
        )?;
        for event in requested_events {
            self.insert_event(&transaction, event)?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn action_group(&self, event_id: Uuid) -> Result<Option<(serde_json::Value, bool)>> {
        self.connection
            .query_row(
                "SELECT state, status = 'completed' FROM engine_action_groups WHERE event_id = ?1",
                [event_id.to_string()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?)),
            )
            .optional()?
            .map(|(state, completed)| Ok((serde_json::from_str(&state)?, completed)))
            .transpose()
    }

    pub fn completed_action_indices(&self, event_id: Uuid) -> Result<Vec<usize>> {
        let mut statement = self.connection.prepare(
            "SELECT call_index FROM engine_action_results WHERE event_id = ?1 ORDER BY call_index",
        )?;
        Ok(statement
            .query_map([event_id.to_string()], |row| row.get::<_, usize>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn append_action_result(
        &self,
        current_event_id: Uuid,
        call_index: usize,
        effects: &[Event],
        result: &Event,
        enqueue: bool,
    ) -> Result<bool> {
        let transaction = self.connection.unchecked_transaction()?;
        let exists = transaction
            .query_row(
                "SELECT 1 FROM engine_action_results WHERE event_id = ?1 AND call_index = ?2",
                params![current_event_id.to_string(), call_index as i64],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if exists {
            return Ok(false);
        }
        for effect in effects {
            self.insert_event(&transaction, effect)?;
        }
        self.insert_event(&transaction, result)?;
        if enqueue {
            transaction.execute(
                "INSERT INTO event_inbox (event_id, status, available_at) VALUES (?1, 'pending', ?2)",
                params![result.id.to_string(), Utc::now().to_rfc3339()],
            )?;
        }
        transaction.execute(
            "INSERT INTO engine_action_results (event_id, call_index, result_event_id) VALUES (?1, ?2, ?3)",
            params![current_event_id.to_string(), call_index as i64, result.id.to_string()],
        )?;
        transaction.commit()?;
        Ok(true)
    }

    pub fn action_result_ids(&self, event_id: Uuid, count: usize) -> Result<Vec<Uuid>> {
        let mut statement = self.connection.prepare(
            "SELECT call_index, result_event_id FROM engine_action_results WHERE event_id = ?1 ORDER BY call_index",
        )?;
        let rows = statement
            .query_map([event_id.to_string()], |row| {
                Ok((row.get::<_, usize>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        anyhow::ensure!(rows.len() == count, "action group has incomplete results");
        rows.into_iter()
            .enumerate()
            .map(|(expected, (index, id))| {
                anyhow::ensure!(index == expected, "action group result index is missing");
                Ok(Uuid::parse_str(&id)?)
            })
            .collect()
    }

    pub fn complete_action_group(
        &self,
        event_id: Uuid,
        completed: &Event,
        enqueue: bool,
    ) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        self.insert_event(&transaction, completed)?;
        if enqueue {
            transaction.execute(
                "INSERT INTO event_inbox (event_id, status, available_at) VALUES (?1, 'pending', ?2)",
                params![completed.id.to_string(), Utc::now().to_rfc3339()],
            )?;
        }
        let changed = transaction.execute(
            "UPDATE engine_action_groups SET status = 'completed', completed_event_id = ?2
             WHERE event_id = ?1 AND status = 'processing'",
            params![event_id.to_string(), completed.id.to_string()],
        )?;
        anyhow::ensure!(changed == 1, "action group is not processing");
        transaction.commit()?;
        Ok(())
    }

    pub fn recover_processing(&self, owner_id: Uuid) -> Result<usize> {
        Ok(self.connection.execute(
            "UPDATE event_inbox
             SET status = 'pending', claimed_at = NULL, claimed_by = NULL,
                 available_at = ?1, last_error = 'worker stopped during processing'
             WHERE status = 'processing' AND (claimed_by IS NULL OR claimed_by != ?2)",
            params![Utc::now().to_rfc3339(), owner_id.to_string()],
        )?)
    }

    pub fn claim_next(&self, owner_id: Uuid) -> Result<Option<InboxItem>> {
        let now = Utc::now().to_rfc3339();
        let event_id = self
            .connection
            .query_row(
                "UPDATE event_inbox
             SET status = 'processing', claimed_at = ?1, claimed_by = ?2, attempts = attempts + 1
             WHERE event_id = (
                 SELECT inbox.event_id FROM event_inbox inbox
                 JOIN events event ON event.id = inbox.event_id
                 WHERE inbox.status = 'pending'
                 ORDER BY event.sequence ASC LIMIT 1
             ) AND status = 'pending' AND available_at <= ?1
             RETURNING event_id, attempts",
                params![now, owner_id.to_string()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?)),
            )
            .optional()?;
        let Some((event_id, attempts)) = event_id else {
            return Ok(None);
        };
        let event = self
            .get_event(Some(&event_id), None)?
            .context("claimed inbox event is missing")?;
        Ok(Some(InboxItem { event, attempts }))
    }

    pub fn complete_inbox(&self, event_id: Uuid) -> Result<()> {
        let changed = self.connection.execute(
            "UPDATE event_inbox SET status = 'completed', completed_at = ?2, claimed_by = NULL, last_error = NULL
             WHERE event_id = ?1 AND status = 'processing'",
            params![event_id.to_string(), Utc::now().to_rfc3339()],
        )?;
        anyhow::ensure!(changed == 1, "inbox item is not processing");
        Ok(())
    }

    pub fn fail_inbox(&self, event_id: Uuid, error: &str, retry: bool) -> Result<()> {
        let (status, available_at) = if retry {
            (
                "pending",
                (Utc::now() + chrono::Duration::seconds(1)).to_rfc3339(),
            )
        } else {
            ("failed", Utc::now().to_rfc3339())
        };
        let changed = self.connection.execute(
            "UPDATE event_inbox SET status = ?2, available_at = ?3, claimed_at = NULL, claimed_by = NULL,
                 completed_at = CASE WHEN ?2 = 'failed' THEN ?3 ELSE NULL END, last_error = ?4
             WHERE event_id = ?1 AND status = 'processing'",
            params![event_id.to_string(), status, available_at, error],
        )?;
        anyhow::ensure!(changed == 1, "inbox item is not processing");
        Ok(())
    }

    #[cfg(test)]
    pub fn inbox_status(&self, event_id: Uuid) -> Result<Option<String>> {
        Ok(self
            .connection
            .query_row(
                "SELECT status FROM event_inbox WHERE event_id = ?1",
                [event_id.to_string()],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn append_log(&self, log: &LogEntry) -> Result<i64> {
        self.insert_log(&self.connection, log)
    }

    fn insert_log(&self, connection: &Connection, log: &LogEntry) -> Result<i64> {
        connection.execute(
            "INSERT INTO logs (
                id, occurred_at, level, category, name, dispatch_id, event_id,
                correlation_id, action_group_id, action_id, tool_call_id, span_id, parent_span_id, payload
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                log.id.to_string(),
                log.occurred_at.to_rfc3339(),
                log.level,
                log.category,
                log.name,
                log.dispatch_id.to_string(),
                log.event_id.map(|id| id.to_string()),
                log.correlation_id.to_string(),
                log.action_group_id,
                log.action_id,
                log.tool_call_id,
                log.span_id,
                log.parent_span_id,
                serde_json::to_string(&log.payload)?
            ],
        )?;
        Ok(connection.last_insert_rowid())
    }

    pub fn query_events(&self, query: &StoreEventQuery) -> Result<Vec<StoredEvent>> {
        let occurred_after = query.occurred_after.map(|value| value.to_rfc3339());
        let occurred_before = query.occurred_before.map(|value| value.to_rfc3339());
        let mut statement = self.connection.prepare(
            "SELECT sequence, id, event_type, source, occurred_at,
                    causation_id, correlation_id, payload
             FROM (
                 SELECT sequence, id, event_type, source, occurred_at,
                        causation_id, correlation_id, payload
                 FROM events
                 WHERE (?1 IS NULL OR event_type = ?1)
                   AND (?2 IS NULL OR substr(event_type, 1, length(?2)) = ?2)
                   AND (?3 IS NULL OR source = ?3)
                   AND (?4 IS NULL OR id = ?4)
                   AND (?5 IS NULL OR causation_id = ?5)
                   AND (?6 IS NULL OR correlation_id = ?6)
                   AND (?7 IS NULL OR sequence < ?7)
                   AND (?8 IS NULL OR sequence > ?8)
                   AND (?9 IS NULL OR occurred_at >= ?9)
                   AND (?10 IS NULL OR occurred_at <= ?10)
                   AND (?11 IS NULL OR instr(lower(payload), lower(?11)) > 0)
                 ORDER BY sequence DESC
                 LIMIT ?12
             )
             ORDER BY sequence ASC",
        )?;

        let rows = statement
            .query_map(
                params![
                    query.event_type,
                    query.event_type_prefix,
                    query.source,
                    query.event_id.map(|id| id.to_string()),
                    query.causation_id.map(|id| id.to_string()),
                    query.correlation_id.map(|id| id.to_string()),
                    query.before_sequence,
                    query.after_sequence,
                    occurred_after,
                    occurred_before,
                    query.payload_contains,
                    query.limit as i64
                ],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                    ))
                },
            )?
            .map(|row| -> Result<StoredEvent> {
                let (
                    sequence,
                    id,
                    event_type,
                    source,
                    occurred_at,
                    causation_id,
                    correlation_id,
                    payload,
                ) = row?;
                Ok(StoredEvent {
                    sequence,
                    event: Event {
                        id: Uuid::parse_str(&id)?,
                        event_type,
                        source,
                        occurred_at: DateTime::parse_from_rfc3339(&occurred_at)?
                            .with_timezone(&Utc),
                        causation_id: causation_id.map(|id| Uuid::parse_str(&id)).transpose()?,
                        correlation_id: Uuid::parse_str(&correlation_id)?,
                        payload: serde_json::from_str(&payload)?,
                    },
                })
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(rows)
    }

    pub fn latest_event_sequence(&self) -> Result<i64> {
        Ok(self.connection.query_row(
            "SELECT COALESCE(MAX(sequence), 0) FROM events",
            [],
            |row| row.get(0),
        )?)
    }

    pub fn query_event_tail(&self, query: &EventTailQuery) -> Result<Vec<StoredEvent>> {
        let exact_types = if query.event_types.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&query.event_types)?)
        };
        let mut statement = self.connection.prepare(
            "SELECT sequence, id, event_type, source, occurred_at, causation_id, correlation_id, payload
             FROM events
             WHERE sequence > ?1
               AND (?2 IS NULL OR event_type IN (SELECT value FROM json_each(?2)))
               AND (?3 IS NULL OR substr(event_type, 1, length(?3)) = ?3)
               AND (?4 IS NULL OR correlation_id = ?4)
             ORDER BY sequence ASC LIMIT ?5",
        )?;
        statement
            .query_map(
                params![
                    query.after_sequence,
                    exact_types,
                    query.event_type_prefix,
                    query.correlation_id.map(|id| id.to_string()),
                    query.limit.clamp(1, 500) as i64,
                ],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                    ))
                },
            )?
            .map(|row| stored_event_from_parts(row?))
            .collect()
    }

    pub fn get_event(
        &self,
        event_id: Option<&str>,
        sequence: Option<i64>,
    ) -> Result<Option<StoredEvent>> {
        let row = self
            .connection
            .query_row(
                "SELECT sequence, id, event_type, source, occurred_at, causation_id, correlation_id, payload
                 FROM events WHERE (?1 IS NOT NULL AND id = ?1) OR (?2 IS NOT NULL AND sequence = ?2)",
                params![event_id, sequence],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?, row.get::<_, String>(4)?, row.get::<_, Option<String>>(5)?,
                        row.get::<_, String>(6)?, row.get::<_, String>(7)?,
                    ))
                },
            )
            .optional()?;
        row.map(stored_event_from_parts).transpose()
    }

    pub fn related_events(
        &self,
        event_id: &str,
        relation: Option<&str>,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>> {
        let mut statement = self.connection.prepare(
            "SELECT link_id, from_event_id, to_event_id, relation, description, bidirectional
             FROM event_links
             WHERE active = 1 AND (?2 IS NULL OR relation = ?2)
               AND (from_event_id = ?1 OR (bidirectional = 1 AND to_event_id = ?1))
             LIMIT ?3",
        )?;
        let links = statement
            .query_map(params![event_id, relation, limit as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, bool>(5)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        links
            .into_iter()
            .map(
                |(link_id, from, to, relation, description, bidirectional)| {
                    let neighbor_id = if from == event_id { &to } else { &from };
                    let event = self.get_event(Some(neighbor_id), None)?;
                    Ok(serde_json::json!({
                        "link_id": link_id, "from_event_id": from, "to_event_id": to,
                        "relation": relation, "description": description,
                        "bidirectional": bidirectional, "event": event
                    }))
                },
            )
            .collect()
    }

    fn project_link_on(&self, connection: &Connection, event: &Event) -> Result<()> {
        let string = |key: &str| {
            event
                .payload
                .get(key)
                .and_then(serde_json::Value::as_str)
                .with_context(|| format!("event.link.created missing '{key}'"))
        };
        connection.execute(
            "INSERT OR IGNORE INTO event_links (
                link_id, from_event_id, to_event_id, relation, description,
                bidirectional, active, created_event_id
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7)",
            params![
                string("link_id")?,
                string("from_event_id")?,
                string("to_event_id")?,
                string("relation")?,
                event
                    .payload
                    .get("description")
                    .and_then(serde_json::Value::as_str),
                event
                    .payload
                    .get("bidirectional")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(true),
                event.id.to_string()
            ],
        )?;
        Ok(())
    }

    pub fn query_logs(&self, query: &StoreLogQuery) -> Result<Vec<StoredLog>> {
        let occurred_after = query.occurred_after.map(|value| value.to_rfc3339());
        let occurred_before = query.occurred_before.map(|value| value.to_rfc3339());
        let mut statement = self.connection.prepare(
            "SELECT sequence, id, occurred_at, level, category, name, dispatch_id,
                    event_id, correlation_id, action_group_id, action_id, tool_call_id,
                    span_id, parent_span_id, payload
             FROM (
               SELECT * FROM logs WHERE
                 (?1 IS NULL OR level = ?1) AND (?2 IS NULL OR category = ?2)
                 AND (?3 IS NULL OR name = ?3)
                 AND (?4 IS NULL OR substr(name, 1, length(?4)) = ?4)
                 AND (?5 IS NULL OR dispatch_id = ?5)
                 AND (?6 IS NULL OR event_id = ?6)
                 AND (?7 IS NULL OR correlation_id = ?7)
                 AND (?8 IS NULL OR action_group_id = ?8) AND (?9 IS NULL OR action_id = ?9)
                 AND (?10 IS NULL OR tool_call_id = ?10)
                 AND (?11 IS NULL OR sequence < ?11) AND (?12 IS NULL OR sequence > ?12)
                 AND (?13 IS NULL OR occurred_at >= ?13) AND (?14 IS NULL OR occurred_at <= ?14)
                 AND (?15 IS NULL OR instr(lower(payload), lower(?15)) > 0)
               ORDER BY sequence DESC LIMIT ?16
             ) ORDER BY sequence ASC",
        )?;
        let rows = statement.query_map(
            params![
                query.level,
                query.category,
                query.name,
                query.name_prefix,
                query.dispatch_id.map(|id| id.to_string()),
                query.event_id.map(|id| id.to_string()),
                query.correlation_id.map(|id| id.to_string()),
                query.action_group_id,
                query.action_id,
                query.tool_call_id,
                query.before_sequence,
                query.after_sequence,
                occurred_after,
                occurred_before,
                query.payload_contains,
                query.limit as i64
            ],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, Option<String>>(10)?,
                    row.get::<_, Option<String>>(11)?,
                    row.get::<_, Option<String>>(12)?,
                    row.get::<_, Option<String>>(13)?,
                    row.get::<_, String>(14)?,
                ))
            },
        )?;
        rows.map(|row| stored_log_from_parts(row?)).collect()
    }

    pub fn get_log(&self, log_id: &str) -> Result<Option<StoredLog>> {
        let row = self
            .connection
            .query_row(
                "SELECT sequence, id, occurred_at, level, category, name, dispatch_id,
                    event_id, correlation_id, action_group_id, action_id, tool_call_id,
                    span_id, parent_span_id, payload FROM logs WHERE id = ?1",
                [log_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, Option<String>>(7)?,
                        row.get::<_, String>(8)?,
                        row.get::<_, Option<String>>(9)?,
                        row.get::<_, Option<String>>(10)?,
                        row.get::<_, Option<String>>(11)?,
                        row.get::<_, Option<String>>(12)?,
                        row.get::<_, Option<String>>(13)?,
                        row.get::<_, String>(14)?,
                    ))
                },
            )
            .optional()?;
        row.map(stored_log_from_parts).transpose()
    }

    pub fn usage_stats(&self) -> Result<UsageStats> {
        let mut statement = self.connection.prepare(
            "SELECT
                COALESCE(json_extract(payload, '$.provider'), 'unknown'),
                COALESCE(json_extract(payload, '$.model'), 'unknown'),
                COUNT(*),
                COALESCE(SUM(json_extract(payload, '$.usage.input')), 0),
                COALESCE(SUM(json_extract(payload, '$.usage.output')), 0),
                COALESCE(SUM(json_extract(payload, '$.usage.cache_read')), 0),
                COALESCE(SUM(json_extract(payload, '$.usage.cache_write')), 0),
                COALESCE(SUM(json_extract(payload, '$.usage.total_tokens')), 0),
                SUM(json_extract(payload, '$.estimated_cost.total_usd')),
                COUNT(json_extract(payload, '$.estimated_cost.total_usd')),
                MAX(occurred_at)
             FROM logs
             WHERE name = 'model.invocation.completed'
             GROUP BY 1, 2
             ORDER BY COUNT(*) DESC, 1 ASC, 2 ASC",
        )?;
        let models = statement
            .query_map([], |row| {
                Ok(ModelUsageStats {
                    provider: row.get(0)?,
                    model: row.get(1)?,
                    invocations: row.get::<_, i64>(2)? as u64,
                    input_tokens: row.get::<_, i64>(3)? as u64,
                    output_tokens: row.get::<_, i64>(4)? as u64,
                    cache_read_tokens: row.get::<_, i64>(5)? as u64,
                    cache_write_tokens: row.get::<_, i64>(6)? as u64,
                    total_tokens: row.get::<_, i64>(7)? as u64,
                    estimated_cost_usd: row.get(8)?,
                    priced_invocations: row.get::<_, i64>(9)? as u64,
                    last_invocation_at: row.get(10)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let failed_invocations = self.connection.query_row(
            "SELECT COUNT(*) FROM logs WHERE name = 'model.invocation.failed'",
            [],
            |row| row.get::<_, i64>(0),
        )? as u64;
        let estimated_cost_usd = models
            .iter()
            .filter_map(|model| model.estimated_cost_usd)
            .reduce(|left, right| left + right);
        let event_types = self.event_type_usage_stats()?;
        let tools = self.tool_usage_stats()?;
        Ok(UsageStats {
            invocations: models.iter().map(|model| model.invocations).sum(),
            failed_invocations,
            input_tokens: models.iter().map(|model| model.input_tokens).sum(),
            output_tokens: models.iter().map(|model| model.output_tokens).sum(),
            cache_read_tokens: models.iter().map(|model| model.cache_read_tokens).sum(),
            cache_write_tokens: models.iter().map(|model| model.cache_write_tokens).sum(),
            total_tokens: models.iter().map(|model| model.total_tokens).sum(),
            estimated_cost_usd,
            priced_invocations: models.iter().map(|model| model.priced_invocations).sum(),
            models,
            event_types,
            tools,
        })
    }

    fn event_type_usage_stats(&self) -> Result<Vec<EventTypeUsageStats>> {
        let mut statement = self.connection.prepare(
            "SELECT COALESCE(events.event_type, 'unknown'), COUNT(*),
                    COALESCE(SUM(json_extract(logs.payload, '$.usage.input')), 0),
                    COALESCE(SUM(json_extract(logs.payload, '$.usage.output')), 0),
                    COALESCE(SUM(json_extract(logs.payload, '$.usage.total_tokens')), 0),
                    AVG(json_extract(logs.payload, '$.duration_ms')),
                    MAX(logs.occurred_at)
             FROM logs
             INNER JOIN events ON events.id = logs.event_id
             WHERE logs.name = 'model.invocation.completed'
             GROUP BY 1
             ORDER BY COUNT(*) DESC, 1 ASC",
        )?;
        statement
            .query_map([], |row| {
                Ok(EventTypeUsageStats {
                    event_type: row.get(0)?,
                    invocations: row.get::<_, i64>(1)? as u64,
                    input_tokens: row.get::<_, i64>(2)? as u64,
                    output_tokens: row.get::<_, i64>(3)? as u64,
                    total_tokens: row.get::<_, i64>(4)? as u64,
                    average_duration_ms: row.get(5)?,
                    last_invocation_at: row.get(6)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    fn tool_usage_stats(&self) -> Result<Vec<ToolUsageStats>> {
        let mut tools = BTreeMap::<String, ToolUsageStats>::new();
        let mut advertised = self.connection.prepare(
            "SELECT json_extract(item.value, '$.tool'), COUNT(*),
                    COUNT(DISTINCT logs.correlation_id),
                    COALESCE(SUM(json_extract(item.value, '$.estimated_schema_tokens')), 0),
                    MAX(logs.occurred_at)
             FROM logs, json_each(logs.payload, '$.tools') AS item
             WHERE logs.name = 'tool.surface.prepared'
               AND json_extract(item.value, '$.decision') = 'advertised'
               AND json_extract(item.value, '$.tool') IS NOT NULL
             GROUP BY 1",
        )?;
        for row in advertised.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)? as u64,
                row.get::<_, i64>(2)? as u64,
                row.get::<_, i64>(3)? as u64,
                row.get::<_, Option<String>>(4)?,
            ))
        })? {
            let (tool, invocations, chains, schema_tokens, last) = row?;
            tools.insert(
                tool.clone(),
                ToolUsageStats {
                    tool,
                    advertised_invocations: invocations,
                    dispatches_advertised: chains,
                    estimated_schema_tokens: schema_tokens,
                    last_advertised_at: last,
                    ..ToolUsageStats::default()
                },
            );
        }

        let mut calls = self.connection.prepare(
            "SELECT json_extract(payload, '$.tool'), COUNT(*),
                    COUNT(DISTINCT correlation_id), MAX(occurred_at)
             FROM events
             WHERE event_type = 'action.requested'
               AND json_extract(payload, '$.tool') IS NOT NULL
             GROUP BY 1",
        )?;
        for row in calls.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)? as u64,
                row.get::<_, i64>(2)? as u64,
                row.get::<_, Option<String>>(3)?,
            ))
        })? {
            let (tool, count, chains, last) = row?;
            let stats = tools.entry(tool.clone()).or_insert_with(|| ToolUsageStats {
                tool,
                ..ToolUsageStats::default()
            });
            stats.calls = count;
            stats.dispatches_used = chains;
            stats.last_called_at = last;
        }

        let mut outcomes = self.connection.prepare(
            "SELECT json_extract(payload, '$.tool'),
                    SUM(CASE WHEN event_type = 'action.result.succeeded' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN event_type = 'action.result.failed' THEN 1 ELSE 0 END)
             FROM events
             WHERE event_type IN ('action.result.succeeded', 'action.result.failed')
               AND json_extract(payload, '$.tool') IS NOT NULL
             GROUP BY 1",
        )?;
        for row in outcomes.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)? as u64,
                row.get::<_, i64>(2)? as u64,
            ))
        })? {
            let (tool, succeeded, failed) = row?;
            let stats = tools.entry(tool.clone()).or_insert_with(|| ToolUsageStats {
                tool,
                ..ToolUsageStats::default()
            });
            stats.succeeded = succeeded;
            stats.failed = failed;
        }

        let mut durations = self.connection.prepare(
            "SELECT json_extract(payload, '$.tool'),
                    AVG(json_extract(payload, '$.duration_ms'))
             FROM logs
             WHERE name = 'action.execution.completed'
               AND json_extract(payload, '$.tool') IS NOT NULL
             GROUP BY 1",
        )?;
        for row in durations.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<f64>>(1)?))
        })? {
            let (tool, average) = row?;
            let stats = tools.entry(tool.clone()).or_insert_with(|| ToolUsageStats {
                tool,
                ..ToolUsageStats::default()
            });
            stats.average_duration_ms = average;
        }
        Ok(tools.into_values().collect())
    }

    pub fn extension_enabled(&self, extension_id: &str) -> Result<bool> {
        Ok(self
            .connection
            .query_row(
                "SELECT enabled FROM extension_settings WHERE extension_id = ?1",
                [extension_id],
                |row| row.get::<_, bool>(0),
            )
            .optional()?
            .unwrap_or(true))
    }

    pub fn set_extension_enabled(&self, extension_id: &str, enabled: bool) -> Result<()> {
        self.connection.execute(
            "INSERT INTO extension_settings (extension_id, enabled, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(extension_id) DO UPDATE SET
                 enabled = excluded.enabled,
                 updated_at = excluded.updated_at",
            params![extension_id, enabled, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn extension_filesystem_roots(&self, extension_id: &str) -> Result<Vec<String>> {
        let roots = self
            .connection
            .query_row(
                "SELECT filesystem_roots FROM extension_grants WHERE extension_id = ?1",
                [extension_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let grants = roots
            .map(|roots| serde_json::from_str::<Vec<FilesystemRootGrant>>(&roots))
            .transpose()?
            .unwrap_or_default();
        grants
            .into_iter()
            .map(|grant| {
                let metadata = std::fs::metadata(&grant.path).with_context(|| {
                    format!("granted filesystem root '{}' no longer exists", grant.path)
                })?;
                if !metadata.is_dir() || grant.identity != filesystem_identity(&metadata) {
                    anyhow::bail!(
                        "granted filesystem root '{}' changed identity; grant it again",
                        grant.path
                    );
                }
                Ok(grant.path)
            })
            .collect()
    }

    pub fn extension_process_executables(
        &self,
        extension_id: &str,
    ) -> Result<Vec<ProcessExecutableGrant>> {
        let grants = self
            .connection
            .query_row(
                "SELECT executables FROM extension_process_grants WHERE extension_id = ?1",
                [extension_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        grants
            .map(|grants| serde_json::from_str(&grants).map_err(Into::into))
            .unwrap_or_else(|| Ok(Vec::new()))
    }

    pub fn set_extension_process_executables(
        &mut self,
        extension_id: &str,
        grants: &[ProcessExecutableGrant],
    ) -> Result<()> {
        self.connection.execute(
            "INSERT INTO extension_process_grants (extension_id, executables, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(extension_id) DO UPDATE SET
                 executables = excluded.executables,
                 updated_at = excluded.updated_at",
            params![
                extension_id,
                serde_json::to_string(grants)?,
                Utc::now().to_rfc3339()
            ],
        )?;
        Ok(())
    }

    pub fn set_extension_filesystem_roots(
        &self,
        extension_id: &str,
        roots: &[String],
    ) -> Result<()> {
        let grants = roots
            .iter()
            .map(|path| {
                let metadata = std::fs::metadata(path)
                    .with_context(|| format!("filesystem root '{path}' does not exist"))?;
                if !metadata.is_dir() {
                    anyhow::bail!("filesystem root '{path}' is not a directory");
                }
                Ok(FilesystemRootGrant {
                    path: path.clone(),
                    identity: filesystem_identity(&metadata),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        self.connection.execute(
            "INSERT INTO extension_grants (extension_id, filesystem_roots, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(extension_id) DO UPDATE SET
                 filesystem_roots = excluded.filesystem_roots,
                 updated_at = excluded.updated_at",
            params![
                extension_id,
                serde_json::to_string(&grants)?,
                Utc::now().to_rfc3339()
            ],
        )?;
        Ok(())
    }

    pub fn kv_get(&self, extension_id: &str, key: &str) -> Result<Option<serde_json::Value>> {
        let value = self
            .connection
            .query_row(
                "SELECT value FROM extension_kv WHERE extension_id = ?1 AND key = ?2",
                params![extension_id, key],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        value
            .map(|value| serde_json::from_str(&value).map_err(Into::into))
            .transpose()
    }

    pub fn kv_set(&self, extension_id: &str, key: &str, value: &serde_json::Value) -> Result<()> {
        self.connection.execute(
            "INSERT INTO extension_kv (extension_id, key, value, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(extension_id, key) DO UPDATE SET
                 value = excluded.value,
                 updated_at = excluded.updated_at",
            params![
                extension_id,
                key,
                serde_json::to_string(value)?,
                Utc::now().to_rfc3339()
            ],
        )?;
        Ok(())
    }

    pub fn kv_delete(&self, extension_id: &str, key: &str) -> Result<bool> {
        Ok(self.connection.execute(
            "DELETE FROM extension_kv WHERE extension_id = ?1 AND key = ?2",
            params![extension_id, key],
        )? > 0)
    }

    pub fn kv_list(
        &self,
        extension_id: &str,
        prefix: &str,
    ) -> Result<Vec<(String, serde_json::Value)>> {
        let mut statement = self.connection.prepare(
            "SELECT key, value FROM extension_kv
             WHERE extension_id = ?1
               AND substr(key, 1, length(?2)) = ?2
             ORDER BY key ASC",
        )?;
        statement
            .query_map(params![extension_id, prefix], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .map(|row| -> Result<_> {
                let (key, value) = row?;
                Ok((key, serde_json::from_str(&value)?))
            })
            .collect()
    }
}

type StoredEventParts = (
    i64,
    String,
    String,
    String,
    String,
    Option<String>,
    String,
    String,
);

fn stored_event_from_parts(parts: StoredEventParts) -> Result<StoredEvent> {
    let (sequence, id, event_type, source, occurred_at, causation_id, correlation_id, payload) =
        parts;
    Ok(StoredEvent {
        sequence,
        event: Event {
            id: Uuid::parse_str(&id)?,
            event_type,
            source,
            occurred_at: DateTime::parse_from_rfc3339(&occurred_at)?.with_timezone(&Utc),
            causation_id: causation_id.map(|id| Uuid::parse_str(&id)).transpose()?,
            correlation_id: Uuid::parse_str(&correlation_id)?,
            payload: serde_json::from_str(&payload)?,
        },
    })
}

type StoredLogParts = (
    i64,
    String,
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    String,
);

fn stored_log_from_parts(parts: StoredLogParts) -> Result<StoredLog> {
    let (
        sequence,
        id,
        occurred_at,
        level,
        category,
        name,
        dispatch_id,
        event_id,
        correlation_id,
        action_group_id,
        action_id,
        tool_call_id,
        span_id,
        parent_span_id,
        payload,
    ) = parts;
    Ok(StoredLog {
        sequence,
        log: LogEntry {
            id: Uuid::parse_str(&id)?,
            occurred_at: DateTime::parse_from_rfc3339(&occurred_at)?.with_timezone(&Utc),
            level,
            category,
            name,
            dispatch_id: Uuid::parse_str(&dispatch_id)?,
            event_id: event_id.map(|id| Uuid::parse_str(&id)).transpose()?,
            correlation_id: Uuid::parse_str(&correlation_id)?,
            action_group_id,
            action_id,
            tool_call_id,
            span_id,
            parent_span_id,
            payload: serde_json::from_str(&payload)?,
        },
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tempfile::NamedTempFile;
    use uuid::Uuid;

    use super::*;

    #[test]
    fn rejects_unversioned_pre_engine_database() {
        let file = NamedTempFile::new().unwrap();
        rusqlite::Connection::open(file.path())
            .unwrap()
            .execute("CREATE TABLE events (id TEXT)", [])
            .unwrap();
        let error = match EventStore::open(file.path().to_str().unwrap()) {
            Ok(_) => panic!("pre-engine database was accepted"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("new empty database"));
    }

    #[test]
    fn queries_events_without_chat_assumptions() {
        let file = NamedTempFile::new().unwrap();
        let store = EventStore::open(file.path().to_str().unwrap()).unwrap();
        let correlation_id = Uuid::now_v7();

        for event_type in [
            "chat.session.created",
            "other.event",
            "chat.message.created",
        ] {
            store
                .append(&Event::new(
                    event_type,
                    "test",
                    correlation_id,
                    None,
                    json!({}),
                ))
                .unwrap();
        }

        let events = store
            .query_events(&StoreEventQuery {
                event_type_prefix: Some("chat.".into()),
                limit: 10,
                ..StoreEventQuery::default()
            })
            .unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].sequence, 1);
        assert_eq!(events[1].event.event_type, "chat.message.created");
    }

    #[test]
    fn treats_prefix_wildcards_literally() {
        let file = NamedTempFile::new().unwrap();
        let store = EventStore::open(file.path().to_str().unwrap()).unwrap();
        let correlation_id = Uuid::now_v7();
        for event_type in ["chat.%literal", "chat.anything"] {
            store
                .append(&Event::new(
                    event_type,
                    "test",
                    correlation_id,
                    None,
                    json!({}),
                ))
                .unwrap();
        }
        let events = store
            .query_events(&StoreEventQuery {
                event_type_prefix: Some("chat.%".into()),
                limit: 10,
                ..StoreEventQuery::default()
            })
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event.event_type, "chat.%literal");
    }

    #[test]
    fn projects_and_traverses_semantic_links() {
        let file = NamedTempFile::new().unwrap();
        let store = EventStore::open(file.path().to_str().unwrap()).unwrap();
        let correlation_id = Uuid::now_v7();
        let from = Event::new("test.one", "test", correlation_id, None, json!({}));
        let to = Event::new("test.two", "test", correlation_id, None, json!({}));
        store.append(&from).unwrap();
        store.append(&to).unwrap();
        store
            .append(&Event::new(
                "event.link.created",
                "habibi",
                correlation_id,
                None,
                json!({
                    "link_id": Uuid::now_v7(), "from_event_id": from.id,
                    "to_event_id": to.id, "relation": "related", "bidirectional": true
                }),
            ))
            .unwrap();
        let links = store
            .related_events(&to.id.to_string(), Some("related"), 10)
            .unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0]["event"]["id"], from.id.to_string());
    }

    #[test]
    fn aggregates_usage_cache_and_estimated_cost() {
        let file = NamedTempFile::new().unwrap();
        let store = EventStore::open(file.path().to_str().unwrap()).unwrap();
        let dispatch_id = Uuid::now_v7();
        store
            .append_log(&LogEntry::new(
                "info",
                "model",
                "model.invocation.completed",
                dispatch_id,
                None,
                dispatch_id,
                json!({
                    "model": "gpt-test",
                    "usage": { "input": 80, "output": 20, "cache_read": 40, "cache_write": 5, "total_tokens": 145 },
                    "estimated_cost": { "total_usd": 0.00125 }
                }),
            ))
            .unwrap();
        let stats = store.usage_stats().unwrap();
        assert_eq!(stats.invocations, 1);
        assert_eq!(stats.cache_read_tokens, 40);
        assert_eq!(stats.cache_write_tokens, 5);
        assert_eq!(stats.estimated_cost_usd, Some(0.00125));
        assert_eq!(stats.models[0].model, "gpt-test");
        assert!(stats.event_types.is_empty());
    }

    #[test]
    fn aggregates_tool_advertisements_calls_outcomes_and_duration() {
        let file = NamedTempFile::new().unwrap();
        let store = EventStore::open(file.path().to_str().unwrap()).unwrap();
        let correlation_id = Uuid::now_v7();
        store
            .append_log(&LogEntry::new(
                "debug",
                "tool",
                "tool.surface.prepared",
                correlation_id,
                None,
                correlation_id,
                json!({
                    "tools": [{
                        "tool": "example.read",
                        "decision": "advertised",
                        "estimated_schema_tokens": 42
                    }]
                }),
            ))
            .unwrap();
        let requested = Event::new(
            "action.requested",
            "habibi",
            correlation_id,
            None,
            json!({ "tool": "example.read" }),
        );
        store.append(&requested).unwrap();
        store
            .append(&Event::new(
                "action.result.succeeded",
                "habibi",
                correlation_id,
                Some(requested.id),
                json!({ "tool": "example.read" }),
            ))
            .unwrap();
        store
            .append_log(&LogEntry::new(
                "info",
                "action",
                "action.execution.completed",
                correlation_id,
                Some(requested.id),
                correlation_id,
                json!({ "tool": "example.read", "duration_ms": 12 }),
            ))
            .unwrap();
        store
            .append(&Event::new(
                "action.requested",
                "legacy",
                correlation_id,
                None,
                json!({ "legacy_shape": true }),
            ))
            .unwrap();
        store
            .append_log(&LogEntry::new(
                "info",
                "action",
                "action.execution.completed",
                correlation_id,
                None,
                correlation_id,
                json!({ "details": { "legacy_shape": true } }),
            ))
            .unwrap();
        let stats = store.usage_stats().unwrap();
        let tool = &stats.tools[0];
        assert_eq!(tool.tool, "example.read");
        assert_eq!(tool.advertised_invocations, 1);
        assert_eq!(tool.dispatches_advertised, 1);
        assert_eq!(tool.calls, 1);
        assert_eq!(tool.succeeded, 1);
        assert_eq!(tool.estimated_schema_tokens, 42);
        assert_eq!(tool.average_duration_ms, Some(12.0));
    }

    #[test]
    fn stores_and_queries_operational_logs_separately() {
        let file = NamedTempFile::new().unwrap();
        let store = EventStore::open(file.path().to_str().unwrap()).unwrap();
        let dispatch_id = Uuid::now_v7();
        store
            .append_log(&LogEntry::new(
                "info",
                "model",
                "model.invocation.started",
                dispatch_id,
                None,
                dispatch_id,
                json!({ "request": { "model": "test" } }),
            ))
            .unwrap();
        let logs = store
            .query_logs(&StoreLogQuery {
                category: Some("model".into()),
                payload_contains: Some("test".into()),
                limit: 10,
                ..StoreLogQuery::default()
            })
            .unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].log.name, "model.invocation.started");
        assert!(
            store
                .query_events(&StoreEventQuery {
                    limit: 10,
                    ..StoreEventQuery::default()
                })
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn inbox_is_transactional_fifo_and_recoverable() {
        let file = NamedTempFile::new().unwrap();
        let path = file.path().to_str().unwrap();
        let correlation = Uuid::now_v7();
        let first = Event::new("test.first", "test", correlation, None, json!({}));
        let second = Event::new("test.second", "test", correlation, None, json!({}));
        let not_enqueued = Event::new("test.history", "test", correlation, None, json!({}));
        let first_owner = Uuid::now_v7();
        {
            let store = EventStore::open(path).unwrap();
            store.append(&not_enqueued).unwrap();
            store.append_and_enqueue(&first).unwrap();
            store.append_and_enqueue(&second).unwrap();
            assert_eq!(store.inbox_status(not_enqueued.id).unwrap(), None);
            assert_eq!(
                store
                    .claim_next(first_owner)
                    .unwrap()
                    .unwrap()
                    .event
                    .event
                    .id,
                first.id
            );
        }
        let store = EventStore::open(path).unwrap();
        let second_owner = Uuid::now_v7();
        assert_eq!(store.recover_processing(second_owner).unwrap(), 1);
        let reclaimed = store.claim_next(second_owner).unwrap().unwrap();
        assert_eq!(reclaimed.event.event.id, first.id);
        assert_eq!(reclaimed.attempts, 2);
        store.complete_inbox(first.id).unwrap();
        assert_eq!(
            store
                .claim_next(second_owner)
                .unwrap()
                .unwrap()
                .event
                .event
                .id,
            second.id
        );
    }

    #[test]
    fn engine_owner_prevents_live_claim_recovery() {
        let file = NamedTempFile::new().unwrap();
        let path = file.path().to_str().unwrap();
        let first = EventStore::open(path).unwrap();
        let second = EventStore::open(path).unwrap();
        let first_owner = Uuid::now_v7();
        let second_owner = Uuid::now_v7();
        first.acquire_engine_owner(first_owner).unwrap();
        let event = Event::new("test.event", "test", Uuid::now_v7(), None, json!({}));
        first.append_and_enqueue(&event).unwrap();
        first.claim_next(first_owner).unwrap().unwrap();
        assert_eq!(first.recover_processing(first_owner).unwrap(), 0);
        assert!(
            second
                .acquire_engine_owner(second_owner)
                .unwrap_err()
                .to_string()
                .contains("owns")
        );
        first.renew_engine_owner(first_owner).unwrap();
        first.release_engine_owner(first_owner).unwrap();
        second.acquire_engine_owner(second_owner).unwrap();
    }

    #[test]
    fn retrying_oldest_event_cannot_be_overtaken() {
        let store = EventStore::open(":memory:").unwrap();
        let owner = Uuid::now_v7();
        let correlation = Uuid::now_v7();
        let first = Event::new("test.first", "test", correlation, None, json!({}));
        let second = Event::new("test.second", "test", correlation, None, json!({}));
        store.append_and_enqueue(&first).unwrap();
        store.append_and_enqueue(&second).unwrap();
        store.claim_next(owner).unwrap().unwrap();
        store.fail_inbox(first.id, "retry", true).unwrap();
        assert!(store.claim_next(owner).unwrap().is_none());
        store
            .connection
            .execute(
                "UPDATE event_inbox SET available_at = ?2 WHERE event_id = ?1",
                params![first.id.to_string(), Utc::now().to_rfc3339()],
            )
            .unwrap();
        assert_eq!(
            store.claim_next(owner).unwrap().unwrap().event.event.id,
            first.id
        );
    }

    #[test]
    fn failed_inbox_items_can_retry_then_stop() {
        let store = EventStore::open(":memory:").unwrap();
        let owner = Uuid::now_v7();
        let event = Event::new("test.event", "test", Uuid::now_v7(), None, json!({}));
        store.append_and_enqueue(&event).unwrap();
        store.claim_next(owner).unwrap().unwrap();
        store.fail_inbox(event.id, "temporary", true).unwrap();
        assert_eq!(
            store.inbox_status(event.id).unwrap().as_deref(),
            Some("pending")
        );
        store
            .connection
            .execute(
                "UPDATE event_inbox SET available_at = ?2 WHERE event_id = ?1",
                params![event.id.to_string(), Utc::now().to_rfc3339()],
            )
            .unwrap();
        store.claim_next(owner).unwrap().unwrap();
        store.fail_inbox(event.id, "permanent", false).unwrap();
        assert_eq!(
            store.inbox_status(event.id).unwrap().as_deref(),
            Some("failed")
        );
        assert!(store.claim_next(owner).unwrap().is_none());
    }

    #[test]
    fn dispatch_outcome_is_durable_before_action_group_creation() {
        let store = EventStore::open(":memory:").unwrap();
        let event = Event::new("test.event", "test", Uuid::now_v7(), None, json!({}));
        store.append_and_enqueue(&event).unwrap();
        let outcome = json!({ "calls": [], "model_response": { "content": "exact" } });
        let log = LogEntry::new(
            "info",
            "model",
            "model.invocation.completed",
            event.id,
            Some(event.id),
            event.correlation_id,
            json!({"content":"exact"}),
        );
        store
            .save_dispatch_outcome(event.id, &outcome, &log)
            .unwrap();
        assert_eq!(store.dispatch_outcome(event.id).unwrap(), Some(outcome));
        assert!(store.get_log(&log.id.to_string()).unwrap().is_some());
        assert!(store.action_group(event.id).unwrap().is_none());
    }

    #[test]
    fn validation_retry_state_and_logs_are_atomic_and_cleared_by_outcome() {
        let store = EventStore::open(":memory:").unwrap();
        let event = Event::new("test.event", "test", Uuid::now_v7(), None, json!({}));
        store.append_and_enqueue(&event).unwrap();
        let completed = LogEntry::new(
            "info",
            "model",
            "model.invocation.completed",
            event.id,
            Some(event.id),
            event.correlation_id,
            json!({}),
        );
        let validation = LogEntry::new(
            "warn",
            "tool",
            "tool.call_validation.failed",
            event.id,
            Some(event.id),
            event.correlation_id,
            json!({}),
        );
        let state = json!({ "failed_attempts": 1, "feedback": [] });
        store
            .save_validation_retry(event.id, &state, &completed, &validation)
            .unwrap();
        assert_eq!(store.validation_retry(event.id).unwrap(), Some(state));
        assert!(store.get_log(&completed.id.to_string()).unwrap().is_some());
        assert!(store.get_log(&validation.id.to_string()).unwrap().is_some());

        let outcome = json!({ "calls": [] });
        let final_log = LogEntry::new(
            "info",
            "model",
            "model.invocation.completed",
            event.id,
            Some(event.id),
            event.correlation_id,
            json!({}),
        );
        let terminal_log = LogEntry::new(
            "error",
            "tool",
            "tool.call_validation.exhausted",
            event.id,
            Some(event.id),
            event.correlation_id,
            json!({}),
        );
        store
            .save_terminal_dispatch_outcome(event.id, &outcome, &final_log, &terminal_log)
            .unwrap();
        assert_eq!(store.validation_retry(event.id).unwrap(), None);
        assert_eq!(store.dispatch_outcome(event.id).unwrap(), Some(outcome));
        assert!(store.get_log(&final_log.id.to_string()).unwrap().is_some());
        assert!(
            store
                .get_log(&terminal_log.id.to_string())
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn idempotent_acceptance_returns_original_event_and_response() {
        let store = EventStore::open(":memory:").unwrap();
        let correlation = Uuid::now_v7();
        let first = Event::new(
            "chat.message.created",
            "extension:chat",
            correlation,
            None,
            json!({"message_id":"m"}),
        );
        let accepted = store
            .append_and_enqueue_idempotent(&first, "m", Some(&json!({"session_id":"s"})))
            .unwrap();
        let duplicate = Event::new(
            "chat.message.created",
            "extension:chat",
            Uuid::now_v7(),
            None,
            json!({"message_id":"m"}),
        );
        let repeated = store
            .append_and_enqueue_idempotent(&duplicate, "m", Some(&json!({"session_id":"wrong"})))
            .unwrap();
        assert!(!accepted.3);
        assert!(repeated.3);
        assert_eq!(repeated.1.id, first.id);
        assert_eq!(repeated.1.correlation_id, correlation);
        assert_eq!(repeated.2, Some(json!({"session_id":"s"})));
    }

    #[test]
    fn action_group_receipts_make_successor_delivery_resumable() {
        let store = EventStore::open(":memory:").unwrap();
        let correlation = Uuid::now_v7();
        let current = Event::new("test.current", "test", correlation, None, json!({}));
        store.append_and_enqueue(&current).unwrap();
        store.claim_next(Uuid::now_v7()).unwrap().unwrap();
        let requested = Event::new(
            "action.requested",
            "habibi",
            correlation,
            Some(current.id),
            json!({}),
        );
        store
            .create_action_group(
                current.id,
                &json!({ "group": "state" }),
                std::slice::from_ref(&requested),
            )
            .unwrap();
        let result = Event::new(
            "action.result.succeeded",
            "habibi",
            correlation,
            Some(requested.id),
            json!({}),
        );
        assert!(
            store
                .append_action_result(current.id, 0, &[], &result, true)
                .unwrap()
        );
        assert!(
            !store
                .append_action_result(current.id, 0, &[], &result, true)
                .unwrap()
        );
        assert_eq!(store.completed_action_indices(current.id).unwrap(), vec![0]);
        assert_eq!(
            store.action_result_ids(current.id, 1).unwrap(),
            vec![result.id]
        );
        let completed = Event::new(
            "actions.completed",
            "habibi",
            correlation,
            Some(current.id),
            json!({}),
        );
        store
            .complete_action_group(current.id, &completed, false)
            .unwrap();
        assert!(store.action_group(current.id).unwrap().unwrap().1);
    }

    #[test]
    fn event_tail_is_oldest_first_and_exactly_filtered() {
        let store = EventStore::open(":memory:").unwrap();
        let correlation = Uuid::now_v7();
        let other = Uuid::now_v7();
        for (event_type, correlation_id) in [
            ("chat.one", correlation),
            ("other.one", correlation),
            ("chat.two", other),
            ("chat.two", correlation),
        ] {
            store
                .append(&Event::new(
                    event_type,
                    "test",
                    correlation_id,
                    None,
                    json!({}),
                ))
                .unwrap();
        }
        let events = store
            .query_event_tail(&EventTailQuery {
                event_types: vec!["chat.one".into(), "chat.two".into()],
                event_type_prefix: Some("chat.".into()),
                correlation_id: Some(correlation),
                after_sequence: 0,
                limit: 2,
            })
            .unwrap();
        assert_eq!(
            events
                .iter()
                .map(|event| event.sequence)
                .collect::<Vec<_>>(),
            vec![1, 4]
        );
    }

    #[test]
    fn isolates_extension_kv_namespaces() {
        let file = NamedTempFile::new().unwrap();
        let store = EventStore::open(file.path().to_str().unwrap()).unwrap();
        store.kv_set("chat", "theme", &json!("dark")).unwrap();
        store.kv_set("other", "theme", &json!("light")).unwrap();

        assert_eq!(store.kv_get("chat", "theme").unwrap(), Some(json!("dark")));
        assert_eq!(
            store.kv_get("other", "theme").unwrap(),
            Some(json!("light"))
        );
    }
}

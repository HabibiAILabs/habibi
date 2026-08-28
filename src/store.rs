use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use crate::event::{Event, StoredEvent};

pub type SharedEventStore = Arc<Mutex<EventStore>>;

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

pub struct EventStore {
    connection: Connection,
}

impl EventStore {
    pub fn open(path: &str) -> Result<Self> {
        let connection = Connection::open(path)
            .with_context(|| format!("failed to open event store at {path}"))?;
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

             INSERT OR IGNORE INTO event_links (
                 link_id, from_event_id, to_event_id, relation, description,
                 bidirectional, active, created_event_id
             )
             SELECT json_extract(payload, '$.link_id'), json_extract(payload, '$.from_event_id'),
                    json_extract(payload, '$.to_event_id'), json_extract(payload, '$.relation'),
                    json_extract(payload, '$.description'),
                    COALESCE(json_extract(payload, '$.bidirectional'), 1), 1, id
             FROM events WHERE event_type = 'event.link.created';

             UPDATE event_links SET active = 0 WHERE link_id IN (
                 SELECT json_extract(payload, '$.link_id')
                 FROM events WHERE event_type = 'event.link.removed'
             );",
        )?;

        Ok(Self { connection })
    }

    pub fn shared(self) -> SharedEventStore {
        Arc::new(Mutex::new(self))
    }

    pub fn append(&self, event: &Event) -> Result<i64> {
        let payload = serde_json::to_string(&event.payload)?;
        self.connection.execute(
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
                payload,
            ],
        )?;
        let sequence = self.connection.last_insert_rowid();
        if event.event_type == "event.link.created" {
            self.project_link(event)?;
        } else if event.event_type == "event.link.removed"
            && let Some(link_id) = event
                .payload
                .get("link_id")
                .and_then(serde_json::Value::as_str)
        {
            self.connection.execute(
                "UPDATE event_links SET active = 0 WHERE link_id = ?1",
                [link_id],
            )?;
        }
        Ok(sequence)
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

    fn project_link(&self, event: &Event) -> Result<()> {
        let string = |key: &str| {
            event
                .payload
                .get(key)
                .and_then(serde_json::Value::as_str)
                .with_context(|| format!("event.link.created missing '{key}'"))
        };
        self.connection.execute(
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

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tempfile::NamedTempFile;
    use uuid::Uuid;

    use super::*;

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

use anyhow::{Context, Result};
use rusqlite::{Connection, params};

use crate::event::{ConversationMessage, Event};

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
                 ON events(event_type, sequence);",
        )?;

        Ok(Self { connection })
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

        Ok(self.connection.last_insert_rowid())
    }

    pub fn recent_conversation(&self, limit: usize) -> Result<Vec<ConversationMessage>> {
        let mut statement = self.connection.prepare(
            "SELECT event_type, payload
             FROM (
                 SELECT sequence, event_type, payload
                 FROM events
                 WHERE event_type IN ('user.message', 'assistant.message')
                 ORDER BY sequence DESC
                 LIMIT ?1
             )
             ORDER BY sequence ASC",
        )?;

        let messages = statement
            .query_map([limit as i64], |row| {
                let event_type: String = row.get(0)?;
                let payload: String = row.get(1)?;
                Ok((event_type, payload))
            })?
            .map(|row| -> Result<ConversationMessage> {
                let (event_type, payload) = row?;
                let payload: serde_json::Value = serde_json::from_str(&payload)?;
                let content = payload
                    .get("content")
                    .and_then(|value| value.as_str())
                    .context("conversation event is missing string field 'content'")?;

                Ok(ConversationMessage {
                    role: if event_type == "user.message" {
                        "user".into()
                    } else {
                        "assistant".into()
                    },
                    content: content.into(),
                })
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(messages)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tempfile::NamedTempFile;
    use uuid::Uuid;

    use super::*;

    #[test]
    fn preserves_one_ordered_conversation() {
        let file = NamedTempFile::new().unwrap();
        let store = EventStore::open(file.path().to_str().unwrap()).unwrap();
        let correlation_id = Uuid::now_v7();

        for (event_type, content) in [
            ("user.message", "hello"),
            ("assistant.message", "hi"),
            ("runtime.note", "not conversational"),
            ("user.message", "remember me"),
        ] {
            store
                .append(&Event::new(
                    event_type,
                    "test",
                    correlation_id,
                    None,
                    json!({ "content": content }),
                ))
                .unwrap();
        }

        let messages = store.recent_conversation(2).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content, "hi");
        assert_eq!(messages[1].content, "remember me");
    }
}

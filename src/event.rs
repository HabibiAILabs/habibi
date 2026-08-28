use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: Uuid,
    pub event_type: String,
    pub source: String,
    pub occurred_at: DateTime<Utc>,
    pub causation_id: Option<Uuid>,
    pub correlation_id: Uuid,
    pub payload: Value,
}

impl Event {
    pub fn new(
        event_type: impl Into<String>,
        source: impl Into<String>,
        correlation_id: Uuid,
        causation_id: Option<Uuid>,
        payload: Value,
    ) -> Self {
        Self {
            id: Uuid::now_v7(),
            event_type: event_type.into(),
            source: source.into(),
            occurred_at: Utc::now(),
            causation_id,
            correlation_id,
            payload,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct StoredEvent {
    pub sequence: i64,
    #[serde(flatten)]
    pub event: Event,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub id: Uuid,
    pub occurred_at: DateTime<Utc>,
    pub level: String,
    pub category: String,
    pub name: String,
    pub reaction_id: Uuid,
    pub trigger_event_id: Option<Uuid>,
    pub correlation_id: Uuid,
    pub batch_id: Option<String>,
    pub action_id: Option<String>,
    pub tool_call_id: Option<String>,
    pub span_id: Option<String>,
    pub parent_span_id: Option<String>,
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct StoredLog {
    pub sequence: i64,
    #[serde(flatten)]
    pub log: LogEntry,
}

impl LogEntry {
    pub fn new(
        level: impl Into<String>,
        category: impl Into<String>,
        name: impl Into<String>,
        reaction_id: Uuid,
        trigger_event_id: Option<Uuid>,
        correlation_id: Uuid,
        payload: Value,
    ) -> Self {
        Self {
            id: Uuid::now_v7(),
            occurred_at: Utc::now(),
            level: level.into(),
            category: category.into(),
            name: name.into(),
            reaction_id,
            trigger_event_id,
            correlation_id,
            batch_id: None,
            action_id: None,
            tool_call_id: None,
            span_id: None,
            parent_span_id: None,
            payload,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationMessage {
    pub role: String,
    pub content: String,
}

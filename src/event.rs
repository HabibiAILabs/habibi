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
    pub dispatch_id: Uuid,
    pub event_id: Option<Uuid>,
    pub correlation_id: Uuid,
    pub action_group_id: Option<String>,
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
        dispatch_id: Uuid,
        event_id: Option<Uuid>,
        correlation_id: Uuid,
        payload: Value,
    ) -> Self {
        Self {
            id: Uuid::now_v7(),
            occurred_at: Utc::now(),
            level: level.into(),
            category: category.into(),
            name: name.into(),
            dispatch_id,
            event_id,
            correlation_id,
            action_group_id: None,
            action_id: None,
            tool_call_id: None,
            span_id: None,
            parent_span_id: None,
            payload,
        }
    }
}

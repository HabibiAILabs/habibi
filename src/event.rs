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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConversationMessage {
    pub role: String,
    pub content: String,
}

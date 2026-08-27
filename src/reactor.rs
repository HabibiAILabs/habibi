use anyhow::Result;
use serde_json::json;
use uuid::Uuid;

use crate::{event::Event, model::ModelClient, store::EventStore};

pub struct Reactor {
    store: EventStore,
    model: ModelClient,
    context_message_limit: usize,
}

impl Reactor {
    pub fn new(store: EventStore, model: ModelClient, context_message_limit: usize) -> Self {
        Self {
            store,
            model,
            context_message_limit,
        }
    }

    pub fn record_runtime_started(&self) -> Result<()> {
        let correlation_id = Uuid::now_v7();
        self.store.append(&Event::new(
            "runtime.started",
            "habibi",
            correlation_id,
            None,
            json!({ "model": self.model.model_name() }),
        ))?;
        Ok(())
    }

    pub async fn receive_user_message(&self, content: String) -> Result<Option<String>> {
        let correlation_id = Uuid::now_v7();
        let user_event = Event::new(
            "user.message",
            "cli",
            correlation_id,
            None,
            json!({ "content": content }),
        );
        self.store.append(&user_event)?;

        let conversation = self.store.recent_conversation(self.context_message_limit)?;
        let invocation = Event::new(
            "model.invocation.started",
            "habibi",
            correlation_id,
            Some(user_event.id),
            json!({
                "model": self.model.model_name(),
                "conversation_messages": conversation.len()
            }),
        );
        self.store.append(&invocation)?;

        let response = match self.model.invoke(&conversation).await {
            Ok(response) => response,
            Err(error) => {
                self.store.append(&Event::new(
                    "model.invocation.failed",
                    "habibi",
                    correlation_id,
                    Some(invocation.id),
                    json!({ "error": error.to_string() }),
                ))?;
                return Err(error);
            }
        };

        let content = response.content.trim().to_owned();
        let decision = if content.is_empty() {
            "idle"
        } else {
            "respond"
        };
        let completed = Event::new(
            "model.invocation.completed",
            "habibi",
            correlation_id,
            Some(invocation.id),
            json!({
                "provider": response.provider,
                "model": response.model,
                "decision": decision,
                "content": content,
                "usage": response.usage
            }),
        );
        self.store.append(&completed)?;

        if decision == "idle" {
            return Ok(None);
        }

        self.store.append(&Event::new(
            "assistant.message",
            "habibi",
            correlation_id,
            Some(completed.id),
            json!({ "content": content }),
        ))?;

        Ok(Some(content))
    }
}

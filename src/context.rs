use std::fmt::Write;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{event::Event, store::SharedEventStore};

pub const MAX_CONTEXT_BYTES: usize = 512 * 1024;
const MAX_CONTEXT_CONTRIBUTION_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContextContribution {
    #[serde(default)]
    pub content: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompiledContext {
    pub content: String,
    pub rendered_bytes: usize,
    pub estimated_tokens: usize,
}

pub fn compile_context(contribution: &ContextContribution) -> Result<CompiledContext> {
    let rendered_bytes = contribution.content.len();
    if rendered_bytes > MAX_CONTEXT_CONTRIBUTION_BYTES {
        bail!("context hook rendered more than 256 KiB");
    }
    Ok(CompiledContext {
        content: contribution.content.clone(),
        rendered_bytes,
        estimated_tokens: rendered_bytes.div_ceil(4),
    })
}

pub fn system_context(
    sections: &[(String, String, String)],
    feedback: &[String],
) -> Result<String> {
    let mut rendered = String::new();
    if !sections.is_empty() {
        rendered.push_str("\n\nExtension-provided system context follows.\n");
    }
    for (extension, hook, content) in sections {
        if content.is_empty() {
            continue;
        }
        rendered.push_str(&format!(
            "\n<context extension={} hook={}>\n{}\n</context>\n",
            serde_json::to_string(extension)?,
            serde_json::to_string(hook)?,
            content
        ));
        if rendered.len() > MAX_CONTEXT_BYTES {
            bail!("combined extension context rendered more than 512 KiB");
        }
    }
    for item in feedback {
        rendered.push_str("\n<engine-validation>\n");
        rendered.push_str(item);
        rendered.push_str("\n</engine-validation>\n");
        if rendered.len() > MAX_CONTEXT_BYTES {
            bail!("combined system context rendered more than 512 KiB");
        }
    }
    Ok(rendered)
}

pub fn current_event_input(_store: &SharedEventStore, event: &Event) -> Result<Value> {
    Ok(user_input(event_markdown(event)))
}

fn event_markdown(event: &Event) -> String {
    let mut output = format!(
        "# Current event\n\n- **Event type:** `{}`\n- **Source:** `{}`\n- **ID:** `{}`\n- **Correlation ID:** `{}`\n",
        event.event_type, event.source, event.id, event.correlation_id
    );
    if let Some(causation_id) = event.causation_id {
        let _ = writeln!(output, "- **Causation ID:** `{causation_id}`");
    }
    let _ = writeln!(
        output,
        "- **Occurred at:** `{}`\n\n## Payload",
        event.occurred_at
    );
    append_markdown_value(&mut output, &event.payload, 0);
    output
}

fn append_markdown_value(output: &mut String, value: &Value, depth: usize) {
    let indent = "  ".repeat(depth);
    match value {
        Value::Object(fields) => {
            for (key, value) in fields {
                let label = key.replace('_', " ");
                match value {
                    Value::Object(_) | Value::Array(_) => {
                        let _ = writeln!(output, "{indent}- **{label}:**");
                        append_markdown_value(output, value, depth + 1);
                    }
                    _ => {
                        let _ = writeln!(
                            output,
                            "{indent}- **{label}:** {}",
                            markdown_scalar(value, depth + 1)
                        );
                    }
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                match item {
                    Value::Object(_) | Value::Array(_) => {
                        let _ = writeln!(output, "{indent}-");
                        append_markdown_value(output, item, depth + 1);
                    }
                    _ => {
                        let _ = writeln!(output, "{indent}- {}", markdown_scalar(item, depth + 1));
                    }
                }
            }
        }
        _ => {
            let _ = writeln!(output, "{indent}{}", markdown_scalar(value, depth));
        }
    }
}

fn markdown_scalar(value: &Value, depth: usize) -> String {
    match value {
        Value::Null => "—".into(),
        Value::Bool(value) => if *value { "yes" } else { "no" }.into(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.replace('\n', &format!("\n{}  ", "  ".repeat(depth))),
        _ => unreachable!("compound values are rendered recursively"),
    }
}

fn user_input(text: String) -> Value {
    json!({
        "role": "user",
        "content": [{ "type": "input_text", "text": text }]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_event_is_rendered_as_markdown() {
        let event = Event::new(
            "chat.message.created",
            "extension:chat",
            uuid::Uuid::now_v7(),
            None,
            json!({ "content": "hello", "role": "user" }),
        );
        let text = event_markdown(&event);
        assert!(text.contains("# Current event"));
        assert!(text.contains("- **content:** hello"));
        assert!(!text.contains("{\"current_event\""));
    }

    #[test]
    fn compiles_extension_formatted_text_without_provider_roles() {
        let compiled = compile_context(&ContextContribution {
            content: "formatted by extension".into(),
        })
        .unwrap();
        assert_eq!(compiled.content, "formatted by extension");
        assert_eq!(compiled.rendered_bytes, 22);
    }

    #[test]
    fn rejects_oversized_extension_context_before_provider_invocation() {
        let error = compile_context(&ContextContribution {
            content: "x".repeat(MAX_CONTEXT_CONTRIBUTION_BYTES + 1),
        })
        .unwrap_err();
        assert!(error.to_string().contains("256 KiB"));
    }

    #[test]
    fn system_context_has_bounded_labeled_sections_and_feedback() {
        let rendered = system_context(
            &[("memory".into(), "retrieve".into(), "event data".into())],
            &["validation data".into()],
        )
        .unwrap();
        assert!(rendered.contains("<context extension=\"memory\" hook=\"retrieve\">"));
        assert!(rendered.contains("event data"));
        assert!(rendered.contains("<engine-validation>"));
    }
}

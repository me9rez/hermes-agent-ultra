use serde_json::{Value, json};

use crate::types::EventKind;

pub const SCHEMA_VERSION: u32 = 1;

pub fn event_kind_schema(kind: EventKind) -> Value {
    match kind {
        EventKind::Instruction => json!({
            "type": "object",
            "required": ["text"],
            "properties": { "text": { "type": "string" } }
        }),
        EventKind::Plan => json!({
            "type": "object",
            "required": ["steps"],
            "properties": {
                "steps": { "type": "array", "items": { "type": "string" } }
            }
        }),
        EventKind::Thinking => json!({
            "type": "object",
            "required": ["content"],
            "properties": { "content": { "type": "string" } }
        }),
        EventKind::ToolCall => json!({
            "type": "object",
            "required": ["tool_name", "args"],
            "properties": {
                "tool_name": { "type": "string" },
                "args": { "type": "object" }
            }
        }),
        EventKind::ToolResult => json!({
            "type": "object",
            "required": ["tool_name", "result"],
            "properties": {
                "tool_name": { "type": "string" },
                "result": {},
                "is_error": { "type": "boolean" }
            }
        }),
        EventKind::SubagentSpawn => json!({
            "type": "object",
            "required": ["sub_task_id"],
            "properties": { "sub_task_id": { "type": "string" } }
        }),
        EventKind::Message => json!({
            "type": "object",
            "required": ["text"],
            "properties": { "text": { "type": "string" }, "role": { "type": "string" } }
        }),
        EventKind::Artifact => json!({
            "type": "object",
            "required": ["artifact_id"],
            "properties": {
                "artifact_id": { "type": "string" },
                "name": { "type": "string" },
                "mime_type": { "type": "string" }
            }
        }),
        EventKind::ApprovalRequest => json!({
            "type": "object",
            "required": ["summary"],
            "properties": {
                "summary": { "type": "string" },
                "details": { "type": "object" }
            }
        }),
        EventKind::ApprovalResponse => json!({
            "type": "object",
            "required": ["approved"],
            "properties": { "approved": { "type": "boolean" }, "reason": { "type": "string" } }
        }),
        EventKind::Checkpoint => json!({
            "type": "object",
            "required": ["last_event_id"],
            "properties": {
                "last_event_id": { "type": "string" },
                "agent_state": { "type": "object" },
                "working_memory": { "type": "object" }
            }
        }),
        EventKind::Error => json!({
            "type": "object",
            "required": ["message"],
            "properties": { "message": { "type": "string" }, "code": { "type": "string" } }
        }),
        EventKind::System => json!({
            "type": "object",
            "properties": { "message": { "type": "string" } }
        }),
    }
}

pub fn all_event_schemas() -> Value {
    json!({
        "schema_version": SCHEMA_VERSION,
        "kinds": {
            "instruction": event_kind_schema(EventKind::Instruction),
            "plan": event_kind_schema(EventKind::Plan),
            "thinking": event_kind_schema(EventKind::Thinking),
            "tool_call": event_kind_schema(EventKind::ToolCall),
            "tool_result": event_kind_schema(EventKind::ToolResult),
            "subagent_spawn": event_kind_schema(EventKind::SubagentSpawn),
            "message": event_kind_schema(EventKind::Message),
            "artifact": event_kind_schema(EventKind::Artifact),
            "approval_request": event_kind_schema(EventKind::ApprovalRequest),
            "approval_response": event_kind_schema(EventKind::ApprovalResponse),
            "checkpoint": event_kind_schema(EventKind::Checkpoint),
            "error": event_kind_schema(EventKind::Error),
            "system": event_kind_schema(EventKind::System),
        }
    })
}

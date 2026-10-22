use big_head_ai::{ToolCall, ToolDefinition};
use big_head_world::{Lens, LensBucket, LensPermission, WorldManifest, WorldObjectRef};

pub(crate) fn tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "create_lens".into(),
            description: "Create a Lens exchange object in this world's outbox. Use Lens to share \
                          perspective, request review/work, hand off context, or reply to another \
                          world without directly mutating that world's ECS. The receiver decides \
                          whether to accept, reject, or derive local ECS facts."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "to_world": {
                        "type": "string",
                        "description": "Optional target world id, e.g. 'peer:alice'. Omit for a broadcast/local draft lens."
                    },
                    "kind": {
                        "type": "string",
                        "description": "Lens kind: proposal, request, reply, handoff, observation, or artifact."
                    },
                    "intent": {
                        "type": "string",
                        "description": "What the sender wants the receiver to do, e.g. review, extend, compare, store, execute."
                    },
                    "summary": {
                        "type": "string",
                        "description": "Short human/agent-readable summary."
                    },
                    "refs": {
                        "type": "array",
                        "description": "Stable cross-world object references. Never use Bevy Entity ids as cross-world object ids unless wrapped as local-only context.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "world_id": { "type": "string" },
                                "object_id": { "type": "string" },
                                "object_type": { "type": "string" },
                                "version": { "type": "string" }
                            },
                            "required": ["world_id", "object_id", "object_type"]
                        }
                    },
                    "payload": {
                        "description": "Structured JSON payload. Keep it concise; large content should be stored as shards or source refs and referenced."
                    },
                    "permissions": {
                        "type": "object",
                        "properties": {
                            "can_store": { "type": "boolean" },
                            "can_derive": { "type": "boolean" },
                            "can_forward": { "type": "boolean" }
                        }
                    }
                },
                "required": ["kind", "intent", "summary"]
            }),
        },
        ToolDefinition {
            name: "list_lens_inbox".into(),
            description: "List pending Lens objects in this world's inbox. This only observes the exchange bucket; it does not import anything into ECS.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
        ToolDefinition {
            name: "read_lens".into(),
            description: "Read one pending Lens from this world's inbox by lens_id. After reading, decide whether to accept, reject, or derive local ECS facts yourself.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "lens_id": { "type": "string", "description": "Lens id from list_lens_inbox." }
                },
                "required": ["lens_id"]
            }),
        },
        ToolDefinition {
            name: "accept_lens".into(),
            description: "Mark a pending inbox Lens as accepted by appending it to accepted.jsonl. This does not automatically write ECS facts.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "lens_id": { "type": "string" },
                    "note": { "type": "string", "description": "Optional reason or next-step note." }
                },
                "required": ["lens_id"]
            }),
        },
        ToolDefinition {
            name: "reject_lens".into(),
            description: "Mark a pending inbox Lens as rejected by appending it to rejected.jsonl. Use when the Lens is irrelevant, unsafe, stale, or needs a different request.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "lens_id": { "type": "string" },
                    "reason": { "type": "string", "description": "Optional rejection reason." }
                },
                "required": ["lens_id"]
            }),
        },
    ]
}

pub(crate) fn dispatch(manifest: &WorldManifest, call: &ToolCall) -> Option<(String, bool)> {
    let result = match call.name.as_str() {
        "create_lens" => create_lens(manifest, call),
        "list_lens_inbox" => list_lens_inbox(manifest),
        "read_lens" => read_lens(manifest, call),
        "accept_lens" => accept_lens(manifest, call),
        "reject_lens" => reject_lens(manifest, call),
        _ => return None,
    };
    Some(match result {
        Ok(output) => (output, false),
        Err(error) => (error, true),
    })
}

fn create_lens(manifest: &WorldManifest, call: &ToolCall) -> Result<String, String> {
    let kind = required_str(call, "kind")?;
    let intent = required_str(call, "intent")?;
    let summary = required_str(call, "summary")?;
    let to_world = call
        .input
        .get("to_world")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string());

    let mut lens = Lens::new(
        manifest.identity.world_id.clone(),
        to_world,
        kind,
        intent,
        summary,
    );
    lens.refs = parse_refs(call)?;
    lens.payload = call
        .input
        .get("payload")
        .cloned()
        .unwrap_or_else(|| serde_json::Value::Object(Default::default()));
    if let Some(permissions) = call.input.get("permissions") {
        lens.permissions = parse_permissions(permissions)?;
    }

    let bucket = LensBucket::new(&manifest.paths.outbox_dir);
    bucket.append_pending(&lens)?;
    let ref_count = lens.refs.len();
    Ok(format!(
        "Lens created in outbox: {} kind={} intent={} refs={} path={}",
        lens.lens_id,
        lens.kind,
        lens.intent,
        ref_count,
        bucket.pending_path().display()
    ))
}

fn list_lens_inbox(manifest: &WorldManifest) -> Result<String, String> {
    let bucket = LensBucket::new(&manifest.paths.inbox_dir);
    let lenses = bucket.read_pending()?;
    if lenses.is_empty() {
        return Ok(format!(
            "No pending inbox lenses. path={}",
            bucket.pending_path().display()
        ));
    }
    let lines = lenses
        .iter()
        .map(|lens| {
            format!(
                "- {} from={} to={} kind={} intent={} refs={} summary={}",
                lens.lens_id,
                lens.from_world,
                lens.to_world.as_deref().unwrap_or("*"),
                lens.kind,
                lens.intent,
                lens.refs.len(),
                one_line(&lens.summary)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    Ok(format!("Pending inbox lenses ({}):\n{lines}", lenses.len()))
}

fn read_lens(manifest: &WorldManifest, call: &ToolCall) -> Result<String, String> {
    let lens_id = required_str(call, "lens_id")?;
    let bucket = LensBucket::new(&manifest.paths.inbox_dir);
    let lens = bucket
        .find_pending(lens_id)?
        .ok_or_else(|| format!("pending lens not found: {lens_id}"))?;
    serde_json::to_string_pretty(&lens).map_err(|e| format!("serialize lens: {e}"))
}

fn accept_lens(manifest: &WorldManifest, call: &ToolCall) -> Result<String, String> {
    let lens_id = required_str(call, "lens_id")?;
    let bucket = LensBucket::new(&manifest.paths.inbox_dir);
    let lens = bucket.accept_pending(lens_id)?;
    let note = optional_note(call, "note");
    Ok(format!(
        "Lens accepted: {} from={} kind={} intent={}{}",
        lens.lens_id, lens.from_world, lens.kind, lens.intent, note
    ))
}

fn reject_lens(manifest: &WorldManifest, call: &ToolCall) -> Result<String, String> {
    let lens_id = required_str(call, "lens_id")?;
    let bucket = LensBucket::new(&manifest.paths.inbox_dir);
    let lens = bucket.reject_pending(lens_id)?;
    let reason = optional_note(call, "reason");
    Ok(format!(
        "Lens rejected: {} from={} kind={} intent={}{}",
        lens.lens_id, lens.from_world, lens.kind, lens.intent, reason
    ))
}

fn parse_refs(call: &ToolCall) -> Result<Vec<WorldObjectRef>, String> {
    let Some(value) = call.input.get("refs") else {
        return Ok(Vec::new());
    };
    serde_json::from_value(value.clone()).map_err(|e| format!("invalid refs: {e}"))
}

fn parse_permissions(value: &serde_json::Value) -> Result<LensPermission, String> {
    let mut permissions = LensPermission::default();
    if let Some(can_store) = value.get("can_store").and_then(|v| v.as_bool()) {
        permissions.can_store = can_store;
    }
    if let Some(can_derive) = value.get("can_derive").and_then(|v| v.as_bool()) {
        permissions.can_derive = can_derive;
    }
    if let Some(can_forward) = value.get("can_forward").and_then(|v| v.as_bool()) {
        permissions.can_forward = can_forward;
    }
    Ok(permissions)
}

fn required_str<'a>(call: &'a ToolCall, field: &str) -> Result<&'a str, String> {
    call.input
        .get(field)
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("Missing or empty '{field}' field"))
}

fn optional_note(call: &ToolCall, field: &str) -> String {
    call.input
        .get(field)
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(|s| format!(" note={}", one_line(s)))
        .unwrap_or_default()
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

//! Bounded, tool-specific recovery advice appended after failures.

pub(super) fn append_recovery_hint(tool_name: &str, output: String) -> String {
    if let Some(hint) = targeted_recovery_hint(tool_name, &output) {
        return format!("{output}\n\n{hint}");
    }

    if !is_schema_or_type_error(&output) {
        return output;
    }

    format!(
        "{output}\n\nRecovery hint for {tool_name}:\n\
         - Inspect get_world_overview for the target component schema before retrying.\n\
         - For dynamic components, component values must be JSON objects, not strings like \"\".\n\
         - spawn_entity components[*].value must include every required field with the correct JSON type.\n\
         - modify_component value must be a JSON object patch for structured components.\n\
         - number/integer fields must be unquoted JSON numbers; boolean fields must be true/false."
    )
}

fn targeted_recovery_hint(tool_name: &str, output: &str) -> Option<&'static str> {
    if tool_name == "execute_cli"
        && output.contains("whole")
        && output.contains("Search from the project/workspace root")
    {
        return Some(
            "Recovery hint for execute_cli:\n\
             - Do not retry with another whole-machine search.\n\
             - First inspect ECS traces: query_entities(\"ActionRecord\"), query_entities(\"TransactionRecord\"), or query_entities(\"ShardData\").\n\
             - For files, search only the workspace: rg --files, rg <keyword> crates docs, or find . -name ... -not -path './target/*'.\n\
             - Carry forward what this failed command already proved before choosing the next path.",
        );
    }

    if tool_name == "render_view"
        && (output.contains("deserialize failed")
            || output.contains("missing field")
            || output.contains("invalid type"))
    {
        return Some(
            "Recovery hint for render_view:\n\
             - To update an existing panel, use exactly: {\"op\":\"update\",\"id\":\"mind-view\",\"patch\":{\"content\":{\"kind\":\"html\",\"markup\":\"<div>...</div>\"}}}.\n\
             - To create a 2D panel, use: {\"op\":\"spawn\",\"id\":\"panel-id\",\"surface\":\"html2d\",\"kind\":\"html\",\"content\":{\"kind\":\"html\",\"markup\":\"<div>...</div>\"}}.\n\
             - Do not set patch to \"full\" or an XML/string blob; patch must be a JSON object with content.\n\
             - For full protocol examples, run_system(\"guide_render_view\") before retrying.",
        );
    }

    if tool_name == "query_entities" && output.contains("Query error") {
        return Some(
            "Recovery hint for query_entities:\n\
             - Start broad with a component name: \"ViewState\", \"AgentState\", \"AmbientSignal\", \"ActionRecord\", or \"ShardData\".\n\
             - Do not write pseudo paths like \"ViewState.panels contains ...\" unless the query DSL guide explicitly supports that syntax.\n\
             - If unsure, call get_world_overview first and then query the concrete component name.",
        );
    }

    if matches!(tool_name, "define_system" | "update_system" | "run_system")
        && (output.contains("Rhai syntax error") || output.contains("Function not found"))
    {
        return Some(
            "Recovery hint for Rhai systems:\n\
             - Run run_system(\"guide_rhai_syntax\") before retrying.\n\
             - Rhai ECS spawn shape is #{ComponentName: #{field: value}}, not the tool JSON shape {components:[...]}.\n\
             - Only use functions listed in the Rhai guide; arbitrary helpers such as read_file or take_head may not exist.\n\
             - Avoid reserved words as variable names.",
        );
    }

    None
}

fn is_schema_or_type_error(output: &str) -> bool {
    output.contains("schema validation")
        || output.contains("component value must be a JSON object")
        || output.contains("invalid type")
        || output.contains("missing field")
        || output.contains("expected number")
        || output.contains("expected integer")
        || output.contains("expected u64")
        || output.contains("expected boolean")
        || output.contains("expected string")
        || output.contains("unknown field")
}

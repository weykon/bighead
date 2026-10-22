//! Lightweight tool pipeline guards.
//!
//! The dispatcher still owns concrete tool execution. This module holds
//! cross-cutting checks that should apply before a tool is allowed to run.

use big_head_ai::ToolCall;

const MAX_IDENTICAL_FAILURES_BEFORE_BLOCK: usize = 2;

pub(crate) fn repeated_failure_guard(
    tool_call_log: &[(String, String, String, bool)],
    call: &ToolCall,
) -> Option<String> {
    let input_summary = call.input.to_string();
    let failures = tool_call_log
        .iter()
        .rev()
        .take_while(|(tool_name, input, _, is_error)| {
            tool_name == &call.name && input == &input_summary && *is_error
        })
        .count();

    if failures < MAX_IDENTICAL_FAILURES_BEFORE_BLOCK {
        return None;
    }

    Some(format!(
        "Tool pipeline blocked repeated identical failing call: tool='{}' failed {} time(s) with the same input in this cycle.\n\n\
         Recovery hint:\n\
         - Do not call the same tool with the same arguments again.\n\
         - Inspect the previous tool error already in this conversation.\n\
         - Query ActionRecord/ToolHistory if you need the exact prior failure.\n\
         - Change strategy: read the relevant guide, inspect schema, narrow the query, or ask the user for confirmation.",
        call.name, failures
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn call() -> ToolCall {
        ToolCall {
            id: "t3".into(),
            name: "render_view".into(),
            input: json!({"id": "mind-view", "patch": "bad"}),
        }
    }

    #[test]
    fn allows_first_identical_failures() {
        let call = call();
        let log = vec![(
            call.name.clone(),
            call.input.to_string(),
            "bad payload".into(),
            true,
        )];

        assert!(repeated_failure_guard(&log, &call).is_none());
    }

    #[test]
    fn blocks_third_identical_failure() {
        let call = call();
        let log = vec![
            (
                call.name.clone(),
                call.input.to_string(),
                "bad payload".into(),
                true,
            ),
            (
                call.name.clone(),
                call.input.to_string(),
                "bad payload again".into(),
                true,
            ),
        ];

        let blocked = repeated_failure_guard(&log, &call).expect("third repeat should be blocked");
        assert!(blocked.contains("Tool pipeline blocked repeated identical failing call"));
        assert!(blocked.contains("ActionRecord/ToolHistory"));
    }
}

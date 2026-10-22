use big_head_loop::AgentRuntime;
use big_head_view::{
    StatusCommand, StatusItem, StatusItemControl, StatusItemState, StatusMessage, StatusMessageRole,
};

pub(crate) fn started(rt: &AgentRuntime, text: &str) {
    send_message(rt, StatusMessageRole::User, text);
    send_status(
        rt,
        StatusItem {
            id: "desktop-last-message".into(),
            title: summarize(text, 48),
            state: StatusItemState::Running,
            control: Some(StatusItemControl::Item),
            description: Some("Agent is processing your message...".into()),
            progress: Some(0.25),
            checked: None,
            group: Some("desktop".into()),
            priority: Some(100),
            updated_at: Some(now_secs()),
        },
    );
}

pub(crate) fn finished(rt: &AgentRuntime, responses: &[(String, String)], errors: &[String]) {
    if !errors.is_empty() {
        send_message(rt, StatusMessageRole::Error, errors.join("\n\n"));
        send_status(
            rt,
            StatusItem {
                id: "desktop-last-message".into(),
                title: "Last message failed".into(),
                state: StatusItemState::Failed,
                control: Some(StatusItemControl::Item),
                description: Some(summarize(&errors.join(" "), 180)),
                progress: Some(1.0),
                checked: None,
                group: Some("desktop".into()),
                priority: Some(100),
                updated_at: Some(now_secs()),
            },
        );
        return;
    }

    let Some((agent, response)) = responses.last() else {
        send_message(
            rt,
            StatusMessageRole::System,
            "Message was processed without a visible text response.",
        );
        send_status(
            rt,
            StatusItem {
                id: "desktop-last-message".into(),
                title: "No agent response".into(),
                state: StatusItemState::Done,
                control: Some(StatusItemControl::Item),
                description: Some("Message was processed without a visible text response.".into()),
                progress: Some(1.0),
                checked: None,
                group: Some("desktop".into()),
                priority: Some(100),
                updated_at: Some(now_secs()),
            },
        );
        return;
    };

    send_message(rt, StatusMessageRole::Assistant, response.clone());
    send_status(
        rt,
        StatusItem {
            id: "desktop-last-message".into(),
            title: format!("{agent} replied"),
            state: StatusItemState::Done,
            control: Some(StatusItemControl::Item),
            description: Some(summarize(response, 180)),
            progress: Some(1.0),
            checked: None,
            group: Some("desktop".into()),
            priority: Some(100),
            updated_at: Some(now_secs()),
        },
    );
}

fn send_status(rt: &AgentRuntime, item: StatusItem) {
    if let Some(channel) = rt.view_handler.as_ref().and_then(|h| h.channel.as_ref()) {
        channel.send_status(StatusCommand::UpsertItem { item });
    }
}

fn send_message(rt: &AgentRuntime, role: StatusMessageRole, text: impl Into<String>) {
    if let Some(channel) = rt.view_handler.as_ref().and_then(|h| h.channel.as_ref()) {
        let created_at = now_secs();
        channel.send_status(StatusCommand::AppendMessage {
            message: StatusMessage {
                id: format!("desktop-message-{created_at}-{}", rand_suffix()),
                role,
                text: text.into(),
                created_at: Some(created_at),
            },
        });
    }
}

fn summarize(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    let mut last_was_space = false;
    for c in text.chars() {
        let c = if c.is_whitespace() { ' ' } else { c };
        if c == ' ' && last_was_space {
            continue;
        }
        last_was_space = c == ' ';
        if out.chars().count() >= max_chars {
            out.push_str("...");
            break;
        }
        out.push(c);
    }
    out.trim().to_string()
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn rand_suffix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as u64
}

use big_head_view::{StatusCommand, StatusItemControl, StatusItemState, StatusMessageRole};

#[test]
fn desktop_status_smoke_fixture_decodes_and_covers_native_groups() {
    let raw = include_str!("fixtures/desktop_status_smoke.json");
    let command: StatusCommand = serde_json::from_str(raw).expect("fixture should decode");

    let StatusCommand::SetState { state } = command else {
        panic!("fixture should be a set_state command");
    };

    assert_eq!(state.title.as_deref(), Some("BigHead desktop smoke"));
    assert!(
        state
            .items
            .iter()
            .any(|item| item.group.as_deref() == Some("choice"))
    );
    assert!(state.items.iter().any(|item| {
        item.group.as_deref() == Some("checklist")
            && item.control == Some(StatusItemControl::Checkbox)
            && item.checked == Some(true)
    }));
    assert!(state.items.iter().any(|item| {
        item.group.as_deref() == Some("progress")
            && item.control == Some(StatusItemControl::Progress)
            && item.state == StatusItemState::Running
            && item.progress == Some(0.6)
    }));
    assert!(
        state
            .messages
            .iter()
            .any(|message| message.role == StatusMessageRole::Assistant
                && message.text.contains("## Current state"))
    );
    assert!(state.documents.iter().any(|document| {
        document.kind.as_deref() == Some("test") && document.references.len() >= 2
    }));
}

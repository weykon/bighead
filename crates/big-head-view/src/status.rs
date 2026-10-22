//! Lightweight status protocol for small external surfaces such as a macOS
//! menu bar popover.
//!
//! This is intentionally separate from [`crate::protocol::ViewCommand`].
//! `ViewCommand` is for rich panels; `StatusCommand` is for ambient task
//! awareness: what is running, what matters now, and how far along it is.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StatusState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default)]
    pub items: Vec<StatusItem>,
    #[serde(default)]
    pub messages: Vec<StatusMessage>,
    #[serde(default)]
    pub documents: Vec<StatusDocument>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusMessage {
    pub id: String,
    pub role: StatusMessageRole,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StatusMessageRole {
    User,
    Assistant,
    System,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusDocument {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    /// Markdown-like document text for native desktop Notes mode.
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<StatusDocumentReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusDocumentReference {
    pub kind: String,
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusItem {
    pub id: String,
    pub title: String,
    pub state: StatusItemState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control: Option<StatusItemControl>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// 0.0 ... 1.0. Omit when progress is unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<i32>,
    /// Unix timestamp in seconds. Agent may omit it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StatusItemState {
    Pending,
    Running,
    Blocked,
    Done,
    Failed,
    Hidden,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StatusItemControl {
    Item,
    Checkbox,
    Progress,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum StatusCommand {
    /// Replace the whole menu bar snapshot.
    SetState { state: StatusState },
    /// Insert or replace a single item by id.
    UpsertItem { item: StatusItem },
    /// Remove one item by id.
    RemoveItem { id: String },
    /// Append one chat/status transcript message.
    AppendMessage { message: StatusMessage },
    /// Insert or replace one desktop document by id.
    UpsertDocument { document: StatusDocument },
    /// Remove one desktop document by id.
    RemoveDocument { id: String },
    /// Clear all items and header fields.
    Clear,
}

impl StatusCommand {
    pub fn apply_to(self, state: &mut StatusState) {
        match self {
            StatusCommand::SetState { state: next } => *state = next,
            StatusCommand::UpsertItem { item } => {
                if let Some(existing) = state.items.iter_mut().find(|i| i.id == item.id) {
                    *existing = item;
                } else {
                    state.items.push(item);
                }
                state.items.sort_by(|a, b| {
                    b.priority
                        .unwrap_or(0)
                        .cmp(&a.priority.unwrap_or(0))
                        .then_with(|| a.title.cmp(&b.title))
                });
            }
            StatusCommand::RemoveItem { id } => {
                state.items.retain(|item| item.id != id);
            }
            StatusCommand::AppendMessage { message } => {
                state.messages.push(message);
                if state.messages.len() > 80 {
                    let drop_count = state.messages.len() - 80;
                    state.messages.drain(0..drop_count);
                }
            }
            StatusCommand::UpsertDocument { document } => {
                if let Some(existing) = state.documents.iter_mut().find(|d| d.id == document.id) {
                    *existing = document;
                } else {
                    state.documents.push(document);
                }
                state
                    .documents
                    .sort_by(|a, b| b.updated_at.unwrap_or(0).cmp(&a.updated_at.unwrap_or(0)));
            }
            StatusCommand::RemoveDocument { id } => {
                state.documents.retain(|document| document.id != id);
            }
            StatusCommand::Clear => *state = StatusState::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_replaces_existing_item() {
        let mut state = StatusState::default();
        StatusCommand::UpsertItem {
            item: StatusItem {
                id: "task".into(),
                title: "Old".into(),
                state: StatusItemState::Pending,
                control: None,
                description: None,
                progress: Some(0.1),
                checked: None,
                group: None,
                priority: None,
                updated_at: None,
            },
        }
        .apply_to(&mut state);
        StatusCommand::UpsertItem {
            item: StatusItem {
                id: "task".into(),
                title: "New".into(),
                state: StatusItemState::Running,
                control: None,
                description: None,
                progress: Some(0.5),
                checked: None,
                group: None,
                priority: None,
                updated_at: None,
            },
        }
        .apply_to(&mut state);

        assert_eq!(state.items.len(), 1);
        assert_eq!(state.items[0].title, "New");
        assert_eq!(state.items[0].state, StatusItemState::Running);
        assert_eq!(state.items[0].progress, Some(0.5));
    }

    #[test]
    fn upsert_replaces_existing_document() {
        let mut state = StatusState::default();
        StatusCommand::UpsertDocument {
            document: StatusDocument {
                id: "note".into(),
                title: "Old".into(),
                subtitle: None,
                text: "old body".into(),
                kind: Some("note".into()),
                references: vec![StatusDocumentReference {
                    kind: "entity".into(),
                    id: "E1".into(),
                    title: "Entity 1".into(),
                    description: None,
                }],
                updated_at: Some(1),
            },
        }
        .apply_to(&mut state);
        StatusCommand::UpsertDocument {
            document: StatusDocument {
                id: "note".into(),
                title: "New".into(),
                subtitle: Some("subtitle".into()),
                text: "new body".into(),
                kind: Some("summary".into()),
                references: vec![StatusDocumentReference {
                    kind: "source".into(),
                    id: "S1".into(),
                    title: "Source 1".into(),
                    description: Some("source description".into()),
                }],
                updated_at: Some(2),
            },
        }
        .apply_to(&mut state);

        assert_eq!(state.documents.len(), 1);
        assert_eq!(state.documents[0].title, "New");
        assert_eq!(state.documents[0].subtitle.as_deref(), Some("subtitle"));
        assert_eq!(state.documents[0].text, "new body");
        assert_eq!(state.documents[0].references.len(), 1);
        assert_eq!(state.documents[0].references[0].kind, "source");
    }
}

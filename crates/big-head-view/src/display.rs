//! Stable display intents for projecting ECS state into the view server.
//!
//! Agents choose an intent; Rust owns the panel id, surface, kind, title, and
//! source component contract. This keeps repeated inspector renders in one
//! predictable slot instead of creating ad-hoc panels on every cycle.

use serde::{Deserialize, Serialize};

use crate::protocol::{PanelKind, ViewSurface};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayIntent {
    #[serde(rename = "inspect_tickactions", alias = "inspect_tick_actions")]
    InspectTickActions,
    InspectSparks,
    InspectContext,
    InspectWorld,
    InspectTools,
    #[serde(alias = "debug_view")]
    InspectView,
    VisualizeRelation,
    ChooseNext,
    TrackWork,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplaySpec {
    pub intent: DisplayIntent,
    pub panel_id: &'static str,
    pub surface: ViewSurface,
    pub kind: PanelKind,
    pub title: &'static str,
    pub source_components: &'static [&'static str],
}

pub fn display_spec(intent: DisplayIntent) -> DisplaySpec {
    use DisplayIntent::*;

    match intent {
        InspectTickActions => inspector_spec(
            intent,
            "inspector-tickactions",
            "TickAction Inspector",
            &["TickAction"],
        ),
        InspectSparks => inspector_spec(
            intent,
            "inspector-sparks",
            "Spark Queue Inspector",
            &["Spark"],
        ),
        InspectContext => inspector_spec(
            intent,
            "inspector-context",
            "Session Context Inspector",
            &["SessionContext", "AmbientSignal"],
        ),
        InspectWorld => inspector_spec(
            intent,
            "inspector-world",
            "ECS World Inspector",
            &["AgentState", "ViewState"],
        ),
        InspectTools => inspector_spec(
            intent,
            "inspector-tools",
            "Tool Activity Inspector",
            &["ToolHistory", "ActionRecord"],
        ),
        InspectView => inspector_spec(
            intent,
            "inspector-view",
            "View Runtime Inspector",
            &["ViewState"],
        ),
        VisualizeRelation => DisplaySpec {
            intent,
            panel_id: "mind-view",
            surface: ViewSurface::Html2D,
            kind: PanelKind::Html,
            title: "Mind View",
            source_components: &[],
        },
        ChooseNext => DisplaySpec {
            intent,
            panel_id: "decision-board",
            surface: ViewSurface::Html2D,
            kind: PanelKind::Form,
            title: "Decision Board",
            source_components: &[],
        },
        TrackWork => inspector_spec(
            intent,
            "work-board",
            "Work Board",
            &["Plan", "WorkItem", "TransactionRecord"],
        ),
    }
}

pub fn display_intent_for_component(component: &str) -> Option<DisplayIntent> {
    match component {
        "TickAction" => Some(DisplayIntent::InspectTickActions),
        "Spark" | "SparkDispatch" => Some(DisplayIntent::InspectSparks),
        "SessionContext" | "AmbientSignal" => Some(DisplayIntent::InspectContext),
        "ToolHistory" | "ActionRecord" | "RuntimeJournal" => Some(DisplayIntent::InspectTools),
        "ViewState" => Some(DisplayIntent::InspectView),
        "Plan" | "WorkItem" | "TransactionRecord" => Some(DisplayIntent::TrackWork),
        "AgentState" | "ShardData" | "RuntimeIdentity" => Some(DisplayIntent::InspectWorld),
        _ => None,
    }
}

fn inspector_spec(
    intent: DisplayIntent,
    panel_id: &'static str,
    title: &'static str,
    source_components: &'static [&'static str],
) -> DisplaySpec {
    DisplaySpec {
        intent,
        panel_id,
        surface: ViewSurface::Html2D,
        kind: PanelKind::Inspector,
        title,
        source_components,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_actions_have_a_stable_html_inspector_slot() {
        let spec = display_spec(DisplayIntent::InspectTickActions);
        assert_eq!(spec.panel_id, "inspector-tickactions");
        assert_eq!(spec.surface, ViewSurface::Html2D);
        assert_eq!(spec.kind, PanelKind::Inspector);
        assert_eq!(spec.source_components, &["TickAction"]);
    }

    #[test]
    fn known_components_resolve_to_display_intents() {
        assert_eq!(
            display_intent_for_component("Spark"),
            Some(DisplayIntent::InspectSparks)
        );
        assert_eq!(
            display_intent_for_component("ViewState"),
            Some(DisplayIntent::InspectView)
        );
        assert_eq!(display_intent_for_component("Unknown"), None);
    }

    #[test]
    fn display_intent_wire_names_match_tool_contract() {
        assert_eq!(
            serde_json::to_value(DisplayIntent::InspectTickActions).unwrap(),
            serde_json::json!("inspect_tickactions")
        );
        assert_eq!(
            serde_json::from_value::<DisplayIntent>(serde_json::json!("debug_view")).unwrap(),
            DisplayIntent::InspectView
        );
    }
}

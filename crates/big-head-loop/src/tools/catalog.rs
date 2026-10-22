//! Tool catalogs assembled from runtime services and registered Rhai tools.

use super::{RuntimeToolExecutor, ecs_capabilities, inspect_view, lens, lens_app, shard_catalog};

impl RuntimeToolExecutor<'_> {
    pub(super) fn runtime_tool_definitions(&self) -> Vec<big_head_ai::ToolDefinition> {
        let mut defs = lens_app::definitions();
        if self.world.contains_resource::<crate::user_question::QuestionUiEnabled>() {
            defs.push(crate::user_question::definition());
        }
        defs.extend(crate::activity::definitions());
        defs.extend(crate::shard_retention::definitions());
        defs.extend(shard_catalog::definitions());
        defs.push(crate::world_shape::definition());
        defs.extend(crate::lens_bindings::definitions());
        defs.extend(crate::lens_learning::definitions());
        defs.extend(crate::dream::definitions());
        defs.extend(crate::interpretation::definitions());
        defs.extend(crate::interpretation_method::definitions());
        defs.extend(ecs_capabilities::definitions());
        defs.push(crate::scene_world::tool_definition());
        defs.extend(crate::work_world::definitions());
        defs.push(super::laya::definition());
        defs.push(crate::laya_spec::definition());
        defs.extend(crate::laya_workflows::definitions());
        defs.push(super::structured_query::definition());
        if self.view_handler.is_some() {
            defs.push(big_head_view::render_document_tool_definition());
            defs.push(inspect_view::tool_definition());
            defs.push(big_head_view::render_mind_view_tool_definition());
            defs.push(big_head_view::render_view_tool_definition());
            defs.push(big_head_view::render_status_tool_definition());
        }
        if self.world_manifest.is_some() {
            defs.extend(lens::tool_definitions());
        }
        defs
    }

    pub(super) fn tool_definitions(&self) -> Vec<big_head_ai::ToolDefinition> {
        let mut defs = self.system_registry.tool_definitions();
        defs.extend(self.runtime_tool_definitions());
        if let Some(allowed) = self.allowed_tools {
            defs.retain(|definition| allowed.iter().any(|name| name == &definition.name));
        }
        defs
    }
}

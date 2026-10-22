//! 动态组件 schema 和存储 —— 将运行时定义的组件名称映射到其 `ComponentId`。

use std::collections::{BTreeMap, HashMap, HashSet};

use bevy_ecs::component::ComponentId;
use serde::{Deserialize, Serialize};

/// 运行时定义的组件的扁平 schema。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DynamicSchema {
    pub fields: BTreeMap<String, FieldType>,
}

/// 动态组件支持的原始字段类型。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FieldType {
    String,
    Integer,
    Number,
    Boolean,
    Any,
}

impl FieldType {
    /// 检查 JSON 值是否匹配此字段类型。
    pub fn matches(&self, value: &serde_json::Value) -> bool {
        match self {
            FieldType::String => value.is_string(),
            FieldType::Integer => value.is_i64() || value.is_u64(),
            FieldType::Number => value.is_number(),
            FieldType::Boolean => value.is_boolean(),
            FieldType::Any => true,
        }
    }

    /// 返回类型的人类可读名称。
    pub fn as_str(&self) -> &'static str {
        match self {
            FieldType::String => "string",
            FieldType::Integer => "integer",
            FieldType::Number => "number",
            FieldType::Boolean => "boolean",
            FieldType::Any => "any",
        }
    }
}

/// 校验完整的组件值是否匹配 schema。拒绝缺少必填字段、
/// 类型不匹配的字段和未知字段。
pub fn validate(schema: &DynamicSchema, value: &serde_json::Value) -> Result<(), String> {
    let obj = value
        .as_object()
        .ok_or_else(|| "component value must be a JSON object".to_string())?;

    let mut errors = Vec::new();

    for name in schema.fields.keys() {
        if !obj.contains_key(name) {
            errors.push(format!("missing field '{}'", name));
        }
    }
    for (name, v) in obj {
        match schema.fields.get(name) {
            None => errors.push(format!("unknown field '{}'", name)),
            Some(ft) => {
                if !ft.matches(v) {
                    errors.push(format!(
                        "field '{}': expected {}, got {}",
                        name,
                        ft.as_str(),
                        v
                    ));
                }
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

/// 校验部分 patch 是否匹配 schema。仅检查实际存在的字段；
/// 仍拒绝未知字段。
pub fn validate_patch(schema: &DynamicSchema, patch: &serde_json::Value) -> Result<(), String> {
    let obj = patch
        .as_object()
        .ok_or_else(|| "patch must be a JSON object".to_string())?;

    let mut errors = Vec::new();
    for (name, v) in obj {
        match schema.fields.get(name) {
            None => errors.push(format!("unknown field '{}'", name)),
            Some(ft) => {
                if !ft.matches(v) {
                    errors.push(format!(
                        "field '{}': expected {}, got {}",
                        name,
                        ft.as_str(),
                        v
                    ));
                }
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

/// 动态组件在 store 中的条目：ComponentId + schema。
pub struct DynamicComponentEntry {
    pub component_id: ComponentId,
    pub schema: DynamicSchema,
}

/// 按名称存储运行时定义的组件，包含其 schema 和 bevy_ecs `ComponentId`。
pub struct DynamicComponentStore {
    pub entries: HashMap<String, DynamicComponentEntry>,
    pub dynamic_ids: HashSet<ComponentId>,
}

impl DynamicComponentStore {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            dynamic_ids: HashSet::new(),
        }
    }

    /// 返回 `id` 是否由本 store 分配（即是否为动态组件）。
    /// 这是内存安全的关键 —— 只有动态 id 才能通过 `dynamic_component::read` 读取。
    pub fn is_dynamic(&self, id: ComponentId) -> bool {
        self.dynamic_ids.contains(&id)
    }

    /// 按（可能为短的）名称查找组件 id。
    pub fn get_id(&self, name: &str) -> Option<ComponentId> {
        let resolved = self.resolve_name(name)?;
        self.entries.get(resolved).map(|e| e.component_id)
    }

    /// 遍历所有已注册动态组件的 (name, schema) 对。
    pub fn iter(&self) -> impl Iterator<Item = (&str, &DynamicSchema)> {
        self.entries
            .iter()
            .map(|(name, entry)| (name.as_str(), &entry.schema))
    }

    /// 将（可能为短的）名称解析为 `entries` 中的规范键。
    pub fn resolve_name<'a>(&'a self, name: &'a str) -> Option<&'a str> {
        resolve_by_short_name(name, self.entries.keys().map(|s| s.as_str()))
    }
}

/// 共享的短名称解析：完全匹配优先，后缀匹配兜底。
pub fn resolve_by_short_name<'a>(
    name: &'a str,
    keys: impl Iterator<Item = &'a str>,
) -> Option<&'a str> {
    let name_short = name.rsplit("::").next().unwrap_or(name);
    let mut best: Option<&'a str> = None;
    for key in keys {
        if key == name {
            return Some(key);
        }
        if best.is_none() {
            let key_short = key.rsplit("::").next().unwrap_or(key);
            if key_short == name_short {
                best = Some(key);
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn review_state_schema() -> DynamicSchema {
        let mut fields = BTreeMap::new();
        fields.insert("reviewer".to_string(), FieldType::String);
        fields.insert("approved".to_string(), FieldType::Boolean);
        fields.insert("notes".to_string(), FieldType::String);
        DynamicSchema { fields }
    }

    #[test]
    fn test_schema_validate_missing_field() {
        let schema = review_state_schema();
        let value = serde_json::json!({"reviewer": "alice", "approved": true});
        let err = validate(&schema, &value).unwrap_err();
        assert!(err.contains("missing field 'notes'"));
    }

    #[test]
    fn test_schema_validate_wrong_type() {
        let schema = review_state_schema();
        let value = serde_json::json!({
            "reviewer": "alice",
            "approved": "yes",  // 应为 boolean
            "notes": "ok"
        });
        let err = validate(&schema, &value).unwrap_err();
        assert!(err.contains("expected boolean"));
    }

    #[test]
    fn test_schema_validate_unknown_field() {
        let schema = review_state_schema();
        let value = serde_json::json!({
            "reviewer": "alice",
            "approved": true,
            "notes": "ok",
            "extra": 1
        });
        let err = validate(&schema, &value).unwrap_err();
        assert!(err.contains("unknown field 'extra'"));
    }
}

impl DynamicComponentStore {
    /// 注销（软删除）一个动态组件类型。
    ///
    /// 从本 store 移除名称→ID 映射，使组件不可再被发现和实例化。
    /// 注意：bevy_ecs 不支持真正的组件类型注销，底层的 ComponentId 会保留
    /// 在 bevy_ecs 的组件注册表中（但不可达）。
    ///
    /// # Returns
    /// - `Ok((component_id, schema))` 如果成功
    /// - `Err(message)` 如果组件不存在
    pub fn undefine(&mut self, name: &str) -> Result<(ComponentId, DynamicSchema), String> {
        let resolved = self
            .resolve_name(name)
            .ok_or_else(|| format!("Component '{}' is not defined", name))?;

        // Copy the resolved name to avoid borrow conflict
        let resolved_owned = resolved.to_string();
        let entry = self
            .entries
            .remove(&resolved_owned)
            .ok_or_else(|| format!("Component '{}' not found in store", name))?;

        self.dynamic_ids.remove(&entry.component_id);

        Ok((entry.component_id, entry.schema))
    }
}

#[cfg(test)]
mod undefine_tests {
    use super::*;

    #[test]
    fn test_undefine_removes_entry() {
        let mut store = DynamicComponentStore::new();

        // Add a component entry
        let component_id = ComponentId::new(42);
        store.entries.insert(
            "TestComp".to_string(),
            DynamicComponentEntry {
                component_id,
                schema: DynamicSchema {
                    fields: BTreeMap::new(),
                },
            },
        );
        store.dynamic_ids.insert(component_id);

        // Undefine it
        let result = store.undefine("TestComp");
        assert!(result.is_ok());

        // Verify it's gone
        assert!(store.get_id("TestComp").is_none());
        assert!(!store.dynamic_ids.contains(&component_id));
    }

    #[test]
    fn test_undefine_nonexistent_fails() {
        let mut store = DynamicComponentStore::new();
        let result = store.undefine("NonExistent");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not defined"));
    }

    #[test]
    fn test_undefine_after_resolve() {
        let mut store = DynamicComponentStore::new();

        // Add with full name
        let component_id = ComponentId::new(99);
        store.entries.insert(
            "my::full::Name".to_string(),
            DynamicComponentEntry {
                component_id,
                schema: DynamicSchema {
                    fields: BTreeMap::new(),
                },
            },
        );
        store.dynamic_ids.insert(component_id);

        // Undefine with short name (should resolve)
        let result = store.undefine("Name");
        assert!(result.is_ok());
        assert!(store.get_id("Name").is_none());
    }
}

//! 声明式 [`SystemDefinition`] + [`SystemRegistry`]。
//!
//! 系统可以是 LLM agent 或 Rhai 脚本，注册后可通过 id 查找或作为工具暴露给 LLM。

use std::borrow::Cow;
use std::collections::HashMap;

use ai_api_provider::ToolDefinition;
use serde::{Deserialize, Serialize};

/// 系统的实现类型。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SystemKind {
    /// LLM 驱动的 agent 系统。
    Llm,
    /// Rhai 脚本系统。
    RhaiScript { source: String },
}

/// 一个系统的完整定义。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemDefinition {
    pub id: String,
    pub description: String,
    #[serde(serialize_with = "serialize_cow", deserialize_with = "deserialize_cow")]
    pub system_prompt: Cow<'static, str>,
    pub kind: SystemKind,
    /// 该系统可调用的工具名称列表。
    pub tool_names: Vec<String>,
    /// 最大推理迭代次数。
    pub max_iterations: u32,
    /// 如果为 true，该系统作为 LLM 可调用的工具暴露。
    #[serde(default)]
    pub is_tool: bool,
}

fn serialize_cow<S: serde::Serializer>(cow: &Cow<'static, str>, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(cow.as_ref())
}

fn deserialize_cow<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Cow<'static, str>, D::Error> {
    let s = String::deserialize(d)?;
    Ok(Cow::Owned(s))
}

/// 系统注册表，按 id 索引。
#[derive(Debug, Default)]
pub struct SystemRegistry {
    systems: HashMap<String, SystemDefinition>,
}

impl SystemRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册一个系统。若 id 已存在则返回错误。
    pub fn register(&mut self, def: SystemDefinition) -> Result<(), String> {
        if self.systems.contains_key(&def.id) {
            return Err(format!("system '{}' already registered", def.id));
        }
        self.systems.insert(def.id.clone(), def);
        Ok(())
    }

    /// 按 id 查找系统。
    pub fn get(&self, id: &str) -> Option<&SystemDefinition> {
        self.systems.get(id)
    }

    /// Replace a definition at an exclusive runtime boundary. Does not execute it.
    pub fn replace(&mut self, def: SystemDefinition) -> Result<SystemDefinition, String> {
        let current = self
            .systems
            .get_mut(&def.id)
            .ok_or_else(|| format!("system '{}' is not registered", def.id))?;
        Ok(std::mem::replace(current, def))
    }

    /// Remove a definition. Callers are responsible for checking dependencies.
    pub fn remove(&mut self, id: &str) -> Result<SystemDefinition, String> {
        self.systems
            .remove(id)
            .ok_or_else(|| format!("system '{id}' is not registered"))
    }

    /// 遍历所有系统。
    pub fn iter(&self) -> impl Iterator<Item = &SystemDefinition> {
        self.systems.values()
    }

    /// 返回所有标记为 `is_tool` 的系统的 [`ToolDefinition`] 列表。
    pub fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.systems
            .values()
            .filter(|def| def.is_tool)
            .map(|def| ToolDefinition {
                name: def.id.clone(),
                description: if def.description.is_empty() {
                    format!("Rhai-defined tool '{}'", def.id)
                } else {
                    def.description.clone()
                },
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "args": {
                            "type": "object",
                            "description": "Arguments passed to the Rhai tool script as a map"
                        }
                    },
                    "required": ["args"]
                }),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_definitions_filters_is_tool() {
        let mut reg = SystemRegistry::new();

        // 普通系统（非工具）
        reg.register(SystemDefinition {
            id: "regular".into(),
            description: "A regular system".into(),
            system_prompt: Cow::Borrowed(""),
            kind: SystemKind::RhaiScript {
                source: "log('hi')".into(),
            },
            tool_names: vec![],
            max_iterations: 1,
            is_tool: false,
        })
        .unwrap();

        // 工具系统
        reg.register(SystemDefinition {
            id: "my_tool".into(),
            description: "My custom tool".into(),
            system_prompt: Cow::Borrowed(""),
            kind: SystemKind::RhaiScript {
                source: "log('tool')".into(),
            },
            tool_names: vec![],
            max_iterations: 1,
            is_tool: true,
        })
        .unwrap();

        let tools = reg.tool_definitions();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "my_tool");
        assert_eq!(tools[0].description, "My custom tool");
    }

    #[test]
    fn test_tool_definitions_empty_description_falls_back() {
        let mut reg = SystemRegistry::new();

        reg.register(SystemDefinition {
            id: "no_desc".into(),
            description: String::new(),
            system_prompt: Cow::Borrowed(""),
            kind: SystemKind::RhaiScript {
                source: "log('x')".into(),
            },
            tool_names: vec![],
            max_iterations: 1,
            is_tool: true,
        })
        .unwrap();

        let tools = reg.tool_definitions();
        assert_eq!(tools[0].description, "Rhai-defined tool 'no_desc'");
    }
}

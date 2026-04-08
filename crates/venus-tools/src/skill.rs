use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

use venus_core::skill::SkillRegistry;
use venus_core::tool::{Tool, ToolContext, ToolResult};

/// Tool that allows the AI to invoke a skill by name.
pub struct SkillTool {
    registry: Arc<SkillRegistry>,
}

impl SkillTool {
    pub fn new(registry: Arc<SkillRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for SkillTool {
    fn name(&self) -> &str {
        "Skill"
    }

    fn description(&self) -> &str {
        "Execute a skill within the main conversation"
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "skill": {
                    "type": "string",
                    "description": "The skill name to invoke"
                },
                "args": {
                    "type": "string",
                    "description": "Optional arguments for the skill"
                }
            },
            "required": ["skill"]
        })
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult> {
        let skill_name = input
            .get("skill")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'skill' parameter"))?;
        let args = input.get("args").and_then(|v| v.as_str()).unwrap_or("");

        match self.registry.find(skill_name) {
            Some(skill) => {
                let mut content = skill.content.clone();
                if !args.is_empty() {
                    content = format!("{}\n\nArguments: {}", content, args);
                }
                Ok(ToolResult::text(content))
            }
            None => {
                let available: Vec<&str> =
                    self.registry.all().iter().map(|s| s.name.as_str()).collect();
                Ok(ToolResult::error(format!(
                    "Skill '{}' not found. Available: {}",
                    skill_name,
                    available.join(", ")
                )))
            }
        }
    }
}

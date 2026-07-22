use crate::tool::Tool;
use serde_json::Value;

pub struct ToolRegistry {
    tools: Vec<Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new(tools: Vec<Box<dyn Tool>>) -> Self {
        Self { tools }
    }

    /// Create a registry filtered by allowed/disallowed tool lists.
    /// `allowed`: if Some, only tools whose names are in this list are kept.
    /// `disallowed`: tools whose names are in this list are removed.
    pub fn new_filtered(
        tools: Vec<Box<dyn Tool>>,
        allowed: Option<&[String]>,
        disallowed: Option<&[String]>,
    ) -> Self {
        let filtered: Vec<Box<dyn Tool>> = tools
            .into_iter()
            .filter(|t| {
                let name = t.name();
                if let Some(dis) = disallowed {
                    if dis.iter().any(|d| d == name) {
                        return false;
                    }
                }
                if let Some(al) = allowed {
                    return al.iter().any(|a| a == name);
                }
                true
            })
            .collect();
        Self { tools: filtered }
    }

    pub fn find_by_name(&self, name: &str) -> Option<&dyn Tool> {
        self.tools
            .iter()
            .find(|t| t.name() == name)
            .map(|t| t.as_ref())
    }

    pub fn all(&self) -> &[Box<dyn Tool>] {
        &self.tools
    }

    pub fn api_definitions(&self) -> Vec<Value> {
        self.tools.iter().map(|t| t.to_api_definition()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::{Tool, ToolContext, ToolResult};
    use async_trait::async_trait;

    struct MockTool {
        name: String,
    }

    #[async_trait]
    impl Tool for MockTool {
        fn name(&self) -> &str {
            &self.name
        }
        fn description(&self) -> &str {
            "A mock tool"
        }
        fn input_schema(&self) -> Value {
            serde_json::json!({"type": "object", "properties": {}})
        }
        async fn execute(&self, _input: Value, _ctx: &ToolContext) -> anyhow::Result<ToolResult> {
            Ok(ToolResult::text("mock result"))
        }
        fn is_read_only(&self) -> bool {
            true
        }
    }

    #[test]
    fn test_find_by_name() {
        let tools: Vec<Box<dyn Tool>> = vec![
            Box::new(MockTool {
                name: "Bash".to_string(),
            }),
            Box::new(MockTool {
                name: "Read".to_string(),
            }),
        ];
        let registry = ToolRegistry::new(tools);
        assert!(registry.find_by_name("Bash").is_some());
        assert!(registry.find_by_name("Read").is_some());
        assert!(registry.find_by_name("Write").is_none());
    }

    #[test]
    fn test_all_returns_all_tools() {
        let tools: Vec<Box<dyn Tool>> = vec![
            Box::new(MockTool {
                name: "A".to_string(),
            }),
            Box::new(MockTool {
                name: "B".to_string(),
            }),
            Box::new(MockTool {
                name: "C".to_string(),
            }),
        ];
        let registry = ToolRegistry::new(tools);
        assert_eq!(registry.all().len(), 3);
    }

    #[test]
    fn test_api_definitions() {
        let tools: Vec<Box<dyn Tool>> = vec![Box::new(MockTool {
            name: "TestTool".to_string(),
        })];
        let registry = ToolRegistry::new(tools);
        let defs = registry.api_definitions();
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0]["name"], "TestTool");
        assert!(defs[0]["description"].is_string());
        assert!(defs[0]["input_schema"].is_object());
    }

    #[test]
    fn test_empty_registry() {
        let registry = ToolRegistry::new(vec![]);
        assert!(registry.all().is_empty());
        assert!(registry.find_by_name("anything").is_none());
        assert!(registry.api_definitions().is_empty());
    }
}

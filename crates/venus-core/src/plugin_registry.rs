use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::plugin::{Plugin, PluginCommandDef, PluginLoader, PluginMcpServer};

/// Central registry that holds all discovered plugins and provides
/// convenience accessors for their contributed tools, MCP servers, and commands.
pub struct PluginRegistry {
    plugins: Vec<Plugin>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
        }
    }

    /// Discover and load all plugins from the given directories.
    pub async fn load_all(&mut self, dirs: &[PathBuf]) -> Result<()> {
        let discovered = PluginLoader::discover_from_dirs(dirs).await;
        self.plugins.extend(discovered);
        Ok(())
    }

    /// All loaded plugins.
    pub fn all_plugins(&self) -> &[Plugin] {
        &self.plugins
    }

    /// Merge MCP server configurations from all plugins into a single map.
    pub fn mcp_server_configs(&self) -> HashMap<String, PluginMcpServer> {
        let mut map = HashMap::new();
        for plugin in &self.plugins {
            for (name, config) in &plugin.manifest.mcp_servers {
                map.insert(name.clone(), config.clone());
            }
        }
        map
    }

    /// All command definitions across plugins (not deduplicated).
    pub fn command_defs(&self) -> Vec<&PluginCommandDef> {
        self.plugins
            .iter()
            .flat_map(|p| p.manifest.commands.iter())
            .collect()
    }
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_plugin(dir: &std::path::Path, name: &str, tools: usize) {
        let mut tools_vec = Vec::new();
        for i in 0..tools {
            tools_vec.push(serde_json::json!({
                "name": format!("tool_{}", i),
                "description": format!("Tool {}", i),
                "command": format!("echo tool{}", i),
            }));
        }
        let manifest = serde_json::json!({
            "name": name,
            "version": "1.0.0",
            "tools": tools_vec,
            "mcpServers": {
                format!("{}-server", name): {
                    "command": "npx",
                    "args": ["-y", format!("{}-mcp", name)]
                }
            },
            "commands": [
                {
                    "name": format!("{}_cmd", name),
                    "description": format!("A command from {}", name),
                    "type": "prompt",
                    "prompt": format!("Prompt from {}", name)
                }
            ]
        });
        std::fs::write(dir.join("plugin.json"), manifest.to_string()).unwrap();
    }

    #[tokio::test]
    async fn test_registry_load_all() {
        let tmp = TempDir::new().unwrap();
        let plugins_dir = tmp.path().join("plugins");
        std::fs::create_dir_all(&plugins_dir).unwrap();

        let p1 = plugins_dir.join("p1");
        std::fs::create_dir_all(&p1).unwrap();
        write_plugin(&p1, "p1", 2);

        let p2 = plugins_dir.join("p2");
        std::fs::create_dir_all(&p2).unwrap();
        write_plugin(&p2, "p2", 1);

        let mut registry = PluginRegistry::new();
        registry.load_all(&[plugins_dir]).await.unwrap();

        assert_eq!(registry.all_plugins().len(), 2);
    }

    #[test]
    fn test_empty_registry() {
        let registry = PluginRegistry::new();
        assert!(registry.all_plugins().is_empty());
        assert!(registry.mcp_server_configs().is_empty());
        assert!(registry.command_defs().is_empty());
    }

    #[tokio::test]
    async fn test_mcp_server_configs_merge() {
        let tmp = TempDir::new().unwrap();
        let plugins_dir = tmp.path().join("plugins");
        std::fs::create_dir_all(&plugins_dir).unwrap();

        let p1 = plugins_dir.join("p1");
        std::fs::create_dir_all(&p1).unwrap();
        write_plugin(&p1, "p1", 0);

        let p2 = plugins_dir.join("p2");
        std::fs::create_dir_all(&p2).unwrap();
        write_plugin(&p2, "p2", 0);

        let mut registry = PluginRegistry::new();
        registry.load_all(&[plugins_dir]).await.unwrap();

        let configs = registry.mcp_server_configs();
        assert!(configs.contains_key("p1-server"));
        assert!(configs.contains_key("p2-server"));
    }

    #[tokio::test]
    async fn test_command_defs() {
        let tmp = TempDir::new().unwrap();
        let plugins_dir = tmp.path().join("plugins");
        std::fs::create_dir_all(&plugins_dir).unwrap();

        let p1 = plugins_dir.join("p1");
        std::fs::create_dir_all(&p1).unwrap();
        write_plugin(&p1, "p1", 0);

        let mut registry = PluginRegistry::new();
        registry.load_all(&[plugins_dir]).await.unwrap();

        let cmds = registry.command_defs();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].name, "p1_cmd");
    }
}

use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Top-level manifest for a plugin, stored as `plugin.json` in the plugin directory.
#[derive(Debug, Clone, Deserialize)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    #[serde(default)]
    pub tools: Vec<PluginToolDef>,
    #[serde(default, rename = "mcpServers")]
    pub mcp_servers: HashMap<String, PluginMcpServer>,
    #[serde(default)]
    pub commands: Vec<PluginCommandDef>,
}

/// A tool exposed by a plugin. The tool runs an external command, piping JSON
/// input on stdin and capturing stdout as the result.
#[derive(Debug, Clone, Deserialize)]
pub struct PluginToolDef {
    pub name: String,
    pub description: String,
    pub command: String,
    #[serde(rename = "inputSchema", default)]
    pub input_schema: Option<serde_json::Value>,
}

/// MCP server configuration contributed by a plugin.
#[derive(Debug, Clone, Deserialize)]
pub struct PluginMcpServer {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

/// A slash-command exposed by a plugin.
#[derive(Debug, Clone, Deserialize)]
pub struct PluginCommandDef {
    pub name: String,
    pub description: String,
    #[serde(rename = "type")]
    pub cmd_type: String,
    pub prompt: Option<String>,
}

/// A loaded plugin with its manifest and resolved base directory.
#[derive(Debug)]
pub struct Plugin {
    pub manifest: PluginManifest,
    pub base_dir: PathBuf,
}

/// Discovers and loads plugins from a set of candidate directories.
pub struct PluginLoader;

impl PluginLoader {
    /// Scan each directory for sub-directories that contain a `plugin.json`.
    pub async fn discover_from_dirs(dirs: &[PathBuf]) -> Vec<Plugin> {
        let mut plugins = Vec::new();
        for dir in dirs {
            if !dir.is_dir() {
                continue;
            }
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        match Self::load_plugin(&path).await {
                            Ok(plugin) => plugins.push(plugin),
                            Err(e) => {
                                tracing::warn!(
                                    "Skipping plugin directory {}: {}",
                                    path.display(),
                                    e
                                );
                            }
                        }
                    }
                }
            }
        }
        plugins
    }

    /// Load a single plugin from a directory that must contain `plugin.json`.
    pub async fn load_plugin(dir: &Path) -> Result<Plugin> {
        let manifest_path = dir.join("plugin.json");
        let data = std::fs::read_to_string(&manifest_path)
            .with_context(|| format!("failed to read {}", manifest_path.display()))?;
        let manifest: PluginManifest = serde_json::from_str(&data)
            .with_context(|| format!("failed to parse {}", manifest_path.display()))?;
        Ok(Plugin {
            manifest,
            base_dir: dir.to_path_buf(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample_manifest_json() -> String {
        serde_json::json!({
            "name": "test-plugin",
            "version": "0.1.0",
            "description": "A test plugin",
            "tools": [
                {
                    "name": "greet",
                    "description": "Greets a user",
                    "command": "./greet.sh",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string" }
                        },
                        "required": ["name"]
                    }
                }
            ],
            "mcpServers": {
                "test-server": {
                    "command": "npx",
                    "args": ["-y", "test-mcp"],
                    "env": { "TOKEN": "abc" }
                }
            },
            "commands": [
                {
                    "name": "hello",
                    "description": "Say hello",
                    "type": "prompt",
                    "prompt": "Hello from test-plugin!"
                }
            ]
        })
        .to_string()
    }

    #[test]
    fn test_manifest_parsing() {
        let json = sample_manifest_json();
        let manifest: PluginManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(manifest.name, "test-plugin");
        assert_eq!(manifest.version, "0.1.0");
        assert_eq!(manifest.description.as_deref(), Some("A test plugin"));
        assert_eq!(manifest.tools.len(), 1);
        assert_eq!(manifest.tools[0].name, "greet");
        assert_eq!(manifest.tools[0].command, "./greet.sh");
        assert!(manifest.tools[0].input_schema.is_some());
        assert_eq!(manifest.mcp_servers.len(), 1);
        assert!(manifest.mcp_servers.contains_key("test-server"));
        assert_eq!(manifest.commands.len(), 1);
        assert_eq!(manifest.commands[0].name, "hello");
    }

    #[test]
    fn test_manifest_minimal() {
        let json = serde_json::json!({
            "name": "minimal",
            "version": "1.0.0"
        })
        .to_string();
        let manifest: PluginManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(manifest.name, "minimal");
        assert!(manifest.tools.is_empty());
        assert!(manifest.mcp_servers.is_empty());
        assert!(manifest.commands.is_empty());
        assert!(manifest.description.is_none());
    }

    #[tokio::test]
    async fn test_load_plugin() {
        let tmp = TempDir::new().unwrap();
        let plugin_dir = tmp.path().join("my-plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(plugin_dir.join("plugin.json"), sample_manifest_json()).unwrap();

        let plugin = PluginLoader::load_plugin(&plugin_dir).await.unwrap();
        assert_eq!(plugin.manifest.name, "test-plugin");
        assert_eq!(plugin.base_dir, plugin_dir);
    }

    #[tokio::test]
    async fn test_load_plugin_missing_manifest() {
        let tmp = TempDir::new().unwrap();
        let plugin_dir = tmp.path().join("empty-plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();

        let result = PluginLoader::load_plugin(&plugin_dir).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_discover_from_dirs() {
        let tmp = TempDir::new().unwrap();
        let plugins_dir = tmp.path().join("plugins");
        std::fs::create_dir_all(&plugins_dir).unwrap();

        // Create two valid plugins
        let p1_dir = plugins_dir.join("alpha");
        std::fs::create_dir_all(&p1_dir).unwrap();
        std::fs::write(
            p1_dir.join("plugin.json"),
            serde_json::json!({"name": "alpha", "version": "1.0.0"}).to_string(),
        )
        .unwrap();

        let p2_dir = plugins_dir.join("beta");
        std::fs::create_dir_all(&p2_dir).unwrap();
        std::fs::write(
            p2_dir.join("plugin.json"),
            serde_json::json!({"name": "beta", "version": "2.0.0"}).to_string(),
        )
        .unwrap();

        // Create a directory without plugin.json (should be skipped)
        let skip_dir = plugins_dir.join("gamma");
        std::fs::create_dir_all(&skip_dir).unwrap();

        let plugins = PluginLoader::discover_from_dirs(&[plugins_dir]).await;
        assert_eq!(plugins.len(), 2);
        let names: Vec<&str> = plugins.iter().map(|p| p.manifest.name.as_str()).collect();
        assert!(names.contains(&"alpha"));
        assert!(names.contains(&"beta"));
    }

    #[tokio::test]
    async fn test_discover_nonexistent_dir() {
        let plugins = PluginLoader::discover_from_dirs(&[PathBuf::from("/nonexistent/path")]).await;
        assert!(plugins.is_empty());
    }
}

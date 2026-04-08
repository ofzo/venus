use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, warn};

use venus_core::tool::Tool;
use venus_utils::config::McpServerConfig;

use crate::client::McpClient;
use crate::tool_bridge::McpTool;

/// Manages multiple MCP server connections.
pub struct McpManager {
    clients: HashMap<String, Arc<McpClient>>,
}

impl McpManager {
    /// Connect to all configured MCP servers.
    /// Servers that fail to start are logged and skipped.
    pub async fn start_all(configs: &HashMap<String, McpServerConfig>) -> Result<Self> {
        let mut clients = HashMap::new();

        for (name, config) in configs {
            match McpClient::connect(name, config).await {
                Ok(client) => {
                    info!("MCP server '{}' started with {} tools", name, client.tools().len());
                    clients.insert(name.clone(), Arc::new(client));
                }
                Err(e) => {
                    warn!("Failed to start MCP server '{}': {}", name, e);
                }
            }
        }

        Ok(Self { clients })
    }

    /// Return Tool implementations for all discovered MCP tools.
    pub fn all_tools(&self) -> Vec<Box<dyn Tool>> {
        let mut tools: Vec<Box<dyn Tool>> = Vec::new();
        for (name, client) in &self.clients {
            for tool_def in client.tools() {
                tools.push(Box::new(McpTool::new(name, tool_def.clone(), Arc::clone(client))));
            }
        }
        tools
    }

    /// Gracefully shut down all MCP servers.
    pub async fn shutdown_all(&mut self) -> Result<()> {
        for (name, client) in &self.clients {
            if let Err(e) = client.shutdown().await {
                warn!("Error shutting down MCP server '{}': {}", name, e);
            }
        }
        self.clients.clear();
        Ok(())
    }
}

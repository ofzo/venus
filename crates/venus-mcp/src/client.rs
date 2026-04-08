use anyhow::{Context, Result};
use serde::Deserialize;
use tracing::{debug, warn};

use venus_utils::config::McpServerConfig;

use crate::transport::{
    JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, McpTransport, StdioTransport,
};

/// Describes a single tool exposed by an MCP server.
#[derive(Debug, Clone)]
pub struct McpToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// Client that manages a connection to one MCP server.
pub struct McpClient {
    pub server_name: String,
    transport: Box<dyn McpTransport>,
    tools: Vec<McpToolDef>,
}

// Helper structs for deserializing MCP protocol responses.

#[derive(Debug, Deserialize)]
struct ToolsListResult {
    tools: Vec<RawToolDef>,
}

#[derive(Debug, Deserialize)]
struct RawToolDef {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default, rename = "inputSchema")]
    input_schema: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct CallToolResult {
    content: Vec<ToolContent>,
    #[serde(default)]
    #[allow(dead_code)]
    is_error: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ToolContent {
    #[serde(default)]
    text: Option<String>,
}

impl McpClient {
    /// Connect to an MCP server, perform initialization handshake, and discover tools.
    pub async fn connect(name: &str, config: &McpServerConfig) -> Result<Self> {
        let transport: Box<dyn McpTransport> = match config.transport.as_str() {
            "stdio" => Box::new(
                StdioTransport::spawn(config)
                    .with_context(|| format!("failed to start MCP server '{}'", name))?,
            ),
            other => anyhow::bail!("unsupported MCP transport: {}", other),
        };

        let mut client = Self {
            server_name: name.to_string(),
            transport,
            tools: Vec::new(),
        };

        client.initialize().await?;
        client.tools = client.list_tools().await?;

        debug!(
            "MCP server '{}': discovered {} tools",
            name,
            client.tools.len()
        );

        Ok(client)
    }

    /// Send the `initialize` request followed by `initialized` notification.
    async fn initialize(&self) -> Result<()> {
        let id = self.next_id();
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id,
            method: "initialize".into(),
            params: Some(serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "venus",
                    "version": env!("CARGO_PKG_VERSION"),
                }
            })),
        };

        let resp = self.transport.send(req).await?;
        Self::check_error(&resp)?;

        // Send the `initialized` notification
        self.transport
            .notify(JsonRpcNotification {
                jsonrpc: "2.0".into(),
                method: "notifications/initialized".into(),
                params: None,
            })
            .await?;

        Ok(())
    }

    /// Discover tools from the MCP server.
    async fn list_tools(&self) -> Result<Vec<McpToolDef>> {
        let id = self.next_id();
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id,
            method: "tools/list".into(),
            params: None,
        };

        let resp = self.transport.send(req).await?;
        Self::check_error(&resp)?;

        let result_value = resp
            .result
            .context("tools/list response missing result field")?;

        let list: ToolsListResult = serde_json::from_value(result_value)
            .context("failed to deserialize tools/list result")?;

        Ok(list
            .tools
            .into_iter()
            .map(|raw| McpToolDef {
                name: raw.name,
                description: raw.description.unwrap_or_default(),
                input_schema: raw.input_schema.unwrap_or(serde_json::json!({"type": "object"})),
            })
            .collect())
    }

    /// Invoke a tool on the MCP server.
    pub async fn call_tool(
        &self,
        tool_name: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let id = self.next_id();
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id,
            method: "tools/call".into(),
            params: Some(serde_json::json!({
                "name": tool_name,
                "arguments": args,
            })),
        };

        let resp = self.transport.send(req).await?;
        Self::check_error(&resp)?;

        let result_value = resp
            .result
            .context("tools/call response missing result field")?;

        // Try to parse structured content; fall back to returning raw value.
        match serde_json::from_value::<CallToolResult>(result_value.clone()) {
            Ok(call_result) => {
                // Concatenate text content blocks
                let text: String = call_result
                    .content
                    .iter()
                    .filter_map(|c| c.text.as_deref())
                    .collect::<Vec<_>>()
                    .join("\n");
                Ok(serde_json::Value::String(text))
            }
            Err(_) => Ok(result_value),
        }
    }

    /// Gracefully shut down the MCP server connection.
    pub async fn shutdown(&self) -> Result<()> {
        // Send shutdown request (best-effort)
        let id = self.next_id();
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id,
            method: "shutdown".into(),
            params: None,
        };
        if let Err(e) = self.transport.send(req).await {
            warn!("MCP server '{}' shutdown request failed: {}", self.server_name, e);
        }
        self.transport.close().await
    }

    pub fn tools(&self) -> &[McpToolDef] {
        &self.tools
    }

    fn next_id(&self) -> u64 {
        // Delegate to transport for stdio; use a simple counter otherwise
        // For now we just use a monotonic counter per-client
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    }

    fn check_error(resp: &JsonRpcResponse) -> Result<()> {
        if let Some(ref err) = resp.error {
            anyhow::bail!("MCP error ({}): {}", err.code, err.message);
        }
        Ok(())
    }
}

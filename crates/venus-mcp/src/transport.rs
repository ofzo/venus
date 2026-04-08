use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::Mutex;
use tracing::debug;

use venus_utils::config::McpServerConfig;

// --- JSON-RPC types ---

#[derive(Debug, Serialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

/// JSON-RPC notification (no id field).
#[derive(Debug, Serialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Option<u64>,
    pub result: Option<serde_json::Value>,
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
}

// --- Transport trait ---

#[async_trait]
pub trait McpTransport: Send + Sync {
    async fn send(&self, request: JsonRpcRequest) -> Result<JsonRpcResponse>;
    async fn notify(&self, notification: JsonRpcNotification) -> Result<()>;
    async fn close(&self) -> Result<()>;
}

// --- Stdio transport ---

pub struct StdioTransport {
    child: Mutex<Child>,
    stdin: Mutex<ChildStdin>,
    reader: Mutex<BufReader<ChildStdout>>,
    next_id: AtomicU64,
}

impl StdioTransport {
    /// Launch the MCP server process and return a transport handle.
    pub fn spawn(config: &McpServerConfig) -> Result<Self> {
        let mut cmd = tokio::process::Command::new(&config.command);
        cmd.args(&config.args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());

        if let Some(ref env_map) = config.env {
            for (k, v) in env_map {
                cmd.env(k, v);
            }
        }

        let mut child = cmd
            .spawn()
            .with_context(|| format!("failed to spawn MCP server: {}", config.command))?;

        let stdin = child
            .stdin
            .take()
            .context("failed to capture MCP server stdin")?;
        let stdout = child
            .stdout
            .take()
            .context("failed to capture MCP server stdout")?;

        Ok(Self {
            child: Mutex::new(child),
            stdin: Mutex::new(stdin),
            reader: Mutex::new(BufReader::new(stdout)),
            next_id: AtomicU64::new(1),
        })
    }

    pub fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::SeqCst)
    }
}

#[async_trait]
impl McpTransport for StdioTransport {
    async fn send(&self, request: JsonRpcRequest) -> Result<JsonRpcResponse> {
        let mut line = serde_json::to_string(&request)?;
        line.push('\n');

        debug!("MCP send: {}", line.trim());

        {
            let mut stdin = self.stdin.lock().await;
            stdin.write_all(line.as_bytes()).await?;
            stdin.flush().await?;
        }

        let mut buf = String::new();
        {
            let mut reader = self.reader.lock().await;
            // Read lines until we get a non-empty JSON response
            loop {
                buf.clear();
                let n = reader.read_line(&mut buf).await?;
                if n == 0 {
                    anyhow::bail!("MCP server closed stdout unexpectedly");
                }
                let trimmed = buf.trim();
                if !trimmed.is_empty() {
                    break;
                }
            }
        }

        debug!("MCP recv: {}", buf.trim());
        let response: JsonRpcResponse = serde_json::from_str(buf.trim())
            .with_context(|| format!("failed to parse MCP response: {}", buf.trim()))?;
        Ok(response)
    }

    async fn notify(&self, notification: JsonRpcNotification) -> Result<()> {
        let mut line = serde_json::to_string(&notification)?;
        line.push('\n');

        debug!("MCP notify: {}", line.trim());

        let mut stdin = self.stdin.lock().await;
        stdin.write_all(line.as_bytes()).await?;
        stdin.flush().await?;
        Ok(())
    }

    async fn close(&self) -> Result<()> {
        let mut child = self.child.lock().await;
        // Try to kill the process gracefully
        let _ = child.kill().await;
        Ok(())
    }
}

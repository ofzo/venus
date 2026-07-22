use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;
use tracing::{debug, warn};

/// Manages multiple language server processes, one per language.
pub struct LspManager {
    servers: Mutex<HashMap<String, LspServer>>,
    working_dir: PathBuf,
}

struct LspServer {
    _process: Child,
    stdin: ChildStdin,
    reader: BufReader<ChildStdout>,
    next_id: u64,
    initialized: bool,
}

/// A source location returned by LSP operations.
#[derive(Debug, Clone)]
pub struct LspLocation {
    pub file: String,
    pub line: u32,
    pub character: u32,
    pub end_line: Option<u32>,
    pub end_character: Option<u32>,
}

/// A symbol returned by document/workspace symbol operations.
#[derive(Debug, Clone)]
pub struct LspSymbol {
    pub name: String,
    pub kind: String,
    pub location: LspLocation,
}

/// Result of an LSP operation.
#[derive(Debug, Clone)]
pub enum LspResult {
    Locations(Vec<LspLocation>),
    Hover(String),
    Symbols(Vec<LspSymbol>),
    Error(String),
}

impl LspManager {
    pub fn new(working_dir: PathBuf) -> Self {
        Self {
            servers: Mutex::new(HashMap::new()),
            working_dir,
        }
    }

    /// Execute an LSP operation on the given file at the given position.
    /// `line` and `character` are 1-based (converted to 0-based for the protocol).
    pub async fn execute(
        &self,
        operation: &str,
        file_path: &Path,
        line: u32,
        character: u32,
    ) -> Result<LspResult> {
        let ext = file_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_string();

        let (cmd, args) = detect_language_server(file_path)
            .ok_or_else(|| anyhow!("no language server configured for .{ext} files"))?;

        // Ensure server is started and initialized
        {
            let mut servers = self.servers.lock().await;
            if !servers.contains_key(&ext) {
                let server = start_server(cmd, &args, &self.working_dir).await?;
                servers.insert(ext.clone(), server);
            }
            let server = servers.get_mut(&ext).unwrap();
            if !server.initialized {
                initialize_server(server, &self.working_dir).await?;
            }
        }

        // Send didOpen for the file
        self.did_open(file_path, &ext).await?;

        // Convert to 0-based positions
        let lsp_line = line.saturating_sub(1);
        let lsp_char = character.saturating_sub(1);

        let uri = path_to_uri(file_path);

        match operation {
            "goToDefinition" => {
                self.request_locations(&ext, "textDocument/definition", &uri, lsp_line, lsp_char)
                    .await
            }
            "findReferences" => {
                self.request_references(&ext, &uri, lsp_line, lsp_char)
                    .await
            }
            "hover" => self.request_hover(&ext, &uri, lsp_line, lsp_char).await,
            "documentSymbol" => self.request_document_symbols(&ext, &uri).await,
            "workspaceSymbol" => self.request_workspace_symbols(&ext, "").await,
            "goToImplementation" => {
                self.request_locations(
                    &ext,
                    "textDocument/implementation",
                    &uri,
                    lsp_line,
                    lsp_char,
                )
                .await
            }
            _ => Ok(LspResult::Error(format!("unknown operation: {operation}"))),
        }
    }

    /// Shutdown all managed language servers.
    pub async fn shutdown(&self) -> Result<()> {
        let mut servers = self.servers.lock().await;
        for (lang, server) in servers.iter_mut() {
            debug!("shutting down {lang} language server");
            if let Err(e) = send_request(
                &mut server.stdin,
                &mut server.reader,
                &mut server.next_id,
                "shutdown",
                json!(null),
            )
            .await
            {
                warn!("error shutting down {lang} server: {e}");
            }
            if let Err(e) = send_notification(&mut server.stdin, "exit", json!(null)).await {
                warn!("error sending exit to {lang} server: {e}");
            }
        }
        servers.clear();
        Ok(())
    }

    async fn did_open(&self, file_path: &Path, ext: &str) -> Result<()> {
        let uri = path_to_uri(file_path);
        let language_id = ext_to_language_id(ext);
        let text = tokio::fs::read_to_string(file_path)
            .await
            .unwrap_or_default();

        let mut servers = self.servers.lock().await;
        let server = servers
            .get_mut(ext)
            .ok_or_else(|| anyhow!("server not found for .{ext}"))?;

        send_notification(
            &mut server.stdin,
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": uri,
                    "languageId": language_id,
                    "version": 1,
                    "text": text,
                }
            }),
        )
        .await
    }

    async fn request_locations(
        &self,
        ext: &str,
        method: &str,
        uri: &str,
        line: u32,
        character: u32,
    ) -> Result<LspResult> {
        let params = json!({
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character },
        });
        let mut servers = self.servers.lock().await;
        let server = servers.get_mut(ext).ok_or_else(|| anyhow!("no server"))?;
        let resp = send_request(
            &mut server.stdin,
            &mut server.reader,
            &mut server.next_id,
            method,
            params,
        )
        .await?;
        Ok(LspResult::Locations(parse_locations(&resp)))
    }

    async fn request_references(
        &self,
        ext: &str,
        uri: &str,
        line: u32,
        character: u32,
    ) -> Result<LspResult> {
        let params = json!({
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character },
            "context": { "includeDeclaration": true },
        });
        let mut servers = self.servers.lock().await;
        let server = servers.get_mut(ext).ok_or_else(|| anyhow!("no server"))?;
        let resp = send_request(
            &mut server.stdin,
            &mut server.reader,
            &mut server.next_id,
            "textDocument/references",
            params,
        )
        .await?;
        Ok(LspResult::Locations(parse_locations(&resp)))
    }

    async fn request_hover(
        &self,
        ext: &str,
        uri: &str,
        line: u32,
        character: u32,
    ) -> Result<LspResult> {
        let params = json!({
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character },
        });
        let mut servers = self.servers.lock().await;
        let server = servers.get_mut(ext).ok_or_else(|| anyhow!("no server"))?;
        let resp = send_request(
            &mut server.stdin,
            &mut server.reader,
            &mut server.next_id,
            "textDocument/hover",
            params,
        )
        .await?;
        Ok(LspResult::Hover(parse_hover(&resp)))
    }

    async fn request_document_symbols(&self, ext: &str, uri: &str) -> Result<LspResult> {
        let params = json!({ "textDocument": { "uri": uri } });
        let mut servers = self.servers.lock().await;
        let server = servers.get_mut(ext).ok_or_else(|| anyhow!("no server"))?;
        let resp = send_request(
            &mut server.stdin,
            &mut server.reader,
            &mut server.next_id,
            "textDocument/documentSymbol",
            params,
        )
        .await?;
        Ok(LspResult::Symbols(parse_symbols(&resp)))
    }

    async fn request_workspace_symbols(&self, ext: &str, query: &str) -> Result<LspResult> {
        let params = json!({ "query": query });
        let mut servers = self.servers.lock().await;
        let server = servers.get_mut(ext).ok_or_else(|| anyhow!("no server"))?;
        let resp = send_request(
            &mut server.stdin,
            &mut server.reader,
            &mut server.next_id,
            "workspace/symbol",
            params,
        )
        .await?;
        Ok(LspResult::Symbols(parse_symbols(&resp)))
    }
}

impl Drop for LspManager {
    fn drop(&mut self) {
        // Best-effort kill of child processes
        if let Ok(mut servers) = self.servers.try_lock() {
            for (_lang, server) in servers.iter_mut() {
                // ChildStdin drop will close stdin, causing server to exit
                // We also try to kill explicitly
                let _ = server._process.start_kill();
            }
        }
    }
}

// --- Protocol helpers ---

async fn start_server(cmd: &str, args: &[&str], working_dir: &Path) -> Result<LspServer> {
    debug!("starting language server: {cmd} {}", args.join(" "));
    let mut process = Command::new(cmd)
        .args(args)
        .current_dir(working_dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .with_context(|| format!("failed to start language server '{cmd}'. Is it installed?"))?;

    let stdin = process.stdin.take().unwrap();
    let stdout = process.stdout.take().unwrap();
    let reader = BufReader::new(stdout);

    Ok(LspServer {
        _process: process,
        stdin,
        reader,
        next_id: 1,
        initialized: false,
    })
}

async fn initialize_server(server: &mut LspServer, working_dir: &Path) -> Result<()> {
    let root_uri = path_to_uri(working_dir);
    let params = json!({
        "processId": std::process::id(),
        "rootUri": root_uri,
        "capabilities": {
            "textDocument": {
                "definition": { "dynamicRegistration": false },
                "references": { "dynamicRegistration": false },
                "hover": {
                    "dynamicRegistration": false,
                    "contentFormat": ["plaintext", "markdown"],
                },
                "documentSymbol": { "dynamicRegistration": false },
                "implementation": { "dynamicRegistration": false },
            },
            "workspace": {
                "symbol": { "dynamicRegistration": false },
            },
        },
    });

    let _result = send_request(
        &mut server.stdin,
        &mut server.reader,
        &mut server.next_id,
        "initialize",
        params,
    )
    .await
    .context("LSP initialize failed")?;

    // Send initialized notification
    send_notification(&mut server.stdin, "initialized", json!({})).await?;

    server.initialized = true;
    Ok(())
}

async fn send_request(
    stdin: &mut ChildStdin,
    reader: &mut BufReader<ChildStdout>,
    next_id: &mut u64,
    method: &str,
    params: Value,
) -> Result<Value> {
    let id = *next_id;
    *next_id += 1;

    let msg = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    });

    write_message(stdin, &msg).await?;
    read_response(reader, id).await
}

async fn send_notification(stdin: &mut ChildStdin, method: &str, params: Value) -> Result<()> {
    let msg = json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
    });
    write_message(stdin, &msg).await
}

async fn write_message(stdin: &mut ChildStdin, msg: &Value) -> Result<()> {
    let body = serde_json::to_string(msg)?;
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    stdin.write_all(header.as_bytes()).await?;
    stdin.write_all(body.as_bytes()).await?;
    stdin.flush().await?;
    Ok(())
}

async fn read_response(reader: &mut BufReader<ChildStdout>, expected_id: u64) -> Result<Value> {
    // Read messages until we find the response matching our id.
    // Skip notifications and other messages from the server.
    loop {
        let body = read_message(reader).await?;
        let msg: Value = serde_json::from_str(&body)
            .with_context(|| format!("invalid JSON from language server: {body}"))?;

        // Check if this is a response (has "id" field)
        if let Some(msg_id) = msg.get("id").and_then(|v| v.as_u64()) {
            if msg_id == expected_id {
                if let Some(error) = msg.get("error") {
                    let message = error
                        .get("message")
                        .and_then(|m| m.as_str())
                        .unwrap_or("unknown error");
                    return Err(anyhow!("LSP error: {message}"));
                }
                return Ok(msg.get("result").cloned().unwrap_or(Value::Null));
            }
        }
        // Otherwise it's a notification or wrong id — skip it
    }
}

async fn read_message(reader: &mut BufReader<ChildStdout>) -> Result<String> {
    // Read headers until blank line
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            return Err(anyhow!("language server closed connection"));
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            break;
        }
        if let Some(val) = trimmed.strip_prefix("Content-Length:") {
            content_length = Some(val.trim().parse().context("invalid Content-Length")?);
        }
    }

    let length = content_length.ok_or_else(|| anyhow!("missing Content-Length header"))?;
    let mut buf = vec![0u8; length];
    reader.read_exact(&mut buf).await?;
    String::from_utf8(buf).context("invalid UTF-8 from language server")
}

// --- Language detection ---

fn detect_language_server(file_path: &Path) -> Option<(&'static str, Vec<&'static str>)> {
    let ext = file_path.extension()?.to_str()?;
    match ext {
        "rs" => Some(("rust-analyzer", vec![])),
        "ts" | "tsx" | "js" | "jsx" => Some(("typescript-language-server", vec!["--stdio"])),
        "py" => Some(("pylsp", vec![])),
        "go" => Some(("gopls", vec!["serve"])),
        "java" => Some(("jdtls", vec![])),
        _ => None,
    }
}

fn ext_to_language_id(ext: &str) -> &str {
    match ext {
        "rs" => "rust",
        "ts" => "typescript",
        "tsx" => "typescriptreact",
        "js" => "javascript",
        "jsx" => "javascriptreact",
        "py" => "python",
        "go" => "go",
        "java" => "java",
        _ => "plaintext",
    }
}

fn path_to_uri(path: &Path) -> String {
    // Convert an absolute path to a file:// URI
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    format!("file://{}", abs.display())
}

// --- Response parsing ---

fn parse_locations(result: &Value) -> Vec<LspLocation> {
    let mut out = Vec::new();

    // Result can be Location, Location[], or LocationLink[]
    let items = match result {
        Value::Array(arr) => arr.clone(),
        Value::Object(_) => vec![result.clone()],
        _ => return out,
    };

    for item in &items {
        // LocationLink has targetUri/targetRange, Location has uri/range
        let (uri, range) = if let Some(target_uri) = item.get("targetUri") {
            (
                target_uri,
                item.get("targetSelectionRange")
                    .or_else(|| item.get("targetRange")),
            )
        } else {
            (item.get("uri").unwrap_or(&Value::Null), item.get("range"))
        };

        if let (Some(uri_str), Some(range)) = (uri.as_str(), range) {
            let file = uri_to_path(uri_str);
            let start = range.get("start").unwrap_or(&Value::Null);
            let end = range.get("end").unwrap_or(&Value::Null);

            out.push(LspLocation {
                file,
                line: start.get("line").and_then(|v| v.as_u64()).unwrap_or(0) as u32 + 1,
                character: start.get("character").and_then(|v| v.as_u64()).unwrap_or(0) as u32 + 1,
                end_line: end
                    .get("line")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32 + 1),
                end_character: end
                    .get("character")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32 + 1),
            });
        }
    }
    out
}

fn parse_hover(result: &Value) -> String {
    if result.is_null() {
        return "No hover information available.".to_string();
    }

    // Hover result has a "contents" field
    let contents = result.get("contents").unwrap_or(result);

    match contents {
        Value::String(s) => s.clone(),
        Value::Object(obj) => {
            // MarkedString { language, value } or MarkupContent { kind, value }
            obj.get("value")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        }
        Value::Array(arr) => {
            // Array of MarkedString
            arr.iter()
                .filter_map(|item| match item {
                    Value::String(s) => Some(s.clone()),
                    Value::Object(obj) => {
                        obj.get("value").and_then(|v| v.as_str()).map(String::from)
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n\n")
        }
        _ => format!("{}", contents),
    }
}

fn parse_symbols(result: &Value) -> Vec<LspSymbol> {
    let mut out = Vec::new();
    let items = match result {
        Value::Array(arr) => arr,
        _ => return out,
    };

    for item in items {
        let name = item
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let kind_num = item.get("kind").and_then(|v| v.as_u64()).unwrap_or(0);
        let kind = symbol_kind_name(kind_num as u32);

        // DocumentSymbol has "range"/"selectionRange", SymbolInformation has "location"
        let location = if let Some(loc) = item.get("location") {
            parse_single_location(loc)
        } else {
            item.get("selectionRange")
                .or_else(|| item.get("range"))
                .map(|range| LspLocation {
                    file: String::new(),
                    line: range
                        .get("start")
                        .and_then(|s| s.get("line"))
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as u32
                        + 1,
                    character: range
                        .get("start")
                        .and_then(|s| s.get("character"))
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as u32
                        + 1,
                    end_line: range
                        .get("end")
                        .and_then(|e| e.get("line"))
                        .and_then(|v| v.as_u64())
                        .map(|v| v as u32 + 1),
                    end_character: range
                        .get("end")
                        .and_then(|e| e.get("character"))
                        .and_then(|v| v.as_u64())
                        .map(|v| v as u32 + 1),
                })
        };

        if let Some(loc) = location {
            out.push(LspSymbol {
                name,
                kind,
                location: loc,
            });
        }

        // Recurse into children (DocumentSymbol)
        if let Some(children) = item.get("children") {
            let child_symbols = parse_symbols(children);
            out.extend(child_symbols);
        }
    }
    out
}

fn parse_single_location(loc: &Value) -> Option<LspLocation> {
    let uri = loc.get("uri")?.as_str()?;
    let range = loc.get("range")?;
    let start = range.get("start")?;
    let end = range.get("end");

    Some(LspLocation {
        file: uri_to_path(uri),
        line: start.get("line").and_then(|v| v.as_u64()).unwrap_or(0) as u32 + 1,
        character: start.get("character").and_then(|v| v.as_u64()).unwrap_or(0) as u32 + 1,
        end_line: end
            .and_then(|e| e.get("line"))
            .and_then(|v| v.as_u64())
            .map(|v| v as u32 + 1),
        end_character: end
            .and_then(|e| e.get("character"))
            .and_then(|v| v.as_u64())
            .map(|v| v as u32 + 1),
    })
}

fn uri_to_path(uri: &str) -> String {
    uri.strip_prefix("file://").unwrap_or(uri).to_string()
}

fn symbol_kind_name(kind: u32) -> String {
    match kind {
        1 => "File",
        2 => "Module",
        3 => "Namespace",
        4 => "Package",
        5 => "Class",
        6 => "Method",
        7 => "Property",
        8 => "Field",
        9 => "Constructor",
        10 => "Enum",
        11 => "Interface",
        12 => "Function",
        13 => "Variable",
        14 => "Constant",
        15 => "String",
        16 => "Number",
        17 => "Boolean",
        18 => "Array",
        19 => "Object",
        20 => "Key",
        21 => "Null",
        22 => "EnumMember",
        23 => "Struct",
        24 => "Event",
        25 => "Operator",
        26 => "TypeParameter",
        _ => "Unknown",
    }
    .to_string()
}

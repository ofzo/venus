use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::time::Duration;
use venus_core::tool::{Tool, ToolContext, ToolResult};

const DEFAULT_MAX_LENGTH: usize = 50000;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

pub struct WebFetchTool;

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "WebFetch"
    }

    fn description(&self) -> &str {
        "Fetches a URL via HTTP GET and returns the content as text. HTML pages are converted to plain text."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch"
                },
                "max_length": {
                    "type": "number",
                    "description": "Maximum number of characters to return (default: 50000)"
                }
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult> {
        let url = input
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'url' parameter"))?;

        let max_length = input
            .get("max_length")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(DEFAULT_MAX_LENGTH);

        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|e| anyhow::anyhow!("failed to create HTTP client: {}", e))?;

        let response = client
            .get(url)
            .header("User-Agent", "Venus/0.1")
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    anyhow::anyhow!("request timed out after {}s", REQUEST_TIMEOUT.as_secs())
                } else {
                    anyhow::anyhow!("network error: {}", e)
                }
            })?;

        let status = response.status();
        if !status.is_success() {
            return Ok(ToolResult::error(format!(
                "HTTP {} {}",
                status.as_u16(),
                status.canonical_reason().unwrap_or("Unknown")
            )));
        }

        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_lowercase();

        let body = response
            .text()
            .await
            .map_err(|e| anyhow::anyhow!("failed to read response body: {}", e))?;

        let text = if content_type.contains("text/html") {
            strip_html_tags(&body)
        } else {
            body
        };

        let truncated = text.len() > max_length;
        let mut result: String = text.chars().take(max_length).collect();
        if truncated {
            result.push_str("\n\n... (content truncated)");
        }

        Ok(ToolResult::text(result))
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn format_for_display(&self, input: &Value) -> String {
        let url = input.get("url").and_then(|v| v.as_str()).unwrap_or("?");
        format!("fetch: {}", url)
    }
}

/// Strip HTML tags from a string, converting it to plain text.
///
/// Skips content inside `<script>` and `<style>` blocks, and decodes
/// common HTML entities.
fn strip_html_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len() / 2);
    let mut in_tag = false;
    let mut tag_name = String::new();
    let mut capturing_tag_name = false;
    let mut in_script = false;
    let mut in_style = false;
    let chars: Vec<char> = html.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        let c = chars[i];
        match c {
            '<' => {
                in_tag = true;
                tag_name.clear();
                capturing_tag_name = true;
            }
            '>' if in_tag => {
                in_tag = false;
                capturing_tag_name = false;
                let tag_lower = tag_name.to_lowercase();
                if tag_lower == "script" {
                    in_script = true;
                } else if tag_lower == "/script" {
                    in_script = false;
                } else if tag_lower == "style" {
                    in_style = true;
                } else if tag_lower == "/style" {
                    in_style = false;
                } else if tag_lower == "br" || tag_lower == "br/" || tag_lower == "p" || tag_lower == "/p"
                    || tag_lower == "div" || tag_lower == "/div"
                    || tag_lower == "li" || tag_lower == "/li"
                    || tag_lower.starts_with("h1") || tag_lower.starts_with("h2")
                    || tag_lower.starts_with("h3") || tag_lower.starts_with("h4")
                    || tag_lower.starts_with("/h")
                {
                    result.push('\n');
                }
            }
            _ if in_tag => {
                if capturing_tag_name {
                    if c.is_whitespace() || c == '/' && tag_name.is_empty() {
                        if !tag_name.is_empty() {
                            capturing_tag_name = false;
                        } else if c == '/' {
                            tag_name.push(c);
                        }
                    } else {
                        tag_name.push(c);
                    }
                }
            }
            _ if !in_script && !in_style => {
                result.push(c);
            }
            _ => {}
        }
        i += 1;
    }

    let decoded = decode_html_entities(&result);
    collapse_whitespace(&decoded)
}

/// Decode common HTML entities.
fn decode_html_entities(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
}

/// Collapse runs of whitespace into single spaces, and multiple newlines into
/// at most two consecutive newlines.
fn collapse_whitespace(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut newline_count = 0;
    let mut last_was_space = false;

    for c in text.chars() {
        if c == '\n' || c == '\r' {
            if c == '\r' {
                continue;
            }
            newline_count += 1;
            last_was_space = false;
            if newline_count <= 2 {
                result.push('\n');
            }
        } else if c.is_whitespace() {
            newline_count = 0;
            if !last_was_space {
                result.push(' ');
                last_was_space = true;
            }
        } else {
            newline_count = 0;
            last_was_space = false;
            result.push(c);
        }
    }

    result.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_simple_html() {
        let html = "<p>Hello, <b>world</b>!</p>";
        let result = strip_html_tags(html);
        assert!(result.contains("Hello,"));
        assert!(result.contains("world"));
        assert!(result.contains("!"));
        assert!(!result.contains("<"));
    }

    #[test]
    fn test_strip_script_and_style() {
        let html = "<p>Visible</p><script>var x = 1;</script><style>body{color:red}</style><p>Also visible</p>";
        let result = strip_html_tags(html);
        assert!(result.contains("Visible"));
        assert!(result.contains("Also visible"));
        assert!(!result.contains("var x"));
        assert!(!result.contains("color:red"));
    }

    #[test]
    fn test_decode_entities() {
        let html = "<p>&amp; &lt; &gt; &quot; &#39; &nbsp;</p>";
        let result = strip_html_tags(html);
        assert!(result.contains("&"));
        assert!(result.contains("<"));
        assert!(result.contains(">"));
        assert!(result.contains("\""));
        assert!(result.contains("'"));
    }

    #[test]
    fn test_collapse_whitespace() {
        let input = "hello     world\n\n\n\n\ntest";
        let result = collapse_whitespace(input);
        assert_eq!(result, "hello world\n\ntest");
    }

    #[test]
    fn test_strip_empty_html() {
        let result = strip_html_tags("");
        assert_eq!(result, "");
    }

    #[test]
    fn test_plain_text_passthrough() {
        let text = "Just plain text, no HTML here.";
        let result = strip_html_tags(text);
        assert_eq!(result, "Just plain text, no HTML here.");
    }

    #[test]
    fn test_br_and_block_tags_produce_newlines() {
        let html = "Line 1<br>Line 2<br/>Line 3";
        let result = strip_html_tags(html);
        assert!(result.contains("Line 1"));
        assert!(result.contains("Line 2"));
        assert!(result.contains("Line 3"));
    }
}

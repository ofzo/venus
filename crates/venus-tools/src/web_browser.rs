use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use venus_core::tool::{Tool, ToolContext, ToolResult};

pub struct WebBrowserTool;

#[async_trait]
impl Tool for WebBrowserTool {
    fn name(&self) -> &str {
        "WebBrowser"
    }

    fn description(&self) -> &str {
        "Control a web browser to navigate pages, click elements, fill forms, and extract content. \
         Uses HTTP requests for page fetching and basic interaction."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["navigate", "get_text", "get_links", "get_forms", "screenshot"],
                    "description": "Action to perform"
                },
                "url": {
                    "type": "string",
                    "description": "URL to navigate to (required for 'navigate' action)"
                },
                "selector": {
                    "type": "string",
                    "description": "CSS selector for targeting elements (optional)"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult> {
        let action = input
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'action' parameter"))?;

        match action {
            "navigate" => {
                let url = input
                    .get("url")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("missing 'url' for navigate"))?;

                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(30))
                    .redirect(reqwest::redirect::Policy::limited(5))
                    .build()?;

                let response = client.get(url).send().await?;
                let status = response.status();
                let headers: std::collections::HashMap<String, String> = response
                    .headers()
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
                    .collect();
                let body = response.text().await?;

                let content_type = headers
                    .get("content-type")
                    .cloned()
                    .unwrap_or_default();

                let mut result = format!(
                    "Status: {}\nURL: {}\nContent-Type: {}\nContent-Length: {} bytes",
                    status, url, content_type, body.len()
                );

                // Extract text from HTML
                if content_type.contains("text/html") {
                    let text = html_to_text(&body);
                    result.push_str(&format!("\n\nText content:\n{}", &text[..text.len().min(5000)]));
                }

                Ok(ToolResult::text(result))
            }
            "get_text" => {
                let url = input
                    .get("url")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("missing 'url' for get_text"))?;

                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(30))
                    .build()?;

                let body = client.get(url).send().await?.text().await?;
                let text = html_to_text(&body);

                Ok(ToolResult::text(text))
            }
            "get_links" => {
                let url = input
                    .get("url")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("missing 'url' for get_links"))?;

                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(30))
                    .build()?;

                let body = client.get(url).send().await?.text().await?;
                let links = extract_links(&body);

                let mut result = format!("Links found: {}\n", links.len());
                for (i, (text, href)) in links.iter().take(50).enumerate() {
                    result.push_str(&format!("  {}. {} -> {}\n", i + 1, text, href));
                }

                Ok(ToolResult::text(result))
            }
            "get_forms" => {
                let url = input
                    .get("url")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("missing 'url' for get_forms"))?;

                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(30))
                    .build()?;

                let body = client.get(url).send().await?.text().await?;
                let forms = extract_forms(&body);

                let mut result = format!("Forms found: {}\n", forms.len());
                for (i, form) in forms.iter().take(10).enumerate() {
                    result.push_str(&format!("  {}. {}\n", i + 1, form));
                }

                Ok(ToolResult::text(result))
            }
            "screenshot" => {
                Ok(ToolResult::text(
                    "Screenshot not available in HTTP-only mode. \
                     Use 'navigate' to fetch page content or 'get_text' to extract text.".to_string()
                ))
            }
            _ => Ok(ToolResult::error(format!("Unknown action: {}", action))),
        }
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn format_for_display(&self, input: &Value) -> String {
        let action = input.get("action").and_then(|v| v.as_str()).unwrap_or("?");
        let url = input.get("url").and_then(|v| v.as_str()).unwrap_or("");
        format!("WebBrowser({}): {}", action, url)
    }
}

/// Convert HTML to plain text by stripping tags.
fn html_to_text(html: &str) -> String {
    let mut result = String::new();
    let mut in_tag = false;
    let mut in_script = false;
    let mut in_style = false;
    let mut tag_buf = String::new();

    for c in html.chars() {
        match c {
            '<' => {
                in_tag = true;
                tag_buf.clear();
                tag_buf.push(c);
            }
            '>' if in_tag => {
                tag_buf.push(c);
                let lower = tag_buf.to_lowercase();
                if lower.starts_with("<script") {
                    in_script = true;
                } else if lower.starts_with("<style") {
                    in_style = true;
                } else if lower.starts_with("</script") {
                    in_script = false;
                } else if lower.starts_with("</style") {
                    in_style = false;
                }
                in_tag = false;
                if !in_script && !in_style {
                    result.push(' ');
                }
            }
            _ if in_tag => {
                tag_buf.push(c);
            }
            _ if !in_script && !in_style => {
                result.push(c);
            }
            _ => {}
        }
    }

    // Collapse whitespace
    let mut collapsed = String::new();
    let mut last_was_space = false;
    for c in result.chars() {
        if c.is_whitespace() {
            if !last_was_space {
                collapsed.push(' ');
                last_was_space = true;
            }
        } else {
            collapsed.push(c);
            last_was_space = false;
        }
    }

    collapsed.trim().to_string()
}

/// Extract links from HTML.
fn extract_links(html: &str) -> Vec<(String, String)> {
    let mut links = Vec::new();
    let mut pos = 0;

    while let Some(start) = html[pos..].find("<a ") {
        let tag_start = pos + start;
        if let Some(end) = html[tag_start..].find('>') {
            let tag = &html[tag_start..tag_start + end + 1];

            // Extract href
            let href = if let Some(h_start) = tag.find("href=\"") {
                let h_val_start = h_start + 6;
                if let Some(h_end) = tag[h_val_start..].find('"') {
                    tag[h_val_start..h_val_start + h_end].to_string()
                } else {
                    String::new()
                }
            } else {
                String::new()
            };

            // Extract text (between <a> and </a>)
            let text_start = tag_start + end + 1;
            let text = if let Some(text_end) = html[text_start..].find("</a>") {
                html_to_text(&html[text_start..text_start + text_end])
            } else {
                String::new()
            };

            if !href.is_empty() && !href.starts_with('#') {
                links.push((text, href));
            }

            pos = tag_start + end + 1;
        } else {
            break;
        }
    }

    links
}

/// Extract form information from HTML.
fn extract_forms(html: &str) -> Vec<String> {
    let mut forms = Vec::new();
    let mut pos = 0;

    while let Some(start) = html[pos..].find("<form") {
        let tag_start = pos + start;
        if let Some(end) = html[tag_start..].find('>') {
            let tag = &html[tag_start..tag_start + end + 1];

            let action = if let Some(a_start) = tag.find("action=\"") {
                let a_val_start = a_start + 8;
                if let Some(a_end) = tag[a_val_start..].find('"') {
                    tag[a_val_start..a_val_start + a_end].to_string()
                } else {
                    String::new()
                }
            } else {
                "(no action)".to_string()
            };

            let method = if let Some(m_start) = tag.find("method=\"") {
                let m_val_start = m_start + 8;
                if let Some(m_end) = tag[m_val_start..].find('"') {
                    tag[m_val_start..m_val_start + m_end].to_uppercase()
                } else {
                    "GET".to_string()
                }
            } else {
                "GET".to_string()
            };

            forms.push(format!("{} {} ", method, action));

            pos = tag_start + end + 1;
        } else {
            break;
        }
    }

    forms
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_html_to_text() {
        let html = "<html><body><h1>Hello</h1><p>World</p></body></html>";
        let text = html_to_text(html);
        assert!(text.contains("Hello"));
        assert!(text.contains("World"));
        assert!(!text.contains("<"));
    }

    #[test]
    fn test_html_to_text_strips_script() {
        let html = "<html><body><script>alert('xss')</script><p>Safe</p></body></html>";
        let text = html_to_text(html);
        assert!(text.contains("Safe"));
        assert!(!text.contains("alert"));
    }

    #[test]
    fn test_extract_links() {
        let html = "<html><body>\
            <a href=\"https://example.com\">Example</a>\
            <a href=\"/local\">Local</a>\
            <a href=\"#anchor\">Anchor</a>\
        </body></html>";
        let links = extract_links(html);
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].1, "https://example.com");
        assert_eq!(links[1].1, "/local");
    }

    #[test]
    fn test_extract_forms() {
        let html = r#"<html><body>
            <form action="/login" method="post">
                <input type="text" name="user">
            </form>
            <form action="/search">
                <input type="text" name="q">
            </form>
        </body></html>"#;
        let forms = extract_forms(html);
        assert_eq!(forms.len(), 2);
        assert!(forms[0].contains("POST"));
        assert!(forms[0].contains("/login"));
    }
}

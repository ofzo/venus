use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use venus_core::tool::{Tool, ToolContext, ToolResult};

pub struct AskUserTool;

#[async_trait]
impl Tool for AskUserTool {
    fn name(&self) -> &str {
        "AskUserQuestion"
    }

    fn description(&self) -> &str {
        "Ask the user questions during execution to gather preferences, clarify requirements, or get decisions."
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "questions": {
                    "type": "array",
                    "description": "Questions to ask (1-4 questions)",
                    "items": {
                        "type": "object",
                        "properties": {
                            "question": {
                                "type": "string",
                                "description": "The question text"
                            },
                            "header": {
                                "type": "string",
                                "description": "Short label (max 12 chars)"
                            },
                            "options": {
                                "type": "array",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "label": { "type": "string" },
                                        "description": { "type": "string" }
                                    },
                                    "required": ["label", "description"]
                                }
                            },
                            "multiSelect": {
                                "type": "boolean",
                                "default": false
                            }
                        },
                        "required": ["question", "header", "options", "multiSelect"]
                    }
                }
            },
            "required": ["questions"]
        })
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult> {
        let questions = input
            .get("questions")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow::anyhow!("missing 'questions'"))?;

        if questions.is_empty() || questions.len() > 4 {
            return Ok(ToolResult::error(
                "questions array must contain 1-4 items",
            ));
        }

        let mut answers: serde_json::Map<String, Value> = serde_json::Map::new();

        for q in questions {
            let question = q.get("question").and_then(|v| v.as_str()).unwrap_or("?");
            let header = q.get("header").and_then(|v| v.as_str()).unwrap_or("");
            let multi = q
                .get("multiSelect")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let options = q.get("options").and_then(|v| v.as_array());

            // Display question with box-drawing UI
            eprintln!();
            eprintln!("  \x1b[1;36m┌ {}\x1b[0m", header);
            eprintln!("  \x1b[36m│\x1b[0m");
            eprintln!("  \x1b[36m│\x1b[0m \x1b[1m{}\x1b[0m", question);
            eprintln!("  \x1b[36m│\x1b[0m");

            let opts: Vec<String> = if let Some(opts) = options {
                for (i, opt) in opts.iter().enumerate() {
                    let label = opt.get("label").and_then(|v| v.as_str()).unwrap_or("?");
                    let desc = opt
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    eprintln!(
                        "  \x1b[36m│\x1b[0m   \x1b[33m{}.\x1b[0m {}",
                        i + 1,
                        label
                    );
                    if !desc.is_empty() {
                        eprintln!("  \x1b[36m│\x1b[0m      \x1b[2m{}\x1b[0m", desc);
                    }
                }
                // Add "Other" option
                let other_idx = opts.len() + 1;
                eprintln!(
                    "  \x1b[36m│\x1b[0m   \x1b[33m{}.\x1b[0m Other (custom input)",
                    other_idx
                );

                opts.iter()
                    .map(|o| {
                        o.get("label")
                            .and_then(|v| v.as_str())
                            .unwrap_or("?")
                            .to_string()
                    })
                    .collect()
            } else {
                Vec::new()
            };

            eprintln!("  \x1b[36m│\x1b[0m");

            // Read user input
            if multi {
                eprint!("  \x1b[36m└\x1b[0m Enter choices (comma-separated): ");
            } else {
                eprint!(
                    "  \x1b[36m└\x1b[0m Enter choice (1-{}): ",
                    opts.len() + 1
                );
            }
            std::io::Write::flush(&mut std::io::stderr()).ok();

            let mut input_line = String::new();
            std::io::BufRead::read_line(&mut std::io::stdin().lock(), &mut input_line)?;
            let input_line = input_line.trim().to_string();

            let answer = if multi {
                let selections: Vec<String> = input_line
                    .split(',')
                    .filter_map(|s| {
                        let s = s.trim();
                        if let Ok(idx) = s.parse::<usize>() {
                            if idx >= 1 && idx <= opts.len() {
                                Some(opts[idx - 1].clone())
                            } else if idx == opts.len() + 1 {
                                // "Other" selected in multi-select; prompt for custom input
                                eprint!("  Enter custom response: ");
                                std::io::Write::flush(&mut std::io::stderr()).ok();
                                let mut custom = String::new();
                                std::io::BufRead::read_line(
                                    &mut std::io::stdin().lock(),
                                    &mut custom,
                                )
                                .ok();
                                Some(custom.trim().to_string())
                            } else {
                                None
                            }
                        } else {
                            // Treat non-numeric input as direct text
                            Some(s.to_string())
                        }
                    })
                    .collect();
                Value::Array(selections.into_iter().map(Value::String).collect())
            } else if let Ok(idx) = input_line.parse::<usize>() {
                if idx >= 1 && idx <= opts.len() {
                    Value::String(opts[idx - 1].clone())
                } else if idx == opts.len() + 1 {
                    // "Other" - prompt for custom input
                    eprint!("  Enter custom response: ");
                    std::io::Write::flush(&mut std::io::stderr()).ok();
                    let mut custom = String::new();
                    std::io::BufRead::read_line(&mut std::io::stdin().lock(), &mut custom)?;
                    Value::String(custom.trim().to_string())
                } else {
                    Value::String(input_line)
                }
            } else {
                // Treat as direct text input
                Value::String(input_line)
            };

            answers.insert(question.to_string(), answer);
        }

        let result = serde_json::json!({ "answers": answers });
        Ok(ToolResult::text(serde_json::to_string_pretty(&result)?))
    }

    fn format_for_display(&self, input: &Value) -> String {
        let count = input
            .get("questions")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        format!("Asking {} question(s)", count)
    }
}

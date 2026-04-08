use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::atomic::Ordering;
use venus_core::tool::{Tool, ToolContext, ToolResult};

pub struct EnterPlanModeTool;

#[async_trait]
impl Tool for EnterPlanModeTool {
    fn name(&self) -> &str {
        "EnterPlanMode"
    }

    fn description(&self) -> &str {
        "Enter plan mode to design an implementation approach before writing code. In plan mode, explore the codebase and create a detailed plan. Use ExitPlanMode when your plan is ready for review."
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn execute(&self, _input: Value, ctx: &ToolContext) -> Result<ToolResult> {
        if ctx.plan_mode.load(Ordering::Relaxed) {
            return Ok(ToolResult::error("Already in plan mode."));
        }

        ctx.plan_mode.store(true, Ordering::Relaxed);

        Ok(ToolResult::text(
            "Entered plan mode. Explore the codebase and design your implementation approach. \
             Use read-only tools (Read, Glob, Grep) to investigate. \
             Do not write code yet. Use ExitPlanMode when your plan is ready for review."
        ))
    }
}

pub struct ExitPlanModeTool;

#[async_trait]
impl Tool for ExitPlanModeTool {
    fn name(&self) -> &str {
        "ExitPlanMode"
    }

    fn description(&self) -> &str {
        "Exit plan mode after finishing your implementation plan. Present the plan for user approval before proceeding."
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn execute(&self, _input: Value, ctx: &ToolContext) -> Result<ToolResult> {
        if !ctx.plan_mode.load(Ordering::Relaxed) {
            return Ok(ToolResult::error("Not currently in plan mode."));
        }

        ctx.plan_mode.store(false, Ordering::Relaxed);

        Ok(ToolResult::text(
            "Exited plan mode. Present your plan to the user for approval before implementing."
        ))
    }
}

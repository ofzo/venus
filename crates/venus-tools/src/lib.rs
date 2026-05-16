pub mod agent;
pub mod ask_user;
pub mod bash;
pub mod brief;
pub mod config_tool;
pub mod cron_create;
pub mod cron_delete;
pub mod cron_list;
pub mod file_edit;
pub mod file_read;
pub mod file_write;
pub mod glob;
pub mod grep;
pub mod lsp;
pub mod mcp_list_resources;
pub mod mcp_read_resource;
pub mod memory_forget;
pub mod memory_read;
pub mod memory_write;
pub mod monitor;
pub mod notebook_edit;
pub mod plan;
pub mod plugin_tool;
pub mod repl_tool;
pub mod send_message;
pub mod skill;
pub mod snip;
pub mod synthetic_output;
pub mod task_create;
pub mod task_get;
#[cfg(test)]
pub mod test_helpers;
pub mod todo;
pub mod task_list;
pub mod task_output;
pub mod task_stop;
pub mod task_update;
pub mod tool_search;
pub mod web_fetch;
pub mod web_search;
pub mod workflow;
pub mod worktree;

use venus_core::tool::Tool;

pub fn all_tools() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(bash::BashTool),
        Box::new(file_read::FileReadTool),
        Box::new(file_write::FileWriteTool),
        Box::new(file_edit::FileEditTool),
        Box::new(glob::GlobTool),
        Box::new(grep::GrepTool),
        Box::new(lsp::LspTool::new()),
        Box::new(web_fetch::WebFetchTool),
        Box::new(web_search::WebSearchTool),
        Box::new(notebook_edit::NotebookEditTool),
        Box::new(send_message::SendMessageTool),
        Box::new(tool_search::ToolSearchTool),
        Box::new(mcp_list_resources::ListMcpResourcesTool),
        Box::new(mcp_read_resource::ReadMcpResourceTool),
        Box::new(synthetic_output::SyntheticOutputTool),
        Box::new(repl_tool::REPLTool),
        Box::new(snip::SnipTool),
        Box::new(monitor::MonitorTool),
        Box::new(workflow::WorkflowTool),
        Box::new(task_create::TaskCreateTool),
        Box::new(task_update::TaskUpdateTool),
        Box::new(task_get::TaskGetTool),
        Box::new(task_list::TaskListTool),
        Box::new(ask_user::AskUserTool),
        Box::new(plan::EnterPlanModeTool),
        Box::new(plan::ExitPlanModeTool),
        Box::new(worktree::EnterWorktreeTool),
        Box::new(worktree::ExitWorktreeTool),
        Box::new(agent::AgentTool),
        Box::new(memory_write::MemoryWriteTool),
        Box::new(memory_read::MemoryReadTool),
        Box::new(memory_forget::MemoryForgetTool),
        Box::new(task_output::TaskOutputTool),
        Box::new(task_stop::TaskStopTool),
        Box::new(cron_create::CronCreateTool),
        Box::new(cron_delete::CronDeleteTool),
        Box::new(cron_list::CronListTool),
        Box::new(todo::TodoWriteTool),
        Box::new(config_tool::ConfigTool),
        Box::new(brief::BriefTool),
    ]
}

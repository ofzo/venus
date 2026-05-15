pub mod agent;
pub mod ask_user;
pub mod bash;
pub mod cron_create;
pub mod cron_delete;
pub mod cron_list;
pub mod file_edit;
pub mod file_read;
pub mod file_write;
pub mod glob;
pub mod grep;
pub mod lsp;
pub mod memory_forget;
pub mod memory_read;
pub mod memory_write;
pub mod plan;
pub mod plugin_tool;
pub mod skill;
pub mod task_create;
pub mod task_get;
pub mod task_list;
pub mod task_output;
pub mod task_stop;
pub mod task_update;
pub mod web_fetch;
pub mod web_search;
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
    ]
}

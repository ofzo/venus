pub mod ask_user;
pub mod bash;
pub mod file_edit;
pub mod file_read;
pub mod file_write;
pub mod glob;
pub mod grep;
pub mod task_create;
pub mod task_get;
pub mod task_list;
pub mod task_update;
pub mod web_fetch;
pub mod web_search;

use venus_core::tool::Tool;

pub fn all_tools() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(bash::BashTool),
        Box::new(file_read::FileReadTool),
        Box::new(file_write::FileWriteTool),
        Box::new(file_edit::FileEditTool),
        Box::new(glob::GlobTool),
        Box::new(grep::GrepTool),
        Box::new(web_fetch::WebFetchTool),
        Box::new(web_search::WebSearchTool),
        Box::new(task_create::TaskCreateTool),
        Box::new(task_update::TaskUpdateTool),
        Box::new(task_get::TaskGetTool),
        Box::new(task_list::TaskListTool),
        Box::new(ask_user::AskUserTool),
    ]
}

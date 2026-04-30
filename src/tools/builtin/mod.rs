//! Built-in tools that come with the agent.

mod conversation_history;
mod echo;
pub mod extension_tools;
mod file;
mod graph_memory;
mod http;
mod job;
mod json;
pub mod memory;
pub mod path_utils;
pub mod routine;
pub mod sandbox;
pub mod secrets_tools;
pub(crate) mod shell;
pub mod skill_tools;
mod time;
mod tool_info;

pub use conversation_history::{ReadConversationContextTool, SearchConversationHistoryTool};
pub use echo::EchoTool;
pub use extension_tools::{
    ExtensionInfoTool, ToolActivateTool, ToolAuthTool, ToolInstallTool, ToolListTool,
    ToolRemoveTool, ToolSearchTool, ToolUpgradeTool,
};
pub use file::{ApplyPatchTool, ListDirTool, MoveFileTool, ReadFileTool, WriteFileTool};
pub use graph_memory::{
    AddAliasTool, CreateMemoryTool, DeleteMemoryTool, ExplainMemoryRecallTool, GetWorkingMemoryTool,
    ManageBootTool, ManageTriggersTool, ProcessUtteranceTool, ReadMemoryTool, SearchMemoryTool,
    SessionStartTool, UpdateMemoryTool,
};
pub use http::HttpTool;
pub use job::{
    CancelJobTool, CreateJobTool, JobEventsTool, JobPromptTool, JobStatusTool, ListJobsTool,
    SchedulerSlot,
};
pub use json::JsonTool;
pub use memory::{
    BootstrapCompleteTool, WorkspaceApplyPatchTool, WorkspaceBaselineSetTool,
    WorkspaceCheckpointCreateTool, WorkspaceCheckpointListTool, WorkspaceDeleteTool,
    WorkspaceDeleteTreeTool, WorkspaceDiffTool, WorkspaceHistoryTool, WorkspaceMoveTool,
    WorkspaceReadTool, WorkspaceRefreshTool, WorkspaceRestoreTool, WorkspaceSearchTool,
    WorkspaceTreeTool, WorkspaceWriteTool,
};
pub use routine::{
    EventEmitTool, RoutineCreateTool, RoutineDeleteTool, RoutineFireTool, RoutineHistoryTool,
    RoutineListTool, RoutineUpdateTool,
};
pub use secrets_tools::{SecretDeleteTool, SecretListTool};
pub use shell::ShellTool;
pub use skill_tools::{SkillInstallTool, SkillListTool, SkillRemoveTool, SkillSearchTool};
pub use time::TimeTool;
pub use tool_info::ToolInfoTool;
mod html_converter;
pub mod image_analyze;
pub mod image_edit;
pub mod image_gen;

pub use html_converter::convert_html_to_markdown;
pub use image_analyze::ImageAnalyzeTool;
pub use image_edit::ImageEditTool;
pub use image_gen::ImageGenerateTool;

/// Detect image media type from file extension via `mime_guess`.
/// Falls back to `image/jpeg` for unrecognized or non-image extensions.
pub(crate) fn media_type_from_path(path: &str) -> String {
    mime_guess::from_path(path)
        .first_raw()
        .filter(|m| m.starts_with("image/"))
        .unwrap_or("image/jpeg")
        .to_string()
}

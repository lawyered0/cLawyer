//! Built-in tools that come with the agent.

pub mod canlii;
pub mod corporate_compliance;
pub mod court_deadline;
mod echo;
pub mod extension_tools;
mod file;
mod http;
mod job;
mod json;
mod memory;
pub mod ontario_forms;
pub mod ontario_limitation;
pub mod routine;
pub(crate) mod shell;
pub mod skill_tools;
mod time;
pub mod trust_compliance;

pub use canlii::CanLiiSearchTool;
pub use corporate_compliance::CorporateComplianceCheckerTool;
pub use court_deadline::{CourtDeadlineCalculatorTool, ListCourtRulesTool};
pub use echo::EchoTool;
pub use extension_tools::{
    ToolActivateTool, ToolAuthTool, ToolInstallTool, ToolListTool, ToolRemoveTool, ToolSearchTool,
};
pub use file::{ApplyPatchTool, ListDirTool, ReadFileTool, WriteFileTool};
pub use http::HttpTool;
pub use job::{
    CancelJobTool, CreateJobTool, JobEventsTool, JobPromptTool, JobStatusTool, ListJobsTool,
    PromptQueue,
};
pub use json::JsonTool;
pub use memory::{MemoryReadTool, MemorySearchTool, MemoryTreeTool, MemoryWriteTool};
pub use ontario_forms::OntarioCourtFormTool;
pub use ontario_limitation::OntarioLimitationCalculatorTool;
pub use routine::{
    RoutineCreateTool, RoutineDeleteTool, RoutineHistoryTool, RoutineListTool, RoutineUpdateTool,
};
pub use shell::ShellTool;
pub use skill_tools::{SkillInstallTool, SkillListTool, SkillRemoveTool, SkillSearchTool};
pub use time::TimeTool;
pub use trust_compliance::TrustComplianceCheckerTool;

mod html_converter;

pub use html_converter::convert_html_to_markdown;

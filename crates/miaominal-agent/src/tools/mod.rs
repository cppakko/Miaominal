mod apply_patch;
mod approval;
mod glob;
mod grep;
mod job;
mod list;
mod read;
mod rig_adapter;
mod run_shell;
mod web_fetch;
mod web_search;
mod workspace_info;

pub use apply_patch::apply_patch;
pub use approval::approval;
pub use glob::glob;
pub use grep::grep;
pub use job::{poll_job, start_job, stop_job};
pub use list::list;
pub use read::read;
pub use rig_adapter::AgentToolSet;
pub use run_shell::run_shell;
pub use web_fetch::web_fetch;
pub use web_search::web_search;
pub use workspace_info::workspace_info;

pub use list::{ListEntry, ListEntryType};

pub const TOOL_NAMES: &[&str] = &[
    "workspace_info",
    "read",
    "list",
    "glob",
    "grep",
    "apply_patch",
    "run_shell",
    "start_job",
    "poll_job",
    "stop_job",
    "web_search",
    "web_fetch",
    "ask_user",
    "approval",
];

pub fn tool_description(name: &str) -> &'static str {
    match name {
        "workspace_info" => {
            "Return profile workspace metadata, shell, platform, sensitive paths, and remote capabilities."
        }
        "read" => {
            "Read a line range from a remote profile workspace file with byte caps and truncation metadata."
        }
        "list" => "List entries in a remote profile workspace directory.",
        "glob" => "Find remote workspace paths by glob-style pattern under an explicit root.",
        "grep" => "Search remote workspace files with rg when available and grep/find fallback.",
        "apply_patch" => {
            "Create, edit, or delete files by applying an approved unified diff patch in the remote profile workspace. This is the only file-writing/editing tool; do not call `write`, `edit`, or `replace`."
        }
        "run_shell" => {
            "Run an approved non-interactive shell command in the remote profile workspace."
        }
        "start_job" => "Start an approved long-running remote shell job.",
        "poll_job" => "Poll a remote shell job.",
        "stop_job" => "Stop a remote shell job.",
        "web_search" => "Search the web through the configured local provider.",
        "web_fetch" => "Fetch URL text locally with byte caps.",
        "ask_user" => "Ask the user for information or approval.",
        "approval" => "Record a user approval response.",
        _ => "Miaominal agent tool.",
    }
}

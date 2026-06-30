mod apply_patch;
mod approval;
mod glob;
mod grep;
mod job;
mod list;
mod patch_engine;
mod read;
mod rig_adapter;
mod run_shell;
mod web_fetch;
mod web_search;
mod windows;
mod workspace_info;

pub use apply_patch::apply_patch;
pub use approval::{approval, ask_user};
pub use glob::glob;
pub use grep::grep;
pub use job::{list_jobs, poll_job, start_job, stop_job};
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
    "list_jobs",
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
            "Return workspace metadata. The `shell` field is the actual exec-channel syntax for run_shell commands, not merely the SSH login/default shell."
        }
        "read" => {
            "Read a line range from a remote profile workspace file with byte caps, preserving visible content including trailing whitespace."
        }
        "list" => "List entries in a remote profile workspace directory.",
        "glob" => "Find remote workspace paths by glob-style pattern under an explicit root.",
        "grep" => "Search remote workspace files with rg when available and grep/find fallback.",
        "apply_patch" => {
            "Create, edit, or delete files by applying an approved unified diff patch in the remote profile workspace. This is the only file-writing/editing tool; do not call `write`, `edit`, or `replace`."
        }
        "run_shell" => {
            "Run an approved non-interactive command using exactly the syntax reported by workspace_info.shell. If shell is cmd, write CMD commands such as dir/type unless you explicitly launch powershell.exe."
        }
        "start_job" => {
            "Start an approved long-running remote shell job. Use only for commands that may block, stream, watch, serve, deploy, or run longer than a normal run_shell call; poll the returned job_id until completion."
        }
        "list_jobs" => "List known background jobs when a job_id was forgotten or before polling.",
        "poll_job" => {
            "Poll a remote background job by job_id and return structured status, exit code, stdout, and stderr."
        }
        "stop_job" => "Stop a remote shell job.",
        "web_search" => "Search the web through the configured local provider.",
        "web_fetch" => "Fetch URL text locally with byte caps.",
        "ask_user" => {
            "Ask the user a question with up to three suggested choices. The user can also enter a custom response."
        }
        "approval" => "Record a user approval response.",
        _ => "Miaominal agent tool.",
    }
}

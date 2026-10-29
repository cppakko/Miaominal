//! Tool set v2: specs, argument validation, the runtime interface used by
//! the engine, execution over the SSH channel and result shaping.

mod edit;
mod exec;
mod runtime;
pub mod shape;
pub mod specs;
mod terminal;

pub use exec::ChannelToolRuntime;
pub use runtime::{
    ProgressSender, ToolAssessment, ToolInvocation, ToolOutcome, ToolProgress, ToolRuntime,
};
pub use shape::{clean_output, collapse_carriage_returns, strip_ansi};
pub use specs::{ToolClass, ToolSetOptions, tool_class, tool_specs, validate_arguments};
pub use terminal::{
    TerminalContextSource, TerminalReadRequest, TerminalReadResult, TerminalSnapshot,
    refers_to_terminal, render_context,
};

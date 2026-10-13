#[path = "layout/chrome.rs"]
mod chrome;
pub(in crate::ui::shell) use chrome::ChromeAppViewExt;
#[path = "layout/sidebar.rs"]
pub(in crate::ui::shell) mod sidebar;
#[path = "layout/workspace.rs"]
pub(in crate::ui::shell) mod workspace;
#[path = "layout/workspace_panes.rs"]
mod workspace_panes;
pub(in crate::ui::shell) use workspace_panes::WorkspacePanesAppViewExt;
#[path = "layout/workspace_side_panel.rs"]
mod workspace_side_panel;
#[path = "layout/workspace_terminal_menu.rs"]
mod workspace_terminal_menu;
#[path = "layout/workspace_terminal_search.rs"]
mod workspace_terminal_search;

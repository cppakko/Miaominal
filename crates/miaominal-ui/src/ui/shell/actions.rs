#[path = "actions/forwarding.rs"]
mod forwarding;
#[path = "actions/forwarding_sync.rs"]
mod forwarding_sync;
#[path = "actions/keychain.rs"]
mod keychain;
#[path = "actions/nav.rs"]
mod nav;
#[path = "actions/notification.rs"]
mod notification;
#[path = "actions/onboarding.rs"]
mod onboarding;
#[path = "actions/profile.rs"]
mod profile;
#[path = "actions/profile_import.rs"]
mod profile_import;
#[path = "actions/profile_proxy_env.rs"]
mod profile_proxy_env;
#[path = "actions/search.rs"]
mod search;
#[path = "actions/session.rs"]
mod session;
#[path = "actions/session_agent.rs"]
mod session_agent;
#[path = "actions/session_sftp.rs"]
mod session_sftp;
#[path = "actions/session_sftp_edit.rs"]
mod session_sftp_edit;
#[path = "actions/session_sftp_sync.rs"]
mod session_sftp_sync;
#[path = "actions/session_sftp_transfer.rs"]
mod session_sftp_transfer;
#[path = "actions/session_terminal.rs"]
mod session_terminal;
#[path = "actions/settings.rs"]
mod settings;
#[path = "actions/snippets.rs"]
mod snippets;

pub(in crate::ui::shell) use notification::{ValidationFailure, ValidationNotificationKind};
pub(in crate::ui::shell) use session_agent::{PromptHistoryDirection, SessionAgentTargetCandidate};
pub(in crate::ui::shell) use settings::{
    ai_provider_kind_chat_supported, ai_provider_kind_label_key, ai_provider_select_options,
    web_search_endpoint_placeholder, web_search_provider_kind_label_key,
};

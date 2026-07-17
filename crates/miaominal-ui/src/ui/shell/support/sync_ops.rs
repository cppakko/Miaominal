use super::super::*;
use crate::ui::i18n;
use miaominal_sync::{SyncProvider, SyncStatus};

const SYNC_STATUS_ERROR_SUMMARY_MAX_CHARS: usize = 96;

fn sync_provider_label(provider: SyncProvider) -> String {
    match provider {
        SyncProvider::None => i18n::string("settings.sync.providers.none"),
        SyncProvider::GithubGist => i18n::string("settings.sync.providers.gist"),
        SyncProvider::WebDav => i18n::string("settings.sync.providers.webdav"),
    }
}

fn summarize_sync_error(error: &str) -> String {
    let normalized = error.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_with_ellipsis(&normalized, SYNC_STATUS_ERROR_SUMMARY_MAX_CHARS)
}

pub(in crate::ui::shell) fn sync_status_summary(status: &SyncStatus) -> String {
    match status {
        SyncStatus::Idle => i18n::string("settings.sync.status.state.idle"),
        SyncStatus::Syncing => i18n::string("settings.sync.status.state.syncing"),
        SyncStatus::RemoteBindingRequired { provider } => match provider {
            SyncProvider::GithubGist => {
                i18n::string("settings.sync.status.state.github_gist_binding_required")
            }
            _ => i18n::string_args(
                "settings.sync.status.state.remote_binding_required",
                &[("provider", &sync_provider_label(*provider))],
            ),
        },
        SyncStatus::Pulled { at } => {
            let timestamp = format_local_timestamp(Some(
                std::time::UNIX_EPOCH + std::time::Duration::from_secs(*at),
            ))
            .to_string();
            i18n::string_args(
                "settings.sync.status.state.pulled_at",
                &[("time", &timestamp)],
            )
        }
        SyncStatus::Pushed { at } => {
            let timestamp = format_local_timestamp(Some(
                std::time::UNIX_EPOCH + std::time::Duration::from_secs(*at),
            ))
            .to_string();
            i18n::string_args(
                "settings.sync.status.state.pushed_at",
                &[("time", &timestamp)],
            )
        }
        SyncStatus::PullRequired { remote_at } => {
            let timestamp = format_local_timestamp(Some(
                std::time::UNIX_EPOCH + std::time::Duration::from_secs(*remote_at),
            ))
            .to_string();
            i18n::string_args(
                "settings.sync.status.state.pull_required_at",
                &[("time", &timestamp)],
            )
        }
        SyncStatus::UpToDate { .. } => i18n::string("settings.sync.status.state.up_to_date"),
        SyncStatus::Error(error) => i18n::string_args(
            "settings.sync.status.state.error",
            &[("message", &summarize_sync_error(error))],
        ),
    }
}

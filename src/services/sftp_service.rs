use crate::domain::profile::SessionProfile;
use crate::infra::known_hosts_store::KnownHostsStore;
use crate::infra::sftp::{self, SftpCommandSender, SftpConnection, SftpEntry, TransferId};
use crate::secrets::SecretStore;
use anyhow::Result;
use std::path::{Path, PathBuf};
use tokio::runtime::Handle as TokioHandle;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlannedSftpUpload {
    pub local_path: PathBuf,
    pub remote_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlannedSftpDownload {
    pub remote_path: String,
    pub local_path: PathBuf,
}

#[derive(Clone)]
pub(crate) struct SftpService {
    runtime: TokioHandle,
    secrets: SecretStore,
    known_hosts: KnownHostsStore,
}

impl SftpService {
    pub(crate) fn new(
        runtime: TokioHandle,
        secrets: SecretStore,
        known_hosts: KnownHostsStore,
    ) -> Self {
        Self {
            runtime,
            secrets,
            known_hosts,
        }
    }

    pub(crate) fn start_session(
        &self,
        profile: SessionProfile,
        all_profiles: Vec<SessionProfile>,
    ) -> SftpConnection {
        sftp::start_session(
            &self.runtime,
            profile,
            all_profiles,
            self.secrets.clone(),
            self.known_hosts.clone(),
        )
    }

    pub(crate) fn display_local_path(path: &Path) -> String {
        let path = path.display().to_string();

        #[cfg(windows)]
        {
            if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
                return format!(r"\\{rest}");
            }
            if let Some(rest) = path.strip_prefix(r"\\?\") {
                return rest.to_string();
            }
        }

        path
    }

    pub(crate) fn remote_file_name(path: &str) -> String {
        let trimmed = path.trim_end_matches('/');
        if trimmed.is_empty() || trimmed == "/" {
            return String::new();
        }

        trimmed
            .rsplit_once('/')
            .map(|(_, name)| name.to_string())
            .unwrap_or_else(|| trimmed.to_string())
    }

    pub(crate) fn join_remote_path(base: &str, name: &str) -> String {
        let trimmed_name = name.trim_matches('/');
        if base.is_empty() || base == "." {
            return trimmed_name.to_string();
        }

        if base == "/" {
            return format!("/{trimmed_name}");
        }

        format!("{}/{}", base.trim_end_matches('/'), trimmed_name)
    }

    pub(crate) fn plan_uploads(
        local_paths: Vec<PathBuf>,
        remote_base: &str,
    ) -> Vec<PlannedSftpUpload> {
        local_paths
            .into_iter()
            .filter_map(|local_path| {
                let filename = local_path.file_name()?.to_string_lossy().into_owned();
                Some(PlannedSftpUpload {
                    remote_path: Self::join_remote_path(remote_base, &filename),
                    local_path,
                })
            })
            .collect()
    }

    pub(crate) fn count_remote_conflicts(
        uploads: &[PlannedSftpUpload],
        remote_entries: &[SftpEntry],
    ) -> usize {
        uploads
            .iter()
            .filter(|upload| {
                remote_entries
                    .iter()
                    .any(|entry| entry.path == upload.remote_path)
            })
            .count()
    }

    pub(crate) fn queue_uploads(
        commands: &SftpCommandSender,
        uploads: &[PlannedSftpUpload],
    ) -> Result<()> {
        for upload in uploads {
            commands.queue_upload(upload.local_path.clone(), upload.remote_path.clone())?;
        }
        Ok(())
    }

    pub(crate) fn queue_downloads(
        commands: &SftpCommandSender,
        downloads: &[PlannedSftpDownload],
    ) -> Result<()> {
        for download in downloads {
            commands.queue_download(download.remote_path.clone(), download.local_path.clone())?;
        }
        Ok(())
    }

    pub(crate) fn pause_transfer(
        commands: &SftpCommandSender,
        transfer_id: TransferId,
    ) -> Result<()> {
        commands.pause_transfer(transfer_id)
    }

    pub(crate) fn resume_transfer(
        commands: &SftpCommandSender,
        transfer_id: TransferId,
    ) -> Result<()> {
        commands.resume_transfer(transfer_id)
    }

    pub(crate) fn cancel_transfer(
        commands: &SftpCommandSender,
        transfer_id: TransferId,
    ) -> Result<()> {
        commands.cancel_transfer(transfer_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_remote_path_handles_root_and_relative_paths() {
        assert_eq!(SftpService::join_remote_path("/", "child"), "/child");
        assert_eq!(SftpService::join_remote_path(".", "child"), "child");
        assert_eq!(
            SftpService::join_remote_path("/tmp/base", "child"),
            "/tmp/base/child"
        );
    }

    #[test]
    fn remote_file_name_trims_trailing_separators() {
        assert_eq!(
            SftpService::remote_file_name("/tmp/archive.tar.gz"),
            "archive.tar.gz"
        );
        assert_eq!(SftpService::remote_file_name("/tmp/folder/"), "folder");
    }
}

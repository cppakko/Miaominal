use anyhow::Result;
use miaominal_core::profile::SessionProfile;
use miaominal_secrets::SecretStore;
use miaominal_sftp::{
    self as sftp, SftpCommandSender, SftpConnection, SftpEntry, SftpEntryKind, TransferId,
};
use miaominal_storage::known_hosts_store::KnownHostsStore;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tokio::runtime::Handle as TokioHandle;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedSftpUpload {
    pub local_path: PathBuf,
    pub remote_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedSftpDownload {
    pub remote_path: String,
    pub local_path: PathBuf,
}

#[derive(Clone)]
pub struct SftpService {
    runtime: TokioHandle,
    secrets: SecretStore,
    known_hosts: KnownHostsStore,
}

impl SftpService {
    pub fn new(runtime: TokioHandle, secrets: SecretStore, known_hosts: KnownHostsStore) -> Self {
        Self {
            runtime,
            secrets,
            known_hosts,
        }
    }

    pub fn start_session(
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

    pub fn replace_secrets(&mut self, secrets: SecretStore) {
        self.secrets = secrets;
    }

    pub fn display_local_path(path: &Path) -> String {
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

    pub fn remote_file_name(path: &str) -> String {
        let trimmed = path.trim_end_matches('/');
        if trimmed.is_empty() || trimmed == "/" {
            return String::new();
        }

        trimmed
            .rsplit_once('/')
            .map(|(_, name)| name.to_string())
            .unwrap_or_else(|| trimmed.to_string())
    }

    pub fn join_remote_path(base: &str, name: &str) -> String {
        let trimmed_name = name.trim_matches('/');
        if base.is_empty() || base == "." {
            return trimmed_name.to_string();
        }

        if base == "/" {
            return format!("/{trimmed_name}");
        }

        format!("{}/{}", base.trim_end_matches('/'), trimmed_name)
    }

    pub fn plan_uploads(local_paths: Vec<PathBuf>, remote_base: &str) -> Vec<PlannedSftpUpload> {
        Self::plan_uploads_with_known_directories(local_paths, &[], remote_base)
    }

    pub fn plan_uploads_with_known_directories(
        local_paths: Vec<PathBuf>,
        known_directory_paths: &[PathBuf],
        remote_base: &str,
    ) -> Vec<PlannedSftpUpload> {
        Self::normalize_local_upload_roots(local_paths, known_directory_paths)
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

    pub fn plan_downloads(
        selected_entries: Vec<SftpEntry>,
        local_base: &Path,
    ) -> Vec<PlannedSftpDownload> {
        let mut seen_paths = HashSet::new();
        let unique_entries: Vec<_> = selected_entries
            .into_iter()
            .filter(|entry| seen_paths.insert(entry.path.clone()))
            .collect();
        let directory_paths: HashSet<_> = unique_entries
            .iter()
            .filter(|entry| entry.kind == SftpEntryKind::Directory)
            .filter_map(|entry| {
                let path = Self::remote_path_without_trailing_separator(&entry.path);
                Self::remote_path_is_safe(path).then_some(path)
            })
            .collect();

        unique_entries
            .iter()
            .filter(|candidate| {
                !Self::remote_path_has_selected_directory_ancestor(
                    &candidate.path,
                    &directory_paths,
                )
            })
            .map(|entry| PlannedSftpDownload {
                remote_path: entry.path.clone(),
                local_path: local_base.join(&entry.filename),
            })
            .collect()
    }

    fn normalize_local_upload_roots(
        local_paths: Vec<PathBuf>,
        known_directory_paths: &[PathBuf],
    ) -> Vec<PathBuf> {
        let mut seen_paths = HashSet::new();
        let unique_paths: Vec<_> = local_paths
            .into_iter()
            .filter(|path| seen_paths.insert(path.clone()))
            .collect();
        let selected_paths: HashSet<_> = unique_paths.iter().cloned().collect();
        let known_directory_paths: HashSet<_> = known_directory_paths.iter().collect();
        let mut possible_directory_paths = HashSet::new();

        for path in &unique_paths {
            let mut parent = path.parent();
            while let Some(ancestor) = parent {
                if selected_paths.contains(ancestor) {
                    possible_directory_paths.insert(ancestor.to_path_buf());
                }
                parent = ancestor.parent();
            }
        }

        let directory_paths: HashSet<_> = possible_directory_paths
            .into_iter()
            .filter(|path| {
                known_directory_paths.contains(path)
                    || std::fs::symlink_metadata(path)
                        .map(|metadata| metadata.file_type().is_dir())
                        .unwrap_or(false)
            })
            .collect();

        unique_paths
            .into_iter()
            .filter(|candidate| {
                let mut parent = candidate.parent();
                while let Some(ancestor) = parent {
                    if directory_paths.contains(ancestor) {
                        return false;
                    }
                    parent = ancestor.parent();
                }
                true
            })
            .collect()
    }

    fn remote_path_has_selected_directory_ancestor(
        path: &str,
        directory_paths: &HashSet<&str>,
    ) -> bool {
        let path = Self::remote_path_without_trailing_separator(path);
        if !Self::remote_path_is_safe(path) {
            return false;
        }

        let mut current = path;
        while let Some(parent) = Self::remote_parent_path(current) {
            if directory_paths.contains(parent) {
                return true;
            }
            current = parent;
        }
        false
    }

    fn remote_path_is_safe(path: &str) -> bool {
        !path
            .split('/')
            .any(|component| matches!(component, "." | ".."))
    }

    fn remote_path_without_trailing_separator(path: &str) -> &str {
        let trimmed = path.trim_end_matches('/');
        if trimmed.is_empty() && path.starts_with('/') {
            "/"
        } else {
            trimmed
        }
    }

    fn remote_parent_path(path: &str) -> Option<&str> {
        let path = Self::remote_path_without_trailing_separator(path);
        if path.is_empty() || path == "/" {
            return None;
        }

        match path.rfind('/') {
            Some(0) => Some("/"),
            Some(index) => Some(Self::remote_path_without_trailing_separator(&path[..index])),
            None => None,
        }
    }

    pub fn count_remote_conflicts(
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

    pub fn queue_uploads(
        commands: &SftpCommandSender,
        uploads: &[PlannedSftpUpload],
    ) -> Result<()> {
        for upload in uploads {
            commands.queue_upload(upload.local_path.clone(), upload.remote_path.clone())?;
        }
        Ok(())
    }

    pub fn queue_downloads(
        commands: &SftpCommandSender,
        downloads: &[PlannedSftpDownload],
    ) -> Result<()> {
        for download in downloads {
            commands.queue_download(download.remote_path.clone(), download.local_path.clone())?;
        }
        Ok(())
    }

    pub fn pause_transfer(commands: &SftpCommandSender, transfer_id: TransferId) -> Result<()> {
        commands.pause_transfer(transfer_id)
    }

    pub fn resume_transfer(commands: &SftpCommandSender, transfer_id: TransferId) -> Result<()> {
        commands.resume_transfer(transfer_id)
    }

    pub fn cancel_transfer(commands: &SftpCommandSender, transfer_id: TransferId) -> Result<()> {
        commands.cancel_transfer(transfer_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remote_entry(path: &str, kind: SftpEntryKind) -> SftpEntry {
        SftpEntry {
            filename: SftpService::remote_file_name(path),
            path: path.to_string(),
            kind,
            size: None,
            modified: None,
            attributes: None,
            owner: None,
        }
    }

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

    #[test]
    fn plan_downloads_prunes_directory_descendants_regardless_of_input_order() {
        let local_base = PathBuf::from("downloads");
        let downloads = SftpService::plan_downloads(
            vec![
                remote_entry("/archive/child.txt", SftpEntryKind::File),
                remote_entry("/standalone.txt", SftpEntryKind::File),
                remote_entry("/archive/nested/grandchild.txt", SftpEntryKind::File),
                remote_entry("/archive", SftpEntryKind::Directory),
                remote_entry("/archive/child.txt", SftpEntryKind::File),
            ],
            &local_base,
        );

        assert_eq!(
            downloads,
            vec![
                PlannedSftpDownload {
                    remote_path: "/standalone.txt".into(),
                    local_path: local_base.join("standalone.txt"),
                },
                PlannedSftpDownload {
                    remote_path: "/archive".into(),
                    local_path: local_base.join("archive"),
                },
            ]
        );
    }

    #[test]
    fn plan_downloads_uses_directory_kind_and_posix_component_boundaries() {
        let local_base = PathBuf::from("downloads");
        let downloads = SftpService::plan_downloads(
            vec![
                remote_entry("/foo", SftpEntryKind::Directory),
                remote_entry("/foo/child.txt", SftpEntryKind::File),
                remote_entry("/foobar.txt", SftpEntryKind::File),
                remote_entry("/foo-bar/child.txt", SftpEntryKind::File),
                remote_entry("/link", SftpEntryKind::Symlink),
                remote_entry("/link/child.txt", SftpEntryKind::File),
                remote_entry("/other", SftpEntryKind::Other),
                remote_entry("/other/child.txt", SftpEntryKind::File),
                remote_entry("/foo", SftpEntryKind::Directory),
                remote_entry("/foo/../escaped.txt", SftpEntryKind::File),
            ],
            &local_base,
        );

        let remote_paths: Vec<_> = downloads
            .iter()
            .map(|download| download.remote_path.as_str())
            .collect();
        assert_eq!(
            remote_paths,
            vec![
                "/foo",
                "/foobar.txt",
                "/foo-bar/child.txt",
                "/link",
                "/link/child.txt",
                "/other",
                "/other/child.txt",
                "/foo/../escaped.txt",
            ]
        );
    }

    #[test]
    fn plan_downloads_root_covers_only_absolute_descendants() {
        let local_base = PathBuf::from("downloads");
        let downloads = SftpService::plan_downloads(
            vec![
                remote_entry("/absolute.txt", SftpEntryKind::File),
                remote_entry("relative.txt", SftpEntryKind::File),
                remote_entry("/", SftpEntryKind::Directory),
            ],
            &local_base,
        );

        let remote_paths: Vec<_> = downloads
            .iter()
            .map(|download| download.remote_path.as_str())
            .collect();
        assert_eq!(remote_paths, vec!["relative.txt", "/"]);
    }

    #[test]
    fn plan_uploads_prunes_real_directory_descendants_and_preserves_survivor_order() {
        let temp = tempfile::tempdir().expect("create temporary directory");
        let folder = temp.path().join("folder");
        let nested = folder.join("nested");
        let child = folder.join("child.txt");
        let grandchild = nested.join("grandchild.txt");
        let standalone = temp.path().join("standalone.txt");
        std::fs::create_dir_all(&nested).expect("create nested directories");
        std::fs::write(&child, b"child").expect("write child");
        std::fs::write(&grandchild, b"grandchild").expect("write grandchild");
        std::fs::write(&standalone, b"standalone").expect("write standalone");

        let uploads = SftpService::plan_uploads(
            vec![
                child.clone(),
                standalone.clone(),
                grandchild,
                folder.clone(),
                child,
            ],
            "/remote",
        );

        assert_eq!(
            uploads,
            vec![
                PlannedSftpUpload {
                    local_path: standalone,
                    remote_path: "/remote/standalone.txt".into(),
                },
                PlannedSftpUpload {
                    local_path: folder,
                    remote_path: "/remote/folder".into(),
                },
            ]
        );
    }

    #[test]
    fn plan_uploads_requires_a_real_directory_ancestor_and_uses_path_components() {
        let temp = tempfile::tempdir().expect("create temporary directory");
        let foo = temp.path().join("foo");
        let foobar = temp.path().join("foobar");
        let foobar_child = foobar.join("child.txt");
        let regular_file = temp.path().join("regular");
        let synthetic_child = regular_file.join("child.txt");
        std::fs::create_dir_all(&foo).expect("create foo directory");
        std::fs::create_dir_all(&foobar).expect("create foobar directory");
        std::fs::write(&foobar_child, b"child").expect("write foobar child");
        std::fs::write(&regular_file, b"regular").expect("write regular file");

        let uploads = SftpService::plan_uploads(
            vec![
                foo.clone(),
                foobar_child.clone(),
                regular_file.clone(),
                synthetic_child.clone(),
            ],
            "/remote",
        );

        assert_eq!(
            uploads,
            vec![
                PlannedSftpUpload {
                    local_path: foo,
                    remote_path: "/remote/foo".into(),
                },
                PlannedSftpUpload {
                    local_path: foobar_child,
                    remote_path: "/remote/child.txt".into(),
                },
                PlannedSftpUpload {
                    local_path: regular_file,
                    remote_path: "/remote/regular".into(),
                },
                PlannedSftpUpload {
                    local_path: synthetic_child,
                    remote_path: "/remote/child.txt".into(),
                },
            ]
        );
    }

    #[test]
    fn plan_uploads_uses_known_directory_paths_without_filesystem_metadata() {
        let root = PathBuf::from("missing-root");
        let child = root.join("child.txt");
        let uploads = SftpService::plan_uploads_with_known_directories(
            vec![child, root.clone()],
            std::slice::from_ref(&root),
            "/remote",
        );

        assert_eq!(
            uploads,
            vec![PlannedSftpUpload {
                local_path: root,
                remote_path: "/remote/missing-root".into(),
            }]
        );
    }
}

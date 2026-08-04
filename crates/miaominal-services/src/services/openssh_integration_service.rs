use crate::{SshBridgeRouteRefresh, SshBridgeService};
use anyhow::{Context, Result, bail};
use miaominal_core::profile::{AuthMethod, SessionProfile};
use miaominal_core::proxy::ProxyProfile;
use miaominal_settings::OpenSshIntegrationMode;
use miaominal_ssh::{SshBridgeRoute, SshBridgeSyncResult};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

const INCLUDE_BEGIN_PREFIX: &str = "# BEGIN MIAOMINAL OPENSSH ";
const INCLUDE_END_PREFIX: &str = "# END MIAOMINAL OPENSSH ";

#[derive(Clone)]
pub struct OpenSshIntegrationService {
    bridge: SshBridgeService,
    ssh_dir: PathBuf,
    instance_id: String,
    helper_executable: PathBuf,
    state: Arc<RwLock<ProjectionState>>,
}

#[derive(Clone)]
struct ProjectionState {
    mode: OpenSshIntegrationMode,
    profiles: Vec<SessionProfile>,
    proxies: Vec<ProxyProfile>,
    last_sync: Option<SshBridgeSyncResult>,
}

impl OpenSshIntegrationService {
    pub fn new(bridge: SshBridgeService, ssh_dir: PathBuf, instance_id: String) -> Self {
        let helper_executable =
            std::env::current_exe().unwrap_or_else(|_| PathBuf::from("miaominal"));
        Self::new_with_executable(bridge, ssh_dir, instance_id, helper_executable)
    }

    pub fn new_with_executable(
        bridge: SshBridgeService,
        ssh_dir: PathBuf,
        instance_id: String,
        helper_executable: PathBuf,
    ) -> Self {
        Self {
            bridge,
            ssh_dir,
            instance_id,
            helper_executable,
            state: Arc::new(RwLock::new(ProjectionState {
                mode: OpenSshIntegrationMode::Disabled,
                profiles: Vec::new(),
                proxies: Vec::new(),
                last_sync: None,
            })),
        }
    }

    pub fn for_current_user(bridge: SshBridgeService, instance_id: String) -> Result<Self> {
        Ok(Self::new(
            bridge,
            Self::current_user_ssh_dir()?,
            instance_id,
        ))
    }

    pub fn current_user_ssh_dir() -> Result<PathBuf> {
        let home = directories::UserDirs::new()
            .map(|directories| directories.home_dir().to_path_buf())
            .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
            .context("failed to locate the current user's home directory")?;
        Ok(home.join(".ssh"))
    }

    pub fn config_path(&self) -> PathBuf {
        self.instance_dir().join("config")
    }

    pub fn sync(
        &self,
        mode: OpenSshIntegrationMode,
        profiles: Vec<SessionProfile>,
        proxies: Vec<ProxyProfile>,
    ) -> Result<SshBridgeSyncResult> {
        let refresh = self
            .bridge
            .refresh_routes(profiles.clone(), proxies.clone());
        let instance_dir = self.instance_dir();
        let config_path = self.config_path();
        let known_hosts_path = self.bridge.known_hosts_path().to_path_buf();

        match mode {
            OpenSshIntegrationMode::Disabled => {
                self.update_root_include(false)?;
                remove_if_exists(&config_path)?;
                remove_if_exists(&known_hosts_path)?;
            }
            OpenSshIntegrationMode::Direct => {
                std::fs::create_dir_all(&instance_dir)
                    .with_context(|| format!("failed to create {}", instance_dir.display()))?;
                let contents = render_direct_config(&profiles, &refresh)?;
                miaominal_paths::atomic_write(&config_path, contents.as_bytes())?;
                remove_if_exists(&known_hosts_path)?;
                self.update_root_include(true)?;
            }
            OpenSshIntegrationMode::Bridge => {
                std::fs::create_dir_all(&instance_dir)
                    .with_context(|| format!("failed to create {}", instance_dir.display()))?;
                let contents = render_bridge_config(
                    &profiles,
                    &refresh,
                    self.bridge.endpoint(),
                    &self.instance_id,
                    &known_hosts_path,
                    &self.helper_executable,
                )?;
                miaominal_paths::atomic_write(&config_path, contents.as_bytes())?;
                self.bridge.ensure_known_hosts_sidecar()?;
                self.update_root_include(true)?;
            }
        }

        let result = SshBridgeSyncResult {
            config_path,
            known_hosts_path,
            exported_profile_count: refresh.exported_profile_count(),
            skipped_profile_count: refresh.skipped_profile_count(),
        };
        if let Ok(mut state) = self.state.write() {
            *state = ProjectionState {
                mode,
                profiles,
                proxies,
                last_sync: Some(result.clone()),
            };
        }
        Ok(result)
    }

    pub fn set_mode(&self, mode: OpenSshIntegrationMode) -> Result<SshBridgeSyncResult> {
        let state = self
            .state
            .read()
            .map(|state| state.clone())
            .unwrap_or(ProjectionState {
                mode: OpenSshIntegrationMode::Disabled,
                profiles: Vec::new(),
                proxies: Vec::new(),
                last_sync: None,
            });
        self.sync(mode, state.profiles, state.proxies)
    }

    pub fn refresh(
        &self,
        profiles: Vec<SessionProfile>,
        proxies: Vec<ProxyProfile>,
    ) -> Result<SshBridgeSyncResult> {
        let mode = self.mode();
        self.sync(mode, profiles, proxies)
    }

    pub fn mode(&self) -> OpenSshIntegrationMode {
        self.state
            .read()
            .map(|state| state.mode)
            .unwrap_or(OpenSshIntegrationMode::Disabled)
    }

    pub fn last_sync_result(&self) -> Option<SshBridgeSyncResult> {
        self.state
            .read()
            .ok()
            .and_then(|state| state.last_sync.clone())
    }

    fn instance_dir(&self) -> PathBuf {
        self.ssh_dir.join("miaominal").join(&self.instance_id)
    }

    fn update_root_include(&self, enabled: bool) -> Result<()> {
        let root_config = self.ssh_dir.join("config");
        if !enabled && !root_config.exists() {
            return Ok(());
        }
        std::fs::create_dir_all(&self.ssh_dir)
            .with_context(|| format!("failed to create {}", self.ssh_dir.display()))?;
        let existing = match std::fs::read_to_string(&root_config) {
            Ok(existing) => existing,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(error).context("failed to read the OpenSSH config"),
        };
        let begin = format!("{INCLUDE_BEGIN_PREFIX}{}", self.instance_id);
        let end = format!("{INCLUDE_END_PREFIX}{}", self.instance_id);
        let mut lines = remove_managed_include_block(&existing, &begin, &end)?;
        while lines.last().is_some_and(|line| line.trim().is_empty()) {
            lines.pop();
        }
        if enabled {
            let mut managed = vec![
                begin,
                format!(
                    "Include {}",
                    openssh_quote(&normalize_path(&self.config_path()))?
                ),
                "Host *".to_string(),
                end,
            ];
            if !lines.is_empty() {
                managed.push(String::new());
                managed.extend(lines);
            }
            lines = managed;
        }
        let contents = if lines.is_empty() {
            String::new()
        } else {
            format!("{}\n", lines.join("\n"))
        };
        miaominal_paths::atomic_write(root_config, contents.as_bytes())?;
        Ok(())
    }
}

fn remove_managed_include_block(existing: &str, begin: &str, end: &str) -> Result<Vec<String>> {
    let mut lines = Vec::new();
    let mut inside = false;
    let mut seen = false;
    for line in existing.lines() {
        let trimmed = line.trim();
        if trimmed == begin {
            if inside {
                bail!("nested Miaominal OpenSSH managed block for this instance");
            }
            if seen {
                bail!("duplicate Miaominal OpenSSH managed block for this instance");
            }
            inside = true;
            seen = true;
            continue;
        }
        if trimmed == end {
            if !inside {
                bail!("Miaominal OpenSSH managed block has an end marker without a begin marker");
            }
            inside = false;
            continue;
        }
        if !inside {
            lines.push(line.to_string());
        }
    }
    if inside {
        bail!("Miaominal OpenSSH managed block is missing its end marker");
    }
    Ok(lines)
}

fn render_bridge_config(
    profiles: &[SessionProfile],
    refresh: &SshBridgeRouteRefresh,
    endpoint: &miaominal_ssh::SshBridgeEndpoint,
    instance_id: &str,
    known_hosts_path: &Path,
    executable: &Path,
) -> Result<String> {
    let by_id = profiles
        .iter()
        .map(|profile| (profile.id.as_str(), profile))
        .collect::<HashMap<_, _>>();
    let aliases = route_aliases(&refresh.routes);
    let executable = openssh_quote(&normalize_path(executable))?;
    let endpoint = openssh_quote(&endpoint.helper_value())?;
    let known_hosts = openssh_quote(&normalize_path(known_hosts_path))?;
    let mut output = String::from("# Generated by Miaominal. Changes will be overwritten.\n\n");
    for route in &refresh.routes {
        let Some(profile) = by_id.get(route.profile_id.as_str()) else {
            continue;
        };
        let alias = aliases
            .get(route.token.as_str())
            .expect("route alias should exist");
        output.push_str(&format!("Host {alias}\n"));
        output.push_str("    HostName miaominal-bridge.invalid\n");
        output.push_str("    User miaominal\n");
        output.push_str(&format!(
            "    ProxyCommand {executable} ssh-bridge-helper --endpoint {endpoint} --route {}\n",
            route.token
        ));
        output.push_str(&format!(
            "    HostKeyAlias miaominal-bridge-{instance_id}\n"
        ));
        output.push_str(&format!("    UserKnownHostsFile {known_hosts}\n"));
        output.push_str("    StrictHostKeyChecking yes\n");
        output.push_str("    BatchMode yes\n");
        output.push_str("    ProxyJump none\n");
        output.push_str("    ForwardAgent no\n");
        output.push_str("    IdentitiesOnly yes\n");
        output.push_str(&format!(
            "    # Miaominal Profile: {}\n\n",
            safe_comment(&profile.name)
        ));
    }
    Ok(output)
}

fn render_direct_config(
    profiles: &[SessionProfile],
    refresh: &SshBridgeRouteRefresh,
) -> Result<String> {
    let by_id = profiles
        .iter()
        .map(|profile| (profile.id.as_str(), profile))
        .collect::<HashMap<_, _>>();
    let aliases = route_aliases(&refresh.routes);
    let alias_by_profile = refresh
        .routes
        .iter()
        .filter_map(|route| {
            aliases
                .get(route.token.as_str())
                .map(|alias| (route.profile_id.as_str(), alias.as_str()))
        })
        .collect::<HashMap<_, _>>();
    let mut output = String::from("# Generated by Miaominal. Changes will be overwritten.\n\n");
    for route in &refresh.routes {
        let Some(profile) = by_id.get(route.profile_id.as_str()) else {
            continue;
        };
        let alias = aliases
            .get(route.token.as_str())
            .expect("route alias should exist");
        output.push_str(&format!("Host {alias}\n"));
        output.push_str(&format!("    HostName {}\n", openssh_quote(&profile.host)?));
        output.push_str(&format!("    User {}\n", openssh_quote(&profile.username)?));
        output.push_str(&format!("    Port {}\n", profile.port));
        if profile.effective_auth_method() == AuthMethod::KeyFile
            && !profile.private_key_path.trim().is_empty()
        {
            output.push_str(&format!(
                "    IdentityFile {}\n",
                openssh_quote(&normalize_path(Path::new(profile.private_key_path.trim())))?
            ));
        }
        let jumps = profile
            .proxy_jump_profile_ids
            .iter()
            .filter_map(|id| alias_by_profile.get(id.as_str()).copied())
            .collect::<Vec<_>>();
        if !jumps.is_empty() {
            output.push_str(&format!("    ProxyJump {}\n", jumps.join(",")));
        }
        output.push_str(&format!(
            "    # Miaominal Profile: {}\n\n",
            safe_comment(&profile.name)
        ));
    }
    Ok(output)
}

fn route_aliases(routes: &[SshBridgeRoute]) -> HashMap<&str, String> {
    let bases = routes
        .iter()
        .map(|route| readable_alias(&route.profile_name, &route.token))
        .collect::<Vec<_>>();
    let mut counts = HashMap::new();
    for base in &bases {
        *counts.entry(base.clone()).or_insert(0usize) += 1;
    }
    let mut ordered = routes.iter().zip(bases.iter()).collect::<Vec<_>>();
    ordered.sort_by(|(left, _), (right, _)| {
        left.token
            .cmp(&right.token)
            .then_with(|| left.profile_id.cmp(&right.profile_id))
    });
    let mut used = HashSet::new();
    ordered
        .into_iter()
        .map(|(route, base)| {
            let preferred = if counts.get(base).copied().unwrap_or(0) > 1 {
                format!("{}-{}", base, &route.token[..8])
            } else {
                base.clone()
            };
            let alias = unique_route_alias(base, &route.token, preferred, &mut used);
            (route.token.as_str(), alias)
        })
        .collect()
}

fn unique_route_alias(
    base: &str,
    token: &str,
    preferred: String,
    used: &mut HashSet<String>,
) -> String {
    if used.insert(preferred.clone()) {
        return preferred;
    }
    for length in [8usize, 12, 16, 24, 32] {
        let candidate = format!("{base}-{}", &token[..length.min(token.len())]);
        if used.insert(candidate.clone()) {
            return candidate;
        }
    }
    let mut ordinal = 2usize;
    loop {
        let candidate = format!("{base}-{token}-{ordinal}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
        ordinal += 1;
    }
}

fn safe_comment(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
}

fn readable_alias(profile_name: &str, token: &str) -> String {
    let mut slug = String::new();
    let mut separator = false;
    for character in profile_name.trim().chars() {
        if character.is_alphanumeric() || matches!(character, '_' | '-') {
            if separator && !slug.is_empty() {
                slug.push('-');
            }
            separator = false;
            slug.extend(character.to_lowercase());
        } else {
            separator = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        slug.push_str(&token[..8]);
    }
    format!("miaominal-{slug}")
}

fn openssh_quote(value: &str) -> Result<String> {
    if value.chars().any(char::is_control) {
        bail!("OpenSSH config values cannot contain control characters");
    }
    Ok(format!(
        "\"{}\"",
        value.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}

fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn remove_if_exists(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("failed to remove {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use miaominal_secrets::SecretStore;
    use miaominal_settings::SshBridgeConfig;
    use miaominal_ssh::SshBridgeEndpoint;
    use miaominal_storage::{BridgeAuditLog, BridgeSecurityStore, KnownHostsStore};
    use tokio::runtime::Runtime;

    #[cfg(windows)]
    fn secure_for_windows_openssh(path: &Path) {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::Security::Authorization::{
            ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
        };
        use windows_sys::Win32::Security::{
            DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, SetFileSecurityW,
        };

        let whoami = std::process::Command::new("whoami")
            .args(["/user", "/fo", "csv", "/nh"])
            .output()
            .expect("query current Windows user SID");
        assert!(whoami.status.success(), "query current Windows user SID");
        let output = String::from_utf8_lossy(&whoami.stdout);
        let sid = output
            .trim()
            .rsplit(',')
            .next()
            .map(|value| value.trim().trim_matches('"'))
            .filter(|value| value.starts_with("S-1-"))
            .expect("whoami should return a Windows user SID");
        let mut sddl = format!("D:P(A;;FA;;;{sid})(A;;FA;;;SY)")
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let mut descriptor = std::ptr::null_mut();
        let converted = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_mut_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        };
        assert_ne!(converted, 0, "build OpenSSH test file ACL");
        let wide_path = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let applied = unsafe {
            SetFileSecurityW(
                wide_path.as_ptr(),
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                descriptor,
            )
        };
        unsafe {
            LocalFree(descriptor);
        }
        assert_ne!(applied, 0, "apply OpenSSH test file ACL");
    }

    #[cfg(not(windows))]
    fn secure_for_windows_openssh(_path: &Path) {}

    fn profile(id: &str, name: &str) -> SessionProfile {
        let mut profile = SessionProfile::blank(id, 1);
        profile.name = name.into();
        profile.host = "example.com".into();
        profile.username = "akko".into();
        profile
    }

    fn service(runtime: &Runtime, root: &Path, ssh_dir: &Path) -> OpenSshIntegrationService {
        let endpoint = SshBridgeEndpoint::derive(root).unwrap();
        let instance_id = SshBridgeEndpoint::instance_id(root).unwrap();
        let bridge = SshBridgeService::new_with_stores(
            runtime.handle().clone(),
            endpoint,
            instance_id.clone(),
            ssh_dir
                .join("miaominal")
                .join(&instance_id)
                .join("bridge_known_hosts"),
            SshBridgeConfig::default(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(root.join("upstream_known_hosts")),
            BridgeSecurityStore::open(&root.join("ssh_bridge_security.db"))
                .map_err(|error| format!("{error:#}")),
            BridgeAuditLog::open(&root.join("ssh_bridge_audit.log"))
                .map_err(|error| format!("{error:#}")),
        );
        OpenSshIntegrationService::new(bridge, ssh_dir.to_path_buf(), instance_id)
    }

    #[test]
    fn bridge_projection_uses_readable_unicode_aliases_and_no_remote_route_directives() {
        let runtime = Runtime::new().unwrap();
        let root = tempfile::tempdir().unwrap();
        let ssh = tempfile::tempdir().unwrap();
        let service = service(&runtime, root.path(), ssh.path());
        let profiles = vec![
            profile("prod", "Production VPS"),
            profile("cn", "香港 主机"),
        ];
        let result = service
            .sync(OpenSshIntegrationMode::Bridge, profiles, vec![])
            .unwrap();
        let config = std::fs::read_to_string(&result.config_path).unwrap();

        assert!(config.contains("Host miaominal-production-vps"));
        assert!(config.contains("Host miaominal-香港-主机"));
        assert!(config.contains("ssh-bridge-helper --endpoint"));
        assert!(config.contains("StrictHostKeyChecking yes"));
        assert!(config.contains("ProxyJump none"));
        assert!(!config.contains("IdentityAgent"));
        assert!(!config.contains("HostName example.com"));
    }

    #[test]
    fn managed_include_blocks_coexist_and_disabled_removes_only_own_block() {
        let runtime = Runtime::new().unwrap();
        let root_a = tempfile::tempdir().unwrap();
        let root_b = tempfile::tempdir().unwrap();
        let ssh = tempfile::tempdir().unwrap();
        let service_a = service(&runtime, root_a.path(), ssh.path());
        let service_b = service(&runtime, root_b.path(), ssh.path());

        service_a
            .sync(
                OpenSshIntegrationMode::Bridge,
                vec![profile("a", "A")],
                vec![],
            )
            .unwrap();
        service_b
            .sync(
                OpenSshIntegrationMode::Bridge,
                vec![profile("b", "B")],
                vec![],
            )
            .unwrap();
        let root_config = ssh.path().join("config");
        let both = std::fs::read_to_string(&root_config).unwrap();
        assert_eq!(both.matches(INCLUDE_BEGIN_PREFIX).count(), 2);

        service_a
            .sync(OpenSshIntegrationMode::Disabled, vec![], vec![])
            .unwrap();
        let remaining = std::fs::read_to_string(root_config).unwrap();
        assert_eq!(remaining.matches(INCLUDE_BEGIN_PREFIX).count(), 1);
        assert!(service_b.config_path().exists());
        assert!(!service_a.config_path().exists());
    }

    #[test]
    fn malformed_managed_include_markers_are_rejected_without_rewriting_user_config() {
        let runtime = Runtime::new().unwrap();
        let cases = [
            "Host user-before\n    HostName before.example\n{begin}\nInclude missing-end\nHost user-after\n    HostName after.example\n",
            "Host user-before\n    HostName before.example\n{end}\nHost user-after\n    HostName after.example\n",
            "{begin}\n{begin}\n{end}\n{end}\nHost user-after\n    HostName after.example\n",
            "{begin}\n{end}\n{begin}\n{end}\nHost user-after\n    HostName after.example\n",
        ];

        for case in cases {
            let root = tempfile::tempdir().unwrap();
            let ssh = tempfile::tempdir().unwrap();
            let service = service(&runtime, root.path(), ssh.path());
            let begin = format!("{INCLUDE_BEGIN_PREFIX}{}", service.instance_id);
            let end = format!("{INCLUDE_END_PREFIX}{}", service.instance_id);
            let original = case.replace("{begin}", &begin).replace("{end}", &end);
            let root_config = ssh.path().join("config");
            std::fs::write(&root_config, &original).unwrap();

            let error = service
                .sync(
                    OpenSshIntegrationMode::Bridge,
                    vec![profile("prod", "Production")],
                    vec![],
                )
                .expect_err("malformed managed markers must be rejected");

            assert!(
                error
                    .to_string()
                    .contains("Miaominal OpenSSH managed block")
            );
            assert_eq!(std::fs::read_to_string(&root_config).unwrap(), original);
        }
    }

    #[test]
    fn root_config_include_precedes_user_scopes_and_bridge_security_options_win() {
        let runtime = Runtime::new().unwrap();
        let root = tempfile::tempdir().unwrap();
        let ssh = tempfile::tempdir().unwrap();
        let service = service(&runtime, root.path(), ssh.path());
        let root_config = ssh.path().join("config");
        std::fs::write(
            &root_config,
            concat!(
                "ServerAliveInterval 37\n",
                "Host *\n",
                "    StrictHostKeyChecking no\n",
                "    BatchMode no\n",
                "Host unrelated\n",
                "    HostName unrelated.example\n",
            ),
        )
        .unwrap();
        let result = service
            .sync(
                OpenSshIntegrationMode::Bridge,
                vec![profile("prod", "Production")],
                vec![],
            )
            .unwrap();
        let root_contents = std::fs::read_to_string(&root_config).unwrap();
        let begin = format!("{INCLUDE_BEGIN_PREFIX}{}", service.instance_id);
        assert!(root_contents.starts_with(&begin));
        assert!(root_contents.contains("\nHost *\n# END MIAOMINAL OPENSSH"));

        let ssh_executable = if cfg!(windows) {
            PathBuf::from(r"C:\Windows\System32\OpenSSH\ssh.exe")
        } else {
            PathBuf::from("ssh")
        };
        secure_for_windows_openssh(&root_config);
        secure_for_windows_openssh(&result.config_path);
        let output = std::process::Command::new(ssh_executable)
            .args([
                "-G",
                "-F",
                &root_config.to_string_lossy(),
                "miaominal-production",
            ])
            .output();
        let Ok(output) = output else { return };
        assert!(
            output.status.success(),
            "ssh -G rejected root config: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let parsed = String::from_utf8_lossy(&output.stdout).to_lowercase();
        assert!(parsed.contains("hostname miaominal-bridge.invalid"));
        assert!(parsed.contains("stricthostkeychecking true"));
        assert!(parsed.contains("batchmode yes"));
        assert!(parsed.contains("serveraliveinterval 37"));
    }

    #[test]
    fn generated_config_is_accepted_by_system_ssh_parser_when_available() {
        let runtime = Runtime::new().unwrap();
        let root = tempfile::tempdir().unwrap();
        let ssh = tempfile::tempdir().unwrap();
        let service = service(&runtime, root.path(), ssh.path());
        let result = service
            .sync(
                OpenSshIntegrationMode::Bridge,
                vec![
                    profile("prod", "Production VPS"),
                    profile("cn", "香港 主机"),
                ],
                vec![],
            )
            .unwrap();
        let ssh_executable = if cfg!(windows) {
            PathBuf::from(r"C:\Windows\System32\OpenSSH\ssh.exe")
        } else {
            PathBuf::from("ssh")
        };
        secure_for_windows_openssh(&result.config_path);
        let output = std::process::Command::new(&ssh_executable)
            .args([
                "-G",
                "-F",
                &result.config_path.to_string_lossy(),
                "miaominal-香港-主机",
            ])
            .output();
        let Ok(output) = output else { return };
        assert!(
            output.status.success(),
            "ssh -G rejected config: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let parsed = String::from_utf8_lossy(&output.stdout).to_lowercase();
        assert!(parsed.contains("hostname miaominal-bridge.invalid"));
        assert!(parsed.contains("stricthostkeychecking true"));
        assert!(parsed.contains("ssh-bridge-helper"));
        assert_eq!(result.exported_profile_count, 2);
    }

    #[test]
    fn bridge_projection_quotes_windows_and_unix_helper_paths() {
        let profile = profile("prod", "Production VPS");
        let route = SshBridgeRoute {
            token: "11111111aaaaaaaaaaaaaaaaaaaaaaaa".into(),
            profile_id: profile.id.clone(),
            profile_name: profile.name.clone(),
        };
        let refresh = SshBridgeRouteRefresh {
            routes: vec![route],
            diagnostics: vec![],
        };

        let windows = render_bridge_config(
            std::slice::from_ref(&profile),
            &refresh,
            &SshBridgeEndpoint::WindowsNamedPipe(r"\\.\pipe\miaominal-ssh-bridge-test".into()),
            "windows-instance",
            Path::new(r"C:\Users\Akko User\.ssh\miaominal\bridge_known_hosts"),
            Path::new(r"C:\Program Files\Miaominal\miaominal.exe"),
        )
        .unwrap();
        assert!(windows.contains(
            "ProxyCommand \"C:/Program Files/Miaominal/miaominal.exe\" ssh-bridge-helper --endpoint \"//./pipe/miaominal-ssh-bridge-test\""
        ));
        assert!(windows.contains(
            "UserKnownHostsFile \"C:/Users/Akko User/.ssh/miaominal/bridge_known_hosts\""
        ));

        let unix = render_bridge_config(
            &[profile],
            &refresh,
            &SshBridgeEndpoint::UnixSocket(PathBuf::from(
                "/run/user/1000/miaominal bridge/bridge.sock",
            )),
            "unix-instance",
            Path::new("/home/akko user/.ssh/miaominal/bridge_known_hosts"),
            Path::new("/opt/Miaominal App/miaominal"),
        )
        .unwrap();
        assert!(unix.contains(
            "ProxyCommand \"/opt/Miaominal App/miaominal\" ssh-bridge-helper --endpoint \"/run/user/1000/miaominal bridge/bridge.sock\""
        ));
        assert!(
            unix.contains(
                "UserKnownHostsFile \"/home/akko user/.ssh/miaominal/bridge_known_hosts\""
            )
        );
    }

    #[test]
    fn direct_projection_preserves_key_files_and_jump_aliases_without_bridge_directives() {
        let runtime = Runtime::new().unwrap();
        let root = tempfile::tempdir().unwrap();
        let ssh = tempfile::tempdir().unwrap();
        let service = service(&runtime, root.path(), ssh.path());
        let jump = profile("jump", "Jump Host");
        let mut target = profile("target", "Production");
        target.auth_method = Some(AuthMethod::KeyFile);
        target.private_key_path = r"C:\Keys\production key".into();
        target.proxy_jump_profile_ids = vec![jump.id.clone()];

        let result = service
            .sync(OpenSshIntegrationMode::Direct, vec![jump, target], vec![])
            .unwrap();
        let config = std::fs::read_to_string(&result.config_path).unwrap();

        assert!(config.contains("Host miaominal-jump-host"));
        assert!(config.contains("Host miaominal-production"));
        assert!(config.contains("ProxyJump miaominal-jump-host"));
        assert!(config.contains("IdentityFile \"C:/Keys/production key\""));
        assert!(!config.contains("ssh-bridge-helper"));
        assert!(!config.contains("miaominal-bridge.invalid"));

        let ssh_executable = if cfg!(windows) {
            PathBuf::from(r"C:\Windows\System32\OpenSSH\ssh.exe")
        } else {
            PathBuf::from("ssh")
        };
        secure_for_windows_openssh(&result.config_path);
        let output = std::process::Command::new(ssh_executable)
            .args([
                "-G",
                "-F",
                &result.config_path.to_string_lossy(),
                "miaominal-production",
            ])
            .output();
        let Ok(output) = output else { return };
        assert!(
            output.status.success(),
            "ssh -G rejected Direct config: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let parsed = String::from_utf8_lossy(&output.stdout).to_lowercase();
        assert!(parsed.contains("hostname example.com"));
        assert!(parsed.contains("proxyjump miaominal-jump-host"));
    }

    #[test]
    fn openssh_values_reject_control_characters_and_comments_replace_them() {
        assert!(openssh_quote("example.com\nProxyCommand evil").is_err());
        assert!(openssh_quote("user\rHost wildcard").is_err());
        assert_eq!(
            safe_comment("line one\nline two\tend"),
            "line one line two end"
        );
    }

    #[test]
    fn duplicate_readable_aliases_are_stable_regardless_of_profile_order() {
        let first = SshBridgeRoute {
            token: "11111111aaaaaaaaaaaaaaaaaaaaaaaa".into(),
            profile_id: "first".into(),
            profile_name: "Production".into(),
        };
        let second = SshBridgeRoute {
            token: "22222222bbbbbbbbbbbbbbbbbbbbbbbb".into(),
            profile_id: "second".into(),
            profile_name: "Production".into(),
        };
        let forward_routes = [first.clone(), second.clone()];
        let reverse_routes = [second, first];
        let forward = route_aliases(&forward_routes);
        let reverse = route_aliases(&reverse_routes);

        assert_eq!(
            forward.get("11111111aaaaaaaaaaaaaaaaaaaaaaaa"),
            Some(&"miaominal-production-11111111".to_string())
        );
        assert_eq!(forward, reverse);
    }

    #[test]
    fn route_aliases_resolve_prefix_and_profile_name_collisions() {
        let first = SshBridgeRoute {
            token: "11111111aaaaaaaaaaaaaaaaaaaaaaaa".into(),
            profile_id: "first".into(),
            profile_name: "Production".into(),
        };
        let second = SshBridgeRoute {
            token: "11111111bbbbbbbbbbbbbbbbbbbbbbbb".into(),
            profile_id: "second".into(),
            profile_name: "Production".into(),
        };
        let named_like_generated_alias = SshBridgeRoute {
            token: "00000000cccccccccccccccccccccccc".into(),
            profile_id: "third".into(),
            profile_name: "Production-11111111".into(),
        };
        let routes = [first, second, named_like_generated_alias];
        let aliases = route_aliases(&routes);
        let unique = aliases.values().collect::<HashSet<_>>();

        assert_eq!(aliases.len(), routes.len());
        assert_eq!(unique.len(), routes.len());
        let mut reversed = routes.to_vec();
        reversed.reverse();
        assert_eq!(aliases, route_aliases(&reversed));
    }

    #[test]
    fn disabled_default_does_not_create_an_ssh_config_and_mode_refresh_reuses_snapshot() {
        let runtime = Runtime::new().unwrap();
        let root = tempfile::tempdir().unwrap();
        let ssh = tempfile::tempdir().unwrap();
        let service = service(&runtime, root.path(), ssh.path());

        service
            .sync(OpenSshIntegrationMode::Disabled, vec![], vec![])
            .unwrap();
        assert!(!ssh.path().join("config").exists());

        service
            .sync(
                OpenSshIntegrationMode::Direct,
                vec![profile("prod", "Production")],
                vec![],
            )
            .unwrap();
        let bridge_result = service.set_mode(OpenSshIntegrationMode::Bridge).unwrap();
        assert_eq!(service.mode(), OpenSshIntegrationMode::Bridge);
        assert_eq!(bridge_result.exported_profile_count, 1);
        assert_eq!(
            service.last_sync_result().map(|result| result.config_path),
            Some(service.config_path())
        );
        assert!(
            std::fs::read_to_string(service.config_path())
                .unwrap()
                .contains("ssh-bridge-helper")
        );
    }
}

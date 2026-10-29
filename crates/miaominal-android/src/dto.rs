//! Typed data-transfer objects for the mobile configuration, credential, key,
//! and trust interfaces.
//!
//! The DTOs never carry password or private-key material: credentials travel
//! through the dedicated `CredentialAction` operations. Plain configuration
//! updates use field-update semantics so Rust merges an edit into the stored
//! profile and keeps every field the Android editor does not display.

use miaominal_core::keychain::{ManagedKeyGenerationAlgorithm, ManagedKeyRecord, ManagedKeySource};
use miaominal_core::profile::{
    AuthMethod, PortForwardKind, PortForwardRule, ProfileKind, SessionEnvironmentVariable,
    SessionProfile, ShellType,
};
use miaominal_core::proxy::{ProxyAuthMode, ProxyProfile, ProxyProtocol};
use miaominal_settings::{
    AppLanguage, AppSettings, LastTabCloseBehavior, LocalVaultAutoLockDuration,
    MonitorHistoryDuration, ThemeId,
};

/// Rust timestamps are seconds; the FFI contract always uses milliseconds.
pub(crate) const MS_PER_SECOND: u64 = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ProfileKindDto {
    Ssh,
    Telnet,
    Rdp,
    Local,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum AuthMethodDto {
    Password,
    KeyFile,
    ManagedKey,
    Agent,
    KeyboardInteractive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ShellTypeDto {
    Posix,
    Fish,
    PowerShell,
    Cmd,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct EnvironmentVariableDto {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ForwardKindDto {
    Local,
    Remote,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ManagedKeySourceDto {
    Generated,
    Imported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ManagedKeyAlgorithmDto {
    Ed25519,
    Rsa4096,
}

/// Credential slots shared with the desktop credential store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CredentialKindDto {
    Password,
    Passphrase,
    ManagedPrivateKey,
    ProxyPassword,
}

/// Explicit credential update: absence of an operation must never delete a
/// stored secret by accident.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CredentialAction {
    Keep,
    Set { value: String },
    Clear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CredentialHealth {
    /// No credential is stored for this target.
    Missing,
    /// The credential is stored and can be read with the active backend.
    Present,
    /// The local vault is locked; the UI must unlock before connecting.
    Locked,
    /// The platform key was invalidated or replaced (for example after a
    /// restore), so the stored ciphertext can no longer be decrypted.
    Invalidated,
    /// The stored credential is damaged and must be entered again.
    Corrupt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum VaultModeDto {
    Disabled,
    Locked,
    Unlocked,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct VaultSnapshot {
    pub mode: VaultModeDto,
    pub file_present: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ForwardRuleDto {
    pub id: String,
    pub profile_id: String,
    pub label: String,
    pub kind: ForwardKindDto,
    pub listen_host: String,
    pub listen_port: u16,
    pub target_host: String,
    pub target_port: u16,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HostDto {
    pub id: String,
    pub name: String,
    pub group: String,
    pub tags: Vec<String>,
    pub kind: ProfileKindDto,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: AuthMethodDto,
    pub private_key_path: String,
    pub managed_key_id: String,
    pub certificate_path: String,
    pub startup_command: String,
    pub environment_variables: Vec<EnvironmentVariableDto>,
    pub shell_type: ShellTypeDto,
    pub is_favorite: bool,
    /// Milliseconds since the Unix epoch; `None` when never connected.
    pub last_connected_at_ms: Option<u64>,
    pub has_stored_password: bool,
    pub has_stored_passphrase: bool,
    /// Reusable entry proxy for every connection of this host (empty = none).
    pub entry_proxy_id: String,
    /// Ordered ProxyJump chain; the saved host ids must exist.
    pub proxy_jump_profile_ids: Vec<String>,
    pub forward_rules: Vec<ForwardRuleDto>,
}

/// Field-update payload. `None` means "keep the stored value"; Rust merges the
/// update into the stored profile so fields the Android editor does not show
/// survive the round trip.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HostUpdate {
    pub id: String,
    pub name: Option<String>,
    pub group: Option<String>,
    pub tags: Option<Vec<String>>,
    pub kind: Option<ProfileKindDto>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub username: Option<String>,
    pub auth_method: Option<AuthMethodDto>,
    pub private_key_path: Option<String>,
    pub managed_key_id: Option<String>,
    pub certificate_path: Option<String>,
    pub startup_command: Option<String>,
    pub environment_variables: Option<Vec<EnvironmentVariableDto>>,
    pub shell_type: Option<ShellTypeDto>,
    pub is_favorite: Option<bool>,
    pub last_connected_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SnippetDto {
    pub id: String,
    pub description: String,
    pub package_name: String,
    pub language: String,
    pub script: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SnippetUpdate {
    pub id: String,
    pub description: Option<String>,
    pub package_name: Option<String>,
    pub language: Option<String>,
    pub script: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ProxyDto {
    pub id: String,
    pub name: String,
    pub protocol: String,
    pub host: String,
    pub port: u16,
    pub auth_mode: String,
    pub username: String,
    pub resolve_dns_through_proxy: bool,
    pub has_stored_password: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ProxyUpdate {
    pub id: String,
    pub name: Option<String>,
    pub protocol: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub auth_mode: Option<String>,
    pub username: Option<String>,
    pub resolve_dns_through_proxy: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ManagedKeyDto {
    pub id: String,
    pub name: String,
    pub algorithm: String,
    pub public_key: String,
    pub source: ManagedKeySourceDto,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct KnownHostDto {
    pub host: String,
    pub port: u16,
    pub algorithm: String,
    pub fingerprint: String,
    /// Milliseconds since the Unix epoch; `None` when the entry predates
    /// timestamp tracking.
    pub added_at_ms: Option<u64>,
}

/// Mobile-visible settings. Display-only preferences such as the theme seed
/// stay in the Android host; business settings live in the Rust settings store
/// and are preserved across sync round trips.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SettingsDto {
    pub language: String,
    pub recent_host_count: u32,
    pub auto_monitoring: bool,
    pub monitor_history: String,
    pub close_last_session_behavior: String,
    pub vault_enabled: bool,
    pub vault_auto_lock_minutes: u32,
    pub onboarding_completed: bool,
    pub terminal_font: String,
    pub cjk_fallback_font: String,
    pub terminal_font_size: f32,
    pub terminal_line_height: f32,
    /// Effective light/dark theme; the terminal palette follows it.
    pub theme_id: String,
    pub seed_color: String,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SettingsUpdate {
    pub language: Option<String>,
    pub recent_host_count: Option<u32>,
    pub auto_monitoring: Option<bool>,
    pub monitor_history: Option<String>,
    pub close_last_session_behavior: Option<String>,
    pub vault_auto_lock_minutes: Option<u32>,
    pub onboarding_completed: Option<bool>,
    pub terminal_font: Option<String>,
    pub cjk_fallback_font: Option<String>,
    pub terminal_font_size: Option<f32>,
    pub terminal_line_height: Option<f32>,
    pub theme_id: Option<String>,
    pub seed_color: Option<String>,
}

pub(crate) fn profile_kind_to_dto(kind: ProfileKind) -> ProfileKindDto {
    match kind {
        ProfileKind::Ssh => ProfileKindDto::Ssh,
        ProfileKind::Telnet => ProfileKindDto::Telnet,
        ProfileKind::Rdp => ProfileKindDto::Rdp,
        ProfileKind::Local => ProfileKindDto::Local,
    }
}

pub(crate) fn profile_kind_from_dto(kind: ProfileKindDto) -> ProfileKind {
    match kind {
        ProfileKindDto::Ssh => ProfileKind::Ssh,
        ProfileKindDto::Telnet => ProfileKind::Telnet,
        ProfileKindDto::Rdp => ProfileKind::Rdp,
        ProfileKindDto::Local => ProfileKind::Local,
    }
}

pub(crate) fn auth_method_to_dto(method: AuthMethod) -> AuthMethodDto {
    match method {
        AuthMethod::Password => AuthMethodDto::Password,
        AuthMethod::KeyFile => AuthMethodDto::KeyFile,
        AuthMethod::ManagedKey => AuthMethodDto::ManagedKey,
        AuthMethod::Agent => AuthMethodDto::Agent,
        AuthMethod::KeyboardInteractive => AuthMethodDto::KeyboardInteractive,
    }
}

pub(crate) fn auth_method_from_dto(method: AuthMethodDto) -> AuthMethod {
    match method {
        AuthMethodDto::Password => AuthMethod::Password,
        AuthMethodDto::KeyFile => AuthMethod::KeyFile,
        AuthMethodDto::ManagedKey => AuthMethod::ManagedKey,
        AuthMethodDto::Agent => AuthMethod::Agent,
        AuthMethodDto::KeyboardInteractive => AuthMethod::KeyboardInteractive,
    }
}

pub(crate) fn shell_type_to_dto(shell_type: ShellType) -> ShellTypeDto {
    match shell_type {
        ShellType::Posix => ShellTypeDto::Posix,
        ShellType::Fish => ShellTypeDto::Fish,
        ShellType::PowerShell => ShellTypeDto::PowerShell,
        ShellType::Cmd => ShellTypeDto::Cmd,
    }
}

pub(crate) fn shell_type_from_dto(shell_type: ShellTypeDto) -> ShellType {
    match shell_type {
        ShellTypeDto::Posix => ShellType::Posix,
        ShellTypeDto::Fish => ShellType::Fish,
        ShellTypeDto::PowerShell => ShellType::PowerShell,
        ShellTypeDto::Cmd => ShellType::Cmd,
    }
}

pub(crate) fn forward_kind_to_dto(kind: PortForwardKind) -> ForwardKindDto {
    match kind {
        PortForwardKind::Local => ForwardKindDto::Local,
        PortForwardKind::Remote => ForwardKindDto::Remote,
    }
}

pub(crate) fn forward_kind_from_dto(kind: ForwardKindDto) -> PortForwardKind {
    match kind {
        ForwardKindDto::Local => PortForwardKind::Local,
        ForwardKindDto::Remote => PortForwardKind::Remote,
    }
}

pub(crate) fn key_source_to_dto(source: ManagedKeySource) -> ManagedKeySourceDto {
    match source {
        ManagedKeySource::Generated => ManagedKeySourceDto::Generated,
        ManagedKeySource::Imported => ManagedKeySourceDto::Imported,
    }
}

pub(crate) fn key_algorithm_from_dto(
    algorithm: ManagedKeyAlgorithmDto,
) -> ManagedKeyGenerationAlgorithm {
    match algorithm {
        ManagedKeyAlgorithmDto::Ed25519 => ManagedKeyGenerationAlgorithm::Ed25519,
        ManagedKeyAlgorithmDto::Rsa4096 => ManagedKeyGenerationAlgorithm::Rsa4096,
    }
}

pub(crate) fn forward_rule_to_dto(profile_id: &str, rule: &PortForwardRule) -> ForwardRuleDto {
    ForwardRuleDto {
        id: rule.id.clone(),
        profile_id: profile_id.to_string(),
        label: rule.label.clone(),
        kind: forward_kind_to_dto(rule.kind),
        listen_host: rule.listen_host.clone(),
        listen_port: rule.listen_port,
        target_host: rule.target_host.clone(),
        target_port: rule.target_port,
        enabled: rule.enabled,
    }
}

pub(crate) fn forward_rule_from_dto(rule: &ForwardRuleDto) -> PortForwardRule {
    PortForwardRule {
        id: rule.id.clone(),
        label: rule.label.clone(),
        kind: forward_kind_from_dto(rule.kind),
        listen_host: rule.listen_host.clone(),
        listen_port: rule.listen_port,
        target_host: rule.target_host.clone(),
        target_port: rule.target_port,
        enabled: rule.enabled,
    }
}

pub(crate) fn host_to_dto(profile: &SessionProfile) -> HostDto {
    HostDto {
        id: profile.id.clone(),
        name: profile.name.clone(),
        group: profile.group.clone(),
        tags: profile.tags.clone(),
        kind: profile_kind_to_dto(profile.kind),
        host: profile.host.clone(),
        port: profile.port,
        username: profile.username.clone(),
        auth_method: auth_method_to_dto(profile.effective_auth_method()),
        private_key_path: profile.private_key_path.clone(),
        managed_key_id: profile.managed_key_id.clone(),
        certificate_path: profile.certificate_path.clone(),
        startup_command: profile.startup_command.clone(),
        environment_variables: profile
            .environment_variables
            .iter()
            .map(|variable| EnvironmentVariableDto {
                name: variable.name.clone(),
                value: variable.value.clone(),
            })
            .collect(),
        shell_type: shell_type_to_dto(profile.shell_type),
        is_favorite: profile.is_favorite,
        last_connected_at_ms: profile
            .last_connected_at
            .map(|seconds| seconds * MS_PER_SECOND),
        has_stored_password: profile.has_stored_password,
        has_stored_passphrase: profile.has_stored_passphrase,
        entry_proxy_id: profile.entry_proxy_id.clone().unwrap_or_default(),
        proxy_jump_profile_ids: profile.proxy_jump_profile_ids.clone(),
        forward_rules: profile
            .port_forwarding_rules
            .iter()
            .map(|rule| forward_rule_to_dto(&profile.id, rule))
            .collect(),
    }
}

/// Merge a field update into an existing profile. `None` fields keep the
/// stored value, including every field absent from the Android editor.
pub(crate) fn apply_host_update(profile: &mut SessionProfile, update: &HostUpdate) {
    if let Some(name) = &update.name {
        profile.name = name.clone();
    }
    if let Some(group) = &update.group {
        profile.group = group.clone();
    }
    if let Some(tags) = &update.tags {
        profile.tags = tags.clone();
    }
    if let Some(kind) = update.kind {
        profile.kind = profile_kind_from_dto(kind);
    }
    if let Some(host) = &update.host {
        profile.host = host.clone();
    }
    if let Some(port) = update.port {
        profile.port = port;
    }
    if let Some(username) = &update.username {
        profile.username = username.clone();
    }
    if let Some(auth_method) = update.auth_method {
        profile.auth_method = Some(auth_method_from_dto(auth_method));
    }
    if let Some(private_key_path) = &update.private_key_path {
        profile.private_key_path = private_key_path.clone();
    }
    if let Some(managed_key_id) = &update.managed_key_id {
        profile.managed_key_id = managed_key_id.clone();
    }
    if let Some(certificate_path) = &update.certificate_path {
        profile.certificate_path = certificate_path.clone();
    }
    if let Some(startup_command) = &update.startup_command {
        profile.startup_command = startup_command.clone();
    }
    if let Some(environment_variables) = &update.environment_variables {
        profile.environment_variables = environment_variables
            .iter()
            .map(|variable| SessionEnvironmentVariable {
                name: variable.name.clone(),
                value: variable.value.clone(),
            })
            .collect();
    }
    if let Some(shell_type) = update.shell_type {
        profile.shell_type = shell_type_from_dto(shell_type);
    }
    if let Some(is_favorite) = update.is_favorite {
        profile.is_favorite = is_favorite;
    }
    if let Some(last_connected_at_ms) = update.last_connected_at_ms {
        profile.last_connected_at = Some(last_connected_at_ms / MS_PER_SECOND);
    }
}

pub(crate) fn key_to_dto(record: &ManagedKeyRecord) -> ManagedKeyDto {
    ManagedKeyDto {
        id: record.id.clone(),
        name: record.name.clone(),
        algorithm: record.algorithm.clone(),
        public_key: record.public_key.clone(),
        source: key_source_to_dto(record.source),
    }
}

pub(crate) fn proxy_to_dto(profile: &ProxyProfile) -> ProxyDto {
    ProxyDto {
        id: profile.id.clone(),
        name: profile.name.clone(),
        protocol: proxy_protocol_name(profile.protocol).to_string(),
        host: profile.host.clone(),
        port: profile.port,
        auth_mode: proxy_auth_mode_name(profile.auth_mode).to_string(),
        username: profile.username.clone(),
        resolve_dns_through_proxy: profile.resolve_dns_through_proxy,
        has_stored_password: profile.has_stored_password,
    }
}

pub(crate) fn proxy_protocol_name(protocol: ProxyProtocol) -> &'static str {
    match protocol {
        ProxyProtocol::Socks5 => "socks5",
        ProxyProtocol::HttpConnect => "http-connect",
    }
}

pub(crate) fn proxy_protocol_from_name(name: &str) -> ProxyProtocol {
    match name {
        "http-connect" | "HttpConnect" | "http_connect" => ProxyProtocol::HttpConnect,
        _ => ProxyProtocol::Socks5,
    }
}

pub(crate) fn proxy_auth_mode_name(mode: ProxyAuthMode) -> &'static str {
    match mode {
        ProxyAuthMode::None => "none",
        ProxyAuthMode::UsernamePassword => "username-password",
    }
}

pub(crate) fn proxy_auth_mode_from_name(name: &str) -> ProxyAuthMode {
    match name {
        "username-password" | "UsernamePassword" | "username_password" => {
            ProxyAuthMode::UsernamePassword
        }
        _ => ProxyAuthMode::None,
    }
}

pub(crate) fn language_name(language: AppLanguage) -> &'static str {
    match language {
        AppLanguage::English => "English",
        AppLanguage::SimplifiedChinese => "SimplifiedChinese",
    }
}

pub(crate) fn language_from_name(name: &str) -> Option<AppLanguage> {
    match name {
        "English" | "en" => Some(AppLanguage::English),
        "SimplifiedChinese" | "zh-CN" | "zh_cn" => Some(AppLanguage::SimplifiedChinese),
        _ => None,
    }
}

pub(crate) fn monitor_history_name(duration: MonitorHistoryDuration) -> &'static str {
    match duration {
        MonitorHistoryDuration::OneMinute => "OneMinute",
        MonitorHistoryDuration::FiveMinutes => "FiveMinutes",
        MonitorHistoryDuration::TenMinutes => "TenMinutes",
        MonitorHistoryDuration::ThirtyMinutes => "ThirtyMinutes",
    }
}

pub(crate) fn monitor_history_from_name(name: &str) -> Option<MonitorHistoryDuration> {
    match name {
        "OneMinute" => Some(MonitorHistoryDuration::OneMinute),
        "FiveMinutes" => Some(MonitorHistoryDuration::FiveMinutes),
        "TenMinutes" => Some(MonitorHistoryDuration::TenMinutes),
        "ThirtyMinutes" => Some(MonitorHistoryDuration::ThirtyMinutes),
        _ => None,
    }
}

pub(crate) fn close_behavior_name(behavior: LastTabCloseBehavior) -> &'static str {
    match behavior {
        LastTabCloseBehavior::ExitApplication => "ExitApplication",
        LastTabCloseBehavior::OpenNewHomeTab => "OpenHosts",
    }
}

pub(crate) fn close_behavior_from_name(name: &str) -> Option<LastTabCloseBehavior> {
    match name {
        "ExitApplication" => Some(LastTabCloseBehavior::ExitApplication),
        "OpenHosts" | "OpenNewHomeTab" => Some(LastTabCloseBehavior::OpenNewHomeTab),
        _ => None,
    }
}

pub(crate) fn vault_auto_lock_to_minutes(duration: LocalVaultAutoLockDuration) -> u32 {
    match duration {
        LocalVaultAutoLockDuration::Off => 0,
        LocalVaultAutoLockDuration::FiveMinutes => 5,
        LocalVaultAutoLockDuration::FifteenMinutes => 15,
        LocalVaultAutoLockDuration::OneHour => 60,
        LocalVaultAutoLockDuration::OneDay => 24 * 60,
    }
}

pub(crate) fn vault_auto_lock_from_minutes(minutes: u32) -> Option<LocalVaultAutoLockDuration> {
    match minutes {
        0 => Some(LocalVaultAutoLockDuration::Off),
        5 => Some(LocalVaultAutoLockDuration::FiveMinutes),
        15 => Some(LocalVaultAutoLockDuration::FifteenMinutes),
        60 => Some(LocalVaultAutoLockDuration::OneHour),
        1440 => Some(LocalVaultAutoLockDuration::OneDay),
        _ => None,
    }
}

pub(crate) fn settings_to_dto(settings: &AppSettings) -> SettingsDto {
    SettingsDto {
        language: language_name(settings.language).to_string(),
        recent_host_count: u32::from(settings.recent_connections_count),
        auto_monitoring: settings.auto_collect_session_monitoring,
        monitor_history: monitor_history_name(settings.monitor_history_duration).to_string(),
        close_last_session_behavior: close_behavior_name(settings.last_tab_close_behavior)
            .to_string(),
        vault_enabled: settings.local_vault_enabled,
        vault_auto_lock_minutes: vault_auto_lock_to_minutes(
            settings.local_vault_auto_lock_duration,
        ),
        onboarding_completed: !settings.should_show_onboarding(),
        terminal_font: settings.terminal_font_family.clone(),
        cjk_fallback_font: settings.font_fallbacks.first().cloned().unwrap_or_default(),
        terminal_font_size: settings.terminal_font_size,
        terminal_line_height: settings.line_height,
        theme_id: theme_id_name(settings.theme_id).to_string(),
        seed_color: settings.seed_color.clone(),
    }
}

pub(crate) fn theme_id_name(theme_id: ThemeId) -> &'static str {
    match theme_id {
        ThemeId::Light => "light",
        ThemeId::Dark => "dark",
    }
}

pub(crate) fn theme_id_from_name(name: &str) -> Option<ThemeId> {
    match name {
        "light" | "Light" => Some(ThemeId::Light),
        "dark" | "Dark" | "one_dark" => Some(ThemeId::Dark),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_update_keeps_undisplayed_fields() {
        let mut profile = SessionProfile::blank("host-1", 1);
        profile.proxy_jump_profile_ids = vec!["host-2".into()];
        profile
            .environment_variables
            .push(miaominal_core::profile::SessionEnvironmentVariable {
                name: "LANG".into(),
                value: "C".into(),
            });
        profile.remote_path_favorites = vec!["/srv".into()];

        apply_host_update(
            &mut profile,
            &HostUpdate {
                id: "host-1".into(),
                name: Some("Renamed".into()),
                group: None,
                tags: None,
                kind: None,
                host: Some("example.com".into()),
                port: Some(2222),
                username: None,
                auth_method: None,
                private_key_path: None,
                managed_key_id: None,
                certificate_path: None,
                startup_command: None,
                environment_variables: None,
                shell_type: None,
                is_favorite: None,
                last_connected_at_ms: None,
            },
        );

        assert_eq!(profile.name, "Renamed");
        assert_eq!(profile.host, "example.com");
        assert_eq!(profile.port, 2222);
        assert_eq!(profile.proxy_jump_profile_ids, vec!["host-2"]);
        assert_eq!(profile.environment_variables.len(), 1);
        assert_eq!(profile.remote_path_favorites, vec!["/srv"]);
    }

    #[test]
    fn host_dto_converts_seconds_to_milliseconds() {
        let mut profile = SessionProfile::blank("host-1", 1);
        profile.last_connected_at = Some(42);

        let dto = host_to_dto(&profile);

        assert_eq!(dto.last_connected_at_ms, Some(42_000));
    }

    #[test]
    fn host_update_applies_shell_and_environment_variables() {
        let mut profile = SessionProfile::blank("host-1", 1);

        apply_host_update(
            &mut profile,
            &HostUpdate {
                id: "host-1".into(),
                name: None,
                group: None,
                tags: None,
                kind: None,
                host: None,
                port: None,
                username: None,
                auth_method: None,
                private_key_path: None,
                managed_key_id: None,
                certificate_path: None,
                startup_command: None,
                environment_variables: Some(vec![EnvironmentVariableDto {
                    name: "APP_ENV".into(),
                    value: "mobile test".into(),
                }]),
                shell_type: Some(ShellTypeDto::PowerShell),
                is_favorite: None,
                last_connected_at_ms: None,
            },
        );

        assert_eq!(profile.shell_type, ShellType::PowerShell);
        assert_eq!(
            profile.environment_variables,
            vec![SessionEnvironmentVariable {
                name: "APP_ENV".into(),
                value: "mobile test".into(),
            }]
        );
        let dto = host_to_dto(&profile);
        assert_eq!(dto.shell_type, ShellTypeDto::PowerShell);
        assert_eq!(dto.environment_variables[0].name, "APP_ENV");
    }

    #[test]
    fn proxy_names_round_trip() {
        for protocol in [ProxyProtocol::Socks5, ProxyProtocol::HttpConnect] {
            assert_eq!(
                proxy_protocol_from_name(proxy_protocol_name(protocol)),
                protocol
            );
        }
        for mode in [ProxyAuthMode::None, ProxyAuthMode::UsernamePassword] {
            assert_eq!(proxy_auth_mode_from_name(proxy_auth_mode_name(mode)), mode);
        }
    }

    #[test]
    fn close_behavior_maps_open_hosts_to_open_new_home_tab() {
        assert_eq!(
            close_behavior_from_name("OpenHosts"),
            Some(LastTabCloseBehavior::OpenNewHomeTab)
        );
        assert_eq!(
            close_behavior_name(LastTabCloseBehavior::OpenNewHomeTab),
            "OpenHosts"
        );
    }
}

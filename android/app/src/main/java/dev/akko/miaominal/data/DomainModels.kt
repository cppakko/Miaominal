package dev.akko.miaominal.data

import com.squareup.moshi.Json

enum class ProfileKind {
    @Json(name = "ssh")
    Ssh,

    @Json(name = "telnet")
    Telnet,

    @Json(name = "rdp")
    Rdp,

    @Json(name = "local")
    Local
}

enum class AuthMethod {
    @Json(name = "password")
    Password,

    @Json(name = "key-file")
    KeyFile,

    @Json(name = "managed-key")
    ManagedKey,

    @Json(name = "agent")
    Agent,

    @Json(name = "keyboard-interactive")
    KeyboardInteractive
}

enum class ForwardKind {
    @Json(name = "local")
    Local,

    @Json(name = "remote")
    Remote
}

enum class ManagedKeySource {
    @Json(name = "generated")
    Generated,

    @Json(name = "imported")
    Imported
}

enum class ShellType {
    @Json(name = "posix")
    Posix,

    @Json(name = "fish")
    Fish,

    @Json(name = "powershell")
    PowerShell,

    @Json(name = "cmd")
    Cmd
}

data class SessionEnvironmentVariable(
    @Json(name = "name") val name: String = "",
    @Json(name = "value") val value: String = ""
)

data class PortForwardRule(
    @Json(name = "id") val id: String = "",
    @Json(name = "profile_id") val profileId: String = "",
    @Json(name = "label") val label: String = "",
    @Json(name = "kind") val kind: ForwardKind = ForwardKind.Local,
    @Json(name = "listen_host") val listenHost: String = "127.0.0.1",
    @Json(name = "listen_port") val listenPort: Int = 0,
    @Json(name = "target_host") val targetHost: String = "127.0.0.1",
    @Json(name = "target_port") val targetPort: Int = 0,
    @Json(name = "enabled") val enabled: Boolean = false
)

data class SessionProfile(
    @Json(name = "id") val id: String = "",
    @Json(name = "name") val name: String = "",
    @Json(name = "group") val group: String = "",
    @Json(name = "tags") val tags: List<String> = emptyList(),
    @Json(name = "kind") val kind: ProfileKind = ProfileKind.Ssh,
    @Json(name = "host") val host: String = "",
    @Json(name = "port") val port: Int = 22,
    @Json(name = "username") val username: String = "",
    @Json(name = "auth_method") val authMethod: AuthMethod = AuthMethod.Password,
    @Json(name = "private_key_path") val privateKeyPath: String = "",
    @Json(name = "managed_key_id") val managedKeyId: String = "",
    @Json(name = "agent_identity") val agentIdentity: String = "",
    @Json(name = "agent_identity_label") val agentIdentityLabel: String = "",
    @Json(name = "certificate_path") val certificatePath: String = "",
    @Json(name = "startup_command") val startupCommand: String = "",
    @Json(name = "environment_variables") val environmentVariables: List<SessionEnvironmentVariable> = emptyList(),
    @Json(name = "shell_type") val shellType: ShellType = ShellType.Posix,
    @Json(name = "is_favorite") val isFavorite: Boolean = false,
    @Json(name = "has_stored_password") val hasStoredPassword: Boolean = false,
    @Json(name = "last_connected_at") val lastConnectedAt: Long? = null,
    /** Reusable entry proxy for every connection of this host (empty = none). */
    @Json(name = "entry_proxy_id") val entryProxyId: String = "",
    /** Ordered ProxyJump chain of saved host ids. */
    @Json(name = "proxy_jump_profile_ids") val proxyJumpProfileIds: List<String> = emptyList()
)

/** One reusable proxy usable as an entry proxy for hosts and forwards. */
data class ProxyProfile(
    @Json(name = "id") val id: String = "",
    @Json(name = "name") val name: String = "",
    @Json(name = "protocol") val protocol: String = "socks5",
    @Json(name = "host") val host: String = "",
    @Json(name = "port") val port: Int = 1080,
    @Json(name = "auth_mode") val authMode: String = "none",
    @Json(name = "username") val username: String = "",
    @Json(name = "resolve_dns_through_proxy") val resolveDnsThroughProxy: Boolean = false,
    @Json(name = "has_stored_password") val hasStoredPassword: Boolean = false
)

data class SnippetRecord(
    @Json(name = "id") val id: String = "",
    @Json(name = "description") val description: String = "",
    @Json(name = "package") val packageName: String = "",
    @Json(name = "language") val language: String = "bash",
    @Json(name = "script") val script: String = ""
)

data class ManagedKeyRecord(
    @Json(name = "id") val id: String = "",
    @Json(name = "name") val name: String = "",
    @Json(name = "algorithm") val algorithm: String = "ssh-ed25519",
    @Json(name = "public_key") val publicKey: String = "",
    @Json(name = "source") val source: ManagedKeySource = ManagedKeySource.Generated
)

data class KnownHostRecord(
    @Json(name = "host") val host: String = "",
    @Json(name = "port") val port: Int = 22,
    @Json(name = "algorithm") val algorithm: String = "",
    @Json(name = "fingerprint") val fingerprint: String = "",
    @Json(name = "added_at") val addedAt: Long? = null
)

data class HostRuntimeStatus(
    val online: Boolean = false,
    val latencyMillis: Int? = null,
    val lastConnectedAt: Long? = null
)

/**
 * Business settings owned by the Rust store. Every field here survives a
 * desktop/mobile sync round trip and is restored after a restart.
 */
data class BusinessSettings(
    val language: String = "SimplifiedChinese",
    val recentHostCount: Int = 5,
    val autoMonitoring: Boolean = false,
    val monitorHistory: String = "FiveMinutes",
    val closeLastSessionBehavior: String = "OpenHosts",
    val vaultEnabled: Boolean = false,
    val vaultAutoLockMinutes: Int = 0,
    val onboardingCompleted: Boolean = false,
    val terminalFont: String = "",
    val cjkFallbackFont: String = "",
    val terminalFontSize: Float = 14f,
    val terminalLineHeight: Float = 18f,
    /** Effective light/dark theme; the Rust terminal palette follows it. */
    val themeId: String = "light",
    val seedColor: String = "#2f6fed"
)

/**
 * Device-local preferences. These are not part of the Rust business snapshot:
 * they select Android-only behavior. Sync configuration and credentials live in
 * the Rust stores since P6.
 */
data class DevicePreferences(
    val proxyEnabled: Boolean = false,
    val proxyAddress: String = "",
    val proxyPort: String = "1080",
    val importSource: String = "OpenSsh",
    /**
     * Background keep-alive is on by default: while any SSH/SFTP/forward/AI task
     * is running the foreground service (and its notification) keeps the
     * process alive, so switching to another app does not close connections.
     * It can be turned off from settings.
     */
    val backgroundKeepAlive: Boolean = true,
    /** Ordered keys shown in the terminal's always-visible bottom shortcut row. */
    val terminalQuickKeys: List<String> = DefaultTerminalQuickKeys
)

val DefaultTerminalQuickKeys = listOf("ESC", "TAB", "INTERRUPT", "LEFT", "UP", "DOWN", "RIGHT")

enum class VaultMode {
    Disabled,
    Locked,
    Unlocked
}

// ---------------------------------------------------------------------------
// Sync (P6)
// ---------------------------------------------------------------------------

enum class SyncProviderModel {
    None,
    Gist,
    WebDav
}

enum class SyncCredentialKindModel {
    GithubToken,
    WebdavPassword,
    Passphrase
}

enum class ImportSourceModel {
    OpenSsh,
    Putty,
    SecureCrt,
    FinalShell
}

/** Typed sync runtime phase; progress itself is pulled, never queued. */
enum class SyncPhaseModel {
    Disabled,
    Idle,
    CheckingCapability,
    CheckingRemote,
    Pulling,
    Pushing,
    PausedVaultLocked,
    PausedCredentials,
    PausedCapability,
    Conflict,
    Cancelling;

    val isRunning: Boolean
        get() = this == CheckingCapability || this == CheckingRemote ||
            this == Pulling || this == Pushing || this == Cancelling
}

data class SyncConfigModel(
    val provider: SyncProviderModel = SyncProviderModel.None,
    val gistId: String = "",
    val webdavUrl: String = "",
    val webdavUsername: String = "",
    val hasGithubToken: Boolean = false,
    val hasWebdavPassword: Boolean = false,
    val hasPassphrase: Boolean = false,
    val autoSyncEnabled: Boolean = false,
    val lastSyncAtMillis: Long = 0L,
    val deviceId: String = "",
    val legacyAutoSyncCompat: Boolean = false,
    val webdavUnsafeWriteConsent: Boolean = false,
    val capabilityProbed: Boolean = false
)

data class SyncCapabilityModel(
    val state: String = "Unchecked",
    val reason: String? = null,
    val step: String = "",
    val httpStatus: Int? = null,
    val cleanupPending: Boolean = false
)

data class SyncFailureModel(
    val kind: String,
    val detail: String
)

sealed interface SyncStatusModel {
    data object Idle : SyncStatusModel
    data object RemoteBindingRequired : SyncStatusModel
    data class Pulled(val atMillis: Long) : SyncStatusModel
    data class Pushed(val atMillis: Long) : SyncStatusModel
    data class UpToDate(val atMillis: Long) : SyncStatusModel
    data class PullRequired(val remoteAtMillis: Long?, val reason: String) : SyncStatusModel
    data object Cancelled : SyncStatusModel
    data class Failed(val failure: SyncFailureModel) : SyncStatusModel
}

data class SyncConflictModel(
    val remoteAtMillis: Long?,
    val reason: String
)

data class SyncStateModel(
    val phase: SyncPhaseModel = SyncPhaseModel.Disabled,
    val status: SyncStatusModel = SyncStatusModel.Idle,
    val config: SyncConfigModel = SyncConfigModel(),
    val capability: SyncCapabilityModel = SyncCapabilityModel(),
    val vaultLocked: Boolean = false,
    val pendingConflict: SyncConflictModel? = null,
    val lastFailure: SyncFailureModel? = null,
    val lastAutoCheckAtMillis: Long = 0L,
    /**
     * `true` only after the backend snapshot has been read at least once.
     * The default state must never drive destructive side effects (for example
     * cancelling the persisted periodic sync job during a cold start).
     */
    val loaded: Boolean = false
)

/** Field-update request; `null` keeps the stored value. */
data class SyncConfigEdit(
    val provider: SyncProviderModel? = null,
    val gistId: String? = null,
    val webdavUrl: String? = null,
    val webdavUsername: String? = null,
    val autoSyncEnabled: Boolean? = null,
    val webdavUnsafeWriteConsent: Boolean? = null
)

data class ImportEntryModel(
    val name: String,
    val group: String,
    val host: String,
    val port: Int,
    val username: String,
    val authMethod: String,
    val privateKeyPath: String,
    val certificatePath: String,
    val hasPassword: Boolean,
    val hasPassphrase: Boolean,
    val agentForwarding: Boolean,
    val startupCommand: String
)

data class ImportWarningModel(
    val kind: String,
    val entryName: String,
    val reason: String
)

data class ImportPreviewModel(
    val previewId: Long,
    val sourceLabel: String,
    val entries: List<ImportEntryModel>,
    val warnings: List<ImportWarningModel>
)

data class VaultStatus(
    val mode: VaultMode = VaultMode.Disabled,
    val filePresent: Boolean = false,
    val busy: Boolean = false,
    val error: String? = null
)

/** Explicit repository state so pages can show loading and failure instead of empty data. */
sealed interface RepositoryStatus {
    data object Initializing : RepositoryStatus
    data object Ready : RepositoryStatus
    data class Failed(val reason: String) : RepositoryStatus
}

/** Credential edit coming from the profile editor. */
sealed interface CredentialUpdate {
    data object Keep : CredentialUpdate
    data class Set(val value: String) : CredentialUpdate
    data object Clear : CredentialUpdate
}

enum class CredentialHealth {
    Missing,
    Present,
    Locked,
    Invalidated,
    Corrupt
}


data class AppPreferences(
    val onboardingCompleted: Boolean = false,
    val recentHostCount: Int = 5,
    val autoMonitoring: Boolean = false,
    val monitorHistory: String = "FiveMinutes",
    val closeLastSessionBehavior: String = "OpenHosts",
    val proxyEnabled: Boolean = false,
    val proxyAddress: String = "",
    val proxyPort: String = "1080",
    val importSource: String = "OpenSsh",
    val biometricEnabled: Boolean = true,
    val vaultAutoLockMinutes: Int = 0,
    val vaultEnabled: Boolean = false,
    val language: String = "SimplifiedChinese",
    val terminalFont: String = "",
    val cjkFallbackFont: String = "",
    val terminalFontSize: Float = 14f,
    val terminalLineHeight: Float = 18f
)

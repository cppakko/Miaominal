package dev.akko.miaominal.data

import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow

/**
 * Page-facing data contract.
 *
 * Screens never talk to the native backend directly: the real implementation
 * ([NativeMiaominalRepository]) refreshes snapshots from the Rust backend, and
 * tests/Previews inject [InMemoryMiaominalRepository]. Every mutation returns a
 * [Result] so a page can keep showing the previous state instead of a false
 * success.
 */
interface MiaominalRepository {
    val transferActions: SftpTransferActions

    val backendState: StateFlow<NativeBackendState>
    val status: StateFlow<RepositoryStatus>

    val hosts: StateFlow<List<SessionProfile>>
    val hostStatus: StateFlow<Map<String, HostRuntimeStatus>>
    val forwards: StateFlow<List<PortForwardRule>>
    val proxies: StateFlow<List<ProxyProfile>>
    val snippets: StateFlow<List<SnippetRecord>>
    val managedKeys: StateFlow<List<ManagedKeyRecord>>
    val knownHosts: StateFlow<List<KnownHostRecord>>
    val settings: StateFlow<BusinessSettings>
    val devicePreferences: StateFlow<DevicePreferences>
    val vault: StateFlow<VaultStatus>

    /** Live and recently ended SSH sessions owned by the native backend. */
    val sessions: StateFlow<List<SessionSnapshotModel>>

    /**
     * Live and recently ended forwarding sessions owned by the native backend,
     * keyed by one rule of one host. A rebuilt page restores start/stop state
     * and pending trust/authentication requests from here.
     */
    val forwardSessions: StateFlow<List<ForwardSessionModel>>

    /**
     * Live and recently ended SFTP sessions owned by the native backend.
     * Transfers themselves are pulled from the session handle snapshot, so a
     * rebuilt page restores the list from the backend instead of local state.
     */
    val sftpSessions: StateFlow<List<SftpSessionModel>>

    /**
     * Transfers that were still running when the native backend was replaced
     * (process restart or explicit restart). They are reported as interrupted
     * instead of pretending they survived.
     */
    val interruptedSftpTransfers: StateFlow<List<SftpTransferModel>>

    /** Non-fatal failures (save/delete) that the UI should surface once. */
    val notices: SharedFlow<String>

    fun start()

    suspend fun ensureStarted(): Result<Unit>
    suspend fun refreshAll(): Result<Unit>

    /** Open a real SSH session for a saved host; fails before any connection. */
    suspend fun openSession(hostId: String, columns: Int, lines: Int): Result<TerminalSessionHandle>

    /** Close a session (idempotent) and drop it from the session list. */
    suspend fun closeSession(connectionId: Long): Result<Unit>

    /** Live handle for a session id, or `null` when it is unknown. */
    fun sessionHandle(connectionId: Long): TerminalSessionHandle?

    /** Re-read session snapshots (used after answering a pending request). */
    suspend fun refreshSessions()

    /**
     * Close every SSH, SFTP, and forwarding session without replacing the
     * backend handle. Used by the background policy after the grace period and
     * by the foreground-service "stop tasks" action.
     */
    suspend fun closeAllSessions(): Result<Unit>

    /** Open a real SFTP session for a saved host; fails before any connection. */
    suspend fun openSftpSession(hostId: String): Result<SftpSessionHandle>

    /** Close an SFTP session (idempotent) and drop it from the session list. */
    suspend fun closeSftpSession(connectionId: Long): Result<Unit>

    /** Live handle for an SFTP session id, or `null` when it is unknown. */
    fun sftpSessionHandle(connectionId: Long): SftpSessionHandle?

    /** Re-read SFTP session snapshots (used after answering a pending request). */
    suspend fun refreshSftpSessions()

    /**
     * Register a document chosen through SAF for upload. Kotlin opens the
     * descriptor, Rust duplicates it (close-on-exec) before returning, and the
     * returned handle owns the duplicate. Runs off the main thread.
     */
    suspend fun registerUploadFile(uri: String, name: String): Result<LocalUploadHandle>

    /** Persist the read/write permission for a SAF tree picked by the user. */
    suspend fun persistSafTreePermission(uri: String): Boolean

    /**
     * List one directory of an authorized SAF tree for the local pane. The
     * implementation moves the provider query off the main thread.
     */
    suspend fun listSafDirectory(treeUri: String, relativePath: String): Result<List<SafLocalEntry>>

    /** Display name of an authorized tree root (falls back to the URI). */
    fun safTreeLabel(treeUri: String): String

    /**
     * Keep a transfer handle alive in the application scope so a rebuilt page
     * can still pause, resume, or cancel the transfer.
     */
    fun trackSftpTransfer(connectionId: Long, handle: SftpTransferHandle)

    /** The tracked handle for one transfer, or `null` when it is unknown. */
    fun sftpTransferHandle(connectionId: Long, transferId: Long): SftpTransferHandle?

    suspend fun saveHost(profile: SessionProfile, password: CredentialUpdate): Result<SessionProfile>
    suspend fun deleteHost(id: String): Result<Unit>
    suspend fun toggleFavorite(id: String): Result<Unit>

    suspend fun hostPasswordHealth(id: String): CredentialHealth

    suspend fun saveSnippet(snippet: SnippetRecord): Result<SnippetRecord>
    suspend fun deleteSnippet(id: String): Result<Unit>

    suspend fun saveForward(rule: PortForwardRule): Result<PortForwardRule>
    suspend fun deleteForward(rule: PortForwardRule): Result<Unit>

    /**
     * Desktop parity for the rule switch (`set_port_forward_rule_enabled`):
     * enabling starts the dedicated forwarding session and stores the rule as
     * enabled; disabling stops the session and clears the stored flag. Starting
     * an already running rule is idempotent.
     */
    suspend fun setForwardEnabled(rule: PortForwardRule, enabled: Boolean): Result<Unit>

    /**
     * Start a dedicated forwarding session for a saved rule. Starting a rule
     * that is already running returns the existing handle instead of opening a
     * second connection. A successful start also stores the rule as enabled.
     */
    suspend fun startForward(rule: PortForwardRule): Result<ForwardSessionHandle>

    /** Stop one forwarding session (idempotent) and clear the stored enabled flag. */
    suspend fun stopForward(profileId: String, ruleId: String): Result<Unit>

    /** Live handle for one forwarding rule, or `null` when it is unknown. */
    fun forwardSessionHandle(profileId: String, ruleId: String): ForwardSessionHandle?

    /** Re-read forwarding snapshots (used after answering a pending request). */
    suspend fun refreshForwardSessions()

    suspend fun saveProxy(proxy: ProxyProfile, password: CredentialUpdate): Result<ProxyProfile>
    suspend fun deleteProxy(id: String): Result<Unit>
    suspend fun proxyPasswordHealth(id: String): CredentialHealth

    /** Replace one host's reusable entry proxy and ProxyJump chain. */
    suspend fun saveHostRouting(
        profileId: String,
        entryProxyId: String,
        proxyJumpProfileIds: List<String>
    ): Result<Unit>

    suspend fun saveKey(key: ManagedKeyRecord): Result<ManagedKeyRecord>
    suspend fun generateKey(name: String, algorithm: String): Result<ManagedKeyRecord>
    suspend fun importKey(name: String, privateKey: String, passphrase: String?): Result<ManagedKeyRecord>
    suspend fun deleteKey(id: String): Result<Unit>

    suspend fun deleteKnownHost(host: String, port: Int): Result<Unit>

    suspend fun updateSettings(transform: (BusinessSettings) -> BusinessSettings): Result<Unit>
    suspend fun updateDevicePreferences(transform: (DevicePreferences) -> DevicePreferences)

    suspend fun vaultEnable(passphrase: String): Result<Unit>
    suspend fun vaultDisable(): Result<Unit>
    suspend fun vaultLock(): Result<Unit>
    suspend fun vaultUnlock(passphrase: String): Result<Unit>
    suspend fun vaultChangePassphrase(current: String, new: String): Result<Unit>

    /** Close running native work, delete business files, then device credentials. */
    suspend fun resetLocalData(): Result<Unit>

    /**
     * Sync state owned by the Rust backend: phase, last result, capability and
     * pending conflict. Progress is pulled from here, never queued.
     */
    val sync: StateFlow<SyncStateModel>

    suspend fun refreshSync()

    suspend fun updateSyncConfig(edit: SyncConfigEdit): Result<Unit>

    /** `Keep / Set / Clear` semantics; the stored value is never read back. */
    suspend fun setSyncCredential(kind: SyncCredentialKindModel, update: CredentialUpdate): Result<Unit>

    suspend fun syncCredentialHealth(kind: SyncCredentialKindModel): CredentialHealth

    /** Copy legacy device entries into the backend credential store once. */
    suspend fun syncMigrateLegacyCredentials(): Result<Unit>

    suspend fun syncCheckCapability(): Result<Unit>

    suspend fun syncPull(force: Boolean): Result<Unit>

    suspend fun syncPush(manual: Boolean): Result<Unit>

    /** Explicit conflict resolution; never applied automatically. */
    suspend fun syncResolveConflict(keepLocal: Boolean): Result<Unit>

    /** Explicit cancel of the in-flight sync operation. */
    fun syncCancel()

    /** Foreground/WorkManager automatic check (shares the execution lock). */
    suspend fun syncAutoCheck(): Result<Unit>

    suspend fun syncSetAutoSync(enabled: Boolean): Result<Unit>

    /**
     * Parse one user-selected document (already read to bytes; SAF URIs are
     * never treated as paths) and keep it until confirmed or discarded.
     */
    suspend fun previewImport(
        source: ImportSourceModel,
        fileName: String,
        bytes: ByteArray
    ): Result<ImportPreviewModel>

    suspend fun commitImport(previewId: Long): Result<Int>

    suspend fun discardImport(previewId: Long)
}

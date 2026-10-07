package dev.akko.miaominal.data

import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import java.util.UUID

/**
 * Test/Preview repository. It never touches the native backend, so UI tests can
 * seed deterministic data without executing Rust.
 */
class InMemoryMiaominalRepository(
    hosts: List<SessionProfile> = emptyList(),
    statuses: Map<String, HostRuntimeStatus> = emptyMap(),
    forwards: List<PortForwardRule> = emptyList(),
    snippets: List<SnippetRecord> = emptyList(),
    keys: List<ManagedKeyRecord> = emptyList(),
    knownHosts: List<KnownHostRecord> = emptyList(),
    settings: BusinessSettings = BusinessSettings(onboardingCompleted = true),
    vault: VaultStatus = VaultStatus(),
    vaultPassphrase: String = "preview-passphrase"
) : MiaominalRepository {

    private val _status = MutableStateFlow<RepositoryStatus>(RepositoryStatus.Ready)
    override val transferActions = SftpTransferActions(this)
    override val status: StateFlow<RepositoryStatus> = _status.asStateFlow()

    private val _hosts = MutableStateFlow(hosts)
    override val hosts: StateFlow<List<SessionProfile>> = _hosts.asStateFlow()

    private val _hostStatus = MutableStateFlow(statuses)
    override val hostStatus: StateFlow<Map<String, HostRuntimeStatus>> = _hostStatus.asStateFlow()

    private val _forwards = MutableStateFlow(forwards)
    override val forwards: StateFlow<List<PortForwardRule>> = _forwards.asStateFlow()

    private val _proxies = MutableStateFlow<List<ProxyProfile>>(emptyList())
    override val proxies: StateFlow<List<ProxyProfile>> = _proxies.asStateFlow()

    private val _forwardSessions = MutableStateFlow<List<ForwardSessionModel>>(emptyList())
    override val forwardSessions: StateFlow<List<ForwardSessionModel>> = _forwardSessions.asStateFlow()

    private val _snippets = MutableStateFlow(snippets)
    override val snippets: StateFlow<List<SnippetRecord>> = _snippets.asStateFlow()

    private val _managedKeys = MutableStateFlow(keys)
    override val managedKeys: StateFlow<List<ManagedKeyRecord>> = _managedKeys.asStateFlow()

    private val _knownHosts = MutableStateFlow(knownHosts)
    override val knownHosts: StateFlow<List<KnownHostRecord>> = _knownHosts.asStateFlow()

    private val _settings = MutableStateFlow(settings)
    override val settings: StateFlow<BusinessSettings> = _settings.asStateFlow()

    private val _devicePreferences = MutableStateFlow(DevicePreferences())
    override val devicePreferences: StateFlow<DevicePreferences> =
        _devicePreferences.asStateFlow()

    private val _vault = MutableStateFlow(vault)
    override val vault: StateFlow<VaultStatus> = _vault.asStateFlow()
    private var masterPassphrase: String? = vaultPassphrase.takeIf { vault.mode != VaultMode.Disabled }

    private val _sessions = MutableStateFlow<List<SessionSnapshotModel>>(emptyList())
    override val sessions: StateFlow<List<SessionSnapshotModel>> = _sessions.asStateFlow()

    private val _sftpSessions = MutableStateFlow<List<SftpSessionModel>>(emptyList())
    override val sftpSessions: StateFlow<List<SftpSessionModel>> = _sftpSessions.asStateFlow()

    private val _interruptedSftpTransfers = MutableStateFlow<List<SftpTransferModel>>(emptyList())
    override val interruptedSftpTransfers: StateFlow<List<SftpTransferModel>> =
        _interruptedSftpTransfers.asStateFlow()

    private val sessionHandles = linkedMapOf<Long, FakeTerminalSession>()
    private val sftpHandles = linkedMapOf<Long, FakeSftpSession>()
    private var nextSftpConnectionId = 1L

    /** Local entries returned by the fake SAF pane. */
    var safLocalEntries: List<SafLocalEntry> = emptyList()
    private var nextConnectionId = 1L

    private val _notices = MutableSharedFlow<String>(extraBufferCapacity = 16)
    override val notices: SharedFlow<String> = _notices.asSharedFlow()

    override val backendState: StateFlow<NativeBackendState> =
        MutableStateFlow<NativeBackendState>(NativeBackendState.Stopped).asStateFlow()

    private val passwords = mutableMapOf<String, String>()
    private val proxyPasswords = mutableMapOf<String, String>()
    private val secrets = mutableMapOf<String, String>()

    /** Seed a fake session that is already waiting on a trust prompt. */
    fun seedSession(
        hostId: String,
        phase: SessionPhase = SessionPhase.Ready
    ): FakeTerminalSession {
        val session = FakeTerminalSession(
            connectionId = nextConnectionId++,
            hostId = hostId,
            phase = phase
        )
        sessionHandles[session.connectionId] = session
        refreshSessionSnapshots()
        return session
    }

    fun seedHosts(hosts: List<SessionProfile>, statuses: Map<String, HostRuntimeStatus>) {
        _hosts.value = hosts
        _hostStatus.value = statuses
    }

    fun seedForwards(forwards: List<PortForwardRule>) {
        _forwards.value = forwards
    }

    fun seedProxies(proxies: List<ProxyProfile>) {
        _proxies.value = proxies
    }

    fun seedSnippets(snippets: List<SnippetRecord>) {
        _snippets.value = snippets
    }

    fun seedKeys(keys: List<ManagedKeyRecord>) {
        _managedKeys.value = keys
    }

    fun seedKnownHosts(knownHosts: List<KnownHostRecord>) {
        _knownHosts.value = knownHosts
    }

    override fun start() = Unit

    override suspend fun ensureStarted(): Result<Unit> = Result.success(Unit)

    override suspend fun refreshAll(): Result<Unit> = Result.success(Unit)

    override suspend fun openSession(
        hostId: String,
        columns: Int,
        lines: Int
    ): Result<TerminalSessionHandle> = runCatching {
        val session = FakeTerminalSession(
            connectionId = nextConnectionId++,
            hostId = hostId,
            columns = columns,
            lines = lines,
            phase = SessionPhase.Ready
        )
        sessionHandles[session.connectionId] = session
        refreshSessionSnapshots()
        session
    }

    override suspend fun closeSession(connectionId: Long): Result<Unit> = runCatching {
        sessionHandles.remove(connectionId)?.cancel()
        refreshSessionSnapshots()
    }

    override fun sessionHandle(connectionId: Long): TerminalSessionHandle? =
        sessionHandles[connectionId]

    override suspend fun refreshSessions() {
        refreshSessionSnapshots()
    }

    /** Refresh snapshots after a test mutated a fake session. */
    fun refreshSessionSnapshots() {
        _sessions.value = sessionHandles.values.map { it.snapshot() }
    }

    override suspend fun openSftpSession(hostId: String): Result<SftpSessionHandle> = runCatching {
        // Reuse a seeded session for the same host so a UI test can prepare
        // state (tree, pending request, transfers) before the screen opens.
        sftpHandles.values.firstOrNull { it.hostId == hostId && !it.snapshot().phase.isTerminal } ?: run {
            val session = FakeSftpSession(
                connectionId = nextSftpConnectionId++,
                hostId = hostId
            )
            sftpHandles[session.connectionId] = session
            refreshSftpSnapshots()
            session
        }
    }

    /** Pre-create a fake SFTP session for a UI test before the screen opens. */
    fun seedSftpSession(hostId: String, phase: SftpPhase = SftpPhase.Ready): FakeSftpSession {
        val session = FakeSftpSession(
            connectionId = nextSftpConnectionId++,
            hostId = hostId,
            phase = phase
        )
        sftpHandles[session.connectionId] = session
        refreshSftpSnapshots()
        return session
    }

    override suspend fun closeSftpSession(connectionId: Long): Result<Unit> = runCatching {
        sftpHandles.remove(connectionId)?.disconnect()
        refreshSftpSnapshots()
    }

    override fun sftpSessionHandle(connectionId: Long): SftpSessionHandle? =
        sftpHandles[connectionId]

    override suspend fun refreshSftpSessions() {
        refreshSftpSnapshots()
    }

    /** Refresh SFTP snapshots after a test mutated a fake session. */
    fun refreshSftpSnapshots() {
        _sftpSessions.value = sftpHandles.values.map { it.snapshot() }
    }

    override suspend fun registerUploadFile(
        uri: String,
        name: String
    ): Result<LocalUploadHandle> =
        Result.success(object : LocalUploadHandle {
            override val name: String = name
            override fun close() = Unit
        })

    override suspend fun persistSafTreePermission(uri: String): Boolean = true

    override suspend fun listSafDirectory(
        treeUri: String,
        relativePath: String
    ): Result<List<SafLocalEntry>> = Result.success(safLocalEntries)

    override fun safTreeLabel(treeUri: String): String =
        dev.akko.miaominal.data.safTreeLabel(treeUri)

    private val sftpTransferHandles = linkedMapOf<String, SftpTransferHandle>()

    override fun trackSftpTransfer(connectionId: Long, handle: SftpTransferHandle) {
        sftpTransferHandles["$connectionId:${handle.transferId}"] = handle
    }

    override fun sftpTransferHandle(connectionId: Long, transferId: Long): SftpTransferHandle? =
        sftpTransferHandles["$connectionId:$transferId"]

    override suspend fun saveHost(
        profile: SessionProfile,
        password: CredentialUpdate
    ): Result<SessionProfile> = runCatching {
        val id = profile.id.ifBlank { "host-${UUID.randomUUID()}" }
        val saved = profile.copy(id = id)
        _hosts.value = _hosts.value.filterNot { it.id == id } + saved
        when (password) {
            CredentialUpdate.Keep -> Unit
            is CredentialUpdate.Set -> passwords[id] = password.value
            CredentialUpdate.Clear -> passwords.remove(id)
        }
        saved
    }

    override suspend fun deleteHost(id: String): Result<Unit> = runCatching {
        _hosts.value = _hosts.value.filterNot { it.id == id }
        _forwards.value = _forwards.value.filterNot { it.profileId == id }
        passwords.remove(id)
    }

    override suspend fun toggleFavorite(id: String): Result<Unit> = runCatching {
        _hosts.value = _hosts.value.map {
            if (it.id == id) it.copy(isFavorite = !it.isFavorite) else it
        }
    }

    override suspend fun hostPasswordHealth(id: String): CredentialHealth =
        if (passwords.containsKey(id)) CredentialHealth.Present else CredentialHealth.Missing

    override suspend fun saveSnippet(snippet: SnippetRecord): Result<SnippetRecord> =
        runCatching {
            val id = snippet.id.ifBlank { "snippet-${UUID.randomUUID()}" }
            val saved = snippet.copy(id = id)
            _snippets.value = _snippets.value.filterNot { it.id == id } + saved
            saved
        }

    override suspend fun deleteSnippet(id: String): Result<Unit> = runCatching {
        _snippets.value = _snippets.value.filterNot { it.id == id }
    }

    override suspend fun saveForward(rule: PortForwardRule): Result<PortForwardRule> =
        runCatching {
            val id = rule.id.ifBlank { "forward-${UUID.randomUUID()}" }
            val saved = rule.copy(id = id)
            _forwards.value = _forwards.value.filterNot {
                it.profileId == rule.profileId && it.id == id
            } + saved
            saved
        }

    override suspend fun deleteForward(rule: PortForwardRule): Result<Unit> = runCatching {
        _forwards.value = _forwards.value.filterNot {
            it.profileId == rule.profileId && it.id == rule.id
        }
    }

    override suspend fun setForwardEnabled(
        rule: PortForwardRule,
        enabled: Boolean
    ): Result<Unit> = if (enabled) {
        // Desktop parity for the rule switch: enabling starts the session.
        startForward(rule).map { }
    } else {
        stopForward(rule.profileId, rule.id)
    }

    private fun markForwardEnabled(profileId: String, ruleId: String, enabled: Boolean) {
        _forwards.value = _forwards.value.map {
            if (it.profileId == profileId && it.id == ruleId) it.copy(enabled = enabled) else it
        }
    }

    private val forwardHandles = linkedMapOf<String, FakeForwardSession>()

    /** Pre-create a fake forwarding session for a UI test. */
    fun seedForwardSession(
        profileId: String,
        ruleId: String,
        phase: ForwardPhaseModel = ForwardPhaseModel.Listening,
        boundPort: Int? = null,
        label: String = ""
    ): FakeForwardSession {
        val session = FakeForwardSession(
            connectionId = (forwardHandles.size + 1).toLong(),
            profileId = profileId,
            ruleId = ruleId,
            label = label.ifBlank { ruleId },
            phase = phase,
            requestedPort = boundPort ?: 0
        )
        session.boundPort = boundPort
        forwardHandles[forwardKey(profileId, ruleId)] = session
        refreshForwardSnapshots()
        return session
    }

    override suspend fun startForward(rule: PortForwardRule): Result<ForwardSessionHandle> =
        runCatching {
            val key = forwardKey(rule.profileId, rule.id)
            forwardHandles[key]?.takeIf { it.snapshot().phase.isActive }?.let { existing ->
                markForwardEnabled(rule.profileId, rule.id, true)
                return@runCatching existing
            }
            val session = FakeForwardSession(
                connectionId = (forwardHandles.size + 1).toLong(),
                profileId = rule.profileId,
                ruleId = rule.id,
                label = rule.label.ifBlank { rule.id },
                kind = rule.kind,
                listenHost = rule.listenHost,
                requestedPort = rule.listenPort,
                phase = ForwardPhaseModel.Listening
            )
            session.boundPort = rule.listenPort.takeIf { it != 0 } ?: 20_000 + forwardHandles.size
            forwardHandles[key] = session
            markForwardEnabled(rule.profileId, rule.id, true)
            refreshForwardSnapshots()
            session
        }

    override suspend fun stopForward(profileId: String, ruleId: String): Result<Unit> = runCatching {
        forwardHandles.remove(forwardKey(profileId, ruleId))?.cancel()
        markForwardEnabled(profileId, ruleId, false)
        refreshForwardSnapshots()
    }

    override fun forwardSessionHandle(profileId: String, ruleId: String): ForwardSessionHandle? =
        forwardHandles[forwardKey(profileId, ruleId)]

    override suspend fun refreshForwardSessions() {
        refreshForwardSnapshots()
    }

    /** Refresh forwarding snapshots after a test mutated a fake session. */
    fun refreshForwardSnapshots() {
        _forwardSessions.value = forwardHandles.values.map { it.snapshot() }
    }

    private fun forwardKey(profileId: String, ruleId: String): String = "$profileId:$ruleId"

    override suspend fun saveProxy(
        proxy: ProxyProfile,
        password: CredentialUpdate
    ): Result<ProxyProfile> = runCatching {
        val id = proxy.id.ifBlank { "proxy-${UUID.randomUUID()}" }
        val saved = proxy.copy(id = id)
        _proxies.value = _proxies.value.filterNot { it.id == id } + saved
        when (password) {
            CredentialUpdate.Keep -> Unit
            is CredentialUpdate.Set -> proxyPasswords[id] = password.value
            CredentialUpdate.Clear -> proxyPasswords.remove(id)
        }
        saved
    }

    override suspend fun deleteProxy(id: String): Result<Unit> = runCatching {
        _proxies.value = _proxies.value.filterNot { it.id == id }
        proxyPasswords.remove(id)
        _hosts.value = _hosts.value.map {
            if (it.entryProxyId == id) it.copy(entryProxyId = "") else it
        }
    }

    override suspend fun proxyPasswordHealth(id: String): CredentialHealth =
        if (proxyPasswords.containsKey(id)) CredentialHealth.Present else CredentialHealth.Missing

    override suspend fun saveHostRouting(
        profileId: String,
        entryProxyId: String,
        proxyJumpProfileIds: List<String>
    ): Result<Unit> = runCatching {
        require(entryProxyId.isBlank() || _proxies.value.any { it.id == entryProxyId }) {
            "proxy $entryProxyId does not exist"
        }
        require(profileId !in proxyJumpProfileIds) {
            "the jump chain cannot reference the host being connected"
        }
        require(proxyJumpProfileIds.distinct().size == proxyJumpProfileIds.size) {
            "the jump chain cannot include the same host twice"
        }
        require(proxyJumpProfileIds.all { id -> _hosts.value.any { it.id == id } }) {
            "a jump host does not exist"
        }
        _hosts.value = _hosts.value.map {
            if (it.id == profileId) {
                it.copy(entryProxyId = entryProxyId, proxyJumpProfileIds = proxyJumpProfileIds)
            } else {
                it
            }
        }
    }

    override suspend fun closeAllSessions(): Result<Unit> = runCatching {
        sessionHandles.values.forEach { it.cancel() }
        sessionHandles.clear()
        sftpHandles.values.forEach { it.disconnect() }
        sftpHandles.clear()
        forwardHandles.values.forEach { it.cancel() }
        refreshSessionSnapshots()
        refreshSftpSnapshots()
        refreshForwardSnapshots()
    }

    override suspend fun saveKey(key: ManagedKeyRecord): Result<ManagedKeyRecord> =
        runCatching {
            _managedKeys.value = _managedKeys.value.map {
                if (it.id == key.id) key else it
            }
            key
        }

    override suspend fun generateKey(name: String, algorithm: String): Result<ManagedKeyRecord> =
        runCatching {
            val key = ManagedKeyRecord(
                id = "managed-key-${UUID.randomUUID()}",
                name = name.ifBlank { "generated key" },
                algorithm = if (algorithm.contains("rsa", ignoreCase = true)) {
                    "ssh-rsa"
                } else {
                    "ssh-ed25519"
                },
                publicKey = "ssh-ed25519 AAAATestGeneratedKey",
                source = ManagedKeySource.Generated
            )
            _managedKeys.value = _managedKeys.value + key
            key
        }

    override suspend fun importKey(
        name: String,
        privateKey: String,
        passphrase: String?
    ): Result<ManagedKeyRecord> = runCatching {
        val key = ManagedKeyRecord(
            id = "managed-key-${UUID.randomUUID()}",
            name = name.ifBlank { "imported key" },
            algorithm = "ssh-ed25519",
            publicKey = "ssh-ed25519 AAAATestImportedKey",
            source = ManagedKeySource.Imported
        )
        _managedKeys.value = _managedKeys.value + key
        key
    }

    override suspend fun deleteKey(id: String): Result<Unit> = runCatching {
        _managedKeys.value = _managedKeys.value.filterNot { it.id == id }
        _hosts.value = _hosts.value.map {
            if (it.managedKeyId == id) it.copy(managedKeyId = "") else it
        }
    }

    override suspend fun deleteKnownHost(host: String, port: Int): Result<Unit> = runCatching {
        _knownHosts.value = _knownHosts.value.filterNot { it.host == host && it.port == port }
    }

    override suspend fun updateSettings(
        transform: (BusinessSettings) -> BusinessSettings
    ): Result<Unit> = runCatching {
        _settings.value = transform(_settings.value)
    }

    override suspend fun updateDevicePreferences(
        transform: (DevicePreferences) -> DevicePreferences
    ) {
        _devicePreferences.value = transform(_devicePreferences.value)
    }

    override suspend fun vaultEnable(passphrase: String): Result<Unit> = runCatching {
        check(_vault.value.mode == VaultMode.Disabled) { "Vault already enabled" }
        require(passphrase.isNotBlank()) { "Passphrase required" }
        masterPassphrase = passphrase
        _vault.value = VaultStatus(mode = VaultMode.Unlocked, filePresent = true)
        _settings.value = _settings.value.copy(vaultEnabled = true)
    }

    override suspend fun vaultDisable(): Result<Unit> = runCatching {
        check(_vault.value.mode == VaultMode.Unlocked) { "Unlock the vault first" }
        masterPassphrase = null
        _vault.value = VaultStatus()
        _settings.value = _settings.value.copy(vaultEnabled = false)
    }

    override suspend fun vaultLock(): Result<Unit> = runCatching {
        check(_vault.value.mode != VaultMode.Disabled) { "Vault disabled" }
        _vault.value = _vault.value.copy(mode = VaultMode.Locked)
    }

    override suspend fun vaultUnlock(passphrase: String): Result<Unit> = runCatching {
        check(_vault.value.mode != VaultMode.Disabled) { "Vault disabled" }
        require(passphrase == masterPassphrase) { "Incorrect passphrase" }
        _vault.value = _vault.value.copy(mode = VaultMode.Unlocked)
    }

    override suspend fun vaultChangePassphrase(current: String, new: String): Result<Unit> = runCatching {
        check(_vault.value.mode == VaultMode.Unlocked) { "Unlock the vault first" }
        require(current == masterPassphrase) { "Incorrect passphrase" }
        require(new.isNotBlank()) { "Passphrase required" }
        masterPassphrase = new
    }

    override suspend fun resetLocalData(): Result<Unit> = runCatching {
        _hosts.value = emptyList()
        _hostStatus.value = emptyMap()
        _forwards.value = emptyList()
        _proxies.value = emptyList()
        _snippets.value = emptyList()
        _managedKeys.value = emptyList()
        _knownHosts.value = emptyList()
        _settings.value = BusinessSettings()
        _vault.value = VaultStatus()
        masterPassphrase = null
        _sync.value = SyncStateModel(loaded = true)
        sftpHandles.clear()
        sftpTransferHandles.clear()
        forwardHandles.clear()
        _sftpSessions.value = emptyList()
        _forwardSessions.value = emptyList()
        passwords.clear()
        proxyPasswords.clear()
        secrets.clear()
    }

    // ------------------------------------------------------------------
    // Sync (P6): deterministic in-memory behaviour for UI tests
    // ------------------------------------------------------------------

    private val _sync = MutableStateFlow(SyncStateModel(loaded = true))
    override val sync: StateFlow<SyncStateModel> = _sync.asStateFlow()

    /** Test/PV hook: publish a sync state without executing Rust. */
    fun seedSync(state: SyncStateModel) {
        _sync.value = state.copy(loaded = true)
    }

    /** Test/PV hook: seed a parsed import preview. */
    fun seedImportPreview(preview: ImportPreviewModel) {
        previews[preview.previewId] = preview
    }

    private val syncSecrets = mutableMapOf<SyncCredentialKindModel, String>()
    private var previewIdSeed = 1L
    private val previews = mutableMapOf<Long, ImportPreviewModel>()

    override suspend fun refreshSync() = Unit

    override suspend fun updateSyncConfig(edit: SyncConfigEdit): Result<Unit> = runCatching {
        val config = _sync.value.config
        _sync.value = _sync.value.copy(
            phase = when {
                edit.provider == SyncProviderModel.None -> SyncPhaseModel.Disabled
                edit.provider != null && _sync.value.phase == SyncPhaseModel.Disabled -> SyncPhaseModel.Idle
                else -> _sync.value.phase
            },
            config = config.copy(
                provider = edit.provider ?: config.provider,
                gistId = edit.gistId ?: config.gistId,
                webdavUrl = edit.webdavUrl ?: config.webdavUrl,
                webdavUsername = edit.webdavUsername ?: config.webdavUsername,
                autoSyncEnabled = edit.autoSyncEnabled ?: config.autoSyncEnabled,
                webdavUnsafeWriteConsent =
                    edit.webdavUnsafeWriteConsent ?: config.webdavUnsafeWriteConsent
            )
        )
    }

    override suspend fun setSyncCredential(
        kind: SyncCredentialKindModel,
        update: CredentialUpdate
    ): Result<Unit> = runCatching {
        when (update) {
            CredentialUpdate.Keep -> Unit
            is CredentialUpdate.Set -> syncSecrets[kind] = update.value
            CredentialUpdate.Clear -> syncSecrets.remove(kind)
        }
        val config = _sync.value.config
        _sync.value = _sync.value.copy(
            config = config.copy(
                hasGithubToken = when (kind) {
                    SyncCredentialKindModel.GithubToken -> syncSecrets[kind] != null
                    else -> config.hasGithubToken
                },
                hasWebdavPassword = when (kind) {
                    SyncCredentialKindModel.WebdavPassword -> syncSecrets[kind] != null
                    else -> config.hasWebdavPassword
                },
                hasPassphrase = when (kind) {
                    SyncCredentialKindModel.Passphrase -> syncSecrets[kind] != null
                    else -> config.hasPassphrase
                }
            )
        )
    }

    override suspend fun syncCredentialHealth(kind: SyncCredentialKindModel): CredentialHealth =
        if (syncSecrets[kind] != null) CredentialHealth.Present else CredentialHealth.Missing

    override suspend fun syncMigrateLegacyCredentials(): Result<Unit> =
        Result.success(Unit)

    override suspend fun syncCheckCapability(): Result<Unit> = runCatching {
        _sync.value = _sync.value.copy(
            phase = SyncPhaseModel.Idle,
            config = _sync.value.config.copy(capabilityProbed = true),
            capability = SyncCapabilityModel(state = "SUPPORTED")
        )
    }

    override suspend fun syncPull(force: Boolean): Result<Unit> = runCatching {
        _sync.value = _sync.value.copy(
            phase = SyncPhaseModel.Idle,
            status = SyncStatusModel.Pulled(System.currentTimeMillis())
        )
    }

    override suspend fun syncPush(manual: Boolean): Result<Unit> = runCatching {
        _sync.value = _sync.value.copy(
            phase = SyncPhaseModel.Idle,
            status = SyncStatusModel.Pushed(System.currentTimeMillis())
        )
    }

    override suspend fun syncResolveConflict(keepLocal: Boolean): Result<Unit> = runCatching {
        _sync.value = _sync.value.copy(
            phase = SyncPhaseModel.Idle,
            pendingConflict = null,
            status = if (keepLocal) {
                SyncStatusModel.Pushed(System.currentTimeMillis())
            } else {
                SyncStatusModel.Pulled(System.currentTimeMillis())
            }
        )
    }

    override fun syncCancel() {
        _sync.value = _sync.value.copy(phase = SyncPhaseModel.Idle)
    }

    override suspend fun syncAutoCheck(): Result<Unit> = Result.success(Unit)

    override suspend fun syncSetAutoSync(enabled: Boolean): Result<Unit> = runCatching {
        _sync.value = _sync.value.copy(
            config = _sync.value.config.copy(autoSyncEnabled = enabled,
                webdavUnsafeWriteConsent = if (enabled) _sync.value.config.webdavUnsafeWriteConsent else false)
        )
    }

    override suspend fun previewImport(
        source: ImportSourceModel,
        fileName: String,
        bytes: ByteArray
    ): Result<ImportPreviewModel> = runCatching {
        val name = fileName.substringAfterLast('/').substringBeforeLast('.')
            .ifBlank { "imported" }
        val preview = ImportPreviewModel(
            previewId = previewIdSeed++,
            sourceLabel = source.name,
            entries = listOf(
                ImportEntryModel(
                    name = name,
                    group = source.name,
                    host = "imported.example.com",
                    port = 22,
                    username = "akko",
                    authMethod = "password",
                    privateKeyPath = "",
                    certificatePath = "",
                    hasPassword = true,
                    hasPassphrase = false,
                    agentForwarding = false,
                    startupCommand = ""
                )
            ),
            warnings = if (bytes.isEmpty()) {
                listOf(ImportWarningModel("invalid_entry", name, "empty_file"))
            } else {
                emptyList()
            }
        )
        previews[preview.previewId] = preview
        preview
    }

    override suspend fun commitImport(previewId: Long): Result<Int> = runCatching {
        val preview = previews.remove(previewId) ?: error("preview expired")
        _hosts.value = _hosts.value + preview.entries.mapIndexed { index, entry ->
            SessionProfile(
                id = "imported-${preview.previewId}-$index",
                name = entry.name,
                group = entry.group,
                host = entry.host,
                port = entry.port,
                username = entry.username
            )
        }
        preview.entries.size
    }

    override suspend fun discardImport(previewId: Long) {
        previews.remove(previewId)
    }
}

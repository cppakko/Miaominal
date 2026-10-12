package dev.akko.miaominal.data

import android.content.Context
import dev.akko.miaominal.nativebridge.AuthMethodDto
import dev.akko.miaominal.nativebridge.Backend
import dev.akko.miaominal.nativebridge.BackendEvent
import dev.akko.miaominal.nativebridge.BackendException
import dev.akko.miaominal.nativebridge.ConfigArea
import dev.akko.miaominal.nativebridge.CredentialAction
import dev.akko.miaominal.nativebridge.CredentialHealth as NativeCredentialHealth
import dev.akko.miaominal.nativebridge.CredentialKindDto
import dev.akko.miaominal.nativebridge.ForwardKindDto
import dev.akko.miaominal.nativebridge.ForwardRuleDto
import dev.akko.miaominal.nativebridge.EnvironmentVariableDto
import dev.akko.miaominal.nativebridge.HostDto
import dev.akko.miaominal.nativebridge.HostUpdate
import dev.akko.miaominal.nativebridge.ImportSourceDto
import dev.akko.miaominal.nativebridge.KnownHostDto
import dev.akko.miaominal.nativebridge.ManagedKeyDto
import dev.akko.miaominal.nativebridge.ManagedKeySourceDto
import dev.akko.miaominal.nativebridge.ProfileKindDto
import dev.akko.miaominal.nativebridge.ProxyDto
import dev.akko.miaominal.nativebridge.ProxyUpdate
import dev.akko.miaominal.nativebridge.SettingsDto
import dev.akko.miaominal.nativebridge.SettingsUpdate
import dev.akko.miaominal.nativebridge.ShellTypeDto
import dev.akko.miaominal.nativebridge.SnippetDto
import dev.akko.miaominal.nativebridge.SnippetUpdate
import dev.akko.miaominal.nativebridge.SyncConfigUpdate
import dev.akko.miaominal.nativebridge.SyncCredentialKind
import dev.akko.miaominal.nativebridge.SyncFailureDto
import dev.akko.miaominal.nativebridge.SyncPhaseDto
import dev.akko.miaominal.nativebridge.SyncProviderDto
import dev.akko.miaominal.nativebridge.SyncResolution
import dev.akko.miaominal.nativebridge.SyncSnapshotDto
import dev.akko.miaominal.nativebridge.SyncStatusDto
import dev.akko.miaominal.nativebridge.VaultModeDto
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import java.util.UUID

/**
 * Real repository backed by the native Rust stores.
 *
 * Writes go through the typed backend interfaces; successful writes publish
 * `ConfigChanged` events that re-read the authoritative Rust snapshot. A page
 * therefore never writes its own business copy.
 */
class NativeMiaominalRepository(
    context: Context,
    private val client: NativeBackendClient,
    private val credentialStore: KeystoreCredentialStore,
    private val beforeReset: suspend () -> Unit = {},
    private val clearDeviceCredentials: suspend () -> Unit = {
        withContext(Dispatchers.IO) { credentialStore.clearAll() }
    }
) : MiaominalRepository {

    private val appContext = context.applicationContext
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val settingsWrites = Mutex()
    private var started = false

    private val _status = MutableStateFlow<RepositoryStatus>(RepositoryStatus.Initializing)
    override val transferActions = SftpTransferActions(this)
    override val status: StateFlow<RepositoryStatus> = _status.asStateFlow()

    private val _hosts = MutableStateFlow<List<SessionProfile>>(emptyList())
    override val hosts: StateFlow<List<SessionProfile>> = _hosts.asStateFlow()

    private val _hostStatus = MutableStateFlow<Map<String, HostRuntimeStatus>>(emptyMap())
    override val hostStatus: StateFlow<Map<String, HostRuntimeStatus>> = _hostStatus.asStateFlow()

    private val _forwards = MutableStateFlow<List<PortForwardRule>>(emptyList())
    override val forwards: StateFlow<List<PortForwardRule>> = _forwards.asStateFlow()

    private val _proxies = MutableStateFlow<List<ProxyProfile>>(emptyList())
    override val proxies: StateFlow<List<ProxyProfile>> = _proxies.asStateFlow()

    private val _forwardSessions = MutableStateFlow<List<ForwardSessionModel>>(emptyList())
    override val forwardSessions: StateFlow<List<ForwardSessionModel>> = _forwardSessions.asStateFlow()

    private val _snippets = MutableStateFlow<List<SnippetRecord>>(emptyList())
    override val snippets: StateFlow<List<SnippetRecord>> = _snippets.asStateFlow()

    private val _managedKeys = MutableStateFlow<List<ManagedKeyRecord>>(emptyList())
    override val managedKeys: StateFlow<List<ManagedKeyRecord>> = _managedKeys.asStateFlow()

    private val _knownHosts = MutableStateFlow<List<KnownHostRecord>>(emptyList())
    override val knownHosts: StateFlow<List<KnownHostRecord>> = _knownHosts.asStateFlow()

    private val _settings = MutableStateFlow(BusinessSettings())
    override val settings: StateFlow<BusinessSettings> = _settings.asStateFlow()

    private val _devicePreferences = MutableStateFlow(loadDevicePreferences())
    override val devicePreferences: StateFlow<DevicePreferences> = _devicePreferences.asStateFlow()

    private val _vault = MutableStateFlow(VaultStatus())
    override val vault: StateFlow<VaultStatus> = _vault.asStateFlow()

    private val _sync = MutableStateFlow(SyncStateModel())
    override val sync: StateFlow<SyncStateModel> = _sync.asStateFlow()

    private val _sessions = MutableStateFlow<List<SessionSnapshotModel>>(emptyList())
    override val sessions: StateFlow<List<SessionSnapshotModel>> = _sessions.asStateFlow()

    private val _sftpSessions = MutableStateFlow<List<SftpSessionModel>>(emptyList())
    override val sftpSessions: StateFlow<List<SftpSessionModel>> = _sftpSessions.asStateFlow()

    private val _interruptedSftpTransfers = MutableStateFlow<List<SftpTransferModel>>(emptyList())
    override val interruptedSftpTransfers: StateFlow<List<SftpTransferModel>> =
        _interruptedSftpTransfers.asStateFlow()

    private val safDocuments = SafDocuments(appContext)

    private val handles = java.util.concurrent.ConcurrentHashMap<Long, TerminalSessionHandle>()
    private val sftpHandles = java.util.concurrent.ConcurrentHashMap<Long, SftpSessionHandle>()

    /** Forwarding handles survive page rebuilds; key is "profileId:ruleId". */
    private val forwardHandles =
        java.util.concurrent.ConcurrentHashMap<String, ForwardSessionHandle>()

    /** Transfer handles survive page rebuilds; key is "connectionId:transferId". */
    private val sftpTransferHandles =
        java.util.concurrent.ConcurrentHashMap<String, SftpTransferHandle>()

    private val _notices = MutableSharedFlow<String>(extraBufferCapacity = 16)
    override val notices: SharedFlow<String> = _notices.asSharedFlow()

    override val backendState: StateFlow<NativeBackendState> = client.state

    override fun start() {
        if (started) return
        started = true
        scope.launch {
            client.configChanges.collect { area -> refreshArea(area) }
        }
        scope.launch {
            client.events.collect { event ->
                when (event) {
                    is BackendEvent.SessionPhaseChanged,
                    is BackendEvent.SessionRequest,
                    is BackendEvent.SessionRequestExpired,
                    is BackendEvent.SessionExited -> refreshSessionsInternal()
                    is BackendEvent.SftpPhaseChanged,
                    is BackendEvent.SftpRequest,
                    is BackendEvent.SftpRequestExpired,
                    is BackendEvent.SftpDirectoryChanged,
                    is BackendEvent.SftpTransferChanged -> refreshSftpSessionsInternal()
                    is BackendEvent.ForwardPhaseChanged,
                    is BackendEvent.ForwardRequest,
                    is BackendEvent.ForwardRequestExpired,
                    is BackendEvent.ForwardRelayFailed -> refreshForwardSessionsInternal()
                    is BackendEvent.SyncPhaseChanged,
                    is BackendEvent.SyncConflictDetected,
                    is BackendEvent.SyncAuthenticationRequired,
                    is BackendEvent.SyncCapabilityChanged -> refreshSyncInternal()
                    else -> Unit
                }
            }
        }
        scope.launch {
            // A restarted backend owns a fresh, empty session registry: drop
            // stale handles instead of pretending connections survived, but
            // remember transfers that were still running so the UI can report
            // them as interrupted.
            client.state.collect { state ->
                if (state is NativeBackendState.Stopped || state is NativeBackendState.Starting) {
                    val interrupted = sftpHandles.values
                        .flatMap { handle -> runCatching { handle.transfersSnapshot() }.getOrDefault(emptyList()) }
                        .filterNot { it.state.isTerminal }
                        .map { transfer ->
                            transfer.copy(
                                state = SftpTransferStateModel.Failed,
                                failureKind = SftpTransferFailureKindModel.Other,
                                error = "应用已重新启动，传输已中断，请重新开始"
                            )
                        }
                    if (interrupted.isNotEmpty()) {
                        _interruptedSftpTransfers.value =
                            (_interruptedSftpTransfers.value + interrupted).distinctBy { it.transferId }
                    }
                    handles.clear()
                    sftpHandles.clear()
                    sftpTransferHandles.clear()
                    forwardHandles.clear()
                    _sessions.value = emptyList()
                    _sftpSessions.value = emptyList()
                    _forwardSessions.value = emptyList()
                }
            }
        }
        scope.launch {
            val started = ensureStarted()
            if (started.isSuccess) {
                refreshAll()
                // Copy the pre-P6 device entries into the backend credential
                // store once; the operation is idempotent and keeps any legacy
                // entry whose copy could not be verified.
                runCatching { backend().syncMigrateLegacyCredentials() }
                refreshSyncInternal()
            }
        }
    }

    override suspend fun ensureStarted(): Result<Unit> = runCatching {
        val state = client.ensureStarted()
        if (state !is NativeBackendState.Running) {
            error("native backend is unavailable (${state::class.simpleName})")
        }
        _status.value = RepositoryStatus.Ready
    }.onFailure { error ->
        _status.value = RepositoryStatus.Failed(describe(error))
    }

    override suspend fun refreshAll(): Result<Unit> = runCatching {
        val backend = backend()
        _hosts.value = backend.hostsSnapshot().map(::toSessionProfile)
        _forwards.value = backend.forwardsSnapshot().map(::toPortForwardRule)
        _proxies.value = backend.proxiesSnapshot().map(::toProxyProfile)
        _snippets.value = backend.snippetsSnapshot().map(::toSnippetRecord)
        _managedKeys.value = backend.keysSnapshot().map(::toManagedKeyRecord)
        _knownHosts.value = backend.knownHostsSnapshot().map(::toKnownHostRecord)
        _settings.value = toBusinessSettings(backend.settingsSnapshot())
        _vault.value = toVaultStatus(backend.vaultSnapshot())
        _sessions.value = backend.sshSessionSnapshots().map { it.toModel() }
        _forwardSessions.value = backend.forwardSessionSnapshots().map { it.toModel() }
        refreshSyncInternal()
        _status.value = RepositoryStatus.Ready
    }.onFailure { error -> fail(error) }

    private suspend fun refreshArea(area: ConfigArea) {
        runCatching {
            val backend = backend()
            when (area) {
                ConfigArea.HOSTS -> _hosts.value = backend.hostsSnapshot().map(::toSessionProfile)
                ConfigArea.FORWARDS ->
                    _forwards.value = backend.forwardsSnapshot().map(::toPortForwardRule)
                ConfigArea.SNIPPETS ->
                    _snippets.value = backend.snippetsSnapshot().map(::toSnippetRecord)
                ConfigArea.CREDENTIALS -> {
                    _managedKeys.value = backend.keysSnapshot().map(::toManagedKeyRecord)
                    _vault.value = toVaultStatus(backend.vaultSnapshot())
                }
                ConfigArea.TRUST ->
                    _knownHosts.value = backend.knownHostsSnapshot().map(::toKnownHostRecord)
                ConfigArea.SETTINGS -> {
                    _settings.value = toBusinessSettings(backend.settingsSnapshot())
                    _vault.value = toVaultStatus(backend.vaultSnapshot())
                }
                ConfigArea.PROXIES ->
                    _proxies.value = backend.proxiesSnapshot().map(::toProxyProfile)
                ConfigArea.SYNC -> refreshSyncInternal()
            }
        }.onFailure { error ->
            if (error is BackendException.Closed) return@onFailure
            _notices.tryEmit(describe(error))
        }
    }

    override suspend fun openSession(
        hostId: String,
        columns: Int,
        lines: Int
    ): Result<TerminalSessionHandle> = runCatching {
        ensureStarted().getOrThrow()
        val backend = backend()
        val session = backend.openSshSession(hostId, columns.toUInt(), lines.toUInt())
        val handle = NativeTerminalSessionHandle(session)
        handles[handle.connectionId] = handle
        refreshSessionsInternal()
        handle
    }.onFailure { error ->
        if (error !is BackendException.Closed) {
            _notices.tryEmit(describe(error))
        }
        fail(error)
    }

    override suspend fun closeSession(connectionId: Long): Result<Unit> = runCatching {
        val backend = backend()
        runCatching { backend.closeSshSession(connectionId.toULong()) }
            .getOrElse { if (it !is BackendException.Closed) throw it }
        handles.remove(connectionId)
        refreshSessionsInternal()
    }.onFailure { error ->
        if (error is BackendException.Closed) return@onFailure
        fail(error)
    }

    override fun sessionHandle(connectionId: Long): TerminalSessionHandle? =
        handles[connectionId]

    override suspend fun refreshSessions() {
        refreshSessionsInternal()
    }

    private fun refreshSessionsInternal() {
        runCatching {
            val snapshots = backend().sshSessionSnapshots().map { it.toModel() }
            _sessions.value = snapshots
        }.onFailure { error ->
            if (error is BackendException.Closed) return@onFailure
            _notices.tryEmit(describe(error))
        }
    }

    override suspend fun openSftpSession(hostId: String): Result<SftpSessionHandle> = runCatching {
        ensureStarted().getOrThrow()
        // An Activity rebuild must reuse the session the backend still owns
        // instead of opening a second connection for the same host.
        sftpHandles.values.firstOrNull { handle ->
            val snapshot = runCatching { handle.snapshot() }.getOrNull()
            snapshot != null && snapshot.hostId == hostId && !snapshot.phase.isTerminal
        } ?: run {
            val backend = backend()
            val session = backend.openSftpSession(hostId)
            val handle = NativeSftpSessionHandle(session)
            sftpHandles[handle.connectionId] = handle
            refreshSftpSessionsInternal()
            handle
        }
    }.onFailure { error ->
        if (error !is BackendException.Closed) {
            _notices.tryEmit(describe(error))
        }
        fail(error)
    }

    override suspend fun closeSftpSession(connectionId: Long): Result<Unit> = runCatching {
        val backend = backend()
        runCatching { backend.closeSftpSession(connectionId.toULong()) }
            .getOrElse { if (it !is BackendException.Closed) throw it }
        sftpHandles.remove(connectionId)
        refreshSftpSessionsInternal()
    }.onFailure { error ->
        if (error is BackendException.Closed) return@onFailure
        fail(error)
    }

    override fun sftpSessionHandle(connectionId: Long): SftpSessionHandle? =
        sftpHandles[connectionId]

    override suspend fun refreshSftpSessions() {
        refreshSftpSessionsInternal()
    }

    private fun refreshSftpSessionsInternal() {
        runCatching {
            val snapshots = backend().sftpSessionSnapshots().map { it.toModel() }
            _sftpSessions.value = snapshots
        }.onFailure { error ->
            if (error is BackendException.Closed) return@onFailure
            _notices.tryEmit(describe(error))
        }
    }

    override suspend fun registerUploadFile(
        uri: String,
        name: String
    ): Result<LocalUploadHandle> = withContext(Dispatchers.IO) {
        runCatching {
            val parsed = android.net.Uri.parse(uri)
            val descriptor = appContext.contentResolver.openFileDescriptor(parsed, "r")
                ?: error("the provider returned no descriptor for $name")
            descriptor.use { file ->
                val handle = backend().registerLocalFile(file.fd, name)
                NativeLocalUploadHandle(handle, name)
            }
        }.onFailure { error -> fail(error) }
    }

    override suspend fun persistSafTreePermission(uri: String): Boolean = withContext(Dispatchers.IO) {
        safDocuments.takePersistablePermission(android.net.Uri.parse(uri))
    }

    override fun defaultLocalTreeUri(): String? = safDocuments.defaultTreeUri()

    override suspend fun listSafDirectory(
        treeUri: String,
        relativePath: String
    ): Result<List<SafLocalEntry>> = withContext(Dispatchers.IO) {
        runCatching { safDocuments.listLocal(treeUri, relativePath) }
            .onFailure { error -> fail(error) }
    }

    override fun safTreeLabel(treeUri: String): String = dev.akko.miaominal.data.safTreeLabel(treeUri)

    override fun trackSftpTransfer(connectionId: Long, handle: SftpTransferHandle) {
        sftpTransferHandles[transferKey(connectionId, handle.transferId)] = handle
    }

    override fun sftpTransferHandle(connectionId: Long, transferId: Long): SftpTransferHandle? =
        sftpTransferHandles[transferKey(connectionId, transferId)]

    private fun transferKey(connectionId: Long, transferId: Long): String =
        "$connectionId:$transferId"

    override suspend fun saveHost(
        profile: SessionProfile,
        password: CredentialUpdate
    ): Result<SessionProfile> = runCatching {
        val backend = backend()
        val id = profile.id.ifBlank { "host-${UUID.randomUUID()}" }
        val saved = backend.saveHost(profile.toUpdate(id))
        // Routing is stored separately so the field-update payload stays
        // additive; the editor always sends its current selection.
        backend.setHostRouting(id, profile.entryProxyId, profile.proxyJumpProfileIds)
        when (password) {
            CredentialUpdate.Keep -> Unit
            is CredentialUpdate.Set -> backend.setHostCredential(
                id,
                CredentialKindDto.PASSWORD,
                CredentialAction.Set(password.value)
            )
            CredentialUpdate.Clear ->
                backend.setHostCredential(id, CredentialKindDto.PASSWORD, CredentialAction.Clear)
        }
        _hosts.value = backend.hostsSnapshot().map(::toSessionProfile)
        toSessionProfile(saved)
    }.onFailure { fail(it) }

    override suspend fun deleteHost(id: String): Result<Unit> = runCatching {
        val backend = backend()
        backend.deleteHost(id)
        _hosts.value = backend.hostsSnapshot().map(::toSessionProfile)
        _forwards.value = backend.forwardsSnapshot().map(::toPortForwardRule)
    }.onFailure { fail(it) }

    override suspend fun toggleFavorite(id: String): Result<Unit> = runCatching {
        val backend = backend()
        val current = _hosts.value.firstOrNull { it.id == id } ?: return@runCatching
        backend.saveHost(
            HostUpdate(
                id = id,
                name = null,
                group = null,
                tags = null,
                kind = null,
                host = null,
                port = null,
                username = null,
                authMethod = null,
                privateKeyPath = null,
                managedKeyId = null,
                certificatePath = null,
                startupCommand = null,
                environmentVariables = null,
                shellType = null,
                isFavorite = !current.isFavorite,
                lastConnectedAtMs = null
            )
        )
        _hosts.value = backend.hostsSnapshot().map(::toSessionProfile)
    }.onFailure { fail(it) }

    override suspend fun hostPasswordHealth(id: String): CredentialHealth =
        runCatching {
            toCredentialHealth(
                backend().hostCredentialHealth(id, CredentialKindDto.PASSWORD)
            )
        }.getOrDefault(CredentialHealth.Missing)

    override suspend fun saveSnippet(snippet: SnippetRecord): Result<SnippetRecord> = runCatching {
        val backend = backend()
        val id = snippet.id.ifBlank { "snippet-${UUID.randomUUID()}" }
        val saved = backend.saveSnippet(
            SnippetUpdate(
                id = id,
                description = snippet.description,
                packageName = snippet.packageName,
                language = snippet.language,
                script = snippet.script
            )
        )
        _snippets.value = backend.snippetsSnapshot().map(::toSnippetRecord)
        toSnippetRecord(saved)
    }.onFailure { fail(it) }

    override suspend fun deleteSnippet(id: String): Result<Unit> = runCatching {
        val backend = backend()
        backend.deleteSnippet(id)
        _snippets.value = backend.snippetsSnapshot().map(::toSnippetRecord)
    }.onFailure { fail(it) }

    override suspend fun saveForward(rule: PortForwardRule): Result<PortForwardRule> =
        runCatching {
            val backend = backend()
            val id = rule.id.ifBlank { "forward-${UUID.randomUUID()}" }
            val saved = backend.upsertForward(
                ForwardRuleDto(
                    id = id,
                    profileId = rule.profileId,
                    label = rule.label,
                    kind = rule.kind.toDto(),
                    listenHost = rule.listenHost,
                    listenPort = rule.listenPort.toUShort(),
                    targetHost = rule.targetHost,
                    targetPort = rule.targetPort.toUShort(),
                    enabled = rule.enabled
                )
            )
            _forwards.value = backend.forwardsSnapshot().map(::toPortForwardRule)
            toPortForwardRule(saved)
        }.onFailure { fail(it) }

    override suspend fun deleteForward(rule: PortForwardRule): Result<Unit> = runCatching {
        val backend = backend()
        backend.deleteForward(rule.profileId, rule.id)
        _forwards.value = backend.forwardsSnapshot().map(::toPortForwardRule)
    }.onFailure { fail(it) }

    override suspend fun setForwardEnabled(
        rule: PortForwardRule,
        enabled: Boolean
    ): Result<Unit> = if (enabled) {
        // The switch is the desktop-parity start/stop control: enabling a rule
        // opens its dedicated session instead of only flipping a stored flag.
        startForward(rule).map { }
    } else {
        stopForward(rule.profileId, rule.id)
    }

    override suspend fun startForward(rule: PortForwardRule): Result<ForwardSessionHandle> =
        runCatching {
            ensureStarted().getOrThrow()
            val key = forwardKey(rule.profileId, rule.id)
            // A rebuilt page must reuse the backend session for this rule.
            forwardHandles[key]?.let { existing ->
                val snapshot = runCatching { existing.snapshot() }.getOrNull()
                if (snapshot != null && snapshot.phase.isActive) {
                    persistForwardEnabled(rule.profileId, rule.id, true)
                    return@runCatching existing
                }
            }
            val session = backend().startForwardSession(rule.profileId, rule.id)
            val handle = NativeForwardSessionHandle(session)
            forwardHandles[key] = handle
            persistForwardEnabled(rule.profileId, rule.id, true)
            refreshForwardSessionsInternal()
            handle
        }.onFailure { error ->
            if (error !is BackendException.Closed) {
                _notices.tryEmit(describe(error))
            }
            fail(error)
        }

    override suspend fun stopForward(profileId: String, ruleId: String): Result<Unit> =
        runCatching {
            val key = forwardKey(profileId, ruleId)
            forwardHandles.remove(key)
            runCatching { backend().closeForwardSession(profileId, ruleId) }
            persistForwardEnabled(profileId, ruleId, false)
            refreshForwardSessionsInternal()
        }.onFailure { error ->
            if (error is BackendException.Closed) return@onFailure
            fail(error)
        }

    /**
     * Store the rule's `enabled` flag after an explicit start/stop, mirroring the
     * desktop controller. Persistence is best-effort: a store failure must not
     * fail the runtime action, so it is reported as a notice instead.
     */
    private suspend fun persistForwardEnabled(profileId: String, ruleId: String, enabled: Boolean) {
        runCatching {
            val backend = backend()
            backend.setForwardEnabled(profileId, ruleId, enabled)
            _forwards.value = backend.forwardsSnapshot().map(::toPortForwardRule)
        }.onFailure { error ->
            if (error is BackendException.Closed) return@onFailure
            _notices.tryEmit(describe(error))
        }
    }

    override fun forwardSessionHandle(profileId: String, ruleId: String): ForwardSessionHandle? =
        forwardHandles[forwardKey(profileId, ruleId)]

    override suspend fun refreshForwardSessions() {
        refreshForwardSessionsInternal()
    }

    private fun refreshForwardSessionsInternal() {
        runCatching {
            val snapshots = backend().forwardSessionSnapshots().map { it.toModel() }
            _forwardSessions.value = snapshots
        }.onFailure { error ->
            if (error is BackendException.Closed) return@onFailure
            _notices.tryEmit(describe(error))
        }
    }

    private fun forwardKey(profileId: String, ruleId: String): String = "$profileId:$ruleId"

    override suspend fun saveProxy(
        proxy: ProxyProfile,
        password: CredentialUpdate
    ): Result<ProxyProfile> = runCatching {
        val backend = backend()
        val id = proxy.id.ifBlank { "proxy-${UUID.randomUUID()}" }
        val saved = backend.saveProxy(
            ProxyUpdate(
                id = id,
                name = proxy.name,
                protocol = proxy.protocol,
                host = proxy.host,
                port = proxy.port.toUShort(),
                authMode = proxy.authMode,
                username = proxy.username,
                resolveDnsThroughProxy = proxy.resolveDnsThroughProxy
            )
        )
        when (password) {
            CredentialUpdate.Keep -> Unit
            is CredentialUpdate.Set ->
                backend.setProxyPassword(id, CredentialAction.Set(password.value))
            CredentialUpdate.Clear ->
                backend.setProxyPassword(id, CredentialAction.Clear)
        }
        _proxies.value = backend.proxiesSnapshot().map(::toProxyProfile)
        toProxyProfile(saved)
    }.onFailure { fail(it) }

    override suspend fun deleteProxy(id: String): Result<Unit> = runCatching {
        val backend = backend()
        backend.deleteProxy(id)
        _proxies.value = backend.proxiesSnapshot().map(::toProxyProfile)
    }.onFailure { fail(it) }

    override suspend fun proxyPasswordHealth(id: String): CredentialHealth = runCatching {
        toCredentialHealth(backend().proxyPasswordHealth(id))
    }.getOrDefault(CredentialHealth.Missing)

    override suspend fun saveHostRouting(
        profileId: String,
        entryProxyId: String,
        proxyJumpProfileIds: List<String>
    ): Result<Unit> = runCatching {
        val backend = backend()
        backend.setHostRouting(profileId, entryProxyId, proxyJumpProfileIds)
        _hosts.value = backend.hostsSnapshot().map(::toSessionProfile)
    }.onFailure { fail(it) }

    override suspend fun closeAllSessions(): Result<Unit> = runCatching {
        val backend = backend()
        handles.keys.toList().forEach { connectionId ->
            handles.remove(connectionId)
            runCatching { backend.closeSshSession(connectionId.toULong()) }
        }
        sftpHandles.keys.toList().forEach { connectionId ->
            sftpHandles.remove(connectionId)
            runCatching { backend.closeSftpSession(connectionId.toULong()) }
        }
        forwardHandles.keys.toList().forEach { key ->
            val handle = forwardHandles.remove(key) ?: return@forEach
            runCatching { handle.disconnect() }
        }
        refreshSessionsInternal()
        refreshSftpSessionsInternal()
        refreshForwardSessionsInternal()
    }.onFailure { error ->
        if (error is BackendException.Closed) return@onFailure
        fail(error)
    }

    override suspend fun saveKey(key: ManagedKeyRecord): Result<ManagedKeyRecord> = runCatching {
        val backend = backend()
        val saved = backend.renameKey(key.id, key.name)
        _managedKeys.value = backend.keysSnapshot().map(::toManagedKeyRecord)
        toManagedKeyRecord(saved)
    }.onFailure { fail(it) }

    override suspend fun generateKey(name: String, algorithm: String): Result<ManagedKeyRecord> =
        runCatching {
            val backend = backend()
            val nativeAlgorithm = when (algorithm) {
                "ssh-rsa", "rsa4096", "RSA4096" -> dev.akko.miaominal.nativebridge.ManagedKeyAlgorithmDto.RSA4096
                else -> dev.akko.miaominal.nativebridge.ManagedKeyAlgorithmDto.ED25519
            }
            val saved = backend.generateKey(nativeAlgorithm, name)
            _managedKeys.value = backend.keysSnapshot().map(::toManagedKeyRecord)
            toManagedKeyRecord(saved)
        }.onFailure { fail(it) }

    override suspend fun importKey(
        name: String,
        privateKey: String,
        passphrase: String?
    ): Result<ManagedKeyRecord> = runCatching {
        val backend = backend()
        val saved = backend.importKey(name, privateKey, passphrase)
        _managedKeys.value = backend.keysSnapshot().map(::toManagedKeyRecord)
        toManagedKeyRecord(saved)
    }.onFailure { fail(it) }

    override suspend fun deleteKey(id: String): Result<Unit> = runCatching {        val backend = backend()
        backend.deleteKey(id)
        _managedKeys.value = backend.keysSnapshot().map(::toManagedKeyRecord)
        _hosts.value = backend.hostsSnapshot().map(::toSessionProfile)
    }.onFailure { fail(it) }

    override suspend fun deleteKnownHost(host: String, port: Int): Result<Unit> = runCatching {
        val backend = backend()
        backend.deleteKnownHost(host, port.toUShort())
        _knownHosts.value = backend.knownHostsSnapshot().map(::toKnownHostRecord)
    }.onFailure { fail(it) }

    override suspend fun updateSettings(
        transform: (BusinessSettings) -> BusinessSettings
    ): Result<Unit> = withContext(Dispatchers.IO) { settingsWrites.withLock { runCatching {
        val current = _settings.value
        val next = transform(current)
        val backend = backend()
        val updated = backend.updateSettings(current.toUpdate(next))
        _settings.value = toBusinessSettings(updated)
    }.onFailure { fail(it) } } }

    override suspend fun updateDevicePreferences(
        transform: (DevicePreferences) -> DevicePreferences
    ) {
        val next = transform(_devicePreferences.value)
        _devicePreferences.value = next
        persistDevicePreferences(next)
    }

    override suspend fun vaultEnable(passphrase: String): Result<Unit> =
        vaultOperation { it.enableVault(passphrase) }

    override suspend fun vaultDisable(): Result<Unit> = vaultOperation { it.disableVault() }

    override suspend fun vaultLock(): Result<Unit> = vaultOperation { it.lockVault() }

    override suspend fun vaultUnlock(passphrase: String): Result<Unit> =
        vaultOperation { it.unlockVault(passphrase) }

    override suspend fun vaultChangePassphrase(current: String, new: String): Result<Unit> =
        vaultOperation { it.changeVaultPassphrase(current, new) }

    private suspend fun vaultOperation(operation: suspend (Backend) -> Any): Result<Unit> =
        withContext(Dispatchers.IO) {
            _vault.value = _vault.value.copy(busy = true, error = null)
            runCatching {
                operation(backend())
                _vault.value = toVaultStatus(backend().vaultSnapshot())
            }.onFailure { error ->
                _vault.value = _vault.value.copy(busy = false, error = describe(error))
                _notices.tryEmit(describe(error))
            }.map { }
        }

    override suspend fun resetLocalData(): Result<Unit> = runCatching {
        beforeReset()
        // 1. Close running native work: restarting replaces the handle and
        //    cancels every task bound to the previous close signal.
        client.restart()
        handles.clear()
        sftpHandles.clear()
        sftpTransferHandles.clear()
        forwardHandles.clear()
        _sessions.value = emptyList()
        _sftpSessions.value = emptyList()
        _forwardSessions.value = emptyList()
        _interruptedSftpTransfers.value = emptyList()
        val backend = backend()
        // 2. Delete business files.
        backend.clearLocalData()
        // 3. Delete device credentials and the wrapping Keystore key.
        clearDeviceCredentials()
        // 4. Re-read the (now empty) snapshots.
        _hosts.value = emptyList()
        _forwards.value = emptyList()
        _proxies.value = emptyList()
        _snippets.value = emptyList()
        _managedKeys.value = emptyList()
        _knownHosts.value = emptyList()
        _settings.value = toBusinessSettings(backend.settingsSnapshot())
        _vault.value = toVaultStatus(backend.vaultSnapshot())
        if (client.currentBackend() != null) {
            _sessions.value = backend.sshSessionSnapshots().map { it.toModel() }
            _sftpSessions.value = backend.sftpSessionSnapshots().map { it.toModel() }
        }
        _status.value = RepositoryStatus.Ready
    }.onFailure { error ->
        handles.clear()
        sftpHandles.clear()
        sftpTransferHandles.clear()
        forwardHandles.clear()
        _sessions.value = emptyList()
        _sftpSessions.value = emptyList()
        _forwardSessions.value = emptyList()
        fail(error)
    }

    // ------------------------------------------------------------------
    // Sync (P6)
    // ------------------------------------------------------------------

    override suspend fun refreshSync() {
        refreshSyncInternal()
    }

    private suspend fun refreshSyncInternal() {
        val backend = client.currentBackend() ?: return
        // `syncSnapshot` is a suspend FFI call: poll it off the main thread so
        // a slow Rust snapshot cannot stall a Compose frame or input dispatch.
        withContext(Dispatchers.IO) {
            runCatching { _sync.value = backend.syncSnapshot().toModel().copy(loaded = true) }
                .onFailure { error ->
                    if (error is BackendException.Closed) return@onFailure
                    _notices.tryEmit(describe(error))
                }
        }
    }

    /**
     * Run one sync backend call on [Dispatchers.IO].
     *
     * UniFFI polls the Rust future on the caller's dispatcher, and the Rust
     * side synchronously calls back into `PlatformHost.credentialGet` (Keystore
     * read/decrypt). Calling these from the Compose main scope therefore blocks
     * the UI thread and can ANR; the repository never lets a backend call run
     * on Main.
     */
    private suspend fun syncOperation(
        operation: suspend (Backend) -> Unit
    ): Result<Unit> = withContext(Dispatchers.IO) {
        runCatching {
            operation(backend())
            refreshSyncInternal()
        }.onFailure { fail(it) }
    }

    override suspend fun updateSyncConfig(edit: SyncConfigEdit): Result<Unit> =
        withContext(Dispatchers.IO) {
            runCatching {
                backend().updateSyncConfig(
                    SyncConfigUpdate(
                        provider = edit.provider?.toDto(),
                        gistId = edit.gistId,
                        webdavUrl = edit.webdavUrl,
                        webdavUsername = edit.webdavUsername,
                        autoSyncEnabled = edit.autoSyncEnabled,
                        webdavUnsafeWriteConsent = edit.webdavUnsafeWriteConsent
                    )
                )
                refreshSyncInternal()
            }.onFailure { fail(it) }
        }

    override suspend fun setSyncCredential(
        kind: SyncCredentialKindModel,
        update: CredentialUpdate
    ): Result<Unit> = withContext(Dispatchers.IO) {
        runCatching {
            backend().setSyncCredential(kind.toDto(), update.toAction())
            refreshSyncInternal()
        }.onFailure { fail(it) }
    }

    override suspend fun syncCredentialHealth(kind: SyncCredentialKindModel): CredentialHealth =
        withContext(Dispatchers.IO) {
            runCatching { toCredentialHealth(backend().syncCredentialHealth(kind.toDto())) }
                .getOrDefault(CredentialHealth.Missing)
        }

    override suspend fun syncMigrateLegacyCredentials(): Result<Unit> =
        syncOperation { it.syncMigrateLegacyCredentials() }

    override suspend fun syncCheckCapability(): Result<Unit> =
        syncOperation { it.syncCheckCapability() }

    override suspend fun syncPull(force: Boolean): Result<Unit> =
        syncOperation { it.syncPull(force) }

    override suspend fun syncPush(manual: Boolean): Result<Unit> =
        syncOperation { it.syncPush(manual) }

    override suspend fun syncResolveConflict(keepLocal: Boolean): Result<Unit> =
        syncOperation {
            it.syncResolveConflict(
                if (keepLocal) SyncResolution.KEEP_LOCAL else SyncResolution.KEEP_REMOTE
            )
        }

    override fun syncCancel() {
        // The cancel entry point is synchronous FFI, so it must not run on the
        // main thread either; cancellation itself is fast.
        scope.launch(Dispatchers.IO) {
            runCatching { client.currentBackend()?.syncCancel() }
            refreshSyncInternal()
        }
    }

    override suspend fun syncAutoCheck(): Result<Unit> =
        syncOperation { it.syncAutoCheck(false) }

    override suspend fun syncSetAutoSync(enabled: Boolean): Result<Unit> =
        syncOperation { it.syncSetAutoSync(enabled) }

    override suspend fun previewImport(
        source: ImportSourceModel,
        fileName: String,
        bytes: ByteArray
    ): Result<ImportPreviewModel> = withContext(Dispatchers.IO) {
        runCatching {
            val preview = backend().previewImport(source.toDto(), fileName, bytes)
            ImportPreviewModel(
                previewId = preview.previewId.toLong(),
                sourceLabel = preview.sourceLabel,
                entries = preview.entries.map {
                    ImportEntryModel(
                        name = it.name,
                        group = it.group,
                        host = it.host,
                        port = it.port.toInt(),
                        username = it.username,
                        authMethod = it.authMethod,
                        privateKeyPath = it.privateKeyPath,
                        certificatePath = it.certificatePath,
                        hasPassword = it.hasPassword,
                        hasPassphrase = it.hasPassphrase,
                        agentForwarding = it.agentForwarding,
                        startupCommand = it.startupCommand
                    )
                },
                warnings = preview.warnings.map {
                    ImportWarningModel(it.kind, it.entryName, it.reason)
                }
            )
        }.onFailure { fail(it) }
    }

    override suspend fun commitImport(previewId: Long): Result<Int> = withContext(Dispatchers.IO) { runCatching {
        val result = backend().commitImport(previewId.toULong())
        refreshAll()
        result.imported.toInt()
    }.onFailure { fail(it) } }

    override suspend fun discardImport(previewId: Long) {
        withContext(Dispatchers.IO) { runCatching { backend().discardImport(previewId.toULong()) } }
    }

    private fun backend(): Backend =
        client.currentBackend() ?: error("native backend is not running")

    private fun fail(error: Throwable) {
        if (error is BackendException.Closed) {
            _status.value = RepositoryStatus.Initializing
            return
        }
        _notices.tryEmit(describe(error))
    }

    private fun loadDevicePreferences(): DevicePreferences {
        val preferences = appContext.getSharedPreferences(DEVICE_PREFERENCES, Context.MODE_PRIVATE)
        return DevicePreferences(
            proxyEnabled = preferences.getBoolean(KEY_PROXY_ENABLED, false),
            proxyAddress = preferences.getString(KEY_PROXY_ADDRESS, "") ?: "",
            proxyPort = preferences.getString(KEY_PROXY_PORT, "1080") ?: "1080",
            importSource = preferences.getString(KEY_IMPORT_SOURCE, "OpenSsh") ?: "OpenSsh",
            backgroundKeepAlive = preferences.getBoolean(KEY_BACKGROUND_KEEP_ALIVE, true),
            terminalQuickKeys = preferences.getString(KEY_TERMINAL_QUICK_KEYS, null)
                ?.split(',')
                ?.filter(String::isNotBlank)
                ?.distinct()
                ?.takeIf(List<String>::isNotEmpty)
                ?: DefaultTerminalQuickKeys
        )
    }

    private fun persistDevicePreferences(value: DevicePreferences) {
        appContext.getSharedPreferences(DEVICE_PREFERENCES, Context.MODE_PRIVATE)
            .edit()
            .putBoolean(KEY_PROXY_ENABLED, value.proxyEnabled)
            .putString(KEY_PROXY_ADDRESS, value.proxyAddress)
            .putString(KEY_PROXY_PORT, value.proxyPort)
            .putString(KEY_IMPORT_SOURCE, value.importSource)
            .putBoolean(KEY_BACKGROUND_KEEP_ALIVE, value.backgroundKeepAlive)
            .putString(KEY_TERMINAL_QUICK_KEYS, value.terminalQuickKeys.joinToString(","))
            .apply()
    }

    companion object {
        private const val DEVICE_PREFERENCES = "device-preferences"
        private const val KEY_PROXY_ENABLED = "proxy_enabled"
        private const val KEY_PROXY_ADDRESS = "proxy_address"
        private const val KEY_PROXY_PORT = "proxy_port"
        private const val KEY_IMPORT_SOURCE = "import_source"
        private const val KEY_BACKGROUND_KEEP_ALIVE = "background_keep_alive"
        private const val KEY_TERMINAL_QUICK_KEYS = "terminal_quick_keys"

        fun describe(error: Throwable): String = when (error) {
            is BackendException.VaultLocked -> "保险库已锁定，请先解锁"
            is BackendException.AuthenticationFailed -> "认证失败"
            is BackendException.PermissionDenied -> "权限不足"
            is BackendException.Network -> "网络错误"
            is BackendException.Conflict -> "数据冲突"
            is BackendException.SyncConflict -> "同步冲突，需要选择保留哪一侧"
            is BackendException.SyncCapability -> "远端不支持安全写入（缺少条件请求支持）"
            is BackendException.SyncAuthentication -> "同步口令错误或远端拒绝了本次认证"
            is BackendException.SyncNotConfigured -> "同步尚未配置完整"
            is BackendException.Cancelled -> "操作已取消"
            is BackendException.Unsupported -> "当前后端不支持该操作"
            is BackendException.Storage -> error.storageFailure
            is BackendException.InvalidArgument -> error.detail
            is BackendException.Platform -> error.platformFailure
            is BackendException.RuntimeUnavailable -> error.runtimeFailure
            is BackendException.UnexpectedPlatform -> error.reason
            is BackendException.Closed -> "后端已关闭"
            else -> error.message?.takeIf(String::isNotBlank) ?: error.javaClass.simpleName
        }
    }
}

// ---------------------------------------------------------------------------
// DTO <-> UI model mapping
// ---------------------------------------------------------------------------

private fun toSessionProfile(dto: HostDto): SessionProfile = SessionProfile(
    id = dto.id,
    name = dto.name,
    group = dto.group,
    tags = dto.tags,
    kind = when (dto.kind) {
        ProfileKindDto.SSH -> ProfileKind.Ssh
        ProfileKindDto.TELNET -> ProfileKind.Telnet
        ProfileKindDto.RDP -> ProfileKind.Rdp
        ProfileKindDto.LOCAL -> ProfileKind.Local
    },
    host = dto.host,
    port = dto.port.toInt(),
    username = dto.username,
    authMethod = when (dto.authMethod) {
        AuthMethodDto.PASSWORD -> AuthMethod.Password
        AuthMethodDto.KEY_FILE -> AuthMethod.KeyFile
        AuthMethodDto.MANAGED_KEY -> AuthMethod.ManagedKey
        AuthMethodDto.AGENT -> AuthMethod.Agent
        AuthMethodDto.KEYBOARD_INTERACTIVE -> AuthMethod.KeyboardInteractive
    },
    privateKeyPath = dto.privateKeyPath,
    managedKeyId = dto.managedKeyId,
    certificatePath = dto.certificatePath,
    startupCommand = dto.startupCommand,
    environmentVariables = dto.environmentVariables.map {
        SessionEnvironmentVariable(name = it.name, value = it.value)
    },
    shellType = when (dto.shellType) {
        ShellTypeDto.POSIX -> ShellType.Posix
        ShellTypeDto.FISH -> ShellType.Fish
        ShellTypeDto.POWER_SHELL -> ShellType.PowerShell
        ShellTypeDto.CMD -> ShellType.Cmd
    },
    isFavorite = dto.isFavorite,
    hasStoredPassword = dto.hasStoredPassword,
    lastConnectedAt = dto.lastConnectedAtMs?.toLong(),
    entryProxyId = dto.entryProxyId,
    proxyJumpProfileIds = dto.proxyJumpProfileIds
)

private fun SessionProfile.toUpdate(id: String): HostUpdate = HostUpdate(
    id = id,
    name = name,
    group = group,
    tags = tags,
    kind = when (kind) {
        ProfileKind.Ssh -> ProfileKindDto.SSH
        ProfileKind.Telnet -> ProfileKindDto.TELNET
        ProfileKind.Rdp -> ProfileKindDto.RDP
        ProfileKind.Local -> ProfileKindDto.LOCAL
    },
    host = host,
    port = port.toUShort(),
    username = username,
    authMethod = when (authMethod) {
        AuthMethod.Password -> AuthMethodDto.PASSWORD
        AuthMethod.KeyFile -> AuthMethodDto.KEY_FILE
        AuthMethod.ManagedKey -> AuthMethodDto.MANAGED_KEY
        AuthMethod.Agent -> AuthMethodDto.AGENT
        AuthMethod.KeyboardInteractive -> AuthMethodDto.KEYBOARD_INTERACTIVE
    },
    privateKeyPath = privateKeyPath,
    managedKeyId = managedKeyId,
    certificatePath = certificatePath,
    startupCommand = startupCommand,
    environmentVariables = environmentVariables.map {
        EnvironmentVariableDto(name = it.name, value = it.value)
    },
    shellType = when (shellType) {
        ShellType.Posix -> ShellTypeDto.POSIX
        ShellType.Fish -> ShellTypeDto.FISH
        ShellType.PowerShell -> ShellTypeDto.POWER_SHELL
        ShellType.Cmd -> ShellTypeDto.CMD
    },
    isFavorite = isFavorite,
    lastConnectedAtMs = lastConnectedAt?.toULong()
)

private fun toPortForwardRule(dto: ForwardRuleDto): PortForwardRule = PortForwardRule(
    id = dto.id,
    profileId = dto.profileId,
    label = dto.label,
    kind = when (dto.kind) {
        ForwardKindDto.LOCAL -> ForwardKind.Local
        ForwardKindDto.REMOTE -> ForwardKind.Remote
    },
    listenHost = dto.listenHost,
    listenPort = dto.listenPort.toInt(),
    targetHost = dto.targetHost,
    targetPort = dto.targetPort.toInt(),
    enabled = dto.enabled
)

private fun ForwardKind.toDto(): ForwardKindDto = when (this) {
    ForwardKind.Local -> ForwardKindDto.LOCAL
    ForwardKind.Remote -> ForwardKindDto.REMOTE
}

private fun toProxyProfile(dto: ProxyDto): ProxyProfile = ProxyProfile(
    id = dto.id,
    name = dto.name,
    protocol = dto.protocol,
    host = dto.host,
    port = dto.port.toInt(),
    authMode = dto.authMode,
    username = dto.username,
    resolveDnsThroughProxy = dto.resolveDnsThroughProxy,
    hasStoredPassword = dto.hasStoredPassword
)

private fun toSnippetRecord(dto: SnippetDto): SnippetRecord = SnippetRecord(
    id = dto.id,
    description = dto.description,
    packageName = dto.packageName,
    language = dto.language,
    script = dto.script
)

private fun toManagedKeyRecord(dto: ManagedKeyDto): ManagedKeyRecord = ManagedKeyRecord(
    id = dto.id,
    name = dto.name,
    algorithm = dto.algorithm,
    publicKey = dto.publicKey,
    source = when (dto.source) {
        ManagedKeySourceDto.GENERATED -> ManagedKeySource.Generated
        ManagedKeySourceDto.IMPORTED -> ManagedKeySource.Imported
    }
)

private fun toKnownHostRecord(dto: KnownHostDto): KnownHostRecord = KnownHostRecord(
    host = dto.host,
    port = dto.port.toInt(),
    algorithm = dto.algorithm,
    fingerprint = dto.fingerprint,
    addedAt = dto.addedAtMs?.toLong()
)

private fun toBusinessSettings(dto: SettingsDto): BusinessSettings = BusinessSettings(
    language = dto.language,
    recentHostCount = dto.recentHostCount.toInt(),
    autoMonitoring = dto.autoMonitoring,
    monitorHistory = dto.monitorHistory,
    closeLastSessionBehavior = dto.closeLastSessionBehavior,
    vaultEnabled = dto.vaultEnabled,
    vaultAutoLockMinutes = dto.vaultAutoLockMinutes.toInt(),
    onboardingCompleted = dto.onboardingCompleted,
    terminalFont = dto.terminalFont,
    cjkFallbackFont = dto.cjkFallbackFont,
    terminalFontSize = dto.terminalFontSize,
    terminalLineHeight = dto.terminalLineHeight,
    themeId = dto.themeId,
    seedColor = dto.seedColor
)

private fun BusinessSettings.toUpdate(next: BusinessSettings): SettingsUpdate = SettingsUpdate(
    language = next.language.takeIf { it != language },
    recentHostCount = next.recentHostCount.takeIf { it != recentHostCount }?.toUInt(),
    autoMonitoring = next.autoMonitoring.takeIf { it != autoMonitoring },
    monitorHistory = next.monitorHistory.takeIf { it != monitorHistory },
    closeLastSessionBehavior = next.closeLastSessionBehavior
        .takeIf { it != closeLastSessionBehavior },
    vaultAutoLockMinutes = next.vaultAutoLockMinutes
        .takeIf { it != vaultAutoLockMinutes }?.toUInt(),
    onboardingCompleted = next.onboardingCompleted.takeIf { it != onboardingCompleted },
    terminalFont = next.terminalFont.takeIf { it != terminalFont },
    cjkFallbackFont = next.cjkFallbackFont.takeIf { it != cjkFallbackFont },
    terminalFontSize = next.terminalFontSize.takeIf { it != terminalFontSize },
    terminalLineHeight = next.terminalLineHeight.takeIf { it != terminalLineHeight },
    themeId = next.themeId.takeIf { it != themeId },
    seedColor = next.seedColor.takeIf { it != seedColor }
)

private fun toVaultStatus(dto: dev.akko.miaominal.nativebridge.VaultSnapshot): VaultStatus =
    VaultStatus(
        mode = when (dto.mode) {
            VaultModeDto.DISABLED -> VaultMode.Disabled
            VaultModeDto.LOCKED -> VaultMode.Locked
            VaultModeDto.UNLOCKED -> VaultMode.Unlocked
        },
        filePresent = dto.filePresent
    )

private fun toCredentialHealth(health: NativeCredentialHealth): CredentialHealth = when (health) {
    NativeCredentialHealth.MISSING -> CredentialHealth.Missing
    NativeCredentialHealth.PRESENT -> CredentialHealth.Present
    NativeCredentialHealth.LOCKED -> CredentialHealth.Locked
    NativeCredentialHealth.INVALIDATED -> CredentialHealth.Invalidated
    NativeCredentialHealth.CORRUPT -> CredentialHealth.Corrupt
}

// ---------------------------------------------------------------------------
// Sync model mapping (P6)
// ---------------------------------------------------------------------------

private fun CredentialUpdate.toAction(): CredentialAction = when (this) {
    CredentialUpdate.Keep -> CredentialAction.Keep
    is CredentialUpdate.Set -> CredentialAction.Set(value)
    CredentialUpdate.Clear -> CredentialAction.Clear
}

private fun SyncProviderModel.toDto(): SyncProviderDto = when (this) {
    SyncProviderModel.None -> SyncProviderDto.NONE
    SyncProviderModel.Gist -> SyncProviderDto.GITHUB_GIST
    SyncProviderModel.WebDav -> SyncProviderDto.WEB_DAV
}

private fun SyncProviderDto.toModel(): SyncProviderModel = when (this) {
    SyncProviderDto.NONE -> SyncProviderModel.None
    SyncProviderDto.GITHUB_GIST -> SyncProviderModel.Gist
    SyncProviderDto.WEB_DAV -> SyncProviderModel.WebDav
}

private fun SyncCredentialKindModel.toDto(): SyncCredentialKind = when (this) {
    SyncCredentialKindModel.GithubToken -> SyncCredentialKind.GITHUB_TOKEN
    SyncCredentialKindModel.WebdavPassword -> SyncCredentialKind.WEBDAV_PASSWORD
    SyncCredentialKindModel.Passphrase -> SyncCredentialKind.PASSPHRASE
}

private fun ImportSourceModel.toDto(): ImportSourceDto = when (this) {
    ImportSourceModel.OpenSsh -> ImportSourceDto.OPEN_SSH
    ImportSourceModel.Putty -> ImportSourceDto.PUTTY
    ImportSourceModel.SecureCrt -> ImportSourceDto.SECURE_CRT
    ImportSourceModel.FinalShell -> ImportSourceDto.FINAL_SHELL
}

private fun SyncPhaseDto.toModel(): SyncPhaseModel = when (this) {
    SyncPhaseDto.DISABLED -> SyncPhaseModel.Disabled
    SyncPhaseDto.IDLE -> SyncPhaseModel.Idle
    SyncPhaseDto.CHECKING_CAPABILITY -> SyncPhaseModel.CheckingCapability
    SyncPhaseDto.CHECKING_REMOTE -> SyncPhaseModel.CheckingRemote
    SyncPhaseDto.PULLING -> SyncPhaseModel.Pulling
    SyncPhaseDto.PUSHING -> SyncPhaseModel.Pushing
    SyncPhaseDto.PAUSED_VAULT_LOCKED -> SyncPhaseModel.PausedVaultLocked
    SyncPhaseDto.PAUSED_CREDENTIALS -> SyncPhaseModel.PausedCredentials
    SyncPhaseDto.PAUSED_CAPABILITY -> SyncPhaseModel.PausedCapability
    SyncPhaseDto.CONFLICT -> SyncPhaseModel.Conflict
    SyncPhaseDto.CANCELLING -> SyncPhaseModel.Cancelling
}

private fun SyncFailureDto.toModel(): SyncFailureModel = SyncFailureModel(kind.name, detail)

private fun SyncStatusDto.toModel(): SyncStatusModel = when (this) {
    SyncStatusDto.Idle -> SyncStatusModel.Idle
    is SyncStatusDto.RemoteBindingRequired -> SyncStatusModel.RemoteBindingRequired
    is SyncStatusDto.Pulled -> SyncStatusModel.Pulled(atMs.toLong())
    is SyncStatusDto.Pushed -> SyncStatusModel.Pushed(atMs.toLong())
    is SyncStatusDto.UpToDate -> SyncStatusModel.UpToDate(atMs.toLong())
    is SyncStatusDto.PullRequired ->
        SyncStatusModel.PullRequired(remoteAtMs?.toLong(), reason.name)
    SyncStatusDto.Cancelled -> SyncStatusModel.Cancelled
    is SyncStatusDto.Failed -> SyncStatusModel.Failed(failure.toModel())
}

private fun SyncSnapshotDto.toModel(): SyncStateModel = SyncStateModel(
    phase = phase.toModel(),
    status = status.toModel(),
    config = SyncConfigModel(
        provider = config.provider.toModel(),
        gistId = config.gistId.orEmpty(),
        webdavUrl = config.webdavUrl,
        webdavUsername = config.webdavUsername,
        hasGithubToken = config.hasGithubToken,
        hasWebdavPassword = config.hasWebdavPassword,
        hasPassphrase = config.hasPassphrase,
        autoSyncEnabled = config.autoSyncEnabled,
        lastSyncAtMillis = config.lastSyncAtMs.toLong(),
        deviceId = config.deviceId,
        legacyAutoSyncCompat = config.legacyAutoSyncCompat,
        webdavUnsafeWriteConsent = config.webdavUnsafeWriteConsent,
        capabilityProbed = config.capabilityProbeMigrated
    ),
    capability = SyncCapabilityModel(
        state = capability.state.name,
        reason = capability.reason?.name,
        step = capability.step,
        httpStatus = capability.httpStatus?.toInt(),
        cleanupPending = capability.cleanupPending
    ),
    vaultLocked = vaultLocked,
    pendingConflict = pendingConflict?.let {
        SyncConflictModel(it.remoteAtMs?.toLong(), it.reason.name)
    },
    lastFailure = lastFailure?.let { SyncFailureModel(it.kind.name, it.detail) },
    lastAutoCheckAtMillis = lastAutoCheckAtMs.toLong()
)

package dev.akko.miaominal.data

/**
 * Scriptable SFTP session for UI tests and Previews.
 *
 * The fake keeps an in-memory remote tree and simulates transfers so the
 * screens can be exercised without the native backend. Delays are short and
 * driven by the caller; nothing here touches the network or file system.
 */
class FakeSftpSession(
    override val connectionId: Long,
    internal val hostId: String,
    private var phase: SftpPhase = SftpPhase.Ready
) : SftpSessionHandle {

    /** Directory requests observed by the fake, for navigation assertions. */
    val requestedPaths = mutableListOf<String>()

    /** Control handle for a seeded transfer, for rebuild tests. */
    fun handleFor(transferId: Long): SftpTransferHandle? = transfers[transferId]

    private var operationError: SftpOperationErrorModel? = null
    private var operationErrorSequence = 0L

    /** Report a failed directory operation, like the native backend does. */
    fun failOperation(
        operation: SftpOperationModel,
        path: String? = null,
        message: String = "permission denied"
    ): Long {
        operationErrorSequence += 1
        operationError = SftpOperationErrorModel(operationErrorSequence, operation, path, message)
        revision += 1
        return operationErrorSequence
    }

    /** Seed one transfer without going through a real upload/download. */
    fun seedTransfer(
        name: String,
        state: SftpTransferStateModel = SftpTransferStateModel.Running,
        direction: SftpTransferDirection = SftpTransferDirection.Upload
    ): Long {
        val id = nextTransferId++
        transfers[id] = FakeTransfer(
            connectionId,
            SftpTransferModel(
                transferId = id,
                connectionId = connectionId,
                direction = direction,
                sourceLabel = name,
                destination = name,
                state = state,
                bytesComplete = 512,
                bytesTotal = 1024,
                error = null,
                failureKind = SftpTransferFailureKindModel.None,
                childCount = 0,
                childrenDone = 0,
                childrenFailed = 0,
                childrenCancelled = 0,
                children = emptyList()
            )
        )
        revision += 1
        return id
    }

    private var revision = 1L
    private var directory = SftpDirectoryModel(
        path = "/home/tester",
        entries = emptyList(),
        error = null,
        loading = false,
        requestId = 0,
        fetchedAtMs = 0
    )
    private val transfers = linkedMapOf<Long, FakeTransfer>()
    private val trees = mutableMapOf<String, List<SftpEntryModel>>()
    private var nextRequestId = 1L
    private var nextTransferId = 1L

    var pendingRequest: SftpRequestModel? = null
        private set

    fun setPhase(next: SftpPhase) {
        phase = next
        revision += 1
    }

    fun seedDirectory(path: String, entries: List<SftpEntryModel>) {
        directory = directory.copy(path = path, entries = entries, loading = false, requestId = nextRequestId++)
        revision += 1
    }

    fun requestHostKey(fingerprint: String = "SHA256:test"): SftpRequestModel {
        val request = SftpRequestModel(
            requestId = nextRequestId++,
            connectionId = connectionId,
            kind = SessionRequestKind.HostKey,
            hopLabel = hostId,
            hopIndex = 1,
            hopTotal = 1,
            host = "127.0.0.1",
            port = 22,
            fingerprint = fingerprint,
            previousFingerprint = null,
            keyChanged = false,
            name = "",
            instructions = "",
            prompts = emptyList()
        )
        pendingRequest = request
        phase = SftpPhase.WaitingForTrust
        revision += 1
        return request
    }

    fun requestKeyboardInteractive(name: String = "Password"): SftpRequestModel {
        val request = SftpRequestModel(
            requestId = nextRequestId++,
            connectionId = connectionId,
            kind = SessionRequestKind.KeyboardInteractive,
            hopLabel = hostId,
            hopIndex = 1,
            hopTotal = 1,
            host = hostId,
            port = 22,
            fingerprint = "",
            previousFingerprint = null,
            keyChanged = false,
            name = name,
            instructions = "Enter the password",
            prompts = listOf(SessionPromptModel("Password: ", false))
        )
        pendingRequest = request
        phase = SftpPhase.WaitingForAuthentication
        revision += 1
        return request
    }

    override fun snapshot(): SftpSessionModel = SftpSessionModel(
        connectionId = connectionId,
        hostId = hostId,
        hostLabel = hostId,
        phase = phase,
        failure = if (phase == SftpPhase.Failed) "connection failed" else null,
        hopLabel = hostId,
        request = pendingRequest,
        revision = revision,
        transferCount = transfers.size,
        activeTransferCount = transfers.values.count { !it.snapshot().state.isTerminal },
        operationError = operationError
    )

    override fun directorySnapshot(): SftpDirectoryModel = directory

    override fun transfersSnapshot(): List<SftpTransferModel> =
        transfers.values.map { it.snapshot() }.sortedByDescending { it.transferId }

    /** Seed an in-memory directory tree for navigation tests. */
    fun seedTree(path: String, entries: List<SftpEntryModel>) {
        trees[path] = entries
    }

    override fun listDirectory(path: String): Result<Long> {
        requestedPaths.add(path)
        val requestId = nextRequestId++
        val normalized = if (path == "." || path.isBlank()) {
            "/home/tester"
        } else {
            path.trimEnd('/').ifBlank { "/" }
        }
        val entries = trees[normalized] ?: trees[path] ?: emptyList()
        directory = SftpDirectoryModel(
            path = normalized,
            entries = entries,
            error = null,
            loading = false,
            requestId = requestId,
            fetchedAtMs = System.currentTimeMillis()
        )
        revision += 1
        return Result.success(requestId)
    }

    override fun createDirectory(path: String): Result<Unit> {
        val name = path.substringAfterLast('/')
        val entry = SftpEntryModel(name, path, SftpEntryKind.Directory, null, null, null, null)
        directory = directory.copy(entries = directory.entries + entry)
        revision += 1
        return Result.success(Unit)
    }

    override fun removeFile(path: String): Result<Unit> {
        directory = directory.copy(entries = directory.entries.filterNot { it.path == path })
        revision += 1
        return Result.success(Unit)
    }

    override fun removeDirectory(path: String): Result<Unit> {
        directory = directory.copy(
            entries = directory.entries.filterNot { it.path == path || it.path.startsWith("$path/") }
        )
        revision += 1
        return Result.success(Unit)
    }

    override fun rename(from: String, to: String): Result<Unit> {
        directory = directory.copy(
            entries = directory.entries.map { entry ->
                if (entry.path == from) {
                    entry.copy(name = to.substringAfterLast('/'), path = to)
                } else {
                    entry
                }
            }
        )
        revision += 1
        return Result.success(Unit)
    }

    override fun answerHostKey(requestId: Long, decision: HostKeyDecision): Result<Unit> {
        val pending = pendingRequest ?: return Result.failure(IllegalStateException("no request"))
        if (pending.requestId != requestId) {
            return Result.failure(IllegalStateException("expired request"))
        }
        pendingRequest = null
        phase = if (decision == HostKeyDecision.Reject) SftpPhase.Failed else SftpPhase.Ready
        revision += 1
        return Result.success(Unit)
    }

    override fun answerKeyboardInteractive(requestId: Long, answers: List<String>): Result<Unit> {
        val pending = pendingRequest ?: return Result.failure(IllegalStateException("no request"))
        if (pending.requestId != requestId) {
            return Result.failure(IllegalStateException("expired request"))
        }
        pendingRequest = null
        phase = SftpPhase.Ready
        revision += 1
        return Result.success(Unit)
    }

    override suspend fun uploadFile(
        file: LocalUploadHandle,
        remotePath: String
    ): Result<SftpTransferHandle> = queue(
        direction = SftpTransferDirection.Upload,
        sourceLabel = file.name,
        destination = remotePath
    )

    override suspend fun uploadDirectory(
        treeUri: String,
        relativePath: String,
        label: String,
        remotePath: String
    ): Result<SftpTransferHandle> = queue(
        direction = SftpTransferDirection.Upload,
        sourceLabel = label.ifBlank { relativePath },
        destination = remotePath
    )

    override suspend fun download(
        remotePath: String,
        treeUri: String,
        targetRelative: String
    ): Result<SftpTransferHandle> = queue(
        direction = SftpTransferDirection.Download,
        sourceLabel = targetRelative,
        destination = remotePath
    )

    override suspend fun disconnect() {
        phase = SftpPhase.Closed
        transfers.values.forEach { it.finish(SftpTransferStateModel.Cancelled) }
        revision += 1
    }

    private fun queue(
        direction: SftpTransferDirection,
        sourceLabel: String,
        destination: String
    ): Result<SftpTransferHandle> {
        val id = nextTransferId++
        val transfer = FakeTransfer(
            connectionId,
            SftpTransferModel(
                transferId = id,
                connectionId = connectionId,
                direction = direction,
                sourceLabel = sourceLabel,
                destination = destination,
                state = SftpTransferStateModel.Running,
                bytesComplete = 0,
                bytesTotal = 1024,
                error = null,
                failureKind = SftpTransferFailureKindModel.None,
                childCount = 0,
                childrenDone = 0,
                childrenFailed = 0,
                childrenCancelled = 0,
                children = emptyList()
            )
        )
        transfers[id] = transfer
        revision += 1
        return Result.success(transfer)
    }
}

private class FakeTransfer(
    override val connectionId: Long,
    private var model: SftpTransferModel
) : SftpTransferHandle {
    override val transferId: Long get() = model.transferId

    override fun snapshot(): SftpTransferModel = model

    override fun pause(): Result<Unit> {
        model = model.copy(state = SftpTransferStateModel.Paused)
        return Result.success(Unit)
    }

    override fun resume(): Result<Unit> {
        model = model.copy(state = SftpTransferStateModel.Running)
        return Result.success(Unit)
    }

    override fun cancel(): Result<Unit> {
        model = model.copy(state = SftpTransferStateModel.Cancelled)
        return Result.success(Unit)
    }

    fun finish(state: SftpTransferStateModel) {
        model = model.copy(state = state, bytesComplete = model.bytesTotal ?: 0)
    }
}

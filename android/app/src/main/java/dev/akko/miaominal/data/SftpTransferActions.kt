package dev.akko.miaominal.data

import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.delay
import kotlinx.coroutines.withTimeoutOrNull
import java.util.concurrent.ConcurrentHashMap

data class TransferKey(val connectionId: Long, val transferId: Long)

/** Recipes contain URIs, never live descriptors. They deliberately survive only this process. */
sealed interface TransferRequest {
    val hostId: String
    val destination: String
    data class UploadFile(override val hostId: String, val uri: String, val name: String,
        override val destination: String) : TransferRequest
    data class UploadTree(override val hostId: String, val treeUri: String, val relativePath: String,
        val label: String, override val destination: String) : TransferRequest
    data class Download(override val hostId: String, val remotePath: String, val treeUri: String,
        override val destination: String) : TransferRequest
}

enum class TransferRetryFailure { InProgress, SourceUnavailable, Reconnect, OriginalHost, DestinationCheck, ConfirmReplacement, NotFailed }
class TransferRetryException(val reason: TransferRetryFailure) : IllegalStateException(reason.name)

class SftpTransferActions(private val repository: MiaominalRepository) {
    private val requests = ConcurrentHashMap<TransferKey, TransferRequest>()
    private val locks = ConcurrentHashMap<TransferKey, Mutex>()
    private val hidden = ConcurrentHashMap.newKeySet<TransferKey>()
    private val failures = ConcurrentHashMap<TransferKey, Pair<String, SftpTransferModel>>()
    private val replacements = ConcurrentHashMap<TransferKey, TransferKey>()

    fun retainFailures(hostId: String, transfers: List<SftpTransferModel>) {
        transfers.filter { it.state == SftpTransferStateModel.Failed }.forEach {
            failures[TransferKey(it.connectionId, it.transferId)] = hostId to it
        }
    }
    fun failureHistory(hostId: String): List<SftpTransferModel> = failures.values.filter { it.first == hostId }.map { it.second }

    fun record(handle: SftpTransferHandle, request: TransferRequest) {
        requests[TransferKey(handle.connectionId, handle.transferId)] = request
        repository.trackSftpTransfer(handle.connectionId, handle)
    }

    fun request(key: TransferKey): TransferRequest? = requests[key]
    fun isHidden(key: TransferKey): Boolean = key in hidden
    fun hideCompleted(transfers: List<SftpTransferModel>) {
        transfers.filter { it.state == SftpTransferStateModel.Done }.forEach {
            hidden += TransferKey(it.connectionId, it.transferId)
        }
    }

    /** The UI must confirm destination replacement before calling this. Never resume a failed handle. */
    suspend fun retry(key: TransferKey, connectionId: Long, allowOverwrite: Boolean = false): Result<SftpTransferHandle> {
        val lock = locks.computeIfAbsent(key) { Mutex() }
        if (!lock.tryLock()) return Result.failure(TransferRetryException(TransferRetryFailure.InProgress))
        try {
            val request = requests[key] ?: return Result.failure(TransferRetryException(TransferRetryFailure.SourceUnavailable))
            val session = repository.sftpSessionHandle(connectionId)
                ?: return Result.failure(TransferRetryException(TransferRetryFailure.Reconnect))
            if (session.snapshot().hostId != request.hostId || session.snapshot().phase != SftpPhase.Ready)
                return Result.failure(TransferRetryException(TransferRetryFailure.OriginalHost))
            replacements[key]?.let { previous ->
                repository.sftpTransferHandle(previous.connectionId, previous.transferId)?.let { running ->
                    if (running.snapshot()?.state?.isTerminal == false) return Result.success(running)
                }
            }
            val original = repository.sftpTransferHandle(key.connectionId, key.transferId)?.snapshot()
            if (original != null && original.state != SftpTransferStateModel.Failed)
                return Result.failure(TransferRetryException(TransferRetryFailure.NotFailed))
            if (request !is TransferRequest.Download) {
                val parent = request.destination.substringBeforeLast('/', ".").ifBlank { "/" }
                val listingId = session.listDirectory(parent).getOrElse { return Result.failure(it) }
                val listing = withTimeoutOrNull(5000L) {
                    while (true) {
                        val snapshot = session.directorySnapshot()
                        if (snapshot.requestId == listingId && !snapshot.loading) return@withTimeoutOrNull snapshot
                        if (session.snapshot().phase != SftpPhase.Ready) return@withTimeoutOrNull null
                        delay(50L)
                    }
                    @Suppress("UNREACHABLE_CODE") null
                } ?: return Result.failure(TransferRetryException(TransferRetryFailure.DestinationCheck))
                if (listing.error != null) return Result.failure(IllegalStateException(listing.error))
                if (!allowOverwrite && listing.entries.any { it.name == request.destination.substringAfterLast('/') })
                    return Result.failure(TransferRetryException(TransferRetryFailure.ConfirmReplacement))
            }
            val result = when (request) {
                is TransferRequest.Download -> {
                    val listing = repository.listSafDirectory(request.treeUri, request.destination.substringBeforeLast('/', ""))
                        .getOrElse { return Result.failure(it) }
                    if (!allowOverwrite && listing.any { it.name == request.destination.substringAfterLast('/') })
                        return Result.failure(TransferRetryException(TransferRetryFailure.ConfirmReplacement))
                    session.download(request.remotePath, request.treeUri, request.destination)
                }
                is TransferRequest.UploadFile -> {
                    val source = repository.registerUploadFile(request.uri, request.name)
                        .getOrElse { return Result.failure(it) }
                    session.uploadFile(source, request.destination).onFailure { source.close() }
                }
                is TransferRequest.UploadTree -> {
                    repository.listSafDirectory(request.treeUri, request.relativePath.substringBeforeLast('/', ""))
                        .getOrElse { return Result.failure(it) }
                    session.uploadDirectory(request.treeUri, request.relativePath, request.label, request.destination)
                }
            }
            result.onSuccess { record(it, request); replacements[key] = TransferKey(it.connectionId, it.transferId) }
            return result
        } finally { lock.unlock() }
    }
}

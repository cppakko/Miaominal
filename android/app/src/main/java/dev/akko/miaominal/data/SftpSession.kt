package dev.akko.miaominal.data

import dev.akko.miaominal.nativebridge.LocalFileHandle
import dev.akko.miaominal.nativebridge.SftpDirectorySnapshot
import dev.akko.miaominal.nativebridge.SftpEntryDto
import dev.akko.miaominal.nativebridge.SftpEntryKindDto
import dev.akko.miaominal.nativebridge.SftpOperation
import dev.akko.miaominal.nativebridge.SftpOperationError
import dev.akko.miaominal.nativebridge.SftpRequest
import dev.akko.miaominal.nativebridge.SftpRequestKind
import dev.akko.miaominal.nativebridge.SftpSession
import dev.akko.miaominal.nativebridge.SftpSessionPhase
import dev.akko.miaominal.nativebridge.SftpSessionSnapshot
import dev.akko.miaominal.nativebridge.SftpTransfer
import dev.akko.miaominal.nativebridge.SftpTransferChildDto
import dev.akko.miaominal.nativebridge.SftpTransferChildStateDto
import dev.akko.miaominal.nativebridge.SftpTransferDirection as NativeSftpTransferDirection
import dev.akko.miaominal.nativebridge.SftpTransferFailureKind
import dev.akko.miaominal.nativebridge.SftpTransferSnapshot
import dev.akko.miaominal.nativebridge.SftpTransferState

/** Connection phase of one interactive SFTP session. */
enum class SftpPhase {
    Connecting,
    WaitingForTrust,
    WaitingForAuthentication,
    OpeningSubsystem,
    Ready,
    Closing,
    Closed,
    Failed;

    val isTerminal: Boolean get() = this == Closed || this == Failed

    companion object {
        fun from(phase: SftpSessionPhase): SftpPhase = when (phase) {
            SftpSessionPhase.CONNECTING -> Connecting
            SftpSessionPhase.WAITING_FOR_TRUST -> WaitingForTrust
            SftpSessionPhase.WAITING_FOR_AUTHENTICATION -> WaitingForAuthentication
            SftpSessionPhase.OPENING_SUBSYSTEM -> OpeningSubsystem
            SftpSessionPhase.READY -> Ready
            SftpSessionPhase.CLOSING -> Closing
            SftpSessionPhase.CLOSED -> Closed
            SftpSessionPhase.FAILED -> Failed
        }
    }
}

enum class SftpEntryKind { File, Directory, Symlink, Other }

data class SftpEntryModel(
    val name: String,
    val path: String,
    val kind: SftpEntryKind,
    val size: Long?,
    val modifiedAtMs: Long?,
    val attributes: String?,
    val owner: String?
) {
    val isDirectory: Boolean get() = kind == SftpEntryKind.Directory
}

data class SftpDirectoryModel(
    val path: String,
    val entries: List<SftpEntryModel>,
    val error: String?,
    val loading: Boolean,
    val requestId: Long,
    val fetchedAtMs: Long
)

data class SftpRequestModel(
    val requestId: Long,
    val connectionId: Long,
    val kind: SessionRequestKind,
    val hopLabel: String,
    val hopIndex: Int,
    val hopTotal: Int,
    val host: String,
    val port: Int,
    val fingerprint: String,
    val previousFingerprint: String?,
    val keyChanged: Boolean,
    val name: String,
    val instructions: String,
    val prompts: List<SessionPromptModel>
) {
    fun toSessionRequest(): SessionRequestModel = SessionRequestModel(
        requestId = requestId,
        connectionId = connectionId,
        kind = kind,
        hopLabel = hopLabel,
        hopIndex = hopIndex,
        hopTotal = hopTotal,
        host = host,
        port = port,
        algorithm = "",
        fingerprint = fingerprint,
        previousFingerprint = previousFingerprint,
        keyChanged = keyChanged,
        name = name,
        instructions = instructions,
        prompts = prompts
    )
}

enum class SftpOperationModel {
    CreateDirectory,
    RemoveFile,
    RemoveDirectory,
    Rename,
    ListDirectory,
    ListSubdirectory
}

data class SftpOperationErrorModel(
    val sequence: Long,
    val operation: SftpOperationModel,
    val path: String?,
    val message: String
)

data class SftpSessionModel(
    val connectionId: Long,
    val hostId: String,
    val hostLabel: String,
    val phase: SftpPhase,
    val failure: String?,
    val hopLabel: String?,
    val request: SftpRequestModel?,
    val revision: Long,
    val transferCount: Int,
    val activeTransferCount: Int,
    val operationError: SftpOperationErrorModel?
)

enum class SftpTransferDirection { Upload, Download }

enum class SftpTransferStateModel {
    Queued,
    Running,
    Paused,
    Done,
    Cancelled,
    Failed;

    val isTerminal: Boolean get() = this == Done || this == Cancelled || this == Failed
}

enum class SftpTransferFailureKindModel { None, Permission, NoSpace, ProviderRefused, LocalHandle, Network, Other }

data class SftpTransferChildModel(
    val childId: Long,
    val relativePath: String,
    val bytesComplete: Long,
    val bytesTotal: Long?,
    val state: SftpTransferStateModel,
    val error: String?
)

data class SftpTransferModel(
    val transferId: Long,
    val connectionId: Long,
    val direction: SftpTransferDirection,
    val sourceLabel: String,
    val destination: String,
    val state: SftpTransferStateModel,
    val bytesComplete: Long,
    val bytesTotal: Long?,
    val error: String?,
    val failureKind: SftpTransferFailureKindModel,
    val childCount: Int,
    val childrenDone: Int,
    val childrenFailed: Int,
    val childrenCancelled: Int,
    val children: List<SftpTransferChildModel>
) {
    val progress: Float
        get() {
            val total = bytesTotal ?: return if (state.isTerminal) 1f else 0f
            if (total <= 0L) return if (state.isTerminal) 1f else 0f
            return (bytesComplete.toDouble() / total.toDouble()).toFloat().coerceIn(0f, 1f)
        }
}

/** One registered upload document whose descriptor Rust owns. */
interface LocalUploadHandle {
    val name: String
    fun close()
}

/**
 * One live SFTP session as seen by the UI. Implementations may be the native
 * backend or a test double; screens only depend on this contract.
 */
interface SftpSessionHandle {
    val connectionId: Long

    fun snapshot(): SftpSessionModel
    fun directorySnapshot(): SftpDirectoryModel
    fun transfersSnapshot(): List<SftpTransferModel>

    fun listDirectory(path: String): Result<Long>
    fun createDirectory(path: String): Result<Unit>
    fun removeFile(path: String): Result<Unit>
    fun removeDirectory(path: String): Result<Unit>
    fun rename(from: String, to: String): Result<Unit>

    fun answerHostKey(requestId: Long, decision: HostKeyDecision): Result<Unit>
    fun answerKeyboardInteractive(requestId: Long, answers: List<String>): Result<Unit>

    suspend fun uploadFile(file: LocalUploadHandle, remotePath: String): Result<SftpTransferHandle>
    suspend fun uploadDirectory(
        treeUri: String,
        relativePath: String,
        label: String,
        remotePath: String
    ): Result<SftpTransferHandle>

    suspend fun download(remotePath: String, treeUri: String, targetRelative: String): Result<SftpTransferHandle>

    /** Explicit cancellation; idempotent. */
    suspend fun disconnect()
}

/** Control surface of one queued or running transfer. */
interface SftpTransferHandle {
    /** Session that owns the transfer; handles survive page rebuilds. */
    val connectionId: Long
    val transferId: Long
    fun snapshot(): SftpTransferModel?
    fun pause(): Result<Unit>
    fun resume(): Result<Unit>
    fun cancel(): Result<Unit>
}

/** Native-backed handle around the generated UniFFI session object. */
class NativeSftpSessionHandle(private val session: SftpSession) : SftpSessionHandle {

    override val connectionId: Long = session.connectionId().toLong()

    override fun snapshot(): SftpSessionModel = session.snapshot().toModel()

    override fun directorySnapshot(): SftpDirectoryModel = session.directorySnapshot().toModel()

    override fun transfersSnapshot(): List<SftpTransferModel> =
        session.transfersSnapshot().map { it.toModel() }

    override fun listDirectory(path: String): Result<Long> =
        runCatching { session.listDirectory(path).toLong() }

    override fun createDirectory(path: String): Result<Unit> =
        runCatching { session.createDirectory(path) }

    override fun removeFile(path: String): Result<Unit> =
        runCatching { session.removeFile(path) }

    override fun removeDirectory(path: String): Result<Unit> =
        runCatching { session.removeDirectory(path) }

    override fun rename(from: String, to: String): Result<Unit> =
        runCatching { session.rename(from, to) }

    override fun answerHostKey(requestId: Long, decision: HostKeyDecision): Result<Unit> =
        runCatching { session.answerHostKey(requestId.toULong(), decision.toNative()) }

    override fun answerKeyboardInteractive(requestId: Long, answers: List<String>): Result<Unit> =
        runCatching { session.answerKeyboardInteractive(requestId.toULong(), answers) }

    override suspend fun uploadFile(
        file: LocalUploadHandle,
        remotePath: String
    ): Result<SftpTransferHandle> = runCatching {
        val native = (file as? NativeLocalUploadHandle)
            ?: error("upload handles must come from NativeMiaominalRepository")
        NativeSftpTransferHandle(connectionId, session.uploadFile(native.handle, remotePath))
    }

    override suspend fun uploadDirectory(
        treeUri: String,
        relativePath: String,
        label: String,
        remotePath: String
    ): Result<SftpTransferHandle> = runCatching {
        NativeSftpTransferHandle(connectionId, session.uploadDirectory(treeUri, relativePath, label, remotePath))
    }

    override suspend fun download(
        remotePath: String,
        treeUri: String,
        targetRelative: String
    ): Result<SftpTransferHandle> = runCatching {
        NativeSftpTransferHandle(connectionId, session.download(remotePath, treeUri, targetRelative))
    }

    override suspend fun disconnect() {
        runCatching { session.disconnect() }
    }
}

class NativeSftpTransferHandle(
    override val connectionId: Long,
    private val transfer: SftpTransfer
) : SftpTransferHandle {
    override val transferId: Long = transfer.transferId().toLong()

    override fun snapshot(): SftpTransferModel? = transfer.snapshot()?.toModel()

    override fun pause(): Result<Unit> = runCatching { transfer.pause() }

    override fun resume(): Result<Unit> = runCatching { transfer.resume() }

    override fun cancel(): Result<Unit> = runCatching { transfer.cancel() }
}

class NativeLocalUploadHandle(val handle: LocalFileHandle, override val name: String) : LocalUploadHandle {
    override fun close() {
        runCatching { handle.release() }
    }
}

fun SftpSessionSnapshot.toModel(): SftpSessionModel = SftpSessionModel(
    connectionId = connectionId.toLong(),
    hostId = hostId,
    hostLabel = hostLabel,
    phase = SftpPhase.from(phase),
    failure = failure,
    hopLabel = hopLabel,
    request = request?.toModel(),
    revision = revision.toLong(),
    transferCount = transferCount.toInt(),
    activeTransferCount = activeTransferCount.toInt(),
    operationError = operationError?.toModel()
)

fun SftpOperationError.toModel(): SftpOperationErrorModel = SftpOperationErrorModel(
    sequence = sequence.toLong(),
    operation = when (operation) {
        SftpOperation.CREATE_DIRECTORY -> SftpOperationModel.CreateDirectory
        SftpOperation.REMOVE_FILE -> SftpOperationModel.RemoveFile
        SftpOperation.REMOVE_DIRECTORY -> SftpOperationModel.RemoveDirectory
        SftpOperation.RENAME -> SftpOperationModel.Rename
        SftpOperation.LIST_DIRECTORY -> SftpOperationModel.ListDirectory
        SftpOperation.LIST_SUBDIRECTORY -> SftpOperationModel.ListSubdirectory
    },
    path = path,
    message = message
)

fun SftpRequest.toModel(): SftpRequestModel = SftpRequestModel(
    requestId = requestId.toLong(),
    connectionId = connectionId.toLong(),
    kind = when (kind) {
        SftpRequestKind.HOST_KEY -> SessionRequestKind.HostKey
        SftpRequestKind.KEYBOARD_INTERACTIVE -> SessionRequestKind.KeyboardInteractive
    },
    hopLabel = hopLabel,
    hopIndex = hopIndex.toInt(),
    hopTotal = hopTotal.toInt(),
    host = host,
    port = port.toInt(),
    fingerprint = fingerprint,
    previousFingerprint = previousFingerprint,
    keyChanged = keyChanged,
    name = name,
    instructions = instructions,
    prompts = prompts.map { SessionPromptModel(it.prompt, it.echo) }
)

fun SftpDirectorySnapshot.toModel(): SftpDirectoryModel = SftpDirectoryModel(
    path = path,
    entries = entries.map { it.toModel() },
    error = error,
    loading = loading,
    requestId = requestId.toLong(),
    fetchedAtMs = fetchedAtMs.toLong()
)

fun SftpEntryDto.toModel(): SftpEntryModel = SftpEntryModel(
    name = name,
    path = path,
    kind = when (kind) {
        SftpEntryKindDto.FILE -> SftpEntryKind.File
        SftpEntryKindDto.DIRECTORY -> SftpEntryKind.Directory
        SftpEntryKindDto.SYMLINK -> SftpEntryKind.Symlink
        SftpEntryKindDto.OTHER -> SftpEntryKind.Other
    },
    size = size?.toLong(),
    modifiedAtMs = modifiedAtMs?.toLong(),
    attributes = attributes,
    owner = owner
)

fun SftpTransferSnapshot.toModel(): SftpTransferModel = SftpTransferModel(
    transferId = transferId.toLong(),
    connectionId = connectionId.toLong(),
    direction = when (direction) {
        NativeSftpTransferDirection.UPLOAD -> SftpTransferDirection.Upload
        NativeSftpTransferDirection.DOWNLOAD -> SftpTransferDirection.Download
    },
    sourceLabel = sourceLabel,
    destination = destination,
    state = state.toModel(),
    bytesComplete = bytesComplete.toLong(),
    bytesTotal = bytesTotal?.toLong(),
    error = error,
    failureKind = when (failureKind) {
        SftpTransferFailureKind.NONE -> SftpTransferFailureKindModel.None
        SftpTransferFailureKind.PERMISSION -> SftpTransferFailureKindModel.Permission
        SftpTransferFailureKind.NO_SPACE -> SftpTransferFailureKindModel.NoSpace
        SftpTransferFailureKind.PROVIDER_REFUSED -> SftpTransferFailureKindModel.ProviderRefused
        SftpTransferFailureKind.LOCAL_HANDLE -> SftpTransferFailureKindModel.LocalHandle
        SftpTransferFailureKind.NETWORK -> SftpTransferFailureKindModel.Network
        SftpTransferFailureKind.OTHER -> SftpTransferFailureKindModel.Other
    },
    childCount = childCount.toInt(),
    childrenDone = childrenDone.toInt(),
    childrenFailed = childrenFailed.toInt(),
    childrenCancelled = childrenCancelled.toInt(),
    children = children.map { it.toModel() }
)

private fun SftpTransferChildDto.toModel(): SftpTransferChildModel = SftpTransferChildModel(
    childId = childId.toLong(),
    relativePath = relativePath,
    bytesComplete = bytesComplete.toLong(),
    bytesTotal = bytesTotal?.toLong(),
    state = state.toModel(),
    error = error
)

private fun SftpTransferChildStateDto.toModel(): SftpTransferStateModel = when (this) {
    SftpTransferChildStateDto.RUNNING -> SftpTransferStateModel.Running
    SftpTransferChildStateDto.DONE -> SftpTransferStateModel.Done
    SftpTransferChildStateDto.CANCELLED -> SftpTransferStateModel.Cancelled
    SftpTransferChildStateDto.FAILED -> SftpTransferStateModel.Failed
}

private fun SftpTransferState.toModel(): SftpTransferStateModel = when (this) {
    SftpTransferState.QUEUED -> SftpTransferStateModel.Queued
    SftpTransferState.RUNNING -> SftpTransferStateModel.Running
    SftpTransferState.PAUSED -> SftpTransferStateModel.Paused
    SftpTransferState.DONE -> SftpTransferStateModel.Done
    SftpTransferState.CANCELLED -> SftpTransferStateModel.Cancelled
    SftpTransferState.FAILED -> SftpTransferStateModel.Failed
}



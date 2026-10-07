package dev.akko.miaominal.data

import dev.akko.miaominal.nativebridge.SshHostKeyDecision
import dev.akko.miaominal.nativebridge.SshRequestKind
import dev.akko.miaominal.nativebridge.SshSession
import dev.akko.miaominal.nativebridge.SshSessionPhase
import dev.akko.miaominal.nativebridge.SshSessionRequest
import dev.akko.miaominal.nativebridge.SshSessionSnapshot
import dev.akko.miaominal.nativebridge.MonitorFailureKind as NativeMonitorFailureKind
import dev.akko.miaominal.nativebridge.MonitorStatus as NativeMonitorStatus
import dev.akko.miaominal.nativebridge.TerminalFrameCell
import dev.akko.miaominal.nativebridge.TerminalFrameModes
import dev.akko.miaominal.nativebridge.TerminalRenderFrame
import dev.akko.miaominal.nativebridge.TerminalSelectionDto
import dev.akko.miaominal.nativebridge.TerminalSelectionKindDto
import dev.akko.miaominal.nativebridge.TerminalUpdate

/** Connection phase of one interactive session. */
enum class SessionPhase {
    Connecting,
    WaitingForTrust,
    WaitingForAuthentication,
    OpeningShell,
    Ready,
    Closing,
    Closed,
    Failed;

    val isTerminal: Boolean get() = this == Closed || this == Failed

    companion object {
        fun from(phase: SshSessionPhase): SessionPhase = when (phase) {
            SshSessionPhase.CONNECTING -> Connecting
            SshSessionPhase.WAITING_FOR_TRUST -> WaitingForTrust
            SshSessionPhase.WAITING_FOR_AUTHENTICATION -> WaitingForAuthentication
            SshSessionPhase.OPENING_SHELL -> OpeningShell
            SshSessionPhase.READY -> Ready
            SshSessionPhase.CLOSING -> Closing
            SshSessionPhase.CLOSED -> Closed
            SshSessionPhase.FAILED -> Failed
        }
    }
}

enum class SessionRequestKind { HostKey, KeyboardInteractive }

enum class HostKeyDecision { AcceptOnce, AcceptAndSave, Reject }

data class SessionPromptModel(val prompt: String, val echo: Boolean)

/** Typed pending trust/authentication request; survives page rebuilds. */
data class SessionRequestModel(
    val requestId: Long,
    val connectionId: Long,
    val kind: SessionRequestKind,
    val hopLabel: String,
    val hopIndex: Int,
    val hopTotal: Int,
    val host: String,
    val port: Int,
    val algorithm: String,
    val fingerprint: String,
    val previousFingerprint: String?,
    val keyChanged: Boolean,
    val name: String,
    val instructions: String,
    val prompts: List<SessionPromptModel>
)

data class SessionSnapshotModel(
    val connectionId: Long,
    val hostId: String,
    val hostLabel: String,
    val phase: SessionPhase,
    val failure: String?,
    val hopLabel: String?,
    val request: SessionRequestModel?,
    val columns: Int,
    val lines: Int,
    val revision: Long,
    val exitCode: Int?,
    val exitSignal: String?
)

data class TerminalCellModel(val text: String, val fg: Int, val bg: Int, val flags: Int)

data class TerminalRowModel(val row: Int, val cells: List<TerminalCellModel>)

data class TerminalSelectionModel(
    val startLine: Int,
    val startColumn: Int,
    val endLine: Int,
    val endColumn: Int,
    val block: Boolean
)

data class TerminalModesModel(
    val bracketedPaste: Boolean,
    val appCursor: Boolean,
    val appKeypad: Boolean,
    val focusEvents: Boolean,
    val mouseProtocol: Int,
    val mouseEncoding: Int,
    val alternateScreen: Boolean
)

data class TerminalFrameModel(
    val frameVersion: Long,
    val revision: Long,
    val full: Boolean,
    val columns: Int,
    val lines: Int,
    val cursorVisible: Boolean,
    val cursorLine: Int,
    val cursorColumn: Int,
    val focused: Boolean,
    val displayOffset: Int,
    val historySize: Int,
    val defaultFg: Int,
    val defaultBg: Int,
    val selection: TerminalSelectionModel?,
    val searchTotal: Int,
    val searchCurrent: Int?,
    val modes: TerminalModesModel,
    val rows: List<TerminalRowModel>,
    val closed: Boolean
)

data class TerminalUpdateModel(val revision: Long, val phase: SessionPhase, val idle: Boolean, val closed: Boolean)

/** Sampling state of the remote monitor; samples are pulled, never pushed. */
enum class MonitorStatusModel {
    Disabled,
    Waiting,
    Ready,
    Failed;

    companion object {
        fun from(status: NativeMonitorStatus): MonitorStatusModel = when (status) {
            NativeMonitorStatus.DISABLED -> Disabled
            NativeMonitorStatus.WAITING -> Waiting
            NativeMonitorStatus.READY -> Ready
            NativeMonitorStatus.FAILED -> Failed
        }
    }
}

/** Typed monitoring failure, so the UI never parses English log strings. */
enum class MonitorFailureKindModel {
    None,
    UnsupportedPlatform,
    CommandFailed,
    Transport,
    Other;

    companion object {
        fun from(kind: NativeMonitorFailureKind): MonitorFailureKindModel = when (kind) {
            NativeMonitorFailureKind.NONE -> None
            NativeMonitorFailureKind.UNSUPPORTED_PLATFORM -> UnsupportedPlatform
            NativeMonitorFailureKind.COMMAND_FAILED -> CommandFailed
            NativeMonitorFailureKind.TRANSPORT -> Transport
            NativeMonitorFailureKind.OTHER -> Other
        }
    }
}

data class MonitorMetricsModel(
    val platform: String,
    val hostname: String?,
    val logicalCpuCount: Int?,
    val uptimeSeconds: Long?,
    val cpuPercent: Double,
    val memoryPercent: Double,
    val memoryUsedBytes: Long,
    val memoryTotalBytes: Long,
    val swapPercent: Double,
    val swapUsedBytes: Long,
    val swapTotalBytes: Long,
    val diskPercent: Double,
    val diskUsedBytes: Long?,
    val diskTotalBytes: Long?,
    val networkRxKbps: Double,
    val networkTxKbps: Double,
    val load: Double
)

/** Latest monitor state; replaces the previous sample instead of queueing. */
data class MonitorSnapshotModel(
    val enabled: Boolean,
    val status: MonitorStatusModel,
    val failureKind: MonitorFailureKindModel,
    val failure: String?,
    val metrics: MonitorMetricsModel?,
    val sampleSequence: Long,
    val updatedAtMs: Long
)

/** Cell attribute bits; kept in sync with the Rust bridge. */
object TerminalCellFlags {
    const val BOLD = 1 shl 0
    const val ITALIC = 1 shl 1
    const val DIM = 1 shl 2
    const val UNDERLINE = 1 shl 3
    const val STRIKETHROUGH = 1 shl 4
    const val WIDE = 1 shl 5
    const val SPACER = 1 shl 6
    const val CURSOR = 1 shl 7
    const val SEARCH_MATCH = 1 shl 8
    const val SEARCH_CURRENT = 1 shl 9
}

object TerminalSelectionKind {
    const val SIMPLE = 0
    const val BLOCK = 1
    const val SEMANTIC = 2
    const val LINES = 3
}

/**
 * One live session as seen by the UI. Implementations may be the native
 * backend or a test double; the screens only depend on this contract.
 */
interface TerminalSessionHandle {
    val connectionId: Long
    var focused: Boolean

    fun snapshot(): SessionSnapshotModel
    fun renderFrame(sinceFrameVersion: Long): TerminalFrameModel
    suspend fun awaitUpdate(sinceRevision: Long, timeoutMs: Long): TerminalUpdateModel

    fun writeBytes(bytes: ByteArray)
    fun handleKey(
        key: String,
        keyChar: String?,
        control: Boolean,
        alt: Boolean,
        shift: Boolean,
        phase: Int = 0
    ): Boolean
    fun paste(text: String)
    fun resize(columns: Int, lines: Int)
    fun scroll(lines: Int)
    fun scrollTo(displayOffset: Int)
    fun search(pattern: String?): Int
    fun nextMatch()
    fun prevMatch()

    fun startSelection(line: Int, column: Int, side: Int, kind: Int)
    fun updateSelection(line: Int, column: Int, side: Int)
    fun clearSelection()
    fun selectionText(): String?

    fun respondHostKey(requestId: Long, decision: HostKeyDecision)
    fun respondKeyboardInteractive(requestId: Long, answers: List<String>)

    /** Enable or pause remote-monitor sampling for this session. */
    fun setMonitoring(enabled: Boolean): Result<Unit>

    /** Latest monitor state; samples replace each other. */
    fun monitorSnapshot(): MonitorSnapshotModel

    /** Explicit cancellation; idempotent. */
    fun cancel()
    suspend fun disconnect()
}

/** Native-backed handle around the generated UniFFI session object. */
class NativeTerminalSessionHandle(private val session: SshSession) : TerminalSessionHandle {

    override val connectionId: Long = session.connectionId().toLong()

    override var focused: Boolean = false
        set(value) {
            field = value
            session.setFocus(value)
        }

    override fun snapshot(): SessionSnapshotModel = session.snapshot().toModel()

    override fun renderFrame(sinceFrameVersion: Long): TerminalFrameModel =
        session.renderFrame(sinceFrameVersion.toULong()).toModel()

    override suspend fun awaitUpdate(sinceRevision: Long, timeoutMs: Long): TerminalUpdateModel =
        session.awaitUpdate(sinceRevision.toULong(), timeoutMs.toULong()).toModel()

    override fun writeBytes(bytes: ByteArray) {
        if (bytes.isEmpty()) return
        session.writeBytes(bytes)
    }

    override fun handleKey(
        key: String,
        keyChar: String?,
        control: Boolean,
        alt: Boolean,
        shift: Boolean,
        phase: Int
    ): Boolean = session.handleKey(key, keyChar, control, alt, shift, phase.toUInt())

    override fun paste(text: String) {
        session.paste(text)
    }

    override fun resize(columns: Int, lines: Int) {
        session.resize(columns.toUInt(), lines.toUInt())
    }

    override fun scroll(lines: Int) {
        session.scroll(lines)
    }

    override fun scrollTo(displayOffset: Int) {
        session.scrollTo(displayOffset.toUInt())
    }

    override fun search(pattern: String?): Int = session.search(pattern).toInt()

    override fun nextMatch() {
        session.nextMatch()
    }

    override fun prevMatch() {
        session.prevMatch()
    }

    override fun startSelection(line: Int, column: Int, side: Int, kind: Int) {
        session.startSelection(line, column.toUInt(), side.toUInt(), kind.toSelectionKind())
    }

    override fun updateSelection(line: Int, column: Int, side: Int) {
        session.updateSelection(line, column.toUInt(), side.toUInt())
    }

    override fun clearSelection() {
        session.clearSelection()
    }

    override fun selectionText(): String? = session.selectionText()

    override fun respondHostKey(requestId: Long, decision: HostKeyDecision) {
        session.respondHostKey(requestId.toULong(), decision.toNative())
    }

    override fun respondKeyboardInteractive(requestId: Long, answers: List<String>) {
        session.respondKeyboardInteractive(requestId.toULong(), answers)
    }

    override fun setMonitoring(enabled: Boolean): Result<Unit> =
        runCatching { session.setMonitoring(enabled) }

    override fun monitorSnapshot(): MonitorSnapshotModel = session.monitorSnapshot().toModel()

    override fun cancel() {
        session.cancel()
    }

    override suspend fun disconnect() {
        runCatching { session.disconnect() }
    }
}

fun SshSessionSnapshot.toModel(): SessionSnapshotModel = SessionSnapshotModel(
    connectionId = connectionId.toLong(),
    hostId = hostId,
    hostLabel = hostLabel,
    phase = SessionPhase.from(phase),
    failure = failure,
    hopLabel = hopLabel,
    request = request?.toModel(),
    columns = columns.toInt(),
    lines = lines.toInt(),
    revision = revision.toLong(),
    exitCode = exitCode?.toInt(),
    exitSignal = exitSignal
)

fun SshSessionRequest.toModel(): SessionRequestModel = SessionRequestModel(
    requestId = requestId.toLong(),
    connectionId = connectionId.toLong(),
    kind = when (kind) {
        SshRequestKind.HOST_KEY -> SessionRequestKind.HostKey
        SshRequestKind.KEYBOARD_INTERACTIVE -> SessionRequestKind.KeyboardInteractive
    },
    hopLabel = hopLabel,
    hopIndex = hopIndex.toInt(),
    hopTotal = hopTotal.toInt(),
    host = host,
    port = port.toInt(),
    algorithm = algorithm,
    fingerprint = fingerprint,
    previousFingerprint = previousFingerprint,
    keyChanged = keyChanged,
    name = name,
    instructions = instructions,
    prompts = prompts.map { SessionPromptModel(it.prompt, it.echo) }
)

private fun TerminalRenderFrame.toModel(): TerminalFrameModel = TerminalFrameModel(
    frameVersion = frameVersion.toLong(),
    revision = revision.toLong(),
    full = full,
    columns = columns.toInt(),
    lines = lines.toInt(),
    cursorVisible = cursorVisible,
    cursorLine = cursorLine,
    cursorColumn = cursorColumn.toInt(),
    focused = focused,
    displayOffset = displayOffset.toInt(),
    historySize = historySize.toInt(),
    defaultFg = defaultFg.toInt(),
    defaultBg = defaultBg.toInt(),
    selection = selection?.toModel(),
    searchTotal = searchTotal.toInt(),
    searchCurrent = searchCurrent?.toInt(),
    modes = modes.toModel(),
    rows = rows.map { row ->
        TerminalRowModel(row.row.toInt(), row.cells.map(::cellToModel))
    },
    closed = closed
)

private fun TerminalUpdate.toModel(): TerminalUpdateModel = TerminalUpdateModel(
    revision = revision.toLong(),
    phase = SessionPhase.from(phase),
    idle = idle,
    closed = closed
)

private fun dev.akko.miaominal.nativebridge.SshMonitorSnapshot.toModel(): MonitorSnapshotModel =
    MonitorSnapshotModel(
        enabled = enabled,
        status = MonitorStatusModel.from(status),
        failureKind = MonitorFailureKindModel.from(failureKind),
        failure = failure,
        metrics = metrics?.toModel(),
        sampleSequence = sampleSequence.toLong(),
        updatedAtMs = updatedAtMs.toLong()
    )

private fun dev.akko.miaominal.nativebridge.MonitorMetricsDto.toModel(): MonitorMetricsModel =
    MonitorMetricsModel(
        platform = platform,
        hostname = hostname,
        logicalCpuCount = logicalCpuCount?.toInt(),
        uptimeSeconds = uptimeSeconds?.toLong(),
        cpuPercent = cpuPercent,
        memoryPercent = memoryPercent,
        memoryUsedBytes = memoryUsedBytes.toLong(),
        memoryTotalBytes = memoryTotalBytes.toLong(),
        swapPercent = swapPercent,
        swapUsedBytes = swapUsedBytes.toLong(),
        swapTotalBytes = swapTotalBytes.toLong(),
        diskPercent = diskPercent,
        diskUsedBytes = diskUsedBytes?.toLong(),
        diskTotalBytes = diskTotalBytes?.toLong(),
        networkRxKbps = networkRxKbps,
        networkTxKbps = networkTxKbps,
        load = load
    )

private fun cellToModel(cell: TerminalFrameCell): TerminalCellModel =
    TerminalCellModel(cell.text, cell.fg.toInt(), cell.bg.toInt(), cell.flags.toInt())

private fun TerminalSelectionDto.toModel(): TerminalSelectionModel = TerminalSelectionModel(
    startLine = startLine,
    startColumn = startColumn.toInt(),
    endLine = endLine,
    endColumn = endColumn.toInt(),
    block = block
)

private fun TerminalFrameModes.toModel(): TerminalModesModel = TerminalModesModel(
    bracketedPaste = bracketedPaste,
    appCursor = appCursor,
    appKeypad = appKeypad,
    focusEvents = focusEvents,
    mouseProtocol = mouseProtocol.toInt(),
    mouseEncoding = mouseEncoding.toInt(),
    alternateScreen = alternateScreen
)

internal fun HostKeyDecision.toNative(): SshHostKeyDecision = when (this) {
    HostKeyDecision.AcceptOnce -> SshHostKeyDecision.ACCEPT_ONCE
    HostKeyDecision.AcceptAndSave -> SshHostKeyDecision.ACCEPT_AND_SAVE
    HostKeyDecision.Reject -> SshHostKeyDecision.REJECT
}

private fun Int.toSelectionKind(): TerminalSelectionKindDto = when (this) {
    TerminalSelectionKind.BLOCK -> TerminalSelectionKindDto.BLOCK
    TerminalSelectionKind.SEMANTIC -> TerminalSelectionKindDto.SEMANTIC
    TerminalSelectionKind.LINES -> TerminalSelectionKindDto.LINES
    else -> TerminalSelectionKindDto.SIMPLE
}

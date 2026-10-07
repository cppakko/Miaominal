package dev.akko.miaominal.data

import kotlinx.coroutines.delay
import java.nio.charset.StandardCharsets

/**
 * Scriptable in-memory session used by UI tests and Previews.
 *
 * Remote output is simulated by echoing writes into a text grid, so a test can
 * assert the input → render echo without a native backend.
 */
class FakeTerminalSession(
    override val connectionId: Long,
    hostId: String,
    private var columns: Int = 80,
    private var lines: Int = 24,
    var phase: SessionPhase = SessionPhase.Connecting
) : TerminalSessionHandle {

    var hostLabel: String = hostId
    var failure: String? = null

    @Volatile
    var revision: Long = 1

    /** Number of render pulls; the frame-budget test counts requests per second. */
    @Volatile
    var renderCalls: Long = 0
        private set

    private var frameVersion: Long = 0

    /** Text grid, one string per row; missing rows are blank. */
    val grid = mutableListOf<String>()
    val writtenBytes = mutableListOf<ByteArray>()
    val handledKeys = mutableListOf<String>()
    val resizes = mutableListOf<Pair<Int, Int>>()
    var request: SessionRequestModel? = null
    var respondedHostKeys = mutableListOf<Pair<Long, HostKeyDecision>>()
    var respondedKeyboardInteractive = mutableListOf<Pair<Long, List<String>>>()
    var selection: String? = null
    var searchPattern: String? = null
    var disconnected = false

    /** Simulates a write racing a disconnect: the next send throws once. */
    var failNextSend: Boolean = false

    private var displayOffset = 0

    private fun requireWritable() {
        if (failNextSend) {
            failNextSend = false
            throw IllegalStateException("session $connectionId is not writable")
        }
        check(!phase.isTerminal) { "session $connectionId is ${phase.name}" }
    }

    override var focused: Boolean = false

    @Synchronized
    override fun snapshot(): SessionSnapshotModel = SessionSnapshotModel(
        connectionId = connectionId,
        hostId = hostLabel,
        hostLabel = hostLabel,
        phase = phase,
        failure = failure,
        hopLabel = hostLabel,
        request = request,
        columns = columns,
        lines = lines,
        revision = revision,
        exitCode = null,
        exitSignal = null
    )

    @Synchronized
    override fun renderFrame(sinceFrameVersion: Long): TerminalFrameModel {
        frameVersion += 1
        renderCalls += 1
        val full = sinceFrameVersion != frameVersion - 1
        val rows = (0 until lines).map { row ->
            val text = grid.getOrNull(row).orEmpty()
            TerminalRowModel(
                row = row,
                cells = (0 until columns).map { column ->
                    TerminalCellModel(
                        text = text.getOrNull(column)?.toString() ?: " ",
                        fg = 0xFFE8EAED.toInt(),
                        bg = 0xFF101418.toInt(),
                        flags = 0
                    )
                }
            )
        }
        return TerminalFrameModel(
            frameVersion = frameVersion,
            revision = revision,
            full = full,
            columns = columns,
            lines = lines,
            cursorVisible = true,
            cursorLine = (grid.size - 1).coerceAtLeast(0),
            cursorColumn = (grid.lastOrNull()?.length ?: 0).coerceAtMost(columns),
            focused = focused,
            displayOffset = displayOffset,
            historySize = 0,
            defaultFg = 0xFFE8EAED.toInt(),
            defaultBg = 0xFF101418.toInt(),
            selection = null,
            searchTotal = if (searchPattern.isNullOrEmpty()) 0 else 1,
            searchCurrent = if (searchPattern.isNullOrEmpty()) null else 0,
            modes = TerminalModesModel(
                bracketedPaste = false,
                appCursor = false,
                appKeypad = false,
                focusEvents = false,
                mouseProtocol = 0,
                mouseEncoding = 0,
                alternateScreen = false
            ),
            rows = rows,
            closed = phase.isTerminal
        )
    }

    override suspend fun awaitUpdate(sinceRevision: Long, timeoutMs: Long): TerminalUpdateModel {
        if (revision != sinceRevision) {
            return TerminalUpdateModel(revision, phase, idle = false, closed = phase.isTerminal)
        }
        delay(timeoutMs.coerceAtLeast(1))
        return TerminalUpdateModel(revision, phase, idle = true, closed = phase.isTerminal)
    }

    override fun writeBytes(bytes: ByteArray) {
        requireWritable()
        writtenBytes += bytes
        echo(String(bytes, StandardCharsets.UTF_8))
    }

    override fun handleKey(
        key: String,
        keyChar: String?,
        control: Boolean,
        alt: Boolean,
        shift: Boolean,
        phase: Int
    ): Boolean {
        requireWritable()
        handledKeys += key
        when (key) {
            "enter", "return" -> echo("\n")
            "backspace" -> echo("\u007f")
            "up" -> echo("up")
            "down" -> echo("down")
            "left" -> echo("left")
            "right" -> echo("right")
            "escape" -> echo("escape")
            "tab" -> echo("tab")
            "c" -> if (control) echo("^C")
            else -> echo(if (shift) key.uppercase() else (keyChar ?: key))
        }
        return true
    }

    override fun paste(text: String) {
        requireWritable()
        writtenBytes += text.toByteArray(StandardCharsets.UTF_8)
        echo(text)
    }

    @Synchronized
    override fun resize(columns: Int, lines: Int) {
        this.columns = columns
        this.lines = lines
        resizes += columns to lines
        bumpRevision()
    }

    override fun scroll(lines: Int) {
        displayOffset = (displayOffset - lines).coerceAtLeast(0)
        bumpRevision()
    }

    override fun scrollTo(displayOffset: Int) {
        this.displayOffset = displayOffset.coerceAtLeast(0)
        bumpRevision()
    }

    override fun search(pattern: String?): Int {
        searchPattern = pattern
        bumpRevision()
        return if (pattern.isNullOrEmpty()) 0 else 1
    }

    override fun nextMatch() = bumpRevision()

    override fun prevMatch() = bumpRevision()

    override fun startSelection(line: Int, column: Int, side: Int, kind: Int) {
        selection = null
    }

    override fun updateSelection(line: Int, column: Int, side: Int) {
        selection = grid.getOrNull(line)
    }

    override fun clearSelection() {
        selection = null
    }

    override fun selectionText(): String? = selection

    override fun respondHostKey(requestId: Long, decision: HostKeyDecision) {
        respondedHostKeys += requestId to decision
        request = null
        bumpRevision()
    }

    override fun respondKeyboardInteractive(requestId: Long, answers: List<String>) {
        respondedKeyboardInteractive += requestId to answers
        request = null
        echo(answers.joinToString(""))
        bumpRevision()
    }

    var monitoringEnabled = false
    var monitor: MonitorSnapshotModel = MonitorSnapshotModel(
        enabled = false,
        status = MonitorStatusModel.Disabled,
        failureKind = MonitorFailureKindModel.None,
        failure = null,
        metrics = null,
        sampleSequence = 0,
        updatedAtMs = 0
    )

    /** Test helper: publish a monitor sample. */
    fun emitMonitor(cpuPercent: Double = 10.0) {
        monitor = MonitorSnapshotModel(
            enabled = true,
            status = MonitorStatusModel.Ready,
            failureKind = MonitorFailureKindModel.None,
            failure = null,
            metrics = MonitorMetricsModel(
                platform = "Linux",
                hostname = hostLabel,
                logicalCpuCount = 4,
                uptimeSeconds = 120,
                cpuPercent = cpuPercent,
                memoryPercent = 25.0,
                memoryUsedBytes = 1024,
                memoryTotalBytes = 4096,
                swapPercent = 0.0,
                swapUsedBytes = 0,
                swapTotalBytes = 0,
                diskPercent = 40.0,
                diskUsedBytes = 40,
                diskTotalBytes = 100,
                networkRxKbps = 1.5,
                networkTxKbps = 2.5,
                load = 0.4
            ),
            sampleSequence = monitor.sampleSequence + 1,
            updatedAtMs = 0
        )
    }

    /** Test helper: publish a typed monitor failure. */
    fun failMonitor(
        kind: MonitorFailureKindModel = MonitorFailureKindModel.UnsupportedPlatform,
        message: String = "unsupported remote OS"
    ) {
        monitor = monitor.copy(
            enabled = true,
            status = MonitorStatusModel.Failed,
            failureKind = kind,
            failure = message
        )
    }

    override fun setMonitoring(enabled: Boolean): Result<Unit> = runCatching {
        monitoringEnabled = enabled
        monitor = monitor.copy(
            enabled = enabled,
            status = if (enabled) MonitorStatusModel.Waiting else MonitorStatusModel.Disabled,
            failure = if (enabled) monitor.failure else null
        )
    }

    override fun monitorSnapshot(): MonitorSnapshotModel = monitor.copy(
        enabled = monitoringEnabled,
        status = when {
            !monitoringEnabled -> MonitorStatusModel.Disabled
            monitor.status == MonitorStatusModel.Failed -> MonitorStatusModel.Failed
            monitor.metrics != null -> MonitorStatusModel.Ready
            else -> MonitorStatusModel.Waiting
        }
    )

    override fun cancel() {
        if (!phase.isTerminal) {
            phase = SessionPhase.Closed
            bumpRevision()
        }
    }

    override suspend fun disconnect() {
        disconnected = true
        cancel()
    }

    /** Test helper: push remote output lines. */
    fun emit(text: String) {
        echo(text)
    }

    /** Test helper: move the session to a phase and refresh watchers. */
    fun transitionTo(next: SessionPhase, failure: String? = null) {
        phase = next
        this.failure = failure
        bumpRevision()
    }

    /** Test helper: publish a pending trust request. */
    fun awaitTrust(requestId: Long = 1L, host: String = hostLabel, changed: Boolean = false) {
        request = SessionRequestModel(
            requestId = requestId,
            connectionId = connectionId,
            kind = SessionRequestKind.HostKey,
            hopLabel = hostLabel,
            hopIndex = 1,
            hopTotal = 1,
            host = host,
            port = 22,
            algorithm = "ssh-ed25519",
            fingerprint = "SHA256:test",
            previousFingerprint = if (changed) "SHA256:old" else null,
            keyChanged = changed,
            name = "",
            instructions = "",
            prompts = emptyList()
        )
        phase = SessionPhase.WaitingForTrust
        bumpRevision()
    }

    /** Test helper: publish a pending keyboard-interactive request. */
    fun awaitAuthentication(requestId: Long = 2L, prompts: List<SessionPromptModel> = listOf(
        SessionPromptModel("Password: ", echo = false)
    )) {
        request = SessionRequestModel(
            requestId = requestId,
            connectionId = connectionId,
            kind = SessionRequestKind.KeyboardInteractive,
            hopLabel = hostLabel,
            hopIndex = 1,
            hopTotal = 1,
            host = hostLabel,
            port = 22,
            algorithm = "",
            fingerprint = "",
            previousFingerprint = null,
            keyChanged = false,
            name = "Two-factor",
            instructions = "Enter the code",
            prompts = prompts
        )
        phase = SessionPhase.WaitingForAuthentication
        bumpRevision()
    }

    @Synchronized
    private fun echo(text: String) {
        val normalized = text.replace("\r\n", "\n").replace('\r', '\n')
        normalized.split('\n').forEachIndexed { index, part ->
            if (index == 0) {
                if (grid.isEmpty()) {
                    grid.add(part)
                } else if (part.isNotEmpty()) {
                    grid[grid.size - 1] = grid.last() + part
                }
            } else {
                grid.add(part)
            }
        }
        while (grid.size > 2000) grid.removeAt(0)
        bumpRevision()
    }

    private fun bumpRevision() {
        revision += 1
    }
}

package dev.akko.miaominal.data.agent

import dev.akko.miaominal.data.ShellType
import dev.akko.miaominal.nativebridge.AgentChannelEvent as NativeAgentEvent
import dev.akko.miaominal.nativebridge.AgentEnvVar
import dev.akko.miaominal.nativebridge.AgentHostKeyDecision as NativeHostKeyDecision
import dev.akko.miaominal.nativebridge.AgentSshSession
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull
import java.util.concurrent.atomic.AtomicLong

/** Host-key decision for one pending prompt. */
enum class AgentHostKeyDecisionModel { AcceptOnce, AcceptAndSave, Reject }

data class AgentPromptModel(val prompt: String, val echo: Boolean)

/**
 * Channel-layer event as seen by the adapters.
 *
 * Kept separate from the generated UniFFI types so adapter logic (and its unit
 * tests) never depends on the native library.
 */
sealed interface AgentChannelEventModel {
    data object Ready : AgentChannelEventModel

    data class ExecOpened(val channelId: Long) : AgentChannelEventModel

    data class ExecFailed(val channelId: Long, val message: String) : AgentChannelEventModel

    data class Stdout(val channelId: Long, val data: ByteArray) : AgentChannelEventModel

    data class Stderr(val channelId: Long, val data: ByteArray) : AgentChannelEventModel

    /**
     * `exitCode` and `signal` are exclusive; both `null` means the channel
     * ended without a status.
     */
    data class ExecExited(
        val channelId: Long,
        val exitCode: Int?,
        val signal: String?
    ) : AgentChannelEventModel

    data class ChannelClosed(val channelId: Long) : AgentChannelEventModel

    data class TunnelReady(val tunnelId: Long, val localPort: Int) : AgentChannelEventModel

    data class TunnelFailed(val tunnelId: Long, val message: String) : AgentChannelEventModel

    data class TunnelClosed(val tunnelId: Long) : AgentChannelEventModel

    data class HostKeyPrompt(
        val requestId: Long,
        val hopLabel: String,
        val host: String,
        val port: Int,
        val algorithm: String,
        val fingerprint: String,
        val previousFingerprint: String?,
        val keyChanged: Boolean
    ) : AgentChannelEventModel

    data class AuthenticationPrompt(
        val requestId: Long,
        val hopLabel: String,
        val name: String,
        val instructions: String,
        val prompts: List<AgentPromptModel>
    ) : AgentChannelEventModel

    data class RequestExpired(val requestId: Long) : AgentChannelEventModel

    data class Error(val message: String) : AgentChannelEventModel

    data object Closed : AgentChannelEventModel
}

/**
 * Channel primitives used by CLI adapters: structured exec channels and
 * temporary loopback tunnels over one authenticated SSH connection.
 */
interface AgentChannel {
    val connectionId: Long

    /** Bounded, reliable event stream; `Closed` is terminal. */
    fun events(): Flow<AgentChannelEventModel>

    suspend fun openExec(
        command: String,
        environment: List<Pair<String, String>> = emptyList()
    ): Long

    suspend fun writeStdin(channelId: Long, data: ByteArray)

    suspend fun signal(channelId: Long, signal: String)

    suspend fun closeChannel(channelId: Long)

    /** Bind a local loopback listener to the remote loopback port. */
    suspend fun openTunnel(remoteHost: String, remotePort: Int): Long

    suspend fun closeTunnel(tunnelId: Long)

    fun respondHostKey(requestId: Long, decision: AgentHostKeyDecisionModel)

    fun respondKeyboardInteractive(requestId: Long, answers: List<String>)

    suspend fun close()
}

/** Native-backed channel around the generated `AgentSshSession` handle. */
class NativeAgentChannel(
    private val session: AgentSshSession,
    scope: CoroutineScope,
    /**
     * Releases the native session registry entry (`Backend.closeAgentSession`).
     * Without it the Rust registry keeps a strong reference and every prepare
     * would leak the previous session and its event queue.
     */
    private val closeNativeSession: suspend (Long) -> Unit = {}
) : AgentChannel {

    override val connectionId: Long = session.connectionId().toLong()

    private val queue = Channel<AgentChannelEventModel>(capacity = EVENT_QUEUE_CAPACITY)
    private val nextChannelId = AtomicLong(1)
    private val nextTunnelId = AtomicLong(1)
    private val poller: Job = scope.launch(Dispatchers.IO) {
        while (isActive) {
            val batch = try {
                session.readEvents(BATCH_SIZE, POLL_TIMEOUT_MS)
            } catch (error: Throwable) {
                queue.trySend(AgentChannelEventModel.Error(describe(error)))
                queue.trySend(AgentChannelEventModel.Closed)
                break
            }
            for (event in batch.events) {
                queue.send(event.toModel())
            }
            if (batch.closed) {
                queue.send(AgentChannelEventModel.Closed)
                break
            }
        }
    }

    override fun events(): Flow<AgentChannelEventModel> = queue.receiveAsFlow()

    override suspend fun openExec(
        command: String,
        environment: List<Pair<String, String>>
    ): Long {
        val id = nextChannelId.getAndIncrement()
        session.openExec(
            id.toULong(),
            command,
            environment.map { (name, value) -> AgentEnvVar(name, value) }
        )
        return id
    }

    override suspend fun writeStdin(channelId: Long, data: ByteArray) {
        session.writeStdin(channelId.toULong(), data)
    }

    override suspend fun signal(channelId: Long, signal: String) {
        session.signal(channelId.toULong(), signal)
    }

    override suspend fun closeChannel(channelId: Long) {
        session.closeChannel(channelId.toULong())
    }

    override suspend fun openTunnel(remoteHost: String, remotePort: Int): Long {
        val id = nextTunnelId.getAndIncrement()
        session.openLoopbackTunnel(id.toULong(), remoteHost, remotePort.toUShort())
        return id
    }

    override suspend fun closeTunnel(tunnelId: Long) {
        session.closeTunnel(tunnelId.toULong())
    }

    override fun respondHostKey(requestId: Long, decision: AgentHostKeyDecisionModel) {
        session.respondHostKey(requestId.toULong(), decision.toNative())
    }

    override fun respondKeyboardInteractive(requestId: Long, answers: List<String>) {
        session.respondKeyboardInteractive(requestId.toULong(), answers)
    }

    override suspend fun close() {
        runCatching { session.cancel() }
        // Keep draining while the worker shuts down. A stopped poller with a
        // full queue would block the final `Closed` send in Rust, and the
        // worker thread (plus closeAgentSession) would wait forever.
        withTimeoutOrNull(CLOSE_DRAIN_TIMEOUT_MS) {
            while (poller.isActive) {
                if (queue.tryReceive().isFailure) delay(CLOSE_DRAIN_POLL_MS)
            }
        }
        poller.cancel()
        queue.close()
        // Reclaim the native session; bounded so a stuck worker cannot hang the
        // caller (the drain above is the normal path).
        withTimeoutOrNull(CLOSE_NATIVE_TIMEOUT_MS) {
            runCatching { closeNativeSession(connectionId) }
        }
    }

    private companion object {
        const val EVENT_QUEUE_CAPACITY = 256
        const val BATCH_SIZE = 32u
        const val POLL_TIMEOUT_MS = 250uL
        const val CLOSE_DRAIN_TIMEOUT_MS = 3_000L
        const val CLOSE_DRAIN_POLL_MS = 10L
        const val CLOSE_NATIVE_TIMEOUT_MS = 5_000L

        fun describe(error: Throwable): String =
            error.message?.takeIf(String::isNotBlank) ?: error.javaClass.simpleName
    }
}

private fun NativeAgentEvent.toModel(): AgentChannelEventModel = when (this) {
    is NativeAgentEvent.Ready -> AgentChannelEventModel.Ready
    is NativeAgentEvent.ExecOpened ->
        AgentChannelEventModel.ExecOpened(channelId.toLong())

    is NativeAgentEvent.ExecFailed ->
        AgentChannelEventModel.ExecFailed(channelId.toLong(), message)

    is NativeAgentEvent.Stdout ->
        AgentChannelEventModel.Stdout(channelId.toLong(), data)

    is NativeAgentEvent.Stderr ->
        AgentChannelEventModel.Stderr(channelId.toLong(), data)

    is NativeAgentEvent.ExecExited -> AgentChannelEventModel.ExecExited(
        channelId = channelId.toLong(),
        exitCode = exitCode?.toInt(),
        signal = signal
    )

    is NativeAgentEvent.ChannelClosed ->
        AgentChannelEventModel.ChannelClosed(channelId.toLong())

    is NativeAgentEvent.TunnelReady -> AgentChannelEventModel.TunnelReady(
        tunnelId = tunnelId.toLong(),
        localPort = localPort.toInt()
    )

    is NativeAgentEvent.TunnelFailed -> AgentChannelEventModel.TunnelFailed(
        tunnelId = tunnelId.toLong(),
        message = message
    )

    is NativeAgentEvent.TunnelClosed ->
        AgentChannelEventModel.TunnelClosed(tunnelId.toLong())

    is NativeAgentEvent.HostKeyPrompt -> AgentChannelEventModel.HostKeyPrompt(
        requestId = prompt.requestId.toLong(),
        hopLabel = prompt.hopLabel,
        host = prompt.host,
        port = prompt.port.toInt(),
        algorithm = prompt.algorithm,
        fingerprint = prompt.fingerprint,
        previousFingerprint = prompt.previousFingerprint,
        keyChanged = prompt.keyChanged
    )

    is NativeAgentEvent.AuthenticationPrompt -> AgentChannelEventModel.AuthenticationPrompt(
        requestId = prompt.requestId.toLong(),
        hopLabel = prompt.hopLabel,
        name = prompt.name,
        instructions = prompt.instructions,
        prompts = prompt.prompts.map { AgentPromptModel(it.prompt, it.echo) }
    )

    is NativeAgentEvent.RequestExpired ->
        AgentChannelEventModel.RequestExpired(requestId.toLong())

    is NativeAgentEvent.Error -> AgentChannelEventModel.Error(message)

    is NativeAgentEvent.Closed -> AgentChannelEventModel.Closed
}

private fun AgentHostKeyDecisionModel.toNative(): NativeHostKeyDecision = when (this) {
    AgentHostKeyDecisionModel.AcceptOnce -> NativeHostKeyDecision.ACCEPT_ONCE
    AgentHostKeyDecisionModel.AcceptAndSave -> NativeHostKeyDecision.ACCEPT_AND_SAVE
    AgentHostKeyDecisionModel.Reject -> NativeHostKeyDecision.REJECT
}

/** Builds the fixed AI CLI startup line for the shell configured on the host. */
object AgentCommands {
    private fun posixEscapePath(path: String): String {
        if (path.isEmpty()) return "''"
        val safe = path.all { character ->
            character.isLetterOrDigit() && character.code < 128 ||
                character in "-_./@%+,:="
        }
        if (safe) return path
        return "'" + path.replace("'", "'\\''") + "'"
    }

    private fun powerShellEscapePath(path: String): String {
        require(path.none { it == '\u0000' || it == '\r' || it == '\n' }) {
            "the Windows working directory contains an invalid control character"
        }
        return "'${path.replace("'", "''")}'"
    }

    private fun cmdEscapePath(path: String): String {
        require(path.none { it == '\u0000' || it == '\r' || it == '\n' || it == '"' }) {
            "the CMD working directory contains an invalid character"
        }
        // CMD expands paired %NAME% tokens even inside quotes. Reject that
        // ambiguous form instead of silently changing directory to a different
        // path; an unmatched percent is a normal literal Windows filename.
        require(!CMD_ENV_REFERENCE.containsMatchIn(path) && '!' !in path) {
            "the CMD working directory contains an environment expansion token"
        }
        return path
    }

    fun cdAndRun(cwd: String, command: String, shellType: ShellType): String = when (shellType) {
        ShellType.Posix -> "cd ${posixEscapePath(cwd)} && $command"
        ShellType.Fish -> "cd ${posixEscapePath(cwd)}; and $command"
        ShellType.PowerShell ->
            "Set-Location -LiteralPath ${powerShellEscapePath(cwd)}; if (\$?) { $command }"
        ShellType.Cmd -> "cd /d \"${cmdEscapePath(cwd)}\" && $command"
    }

    /**
     * Start the CLI without requiring a workspace. History listing and every
     * protocol call that needs a directory carry it explicitly, so the process
     * can stay in the SSH login directory until a conversation pins one.
     */
    fun launch(command: String, cwd: String?, shellType: ShellType): String {
        val directory = cwd?.trim()?.takeIf(String::isNotBlank) ?: return command
        return cdAndRun(directory, command, shellType)
    }

    fun toSftpPath(path: String, shellType: ShellType): String {
        if (shellType != ShellType.PowerShell && shellType != ShellType.Cmd) return path
        val slashes = path.replace('\\', '/')
        if (slashes.matches(WINDOWS_DRIVE_PATH)) return "/$slashes"
        return slashes
    }

    fun fromSftpPath(path: String, shellType: ShellType): String {
        if (shellType != ShellType.PowerShell && shellType != ShellType.Cmd) return path
        val slashes = path.replace('\\', '/')
        val driveMatch = SFTP_DRIVE_PATH.matchEntire(slashes)
        if (driveMatch != null) {
            val drive = driveMatch.groupValues[1]
            val tail = driveMatch.groupValues[2]
            return if (tail.isEmpty()) "$drive:\\" else "$drive:\\${tail.replace('/', '\\')}"
        }
        return slashes.replace('/', '\\')
    }

    private val WINDOWS_DRIVE_PATH = Regex("[A-Za-z]:/.*")
    private val SFTP_DRIVE_PATH = Regex("^/?([A-Za-z]):/?(.*)$")
    private val CMD_ENV_REFERENCE = Regex("%[^%]+%")
}

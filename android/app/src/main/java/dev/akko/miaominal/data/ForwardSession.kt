package dev.akko.miaominal.data

import dev.akko.miaominal.nativebridge.ForwardKindDto
import dev.akko.miaominal.nativebridge.ForwardPhase as NativeForwardPhase
import dev.akko.miaominal.nativebridge.ForwardRequest as NativeForwardRequest
import dev.akko.miaominal.nativebridge.ForwardRequestKind
import dev.akko.miaominal.nativebridge.ForwardFailureKind as NativeForwardFailureKind
import dev.akko.miaominal.nativebridge.ForwardSession
import dev.akko.miaominal.nativebridge.ForwardSnapshot

/** Runtime phase of one dedicated forwarding session. */
enum class ForwardPhaseModel {
    Starting,
    WaitingForTrust,
    WaitingForAuthentication,
    /** A local listener is bound. */
    Listening,
    /** The server accepted the remote listening port. */
    Connected,
    Reconnecting,
    Stopping,
    Stopped,
    Failed;

    val isTerminal: Boolean get() = this == Stopped || this == Failed
    val isActive: Boolean get() = !isTerminal

    companion object {
        fun from(phase: NativeForwardPhase): ForwardPhaseModel = when (phase) {
            NativeForwardPhase.STARTING -> Starting
            NativeForwardPhase.WAITING_FOR_TRUST -> WaitingForTrust
            NativeForwardPhase.WAITING_FOR_AUTHENTICATION -> WaitingForAuthentication
            NativeForwardPhase.LISTENING -> Listening
            NativeForwardPhase.CONNECTED -> Connected
            NativeForwardPhase.RECONNECTING -> Reconnecting
            NativeForwardPhase.STOPPING -> Stopping
            NativeForwardPhase.STOPPED -> Stopped
            NativeForwardPhase.FAILED -> Failed
        }
    }
}

/** Stable forwarding failure categories (never parsed from English text). */
enum class ForwardFailureKindModel {
    PortOccupied,
    RemoteRefused,
    TargetUnreachable,
    Authentication,
    Transport,
    Other
}

data class ForwardFailureModel(
    val kind: ForwardFailureKindModel,
    /** Entry proxy name, jump `i/n`, or the target host. */
    val nodeLabel: String,
    val hopIndex: Int,
    val hopTotal: Int,
    val message: String
)

data class ForwardRequestModel(
    val requestId: Long,
    val connectionId: Long,
    val profileId: String,
    val ruleId: String,
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
) {
    /** Reuse the terminal trust/authentication dialog model. */
    fun toSessionRequest(): SessionRequestModel = SessionRequestModel(
        requestId = requestId,
        connectionId = connectionId,
        kind = kind,
        hopLabel = hopLabel,
        hopIndex = hopIndex,
        hopTotal = hopTotal,
        host = host,
        port = port,
        algorithm = algorithm,
        fingerprint = fingerprint,
        previousFingerprint = previousFingerprint,
        keyChanged = keyChanged,
        name = name,
        instructions = instructions,
        prompts = prompts
    )
}

/** Pull-based runtime state of one forwarding rule. */
data class ForwardSessionModel(
    val connectionId: Long,
    val profileId: String,
    val ruleId: String,
    val label: String,
    val kind: ForwardKind,
    val listenHost: String,
    val requestedPort: Int,
    val boundPort: Int?,
    val phase: ForwardPhaseModel,
    val failure: ForwardFailureModel?,
    val relayFailure: ForwardFailureModel?,
    val request: ForwardRequestModel?,
    val reconnectError: String?,
    val reconnectAttempt: Int,
    val reconnectRetryAfterSecs: Long,
    val revision: Long,
    val startedAtMs: Long
) {
    /** Stable key for UI lookup: one rule of one host. */
    val key: String get() = "$profileId:$ruleId"

    val displayPort: Int get() = boundPort ?: requestedPort
}

/**
 * One live forwarding session as seen by the UI. Implementations may be the
 * native backend or a test double; screens only depend on this contract.
 */
interface ForwardSessionHandle {
    val connectionId: Long
    val profileId: String
    val ruleId: String

    fun snapshot(): ForwardSessionModel

    fun answerHostKey(requestId: Long, decision: HostKeyDecision): Result<Unit>
    fun answerKeyboardInteractive(requestId: Long, answers: List<String>): Result<Unit>

    /** Explicit cancellation; idempotent. */
    fun cancel()
    suspend fun disconnect()
}

/** Native-backed handle around the generated UniFFI forwarding object. */
class NativeForwardSessionHandle(private val session: ForwardSession) : ForwardSessionHandle {

    override val connectionId: Long = session.connectionId().toLong()
    override val profileId: String = session.profileId()
    override val ruleId: String = session.ruleId()

    override fun snapshot(): ForwardSessionModel = session.snapshot().toModel()

    override fun answerHostKey(requestId: Long, decision: HostKeyDecision): Result<Unit> =
        runCatching { session.respondHostKey(requestId.toULong(), decision.toNative()) }

    override fun answerKeyboardInteractive(requestId: Long, answers: List<String>): Result<Unit> =
        runCatching { session.respondKeyboardInteractive(requestId.toULong(), answers) }

    override fun cancel() {
        runCatching { session.cancel() }
    }

    override suspend fun disconnect() {
        runCatching { session.disconnect() }
    }
}

fun ForwardSnapshot.toModel(): ForwardSessionModel = ForwardSessionModel(
    connectionId = connectionId.toLong(),
    profileId = profileId,
    ruleId = ruleId,
    label = label,
    kind = when (kind) {
        ForwardKindDto.LOCAL -> ForwardKind.Local
        ForwardKindDto.REMOTE -> ForwardKind.Remote
    },
    listenHost = listenHost,
    requestedPort = requestedPort.toInt(),
    boundPort = boundPort?.toInt(),
    phase = ForwardPhaseModel.from(phase),
    failure = failure?.toModel(),
    relayFailure = relayFailure?.toModel(),
    request = request?.toModel(),
    reconnectError = reconnectError,
    reconnectAttempt = reconnectAttempt.toInt(),
    reconnectRetryAfterSecs = reconnectRetryAfterSecs.toLong(),
    revision = revision.toLong(),
    startedAtMs = startedAtMs.toLong()
)

fun dev.akko.miaominal.nativebridge.ForwardFailure.toModel(): ForwardFailureModel =
    ForwardFailureModel(
        kind = when (kind) {
            NativeForwardFailureKind.PORT_OCCUPIED -> ForwardFailureKindModel.PortOccupied
            NativeForwardFailureKind.REMOTE_REFUSED -> ForwardFailureKindModel.RemoteRefused
            NativeForwardFailureKind.TARGET_UNREACHABLE -> ForwardFailureKindModel.TargetUnreachable
            NativeForwardFailureKind.AUTHENTICATION -> ForwardFailureKindModel.Authentication
            NativeForwardFailureKind.TRANSPORT -> ForwardFailureKindModel.Transport
            NativeForwardFailureKind.OTHER -> ForwardFailureKindModel.Other
        },
        nodeLabel = nodeLabel,
        hopIndex = hopIndex.toInt(),
        hopTotal = hopTotal.toInt(),
        message = message
    )

fun NativeForwardRequest.toModel(): ForwardRequestModel = ForwardRequestModel(
    requestId = requestId.toLong(),
    connectionId = connectionId.toLong(),
    profileId = profileId,
    ruleId = ruleId,
    kind = when (kind) {
        ForwardRequestKind.HOST_KEY -> SessionRequestKind.HostKey
        ForwardRequestKind.KEYBOARD_INTERACTIVE -> SessionRequestKind.KeyboardInteractive
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

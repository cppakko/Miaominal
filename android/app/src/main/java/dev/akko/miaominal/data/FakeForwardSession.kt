package dev.akko.miaominal.data

/**
 * Scriptable forwarding session for UI tests and Preview. It mirrors the
 * native handle contract so screens can start, stop, and answer prompts without
 * executing Rust.
 */
class FakeForwardSession(
    override val connectionId: Long,
    override val profileId: String,
    override val ruleId: String,
    label: String = "",
    kind: ForwardKind = ForwardKind.Local,
    listenHost: String = "127.0.0.1",
    requestedPort: Int = 0,
    phase: ForwardPhaseModel = ForwardPhaseModel.Starting
) : ForwardSessionHandle {

    var label: String = label
    var kind: ForwardKind = kind
    var listenHost: String = listenHost
    var requestedPort: Int = requestedPort
    var phase: ForwardPhaseModel = phase
    var boundPort: Int? = if (phase == ForwardPhaseModel.Listening || phase == ForwardPhaseModel.Connected) {
        requestedPort
    } else {
        null
    }
    var failure: ForwardFailureModel? = null
    var relayFailure: ForwardFailureModel? = null
    var request: ForwardRequestModel? = null
    var reconnectError: String? = null
    var reconnectAttempt: Int = 0
    var revision: Long = 1

    /** Answer policy: when false, answers are rejected as expired. */
    var acceptsHostKey: Boolean = true

    val hostKeyAnswers = mutableListOf<Pair<Long, HostKeyDecision>>()
    val keyboardAnswers = mutableListOf<Pair<Long, List<String>>>()

    override fun snapshot(): ForwardSessionModel = ForwardSessionModel(
        connectionId = connectionId,
        profileId = profileId,
        ruleId = ruleId,
        label = label,
        kind = kind,
        listenHost = listenHost,
        requestedPort = requestedPort,
        boundPort = boundPort,
        phase = phase,
        failure = failure,
        relayFailure = relayFailure,
        request = request,
        reconnectError = reconnectError,
        reconnectAttempt = reconnectAttempt,
        reconnectRetryAfterSecs = 0,
        revision = revision,
        startedAtMs = 0
    )

    override fun answerHostKey(requestId: Long, decision: HostKeyDecision): Result<Unit> =
        runCatching {
            val pending = request ?: error("no pending request")
            if (!acceptsHostKey || pending.requestId != requestId) {
                error("request $requestId is no longer pending")
            }
            hostKeyAnswers += requestId to decision
            request = null
            revision += 1
        }

    override fun answerKeyboardInteractive(requestId: Long, answers: List<String>): Result<Unit> =
        runCatching {
            val pending = request ?: error("no pending request")
            if (pending.requestId != requestId) {
                error("request $requestId is no longer pending")
            }
            keyboardAnswers += requestId to answers
            request = null
            revision += 1
        }

    /** Test helper: publish a pending host-trust request. */
    fun awaitTrust(requestId: Long = 1L, changed: Boolean = false) {
        request = ForwardRequestModel(
            requestId = requestId,
            connectionId = connectionId,
            profileId = profileId,
            ruleId = ruleId,
            kind = SessionRequestKind.HostKey,
            hopLabel = label,
            hopIndex = 1,
            hopTotal = 1,
            host = "example.test",
            port = 22,
            algorithm = "ssh-ed25519",
            fingerprint = "SHA256:forward",
            previousFingerprint = if (changed) "SHA256:old" else null,
            keyChanged = changed,
            name = "",
            instructions = "",
            prompts = emptyList()
        )
        phase = ForwardPhaseModel.WaitingForTrust
        revision += 1
    }

    override fun cancel() {
        phase = ForwardPhaseModel.Stopped
        boundPort = null
        request = null
        revision += 1
    }

    override suspend fun disconnect() {
        cancel()
    }
}

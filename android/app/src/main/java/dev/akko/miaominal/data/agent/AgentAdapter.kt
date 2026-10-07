package dev.akko.miaominal.data.agent

import kotlinx.coroutines.flow.Flow

/** Result of a CLI availability probe. */
data class AgentProbeResult(
    val available: Boolean,
    val version: String? = null,
    val detail: String? = null
)

/** One question answer mapped back to the original protocol request. */
data class InteractionAnswerEntry(
    val questionId: String,
    val answers: List<String>
)

/** How the user answered one interaction request. */
sealed interface AgentInteractionAnswer {
    /** One of the options advertised by the CLI. */
    data class Option(val optionId: String) : AgentInteractionAnswer

    /** Free-form answers for a follow-up question list. */
    data class Answers(val entries: List<InteractionAnswerEntry>) : AgentInteractionAnswer
}

/**
 * Common contract implemented by every CLI adapter.
 *
 * The adapter owns the CLI process (exec channel or tunnel + HTTP) and turns
 * its protocol into [AgentEvent]s. The repository folds those events through
 * [AgentReducer]; the UI never sees protocol JSON.
 */
interface AgentAdapter {
    val cli: AgentCli
    val capabilities: AgentCapabilities

    /** Events emitted while the adapter runs; `AgentEvent.Closed` is terminal. */
    val events: Flow<AgentEvent>

    /**
     * Start the CLI in `cwd` and complete the protocol handshake. A blank
     * `cwd` starts the CLI in the SSH login directory; every protocol call
     * that needs a directory carries it explicitly afterwards.
     *
     * A returned [AgentProbeResult] with `available = false` means the CLI is
     * missing or not logged in; the adapter must not emit `SessionOpened`.
     */
    suspend fun start(cwd: String): AgentProbeResult

    /**
     * List existing conversations. A blank [cwd] lists every conversation the
     * CLI knows about, across workspaces; each entry carries its own [AgentHistoryEntry.cwd].
     */
    suspend fun listSessions(cwd: String? = null): List<AgentHistoryEntry>

    /** Create a new conversation and return its remote id. */
    suspend fun createSession(cwd: String): String

    /**
     * Load an existing conversation, replay its history, and subscribe.
     * A non-blank [cwd] resumes in that workspace instead of the process directory.
     */
    suspend fun loadSession(remoteId: String, cwd: String? = null)

    suspend fun sendMessage(text: String)

    suspend fun cancelTurn()

    /** Optional model selection; adapters without model support ignore it. */
    suspend fun selectModel(model: AgentModel) {}
    suspend fun selectReasoning(id: String) {}

    /** Map a UI answer back to the original protocol request. */
    suspend fun respondInteraction(requestId: String, answer: AgentInteractionAnswer)

    suspend fun close()
}

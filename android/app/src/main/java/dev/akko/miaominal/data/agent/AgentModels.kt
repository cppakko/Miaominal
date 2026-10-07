package dev.akko.miaominal.data.agent

/**
 * Common agent model shared by every CLI adapter.
 *
 * The UI only ever reads these types; CLI-specific JSON stays inside the
 * adapters. Stable ids (session, message, block, tool, interaction) are kept
 * exactly as the CLI reports them so complete updates can replace state and
 * incremental updates can append without duplicating history.
 */

enum class AgentCli(val id: String, val displayName: String) {
    Codex("codex", "Codex"),
    OpenCode("opencode", "OpenCode");

    companion object {
        fun fromId(id: String): AgentCli? = entries.firstOrNull { it.id == id }
    }
}

enum class AgentPhase { Idle, Connecting, Ready, Failed, Closed }

enum class AgentTurnStatus { Running, WaitingApproval, WaitingInput, Completed, Cancelled, Failed }

val AgentTurnStatus.isTerminal: Boolean
    get() = this == AgentTurnStatus.Completed ||
        this == AgentTurnStatus.Cancelled ||
        this == AgentTurnStatus.Failed

enum class ToolStatus { Pending, Running, Completed, Failed, Declined }

enum class AgentRole { User, Assistant, System }

enum class DeltaKind { Text, Reasoning }

enum class InteractionKind { Approval, Question }

enum class InteractionState { Pending, Answered, Expired }

/** Feature declaration so the UI never assumes a CLI capability. */
data class AgentCapabilities(
    val history: Boolean = true,
    val modelSelection: Boolean = false,
    val approvals: Boolean = true,
    val followUpQuestions: Boolean = false,
    val fileChanges: Boolean = true
)

data class AgentReasoningOption(val id: String, val label: String)

data class AgentModel(
    val providerId: String, val modelId: String, val name: String,
    val reasoningOptions: List<AgentReasoningOption> = emptyList(),
    val defaultReasoning: String? = null
) {
    val key: String get() = "$providerId/$modelId"
}

data class AgentSessionInfo(
    val cli: AgentCli,
    val hostId: String,
    val cwd: String,
    val remoteSessionId: String? = null,
    val title: String? = null
) {
    /** The name shown in the chat top bar. */
    val label: String get() = title?.takeIf { it.isNotBlank() } ?: cwd
}

data class AgentUsage(
    val inputTokens: Long = 0,
    val outputTokens: Long = 0,
    val costUsd: Double = 0.0
)

data class FileChange(val path: String, val kind: String, val diff: String?)

data class PlanStep(val step: String, val status: String)

data class InteractionOption(
    val id: String,
    val label: String,
    val description: String? = null
)

data class InteractionQuestion(
    val id: String,
    val question: String,
    val options: List<InteractionOption> = emptyList(),
    val multiSelect: Boolean = false,
    val freeform: Boolean = false
)

data class InteractionRequest(
    val id: String,
    val kind: InteractionKind,
    val turnId: String?,
    val toolId: String?,
    val title: String,
    val detail: String? = null,
    val options: List<InteractionOption> = emptyList(),
    val questions: List<InteractionQuestion> = emptyList(),
    val state: InteractionState = InteractionState.Pending
)

/** One ordered block inside an assistant or user message. */
sealed interface ContentBlock {
    val id: String
    val streaming: Boolean

    data class Text(
        override val id: String,
        val text: String,
        override val streaming: Boolean = false
    ) : ContentBlock

    data class Reasoning(
        override val id: String,
        val text: String,
        override val streaming: Boolean = false
    ) : ContentBlock

    data class ToolCall(
        override val id: String,
        val callId: String?,
        val name: String,
        val title: String?,
        val status: ToolStatus,
        val input: String? = null,
        val output: String? = null,
        val error: String? = null,
        override val streaming: Boolean = false
    ) : ContentBlock

    data class FileChanges(
        override val id: String,
        val changes: List<FileChange>,
        val status: ToolStatus
    ) : ContentBlock {
        override val streaming: Boolean get() = status == ToolStatus.Running || status == ToolStatus.Pending
    }

    data class Plan(
        override val id: String,
        val explanation: String?,
        val steps: List<PlanStep>
    ) : ContentBlock {
        override val streaming: Boolean get() = false
    }

    data class Notice(override val id: String, val text: String) : ContentBlock {
        override val streaming: Boolean get() = false
    }
}

data class AgentMessage(
    val id: String,
    val role: AgentRole,
    val blocks: List<ContentBlock> = emptyList(),
    val streaming: Boolean = false
)

data class AgentTurn(
    val id: String,
    val status: AgentTurnStatus,
    val messageIds: List<String> = emptyList(),
    val error: String? = null
)

/** One entry of remote CLI history (existing conversations). */
data class AgentHistoryEntry(
    val remoteId: String,
    val title: String,
    val preview: String? = null,
    val updatedAtMs: Long? = null,
    /**
     * Working directory recorded for this conversation. The CLI reports it
     * even when history is listed across every workspace, and resuming uses
     * this directory instead of the CLI process directory.
     */
    val cwd: String? = null
)

/**
 * Workspaces recorded in [entries], newest first, each workspace once.
 * Entries without a directory (older CLI responses) are skipped.
 */
fun historyWorkspaces(entries: List<AgentHistoryEntry>): List<String> =
    entries.sortedByDescending { it.updatedAtMs ?: 0L }
        .mapNotNull { it.cwd?.takeIf(String::isNotBlank) }
        .distinct()

/** Full state rendered by the chat screen. */
data class AgentSessionState(
    val session: AgentSessionInfo? = null,
    val phase: AgentPhase = AgentPhase.Idle,
    val capabilities: AgentCapabilities = AgentCapabilities(),
    val turns: List<AgentTurn> = emptyList(),
    val messages: List<AgentMessage> = emptyList(),
    val interactions: List<InteractionRequest> = emptyList(),
    val models: List<AgentModel> = emptyList(),
    val selectedModel: AgentModel? = null,
    val reasoningOptions: List<AgentReasoningOption> = emptyList(),
    val selectedReasoning: String? = null,
    val usage: AgentUsage = AgentUsage(),
    val notices: List<ContentBlock.Notice> = emptyList(),
    val error: String? = null,
    val connectionLost: Boolean = false,
    val interactionSubmitting: Boolean = false
) {
    val currentTurn: AgentTurn?
        get() = turns.lastOrNull {
            it.status == AgentTurnStatus.Running ||
                it.status == AgentTurnStatus.WaitingApproval ||
                it.status == AgentTurnStatus.WaitingInput
        }

    val pendingInteraction: InteractionRequest?
        get() = interactions.lastOrNull { it.state == InteractionState.Pending }

    val isBusy: Boolean get() = currentTurn != null

    fun message(id: String): AgentMessage? = messages.firstOrNull { it.id == id }

    fun lastAssistantMessage(): AgentMessage? =
        messages.lastOrNull { it.role == AgentRole.Assistant }
}

/** Events produced by adapters; the reducer folds them into [AgentSessionState]. */
sealed interface AgentEvent {
    data class ReasoningConfigured(
        val options: List<AgentReasoningOption>, val selected: String?
    ) : AgentEvent
    data class PhaseChanged(val phase: AgentPhase, val error: String? = null) : AgentEvent

    data class SessionOpened(
        val session: AgentSessionInfo,
        val capabilities: AgentCapabilities
    ) : AgentEvent

    data class ModelsLoaded(
        val models: List<AgentModel>,
        val selected: AgentModel? = null
    ) : AgentEvent

    data class HistoryLoaded(val entries: List<AgentHistoryEntry>) : AgentEvent

    data class TurnStarted(val turnId: String) : AgentEvent

    data class TurnEnded(
        val turnId: String,
        val status: AgentTurnStatus,
        val error: String? = null
    ) : AgentEvent

    data class MessageUpserted(val message: AgentMessage) : AgentEvent

    data class MessageRemoved(val messageId: String) : AgentEvent

    /** Complete block update; replaces a block with the same id. */
    data class BlockUpserted(val messageId: String, val block: ContentBlock) : AgentEvent

    data class BlockDelta(
        val messageId: String,
        val blockId: String,
        val text: String,
        val kind: DeltaKind = DeltaKind.Text
    ) : AgentEvent

    data class BlockRemoved(val messageId: String, val blockId: String) : AgentEvent

    /** Incremental tool output (stdout/stderr of a running tool). */
    data class ToolOutputDelta(
        val messageId: String,
        val blockId: String,
        val text: String
    ) : AgentEvent

    data class InteractionRequested(val request: InteractionRequest) : AgentEvent

    data class InteractionResolved(val requestId: String) : AgentEvent

    data class UsageUpdated(val usage: AgentUsage) : AgentEvent

    data class NoticeRaised(val notice: ContentBlock.Notice) : AgentEvent

    data class ErrorRaised(val message: String) : AgentEvent

    data class ConnectionLost(val message: String) : AgentEvent

    data object Closed : AgentEvent
}

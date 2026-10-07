package dev.akko.miaominal.data.agent

import dev.akko.miaominal.data.CredentialUpdate
import dev.akko.miaominal.data.KeystoreCredentialStore
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import java.net.URI
import java.util.UUID

enum class AgentApiProtocol { Responses, ChatCompletions }

sealed interface AgentConfigSource {
    data object Remote : AgentConfigSource
    data class Local(val profileId: String) : AgentConfigSource
}

/** Public metadata never contains the credential. */
data class AgentConfigProfile(
    val id: String,
    val name: String,
    val protocol: AgentApiProtocol,
    val baseUrl: String,
    val models: List<String>,
    val defaultModel: String,
    val hasApiKey: Boolean = false
) {
    fun supports(cli: AgentCli) = cli != AgentCli.Codex || protocol == AgentApiProtocol.Responses
}

data class AgentConfigState(
    val loaded: Boolean = false,
    val profiles: List<AgentConfigProfile> = emptyList(),
    val error: String? = null
)

/** One process's immutable configuration; deliberately has no generated toString/copy. */
class AgentConfigSnapshot(val profile: AgentConfigProfile, private val apiKey: String) {
    init { require(apiKey.isNotBlank()) { "An AI startup snapshot requires a credential." } }
    private val escapedKey = JsonCodec().stringify(apiKey).removeSurrounding("\"")
    internal fun keyForLaunch(): String = apiKey
    fun redact(text: String): String = text.replace(apiKey, "[REDACTED]")
        .replace(escapedKey, "[REDACTED]")
    override fun toString() = "AgentConfigSnapshot(${profile.id}, credential=REDACTED)"
}

/** Error payloads may echo an upstream Authorization header or JSON API key. */
internal fun AgentConfigSnapshot.redactEvent(event: AgentEvent): AgentEvent = when (event) {
    is AgentEvent.ErrorRaised -> event.copy(message = redact(event.message))
    is AgentEvent.ConnectionLost -> event.copy(message = redact(event.message))
    is AgentEvent.PhaseChanged -> event.copy(error = event.error?.let(::redact))
    is AgentEvent.TurnEnded -> event.copy(error = event.error?.let(::redact))
    is AgentEvent.NoticeRaised -> event.copy(notice = event.notice.copy(text = redact(event.notice.text)))
    is AgentEvent.MessageUpserted -> event.copy(message = event.message.copy(blocks = event.message.blocks.map(::redactBlock)))
    is AgentEvent.BlockUpserted -> event.copy(block = redactBlock(event.block))
    is AgentEvent.BlockDelta -> event.copy(text = redact(event.text))
    is AgentEvent.ToolOutputDelta -> event.copy(text = redact(event.text))
    is AgentEvent.InteractionRequested -> event.copy(request = event.request.copy(
        title = redact(event.request.title), detail = event.request.detail?.let(::redact),
        options = event.request.options.map(::redactOption),
        questions = event.request.questions.map { it.copy(question = redact(it.question), options = it.options.map(::redactOption)) }
    ))
    else -> event
}

/** A credential can straddle JSON-RPC deltas; sanitize the merged block before publication. */
internal fun AgentConfigSnapshot.redactDeltaState(state: AgentSessionState, event: AgentEvent): AgentSessionState {
    val messageId = when (event) {
        is AgentEvent.BlockDelta -> event.messageId
        is AgentEvent.ToolOutputDelta -> event.messageId
        else -> return state
    }
    return state.copy(messages = state.messages.map { message ->
        if (message.id == messageId) message.copy(blocks = message.blocks.map(::redactBlock)) else message
    })
}

private fun AgentConfigSnapshot.redactOption(option: InteractionOption) = option.copy(
    label = redact(option.label), description = option.description?.let(::redact)
)

private fun AgentConfigSnapshot.redactBlock(block: ContentBlock): ContentBlock = when (block) {
    is ContentBlock.Text -> block.copy(text = redact(block.text))
    is ContentBlock.Reasoning -> block.copy(text = redact(block.text))
    is ContentBlock.Notice -> block.copy(text = redact(block.text))
    is ContentBlock.ToolCall -> block.copy(title = block.title?.let(::redact), input = block.input?.let(::redact),
        output = block.output?.let(::redact), error = block.error?.let(::redact))
    is ContentBlock.Plan -> block.copy(explanation = block.explanation?.let(::redact),
        steps = block.steps.map { it.copy(step = redact(it.step)) })
    is ContentBlock.FileChanges -> block.copy(changes = block.changes.map { it.copy(diff = it.diff?.let(::redact)) })
}

interface AgentConfigPersistence {
    fun read(): String?
    fun write(document: String)
    fun delete()
}

class KeystoreAgentConfigPersistence(private val credentials: KeystoreCredentialStore) : AgentConfigPersistence {
    override fun read() = credentials.get(ACCOUNT)
    override fun write(document: String) = credentials.set(ACCOUNT, document)
    override fun delete() = credentials.delete(ACCOUNT)
    companion object { const val ACCOUNT = "miaominal.android.agent-config.v1" }
}

/** Phone-only encrypted document, independent of Rust settings, exports and sync. */
class AgentConfigRepository(private val persistence: AgentConfigPersistence) {
    private class Entry(val profile: AgentConfigProfile, val key: String?)
    private val mutex = Mutex()
    private val json = JsonCodec()
    private var entries = emptyList<Entry>()
    private var selections = emptyMap<String, String>()
    private val mutableState = MutableStateFlow(AgentConfigState())
    val state: StateFlow<AgentConfigState> = mutableState.asStateFlow()

    suspend fun load(): Result<Unit> = withContext(Dispatchers.IO) {
        mutex.withLock { loadLocked() }
    }

    private fun loadLocked(): Result<Unit> = runCatching {
        if (mutableState.value.loaded && mutableState.value.error == null) return@runCatching
        try {
            val text = persistence.read()
            if (text == null) {
                entries = emptyList()
                selections = emptyMap()
            } else {
                val doc = requireNotNull(json.parse(text))
                require(doc.longAt("version") == 1L)
                val parsed = doc.listAt("profiles").map { raw ->
                    val item = requireNotNull(raw.asMap())
                    val key = item.stringAt("apiKey")?.takeIf(String::isNotEmpty)
                    val profile = validate(AgentConfigProfile(
                        id = requireNotNull(item.stringAt("id")),
                        name = requireNotNull(item.stringAt("name")),
                        protocol = AgentApiProtocol.valueOf(requireNotNull(item.stringAt("protocol"))),
                        baseUrl = requireNotNull(item.stringAt("baseUrl")),
                        models = item.listAt("models").map { requireNotNull(it as? String) },
                        defaultModel = requireNotNull(item.stringAt("defaultModel")),
                        hasApiKey = key != null
                    ))
                    if (key != null) validateKey(key)
                    Entry(profile, key)
                }
                require(parsed.map { it.profile.id }.distinct().size == parsed.size)
                val remembered = doc.mapAt("selections").orEmpty().mapValues { (_, value) ->
                    requireNotNull(value as? String)
                }
                entries = parsed
                selections = remembered
            }
            publish()
        } catch (_: Exception) {
            mutableState.value = AgentConfigState(loaded = true, error = STORAGE_ERROR)
            throw IllegalStateException(STORAGE_ERROR)
        }
    }

    suspend fun save(profile: AgentConfigProfile, key: CredentialUpdate): Result<Unit> = operation {
        val valid = validate(profile)
        val previous = entries.firstOrNull { it.profile.id == valid.id }
        val secret = when (key) {
            CredentialUpdate.Keep -> previous?.key
            CredentialUpdate.Clear -> null
            is CredentialUpdate.Set -> key.value.trim().also(::validateKey)
        }
        val entry = Entry(valid.copy(hasApiKey = secret != null), secret)
        val updated = if (previous == null) entries + entry else entries.map {
            if (it.profile.id == valid.id) entry else it
        }
        commit(updated, selections)
    }

    suspend fun delete(id: String): Result<Unit> = operation {
        // Retain the selected ID so deletion never silently enables remote auth.
        // The next connection reports a missing configuration until the user chooses another source.
        commit(entries.filterNot { it.profile.id == id }, selections)
    }

    suspend fun selection(hostId: String, cli: AgentCli): Result<AgentConfigSource> = operation {
        selections[selectionKey(hostId, cli)]?.let(AgentConfigSource::Local) ?: AgentConfigSource.Remote
    }

    suspend fun remember(hostId: String, cli: AgentCli, source: AgentConfigSource): Result<Unit> = operation {
        val key = selectionKey(hostId, cli)
        val next = when (source) {
            AgentConfigSource.Remote -> selections - key
            is AgentConfigSource.Local -> {
                require(entries.any { it.profile.id == source.profileId && it.profile.supports(cli) }) {
                    "This configuration does not support the selected CLI."
                }
                selections + (key to source.profileId)
            }
        }
        commit(entries, next)
    }

    suspend fun resolve(source: AgentConfigSource, cli: AgentCli): Result<AgentConfigSnapshot?> = operation {
        when (source) {
            AgentConfigSource.Remote -> null
            is AgentConfigSource.Local -> {
                val entry = entries.firstOrNull { it.profile.id == source.profileId }
                    ?: error("The selected AI configuration no longer exists.")
                require(entry.profile.supports(cli)) { "Codex requires a Responses API configuration." }
                val secret = entry.key ?: error("Add an API key to this AI configuration first.")
                AgentConfigSnapshot(entry.profile, secret)
            }
        }
    }

    suspend fun clear(): Result<Unit> = withContext(Dispatchers.IO) {
        mutex.withLock {
            runCatching {
                try { persistence.delete() } catch (_: Exception) { error(STORAGE_ERROR) }
                entries = emptyList()
                selections = emptyMap()
                publish()
            }
        }
    }

    /** Called after the application's reset deletes the complete Keystore store. */
    suspend fun credentialsCleared() = clearWithCredentials { }

    /** Serialize the physical credential reset with configuration writes. */
    suspend fun clearWithCredentials(clearCredentials: suspend () -> Unit) = withContext(Dispatchers.IO) {
        mutex.withLock {
            clearCredentials()
            entries = emptyList()
            selections = emptyMap()
            publish()
        }
    }

    private suspend fun <T> operation(block: () -> T): Result<T> = withContext(Dispatchers.IO) {
        mutex.withLock { runCatching { loadLocked().getOrThrow(); block() } }
    }

    private fun commit(next: List<Entry>, remembered: Map<String, String>) {
        val document = json.stringify(mapOf(
            "version" to 1,
            "profiles" to next.map { entry -> with(entry.profile) {
                mapOf("id" to id, "name" to name, "protocol" to protocol.name,
                    "baseUrl" to baseUrl, "models" to models, "defaultModel" to defaultModel,
                    "apiKey" to entry.key)
            } },
            "selections" to remembered
        ))
        try { persistence.write(document) } catch (_: Exception) { error(STORAGE_ERROR) }
        entries = next
        selections = remembered
        publish()
    }

    private fun publish() { mutableState.value = AgentConfigState(true, entries.map { it.profile }) }
    private fun selectionKey(hostId: String, cli: AgentCli) = json.stringify(listOf(hostId, cli.id))

    companion object {
        private const val STORAGE_ERROR = "AI configuration storage is unavailable or cannot be decrypted."
        fun newId(): String = UUID.randomUUID().toString()
        fun validate(profile: AgentConfigProfile): AgentConfigProfile {
            require(profile.id.isNotBlank() && profile.name.trim().isNotEmpty()) { "Enter a configuration name." }
            val url = profile.baseUrl.trim()
            val uri = try { URI(url) } catch (_: Exception) { error("Enter a valid HTTP or HTTPS Base URL.") }
            require(uri.scheme?.lowercase() in listOf("http", "https") && !uri.host.isNullOrBlank() &&
                uri.userInfo == null && uri.fragment == null) { "Enter a valid HTTP or HTTPS Base URL." }
            val models = profile.models.map(String::trim).distinct()
            require(models.isNotEmpty() && models.all { it.isNotBlank() && it.none(Char::isISOControl) }) {
                "Add at least one non-empty model ID."
            }
            val default = profile.defaultModel.trim()
            require(default in models) { "Choose a default model from the model list." }
            return profile.copy(name = profile.name.trim(), baseUrl = url, models = models, defaultModel = default)
        }
        private fun validateKey(key: String) {
            require(key.isNotBlank() && key.length <= 4096 && key.none(Char::isISOControl)) { "Enter an API key without control characters (maximum 4096 characters)." }
        }
    }
}

package dev.akko.miaominal.data.agent

import android.content.Context
import java.io.File

/**
 * Atomic, versioned cache for an unsent AI draft. Older files may still
 * contain a conversation; [retainDraftOnly] removes it on the next start.
 *
 * Writes go to a temporary file in the same directory and replace the target
 * with `renameTo`, so a crash or a killed process can never leave a truncated
 * cache behind. A corrupt or older file is ignored (and removed lazily).
 */
class AgentSessionStore(private val directory: File) {

    constructor(context: Context) : this(File(context.filesDir, DIRECTORY_NAME))

    private val file: File get() = File(directory, FILE_NAME)

    fun load(): AgentCacheCodec.Snapshot? {
        val target = file
        if (!target.isFile) return null
        val text = runCatching { target.readText() }.getOrNull() ?: return null
        val snapshot = AgentCacheCodec.decode(text)
        if (snapshot == null) {
            runCatching { target.delete() }
        }
        return snapshot
    }

    fun save(snapshot: AgentCacheCodec.Snapshot) {
        runCatching {
            directory.mkdirs()
            val temp = File(directory, "$FILE_NAME.tmp")
            temp.writeText(AgentCacheCodec.encode(snapshot))
            if (file.exists() && !file.delete()) {
                // Fall through: renameTo below may still succeed on some file
                // systems that allow replacing an existing file.
            }
            if (!temp.renameTo(file)) {
                temp.copyTo(file, overwrite = true)
                temp.delete()
            }
        }
    }

    fun saveDraft(draft: String) {
        if (draft.isEmpty()) clear()
        else save(AgentCacheCodec.Snapshot(AgentSessionState(), draft))
    }

    /** Migrate a legacy full-session cache without restoring its conversation. */
    fun retainDraftOnly(): String? {
        val draft = load()?.draft ?: return null
        saveDraft(draft)
        return draft
    }

    fun clear() {
        runCatching { file.delete() }
        runCatching { File(directory, "$FILE_NAME.tmp").delete() }
    }

    private companion object {
        const val DIRECTORY_NAME = "agent"
        const val FILE_NAME = "session-cache.json"
    }
}

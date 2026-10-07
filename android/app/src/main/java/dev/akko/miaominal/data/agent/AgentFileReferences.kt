package dev.akko.miaominal.data.agent

/** Paths belong to the remote host, not Android's local filesystem. */
object AgentFileReferences {
    fun prompt(text: String, paths: List<String>): String {
        if (paths.isEmpty()) return text
        // JSON preserves spaces, quotes, backslashes and newlines in file names.
        val references = JsonCodec().stringify(paths.distinct())
        return "$text\n\nReferenced files on the remote host (JSON paths):\n$references"
    }
}

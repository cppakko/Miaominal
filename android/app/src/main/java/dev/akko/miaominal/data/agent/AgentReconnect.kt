package dev.akko.miaominal.data.agent

/** Identity is captured before preparation can replace the repository's visible session. */
internal suspend fun prepareAndResumeConversation(
    session: AgentSessionInfo,
    prepare: suspend (String, AgentCli, String) -> AgentLaunchResult,
    resume: suspend (String) -> Unit
) {
    val remoteId = requireNotNull(session.remoteSessionId?.takeIf { it.isNotBlank() }) {
        "No remote conversation id"
    }
    when (val result = prepare(session.hostId, session.cli, session.cwd)) {
        is AgentLaunchResult.Failed -> error(result.detail)
        is AgentLaunchResult.Unavailable -> error(result.detail)
        is AgentLaunchResult.Ready -> resume(remoteId)
    }
}

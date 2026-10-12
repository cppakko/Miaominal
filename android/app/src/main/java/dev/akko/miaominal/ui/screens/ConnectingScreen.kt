package dev.akko.miaominal.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.FolderOpen
import androidx.compose.material.icons.outlined.Terminal
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.tooling.preview.Preview
import dev.akko.miaominal.R
import dev.akko.miaominal.data.SessionPhase
import dev.akko.miaominal.data.SessionRequestKind
import dev.akko.miaominal.ui.components.ConnectionErrorActions
import dev.akko.miaominal.ui.components.HintIcon as Icon
import dev.akko.miaominal.ui.components.SessionIdentity
import dev.akko.miaominal.ui.components.SessionTopBarCapsule
import dev.akko.miaominal.ui.components.SessionStatusMessage
import dev.akko.miaominal.ui.components.WorkspaceBackHandler as BackHandler
import dev.akko.miaominal.ui.model.HostProfileUi
import dev.akko.miaominal.ui.theme.UiSpacing

/** Presentation only: the status follows the connection owner's phase, never a timer. */
@Composable
fun ConnectingScreen(
    profile: HostProfileUi,
    sftp: Boolean,
    stage: Int,
    phase: SessionPhase? = null,
    failure: String? = null,
    onCancel: () -> Unit,
    modifier: Modifier = Modifier,
    active: Boolean = true,
    onRetry: () -> Unit = {},
    onEditProfile: () -> Unit = {},
    requestKind: SessionRequestKind? = null
) {
    val colors = MaterialTheme.colorScheme
    val failed = failure != null || phase == SessionPhase.Failed
    val closed = phase == SessionPhase.Closing || phase == SessionPhase.Closed
    val finished = failed || closed
    val ready = !finished && (phase == SessionPhase.Ready || (phase == null && stage >= 3))
    // The authentication phase also covers saved credentials; only a pending
    // request means the connection needs user input.
    val waitingForTrust = phase == SessionPhase.WaitingForTrust && requestKind == SessionRequestKind.HostKey
    val waitingForAuth = phase == SessionPhase.WaitingForAuthentication && requestKind == SessionRequestKind.KeyboardInteractive
    val busy = !finished && !ready && !waitingForTrust && !waitingForAuth
    val titleRes = when {
        failed -> R.string.connection_failed
        closed -> R.string.connection_closed
        ready -> R.string.connection_ready
        waitingForTrust -> R.string.connection_waiting_trust
        waitingForAuth -> R.string.connection_waiting_auth
        phase == SessionPhase.WaitingForAuthentication || (phase == null && stage == 1) -> R.string.connection_authenticating
        phase == SessionPhase.OpeningShell || (phase == null && stage == 2) ->
            if (sftp) R.string.connection_opening_files else R.string.connection_opening_terminal
        else -> R.string.connection_contacting
    }
    val cancelLabel = stringResource(if (finished) R.string.connection_return else R.string.connection_cancel)
    BackHandler(enabled = active, onBack = onCancel)

    Box(modifier.fillMaxSize().background(colors.surface)) {
        Box(Modifier.matchParentSize().pointerInput(Unit) {
            // Block the outgoing workspace without consuming content gestures.
            awaitPointerEventScope {
                while (true) awaitPointerEvent().changes.forEach { it.consume() }
            }
        })
        Column(Modifier.fillMaxSize().statusBarsPadding().navigationBarsPadding().displayCutoutPadding()) {
            SessionTopBarCapsule {
                Box(Modifier.size(48.dp), contentAlignment = Alignment.Center) {
                    Icon(
                        if (sftp) Icons.Outlined.FolderOpen else Icons.Outlined.Terminal,
                        contentDescription = null, tint = colors.primary
                    )
                }
                SessionIdentity(profile, connected = ready, modifier = Modifier.weight(1f))
            }
            BoxWithConstraints(Modifier.weight(1f).fillMaxWidth()) {
                Column(
                    Modifier.fillMaxSize().verticalScroll(rememberScrollState())
                        .heightIn(min = maxHeight).padding(UiSpacing.md),
                    horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.Center
                ) {
                    Column(
                        Modifier.widthIn(max = 400.dp).fillMaxWidth(),
                        horizontalAlignment = Alignment.CenterHorizontally
                    ) {
                        SessionStatusMessage(status = stringResource(titleRes), busy = busy, failed = failed)
                        if (failed) {
                            Spacer(Modifier.height(UiSpacing.sm))
                            SelectionContainer {
                                Text(
                                    failure?.takeIf { it.isNotBlank() } ?: stringResource(R.string.connection_failure_hint),
                                    style = MaterialTheme.typography.bodySmall,
                                    color = colors.onSurfaceVariant,
                                    textAlign = TextAlign.Center
                                )
                            }
                            Spacer(Modifier.height(UiSpacing.sm))
                            ConnectionErrorActions(
                                failure.orEmpty(),
                                onReconnect = { if (active) onRetry() },
                                onEdit = { if (active) onEditProfile() }
                            )
                        }
                        Spacer(Modifier.height(UiSpacing.md))
                        TextButton(
                            onClick = onCancel, enabled = active,
                            modifier = Modifier.heightIn(min = 48.dp)
                        ) { Text(cancelLabel) }
                    }
                }
            }
        }
    }
}

@Preview(name = "SSH · connecting", locale = "zh", widthDp = 393, heightDp = 852)
@Preview(name = "SSH · large text", locale = "en", widthDp = 320, heightDp = 740, fontScale = 1.6f)
@Composable
private fun ConnectingScreenPreview() = ConnectionPreview(SessionPhase.Connecting)

@Preview(name = "SSH · verify key · dark", locale = "zh", widthDp = 393, heightDp = 852)
@Composable
private fun ConnectionTrustPreview() = ConnectionPreview(SessionPhase.WaitingForTrust, dark = true)

@Preview(name = "SSH · failure", locale = "zh", widthDp = 393, heightDp = 852)
@Composable
private fun ConnectionFailurePreview() = ConnectionPreview(SessionPhase.Failed, failure = "Connection timed out: 192.168.1.24:22")

@Preview(name = "SFTP · ready", locale = "en", widthDp = 600, heightDp = 900)
@Composable
private fun ConnectionReadyPreview() = ConnectionPreview(SessionPhase.Ready, sftp = true)

@Composable
private fun ConnectionPreview(phase: SessionPhase, dark: Boolean = false, sftp: Boolean = false, failure: String? = null) {
    MaterialTheme(
        colorScheme = dev.akko.miaominal.ui.theme.seededColorScheme(
            dev.akko.miaominal.ui.theme.DEFAULT_THEME_SEED, dark
        ),
        typography = dev.akko.miaominal.ui.theme.Typography,
        shapes = dev.akko.miaominal.ui.theme.Shapes
    ) {
        ConnectingScreen(
            profile = HostProfileUi(
                name = "Development", address = "192.168.1.24", username = "akko",
                group = "", latency = "", lastConnected = "", online = false
            ),
            sftp = sftp, stage = 0, phase = phase, failure = failure,
            active = false, onCancel = {},
            requestKind = if (phase == SessionPhase.WaitingForTrust) SessionRequestKind.HostKey else null
        )
    }
}

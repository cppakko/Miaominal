package dev.akko.miaominal.ui.components

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import dev.akko.miaominal.R
import dev.akko.miaominal.data.HostKeyDecision
import dev.akko.miaominal.data.SessionRequestKind
import dev.akko.miaominal.data.SessionRequestModel

/**
 * Typed trust/authentication dialog.
 *
 * Every answer carries the request id from the backend; a stale dialog cannot
 * answer a newer challenge. The dialog is derived from the backend snapshot, so
 * an Activity rebuild shows the same pending request again.
 */
@Composable
fun SessionRequestDialog(
    request: SessionRequestModel,
    onHostKey: (Long, HostKeyDecision) -> Unit,
    onKeyboardInteractive: (Long, List<String>) -> Unit,
    onCancel: () -> Unit
) {
    when (request.kind) {
        SessionRequestKind.HostKey -> HostKeyDialog(request, onHostKey, onCancel)
        SessionRequestKind.KeyboardInteractive -> KeyboardInteractiveDialog(request, onKeyboardInteractive, onCancel)
    }
}

@Composable
@OptIn(ExperimentalMaterial3Api::class)
private fun HostKeyDialog(
    request: SessionRequestModel,
    onHostKey: (Long, HostKeyDecision) -> Unit,
    onCancel: () -> Unit
) {
    ModalBottomSheet(
        onDismissRequest = onCancel,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        containerColor = MaterialTheme.colorScheme.surfaceContainer
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .heightIn(max = 680.dp)
                .verticalScroll(rememberScrollState())
                .navigationBarsPadding()
                .padding(horizontal = 20.dp)
                .padding(bottom = 24.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp)
        ) {
            Text(
                stringResource(
                    if (request.keyChanged) R.string.session_trust_changed_title
                    else R.string.session_trust_title
                ),
                style = MaterialTheme.typography.titleLarge
            )

            if (request.keyChanged) {
                Surface(
                    color = MaterialTheme.colorScheme.errorContainer,
                    contentColor = MaterialTheme.colorScheme.onErrorContainer,
                    shape = MaterialTheme.shapes.medium
                ) {
                    Text(
                        stringResource(R.string.session_trust_changed_warning),
                        modifier = Modifier.padding(12.dp),
                        style = MaterialTheme.typography.bodyMedium
                    )
                }
            }
            if (request.hopTotal > 1) {
                Text(
                    stringResource(
                        R.string.session_hop,
                        request.hopIndex,
                        request.hopTotal,
                        request.hopLabel
                    ),
                    style = MaterialTheme.typography.labelMedium,
                    color = MaterialTheme.colorScheme.primary
                )
            }
            Text("${request.host}:${request.port}", style = MaterialTheme.typography.titleMedium)
            Text(
                request.algorithm,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
            SelectionContainer {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(
                        stringResource(R.string.session_trust_fingerprint, request.fingerprint),
                        style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace)
                    )
                    request.previousFingerprint?.let { previous ->
                        Text(
                            stringResource(R.string.session_trust_previous_fingerprint, previous),
                            style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace),
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    }
                }
            }

            if (request.keyChanged) {
                Button(
                    onClick = onCancel,
                    modifier = Modifier.fillMaxWidth(),
                    colors = ButtonDefaults.buttonColors(
                        containerColor = MaterialTheme.colorScheme.error,
                        contentColor = MaterialTheme.colorScheme.onError
                    )
                ) {
                    Text(stringResource(R.string.session_trust_reject))
                }
                OutlinedButton(
                    onClick = { onHostKey(request.requestId, HostKeyDecision.AcceptAndSave) },
                    modifier = Modifier.fillMaxWidth()
                ) {
                    Text(stringResource(R.string.session_trust_accept_save))
                }
                TextButton(
                    onClick = { onHostKey(request.requestId, HostKeyDecision.AcceptOnce) },
                    modifier = Modifier.fillMaxWidth()
                ) {
                    Text(stringResource(R.string.session_trust_accept_once))
                }
            } else {
                Button(
                    onClick = { onHostKey(request.requestId, HostKeyDecision.AcceptAndSave) },
                    modifier = Modifier.fillMaxWidth()
                ) {
                    Text(stringResource(R.string.session_trust_accept_save))
                }
                OutlinedButton(
                    onClick = { onHostKey(request.requestId, HostKeyDecision.AcceptOnce) },
                    modifier = Modifier.fillMaxWidth()
                ) {
                    Text(stringResource(R.string.session_trust_accept_once))
                }
                TextButton(
                    onClick = onCancel,
                    modifier = Modifier.fillMaxWidth(),
                    colors = ButtonDefaults.textButtonColors(contentColor = MaterialTheme.colorScheme.error)
                ) {
                    Text(stringResource(R.string.session_trust_reject))
                }
            }
        }
    }
}

@Composable
private fun KeyboardInteractiveDialog(
    request: SessionRequestModel,
    onKeyboardInteractive: (Long, List<String>) -> Unit,
    onCancel: () -> Unit
) {
    val answers = remember(request.requestId) {
        mutableStateListOf<String>().apply { request.prompts.forEach { add("") } }
    }
    AlertDialog(
        onDismissRequest = onCancel,
        title = {
            Text(request.name.ifBlank { stringResource(R.string.session_auth_title) })
        },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                if (request.instructions.isNotBlank()) {
                    Text(request.instructions, style = MaterialTheme.typography.bodyMedium)
                }
                if (request.hopTotal > 1) {
                    Text(
                        stringResource(
                            R.string.session_hop,
                            request.hopIndex,
                            request.hopTotal,
                            request.hopLabel
                        ),
                        style = MaterialTheme.typography.labelMedium
                    )
                }
                request.prompts.forEachIndexed { index, prompt ->
                    OutlinedTextField(
                        value = answers.getOrElse(index) { "" },
                        onValueChange = { value -> if (index < answers.size) answers[index] = value },
                        label = { Text(prompt.prompt) },
                        singleLine = true,
                        visualTransformation = if (prompt.echo) {
                            VisualTransformation.None
                        } else {
                            PasswordVisualTransformation()
                        },
                        keyboardOptions = KeyboardOptions(),
                        modifier = Modifier.fillMaxWidth()
                    )
                }
            }
        },
        confirmButton = {
            TextButton(onClick = {
                onKeyboardInteractive(request.requestId, answers.toList())
            }) {
                Text(stringResource(R.string.session_auth_submit))
            }
        },
        dismissButton = {
            TextButton(onClick = onCancel) {
                Text(stringResource(R.string.connection_cancel))
            }
        }
    )
}

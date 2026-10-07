package dev.akko.miaominal.ui.components

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import dev.akko.miaominal.R

@OptIn(ExperimentalMaterial3Api::class, ExperimentalLayoutApi::class)
@Composable
fun ConnectionErrorActions(message: String, onReconnect: () -> Unit, onEdit: () -> Unit) {
    var details by remember { mutableStateOf(false) }
    val context = LocalContext.current
    Column {
        Text(stringResource(R.string.ux_reconnect_notice), style = MaterialTheme.typography.bodySmall)
        FlowRow(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            FilledTonalButton(onClick = onReconnect) { Text(stringResource(R.string.ux_retry)) }
            TextButton(onClick = onEdit) { Text(stringResource(R.string.ux_edit)) }
            TextButton(onClick = { details = true }) { Text(stringResource(R.string.ux_details)) }
        }
    }
    if (details) ModalBottomSheet(onDismissRequest = { details = false }) {
        Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(24.dp)) {
            Text(stringResource(R.string.ux_details), style = MaterialTheme.typography.headlineSmall)
            SelectionContainer { Text(message) }
            TextButton(onClick = {
                (context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager)
                    .setPrimaryClip(ClipData.newPlainText("connection", message))
            }) { Text(stringResource(R.string.ux_copy)) }
        }
    }
}

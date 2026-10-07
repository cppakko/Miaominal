package dev.akko.miaominal.ui.agent

import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import dev.akko.miaominal.R
import dev.akko.miaominal.data.agent.ContentBlock
import kotlinx.coroutines.delay

@Composable
fun AgentCopyButton(text: String) {
    val clipboard = LocalClipboardManager.current
    var copied by remember { mutableStateOf(false) }
    LaunchedEffect(copied) { if (copied) { delay(1800); copied = false } }
    TextButton(onClick = { clipboard.setText(AnnotatedString(text)); copied = true }) {
        Text(stringResource(if (copied) R.string.agent_copied else R.string.agent_copy))
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun AgentCodeBlock(text: String, language: String = "", previewLines: Int = 12, diff: Boolean = false) {
    var details by rememberSaveable { mutableStateOf(false) }
    val lines = remember(text) { text.lines() }
    Surface(shape = MaterialTheme.shapes.medium, color = MaterialTheme.colorScheme.surfaceContainerHigh,
        modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.padding(10.dp)) {
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                Text(language, style = MaterialTheme.typography.labelMedium, modifier = Modifier.weight(1f).padding(top = 12.dp))
                AgentCopyButton(text)
            }
            if (diff) Text(stringResource(R.string.agent_changes_summary,
                lines.count { it.startsWith("+") && !it.startsWith("+++") },
                lines.count { it.startsWith("-") && !it.startsWith("---") }), style = MaterialTheme.typography.labelMedium)
            AgentCodeText(lines.take(previewLines).joinToString("\n"), diff)
            if (lines.size > previewLines || diff) TextButton(onClick = { details = true }) {
                Text(stringResource(R.string.agent_details))
            }
        }
    }
    if (details) ModalBottomSheet(onDismissRequest = { details = false },
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)) {
        Column(Modifier.fillMaxWidth().heightIn(max = 640.dp).padding(16.dp)) {
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                TextButton(onClick = { details = false }) { Text(stringResource(R.string.agent_close_detail)) }
                AgentCopyButton(text)
            }
            SelectionContainer(Modifier.weight(1f, fill = false).verticalScroll(rememberScrollState())) {
                AgentCodeText(text, diff)
            }
        }
    }
}

@Composable
private fun AgentCodeText(text: String, diff: Boolean) {
    val colors = MaterialTheme.colorScheme
    val content = remember(text, diff, colors) {
        buildAnnotatedString {
            text.lines().forEachIndexed { index, line ->
                if (index > 0) append('\n')
                val color = when {
                    diff && line.startsWith("+") -> colors.primary
                    diff && line.startsWith("-") -> colors.error
                    diff && line.startsWith("@@") -> colors.tertiary
                    else -> colors.onSurface
                }
                withStyle(SpanStyle(color = color)) { append(line) }
            }
        }
    }
    Text(content, style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace),
        modifier = Modifier.horizontalScroll(rememberScrollState()).padding(vertical = 8.dp))
}

/** Older cached notices remain available, but never occupy the composer. */
@Composable
fun AgentEventHistory(notices: List<ContentBlock.Notice>) {
    var expanded by rememberSaveable { mutableStateOf(false) }
    Column {
        TextButton(onClick = { expanded = !expanded }) { Text(stringResource(R.string.agent_events)) }
        if (expanded) notices.forEach { AgentNoticeView(it.text) }
    }
}

@Composable
fun AgentNoticeView(text: String) {
    Surface(shape = MaterialTheme.shapes.medium, color = MaterialTheme.colorScheme.surfaceContainerLow,
        modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.padding(12.dp)) {
            Text(text, style = MaterialTheme.typography.bodySmall)
            if (text.startsWith("Image", ignoreCase = true)) {
                Text(stringResource(R.string.agent_image_unavailable), style = MaterialTheme.typography.bodySmall)
            }
            AgentCopyButton(text.substringAfter('\n', text))
        }
    }
}

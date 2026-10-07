package dev.akko.miaominal.ui.screens

import androidx.compose.foundation.clickable
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.relocation.BringIntoViewRequester
import androidx.compose.foundation.relocation.bringIntoViewRequester
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import dev.akko.miaominal.R
import dev.akko.miaominal.ui.LocalMiaominalRepository
import dev.akko.miaominal.ui.components.FontMetricControl
import dev.akko.miaominal.ui.components.TerminalFontPreview
import dev.akko.miaominal.ui.fonts.*
import dev.akko.miaominal.ui.theme.LocalUiPreferences
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun TerminalTypographySettings() {
    val repository = LocalMiaominalRepository.current
    val settings by repository.settings.collectAsState()
    val ui = LocalUiPreferences.current
    val operations: dev.akko.miaominal.ui.model.SettingsOperations = androidx.lifecycle.viewmodel.compose.viewModel(key = "settings-operations")
    var size by rememberSaveable(settings.terminalFontSize) { mutableFloatStateOf(settings.terminalFontSize) }
    var height by rememberSaveable(settings.terminalLineHeight) { mutableFloatStateOf(settings.terminalLineHeight) }
    var retry by remember { mutableIntStateOf(0) }
    val catalog by produceState<Result<List<SystemFontFamilyOption>>?>(null, retry) {
        value = null
        value = withContext(Dispatchers.Default) { runCatching { loadSystemFontFamilies() } }
    }
    val error = operations.typographyError?.let(::describeSyncError)
    val picker = operations.typographyPicker
    var draft by rememberSaveable { mutableStateOf("") }
    var query by rememberSaveable { mutableStateOf("") }
    val saving = operations.typographyBusy
    LaunchedEffect(operations.typographyError) { if (operations.typographyError != null) { size = repository.settings.value.terminalFontSize; height = repository.settings.value.terminalLineHeight } }
    val families = catalog?.getOrNull().orEmpty()
    SettingsDetailCard {
        TerminalFontPreview(if (picker == "primary") draft else settings.terminalFont,
            if (picker == "fallback") draft else settings.cjkFallbackFont, size, height)
        if (catalog == null) LinearProgressIndicator(Modifier.fillMaxWidth())
        if (catalog?.isFailure == true || (catalog != null && families.isEmpty())) {
            SettingsErrorText(stringResource(R.string.settings_font_load_failed))
            OutlinedButton(onClick = { retry++ }) { Text(stringResource(R.string.settings_retry)) }
        }
        listOf("primary" to settings.terminalFont, "fallback" to settings.cjkFallbackFont).forEach { (kind, font) ->
            ListItem(headlineContent = { Text(stringResource(if (kind == "primary") R.string.desktop_appearance_terminal_font else R.string.desktop_appearance_cjk_fallbacks)) },
                supportingContent = { Text(font.ifBlank { stringResource(R.string.desktop_appearance_system_font_default) }) },
                modifier = Modifier.clickable(enabled = catalog?.isSuccess == true && !saving) { operations.typographyPicker = kind; draft = font; query = "" })
            if (catalog?.isSuccess == true && font.isNotBlank() && families.none { it.familyName == font }) {
                Text(stringResource(R.string.settings_font_fallback, font), color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
        FontMetricControl(size, stringResource(R.string.font_size_label), 8f..32f,
            onCommit = { next -> operations.saveFontMetric(repository, size = next) },
            onDraftChange = { size = it })
        FontMetricControl(height, stringResource(R.string.font_line_height_label), 12f..40f,
            onCommit = { next -> operations.saveFontMetric(repository, height = next) },
            onDraftChange = { height = it })
        ui?.let {
            SettingsToggleRow(title = stringResource(R.string.ux_follow_font), subtitle = stringResource(R.string.ux_follow_font_hint), checked = it.followTerminalFontScale, onCheckedChange = it::updateFollowTerminalFontScale, enabled = !it.busy)
            if (it.saveFailed) SettingsErrorText(stringResource(R.string.settings_save_failed))
        }
        error?.let { SettingsErrorText(it) }
    }
    picker?.let { kind ->
        val errorBring = remember { BringIntoViewRequester() }
        LaunchedEffect(error) { if (error != null) errorBring.bringIntoView() }
        ModalBottomSheet(onDismissRequest = { if (!saving) operations.typographyPicker = null }, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true,
            confirmValueChange = { !saving || it != SheetValue.Hidden }),
            properties = ModalBottomSheetProperties(shouldDismissOnBackPress = !saving)) {
            Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedTextField(query, { query = it }, label = { Text(stringResource(R.string.settings_font_search)) }, singleLine = true, modifier = Modifier.fillMaxWidth())
                TerminalFontPreview(if (kind == "primary") draft else settings.terminalFont, if (kind == "fallback") draft else settings.cjkFallbackFont, size, height)
                LazyColumn(Modifier.heightIn(max = 280.dp)) {
                    item { ListItem(headlineContent = { Text(stringResource(R.string.desktop_appearance_system_font_default)) }, leadingContent = { RadioButton(draft.isBlank(), null) }, modifier = Modifier.selectable(selected = draft.isBlank(), enabled = !saving, role = Role.RadioButton, onClick = { draft = "" })) }
                    items(families.filter { (if (kind == "primary") it.monospace else it.cjk) && it.familyName.contains(query, true) }, key = { it.familyName }) { font ->
                        ListItem(headlineContent = { Text(font.familyName) }, leadingContent = { RadioButton(draft == font.familyName, null) }, modifier = Modifier.selectable(selected = draft == font.familyName, enabled = !saving, role = Role.RadioButton, onClick = { draft = font.familyName }))
                    }
                }
                Box(Modifier.bringIntoViewRequester(errorBring)) { error?.let { SettingsErrorText(it) } }
                if (saving) LinearProgressIndicator(Modifier.fillMaxWidth())
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    TextButton(enabled = !saving, onClick = { operations.typographyPicker = null }) { Text(stringResource(R.string.tool_action_cancel)) }
                    Button(enabled = !saving, onClick = {
                        operations.applyFont(repository, kind, draft)
                    }) { Text(stringResource(R.string.settings_apply)) }
                }
            }
        }
    }
}

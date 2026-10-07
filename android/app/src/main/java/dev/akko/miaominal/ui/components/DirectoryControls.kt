package dev.akko.miaominal.ui.components

import dev.akko.miaominal.ui.theme.UiSpacing
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import dev.akko.miaominal.R
import dev.akko.miaominal.ui.model.FileSort
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.Sort
import dev.akko.miaominal.ui.components.HintIcon as Icon
import androidx.compose.ui.platform.LocalContext
import androidx.compose.foundation.text.selection.SelectionContainer

@Composable
fun DirectoryControls(query: String, onQuery: (String) -> Unit,
    sort: FileSort, onSort: (FileSort) -> Unit, onSelect: () -> Unit) {
    var menu by remember { mutableStateOf(false) }
    Row(Modifier.fillMaxWidth().padding(horizontal = UiSpacing.xs), horizontalArrangement = Arrangement.spacedBy(UiSpacing.xxs)) {
        OutlinedTextField(query, onQuery, modifier = Modifier.weight(1f), singleLine = true,
            label = { Text(stringResource(R.string.ux_filter_directory)) })
        Box {
            IconButton(onClick = { menu = true }) { Icon(Icons.AutoMirrored.Outlined.Sort, stringResource(when (sort) {
                FileSort.Name -> R.string.ux_sort_name
                FileSort.Size -> R.string.ux_sort_size
                FileSort.Modified -> R.string.ux_sort_modified
            })) }
            DropdownMenu(menu, { menu = false }) {
                FileSort.entries.forEach { option -> DropdownMenuItem(
                    text = { Text(stringResource(when (option) {
                        FileSort.Name -> R.string.ux_sort_name
                        FileSort.Size -> R.string.ux_sort_size
                        FileSort.Modified -> R.string.ux_sort_modified
                    })) }, onClick = { onSort(option); menu = false }) }
                DropdownMenuItem(text = { Text(stringResource(R.string.ux_choose)) },
                    onClick = { menu = false; onSelect() })
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PathBreadcrumbs(path: String, onNavigate: (String) -> Unit) {
    val segments = path.split('/').filter { it.isNotEmpty() && it != "." }
    var fullPath by remember { mutableStateOf(false) }
    val context = LocalContext.current
    Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState())) {
        TextButton(onClick = { onNavigate(if (path.startsWith('/')) "/" else ".") }) { Text(if (path.startsWith('/')) "/" else ".") }
        segments.forEachIndexed { index, name ->
            val target = (if (path.startsWith('/')) "/" else "") + segments.take(index + 1).joinToString("/")
            TextButton(onClick = { onNavigate(target) }) { Text(name) }
        }
        TextButton(onClick = { fullPath = true }) { Text(stringResource(R.string.ux_details)) }
    }
    if (fullPath) ModalBottomSheet(onDismissRequest = { fullPath = false }) {
        Column(Modifier.fillMaxWidth().padding(UiSpacing.md)) {
            SelectionContainer { Text(path.ifEmpty { "." }) }
            TextButton(onClick = { (context.getSystemService(android.content.Context.CLIPBOARD_SERVICE) as android.content.ClipboardManager)
                .setPrimaryClip(android.content.ClipData.newPlainText("path", path)) }) { Text(stringResource(R.string.ux_copy_path)) }
        }
    }
}

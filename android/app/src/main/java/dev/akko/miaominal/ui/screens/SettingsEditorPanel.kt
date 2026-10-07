package dev.akko.miaominal.ui.screens

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import dev.akko.miaominal.R
import dev.akko.miaominal.ui.components.HintIcon as Icon
import dev.akko.miaominal.ui.components.BackGestureState
import dev.akko.miaominal.ui.components.LocalBackGesture
import dev.akko.miaominal.ui.components.WorkspaceBackHandler

/** Owns editor insets; leaves the underlying navigation chrome in place. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun SettingsEditorPanel(title: String, busy: Boolean, onDismiss: () -> Unit, scroll: Boolean = true, content: @Composable ColumnScope.() -> Unit) {
    Dialog(onDismissRequest = { if (!busy) onDismiss() }, properties = DialogProperties(
        usePlatformDefaultWidth = false, decorFitsSystemWindows = false,
        dismissOnBackPress = false, dismissOnClickOutside = false)) {
        val gesture = remember { BackGestureState() }
        CompositionLocalProvider(LocalBackGesture provides gesture) {
        WorkspaceBackHandler { if (!busy) onDismiss() }
        Scaffold(modifier = Modifier.fillMaxSize().imePadding().graphicsLayer {
            scaleX = 1f - gesture.progress * 0.04f; scaleY = scaleX
            translationX = gesture.progress * size.width * 0.04f
        }, topBar = {
            TopAppBar(title = { Text(title) }, navigationIcon = {
                IconButton(enabled = !busy, onClick = onDismiss) { Icon(Icons.AutoMirrored.Outlined.ArrowBack, stringResource(R.string.action_back)) }
            })
        }) { padding ->
            Column(Modifier.fillMaxSize().padding(padding).wrapContentWidth(Alignment.CenterHorizontally).widthIn(max = 720.dp).fillMaxWidth()
                .then(if (scroll) Modifier.verticalScroll(rememberScrollState()) else Modifier).padding(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
                if (busy) LinearProgressIndicator(Modifier.fillMaxWidth())
                content()
            }
        }
        }
    }
}

@Composable
internal fun SettingsErrorText(message: String) {
    Text(message, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodyMedium,
        modifier = Modifier.fillMaxWidth())
}

@Composable
internal fun LinearSettingsProgress() { LinearProgressIndicator(Modifier.fillMaxWidth()) }

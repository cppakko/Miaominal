package dev.akko.miaominal.ui.screens

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.ErrorOutline
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.material3.windowsizeclass.ExperimentalMaterial3WindowSizeClassApi
import androidx.compose.material3.windowsizeclass.WindowSizeClass
import androidx.compose.material3.windowsizeclass.WindowWidthSizeClass
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.DpSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import dev.akko.miaominal.R
import dev.akko.miaominal.ui.components.BackGestureState
import dev.akko.miaominal.ui.components.HintIcon as Icon
import dev.akko.miaominal.ui.components.LocalBackGesture
import dev.akko.miaominal.ui.components.WorkspaceBackHandler
import dev.akko.miaominal.ui.model.LicenseDocument
import dev.akko.miaominal.ui.model.licenseTextBlocks
import dev.akko.miaominal.ui.theme.UiSpacing
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.IOException

private sealed interface LicenseContent {
    data object Loading : LicenseContent
    data object Failed : LicenseContent
    data class Loaded(val blocks: List<String>) : LicenseContent
}

@OptIn(ExperimentalMaterial3Api::class, ExperimentalMaterial3WindowSizeClassApi::class)
@Composable
internal fun LicenseDocumentPanel(document: LicenseDocument, onDismiss: () -> Unit) {
    val context = LocalContext.current
    var retry by remember { mutableIntStateOf(0) }
    val content by produceState<LicenseContent>(LicenseContent.Loading, context, document, retry) {
        value = LicenseContent.Loading
        value = withContext(Dispatchers.IO) {
            try {
                val text = context.assets.open(document.assetPath)
                    .bufferedReader(Charsets.UTF_8).use { it.readText() }
                LicenseContent.Loaded(licenseTextBlocks(text))
            } catch (_: IOException) {
                LicenseContent.Failed
            }
        }
    }

    Dialog(
        onDismissRequest = onDismiss,
        properties = DialogProperties(
            usePlatformDefaultWidth = false,
            decorFitsSystemWindows = false,
            dismissOnBackPress = false,
            dismissOnClickOutside = false,
        ),
    ) {
        val gesture = remember { BackGestureState() }
        val scrollBehavior = TopAppBarDefaults.pinnedScrollBehavior()
        CompositionLocalProvider(LocalBackGesture provides gesture) {
            WorkspaceBackHandler(onBack = onDismiss)
            Scaffold(
                modifier = Modifier.fillMaxSize()
                    .nestedScroll(scrollBehavior.nestedScrollConnection)
                    .graphicsLayer {
                        scaleX = 1f - gesture.progress * 0.04f
                        scaleY = scaleX
                        translationX = gesture.progress * size.width * 0.04f
                    },
                containerColor = MaterialTheme.colorScheme.surface,
                contentWindowInsets = WindowInsets.safeDrawing,
                topBar = {
                    TopAppBar(
                        title = {
                            Text(stringResource(document.titleRes), maxLines = 1, overflow = TextOverflow.Ellipsis)
                        },
                        navigationIcon = {
                            IconButton(onClick = onDismiss) {
                                Icon(Icons.Outlined.Close, stringResource(R.string.about_close_license))
                            }
                        },
                        colors = TopAppBarDefaults.topAppBarColors(
                            containerColor = MaterialTheme.colorScheme.surface,
                            scrolledContainerColor = MaterialTheme.colorScheme.surfaceContainer,
                        ),
                        scrollBehavior = scrollBehavior,
                    )
                },
            ) { padding ->
                BoxWithConstraints(
                    modifier = Modifier.fillMaxSize().padding(padding).consumeWindowInsets(padding),
                    contentAlignment = Alignment.TopCenter,
                ) {
                    val sizeClass = WindowSizeClass.calculateFromSize(DpSize(maxWidth, maxHeight))
                    val horizontalPadding = if (sizeClass.widthSizeClass == WindowWidthSizeClass.Compact) UiSpacing.sm else UiSpacing.md
                    val readerModifier = Modifier.widthIn(max = 720.dp).fillMaxSize()
                    when (val loaded = content) {
                        LicenseContent.Loading -> LicenseStatus(
                            modifier = readerModifier,
                            loading = true,
                            onRetry = { retry++ },
                        )
                        LicenseContent.Failed -> LicenseStatus(
                            modifier = readerModifier,
                            loading = false,
                            onRetry = { retry++ },
                        )
                        is LicenseContent.Loaded -> LazyColumn(
                            modifier = readerModifier.testTag("license-document-text"),
                            contentPadding = PaddingValues(horizontal = horizontalPadding, vertical = UiSpacing.md),
                        ) {
                            items(count = loaded.blocks.size, key = { it }, contentType = { "license-text" }) { index ->
                                SelectionContainer {
                                    Text(
                                        text = loaded.blocks[index],
                                        modifier = Modifier.fillMaxWidth(),
                                        style = MaterialTheme.typography.bodyLarge,
                                        color = MaterialTheme.colorScheme.onSurface,
                                    )
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun LicenseStatus(modifier: Modifier, loading: Boolean, onRetry: () -> Unit) {
    Box(modifier, contentAlignment = Alignment.Center) {
        Column(
            modifier = Modifier.fillMaxWidth().padding(UiSpacing.md)
                .semantics(mergeDescendants = true) { liveRegion = LiveRegionMode.Polite },
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(UiSpacing.md),
        ) {
            if (loading) {
                CircularProgressIndicator()
                Text(
                    stringResource(R.string.about_license_loading),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            } else {
                Surface(
                    shape = MaterialTheme.shapes.large,
                    color = MaterialTheme.colorScheme.errorContainer,
                    contentColor = MaterialTheme.colorScheme.onErrorContainer,
                ) {
                    Column(
                        modifier = Modifier.padding(UiSpacing.md),
                        horizontalAlignment = Alignment.CenterHorizontally,
                        verticalArrangement = Arrangement.spacedBy(UiSpacing.sm),
                    ) {
                        Icon(Icons.Outlined.ErrorOutline, contentDescription = null)
                        Text(
                            stringResource(R.string.about_license_read_failed),
                            style = MaterialTheme.typography.bodyLarge,
                            textAlign = TextAlign.Center,
                        )
                    }
                }
                FilledTonalButton(onClick = onRetry) { Text(stringResource(R.string.settings_retry)) }
            }
        }
    }
}

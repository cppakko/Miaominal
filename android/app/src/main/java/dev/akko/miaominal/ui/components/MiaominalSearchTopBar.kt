package dev.akko.miaominal.ui.components

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.CubicBezierEasing
import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import dev.akko.miaominal.ui.components.WorkspaceBackHandler as BackHandler
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.text.input.clearText
import androidx.compose.foundation.text.input.rememberTextFieldState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.Search
import androidx.compose.material3.ExpandedFullScreenSearchBar
import androidx.compose.material3.ExperimentalMaterial3Api
import dev.akko.miaominal.ui.components.HintIcon as Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SearchBarDefaults
import androidx.compose.material3.SearchBarValue
import androidx.compose.material3.Text
import androidx.compose.material3.TopSearchBar
import androidx.compose.material3.rememberSearchBarState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import dev.akko.miaominal.R
import dev.akko.miaominal.ui.components.MiaominalTopBarShape
import kotlinx.coroutines.launch

private val SearchExpandEasing = CubicBezierEasing(0.05f, 0.7f, 0.1f, 1f)

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun MiaominalSearchTopBar(
    placeholder: String,
    modifier: Modifier = Modifier,
    collapsedLeadingIcon: ImageVector = Icons.Outlined.Search,
    collapsedLeadingContentDescription: String? = null,
    onCollapsedLeadingClick: (() -> Unit)? = null,
    collapsedTrailingContent: (@Composable () -> Unit)? = null,
    trailingClickExpands: Boolean = false,
    collapsedWindowInsets: WindowInsets = SearchBarDefaults.windowInsets,
    /** Pre-fills the query; a non-empty value expands the search on first composition. */
    initialQuery: String = "",
    expandedContent: @Composable (query: String, collapse: () -> Unit) -> Unit
) {
    val textFieldState = rememberTextFieldState(initialQuery)
    val searchBarState = rememberSearchBarState(
        animationSpecForExpand = tween(
            durationMillis = 500,
            easing = SearchExpandEasing
        ),
        animationSpecForCollapse = tween(
            durationMillis = 360,
            easing = FastOutSlowInEasing
        )
    )
    val coroutineScope = rememberCoroutineScope()
    val keyboardController = LocalSoftwareKeyboardController.current
    val inputContainerColor = MaterialTheme.colorScheme.surfaceContainerLowest
    val inputFieldColors = SearchBarDefaults.inputFieldColors(
        focusedContainerColor = inputContainerColor,
        unfocusedContainerColor = inputContainerColor,
        disabledContainerColor = inputContainerColor,
        cursorColor = MaterialTheme.colorScheme.primary,
        focusedTrailingIconColor = MaterialTheme.colorScheme.primary,
        unfocusedTrailingIconColor = MaterialTheme.colorScheme.onSurfaceVariant
    )
    val searchBarColors = SearchBarDefaults.colors(
        containerColor = MaterialTheme.colorScheme.surfaceContainer,
        dividerColor = Color.Transparent,
        inputFieldColors = inputFieldColors
    )
    val collapse = {
        keyboardController?.hide()
        coroutineScope.launch { searchBarState.animateToCollapsed() }
        Unit
    }
    // Entering the screen with a prefilled query (a tools-tab search result)
    // should land on the filtered results right away.
    LaunchedEffect(Unit) {
        if (initialQuery.isNotEmpty()) {
            searchBarState.animateToExpanded()
        }
    }
    val inputField = @Composable {
        SearchBarDefaults.InputField(
            textFieldState = textFieldState,
            searchBarState = searchBarState,
            onSearch = { keyboardController?.hide() },
            modifier = Modifier.widthIn(max = 720.dp).fillMaxWidth()
                .padding(horizontal = 8.dp * searchBarState.progress),
            placeholder = { Text(placeholder) },
            leadingIcon = {
                if (searchBarState.currentValue == SearchBarValue.Expanded) {
                    IconButton(onClick = collapse) {
                        Icon(
                            imageVector = Icons.AutoMirrored.Outlined.ArrowBack,
                            contentDescription = stringResource(R.string.action_back)
                        )
                    }
                } else if (onCollapsedLeadingClick != null) {
                    IconButton(onClick = onCollapsedLeadingClick) {
                        Icon(
                            imageVector = collapsedLeadingIcon,
                            contentDescription = collapsedLeadingContentDescription
                        )
                    }
                } else {
                    Icon(
                        imageVector = collapsedLeadingIcon,
                        contentDescription = collapsedLeadingContentDescription
                    )
                }
            },
            trailingIcon = {
                when {
                    textFieldState.text.isNotEmpty() -> {
                        IconButton(onClick = { textFieldState.clearText() }) {
                            Icon(
                                imageVector = Icons.Outlined.Close,
                                contentDescription = stringResource(R.string.search_clear)
                            )
                        }
                    }
                    searchBarState.currentValue == SearchBarValue.Collapsed -> {
                        if (trailingClickExpands) {
                            IconButton(
                                onClick = {
                                    coroutineScope.launch {
                                        searchBarState.animateToExpanded()
                                    }
                                }
                            ) {
                                collapsedTrailingContent?.invoke()
                            }
                        } else {
                            collapsedTrailingContent?.invoke()
                        }
                    }
                }
            },
            colors = inputFieldColors
        )
    }

    TopSearchBar(
        state = searchBarState,
        inputField = inputField,
        modifier = modifier,
        shape = MiaominalTopBarShape,
        colors = searchBarColors,
        tonalElevation = 0.dp,
        shadowElevation = 0.dp,
        windowInsets = collapsedWindowInsets
    )
    ExpandedFullScreenSearchBar(
        state = searchBarState,
        inputField = inputField,
        colors = searchBarColors
    ) {
        // The expanded search is a dialog window, so back has to be handled from
        // inside it: the main window's handlers never receive these events.
        BackHandler { collapse() }
        AnimatedVisibility(
            visible = searchBarState.progress > 0f,
            enter = fadeIn(animationSpec = tween(durationMillis = 180)),
            modifier = Modifier.fillMaxSize()
        ) {
            expandedContent(textFieldState.text.toString().trim(), collapse)
        }
    }
}

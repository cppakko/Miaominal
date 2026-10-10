package dev.akko.miaominal.ui.screens

import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowRight
import androidx.compose.material.icons.outlined.Gavel
import androidx.compose.material.icons.outlined.Layers
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.onClick
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import dev.akko.miaominal.R
import dev.akko.miaominal.ui.components.HintIcon as Icon
import dev.akko.miaominal.ui.model.LicenseDocument

@Composable
internal fun LicenseDocumentCard(document: LicenseDocument, onClick: () -> Unit) {
    val actionLabel = stringResource(R.string.about_view_license)
    val containerColor = MaterialTheme.colorScheme.surfaceContainerLowest
    Card(
        onClick = onClick,
        modifier = Modifier.fillMaxWidth()
            .testTag("about-license-${document.name}")
            .semantics {
                role = Role.Button
                onClick(label = actionLabel, action = null)
            },
        shape = MaterialTheme.shapes.medium,
        colors = CardDefaults.cardColors(containerColor = containerColor),
    ) {
        ListItem(
            headlineContent = { Text(stringResource(document.titleRes)) },
            supportingContent = { Text(stringResource(document.descriptionRes)) },
            leadingContent = {
                Icon(
                    imageVector = when (document) {
                        LicenseDocument.Project -> Icons.Outlined.Gavel
                        LicenseDocument.ThirdParty -> Icons.Outlined.Layers
                    },
                    contentDescription = null,
                )
            },
            trailingContent = {
                Icon(Icons.AutoMirrored.Outlined.KeyboardArrowRight, contentDescription = null)
            },
            colors = ListItemDefaults.colors(
                containerColor = containerColor,
                leadingIconColor = MaterialTheme.colorScheme.primary,
                trailingIconColor = MaterialTheme.colorScheme.onSurfaceVariant,
            ),
        )
    }
}

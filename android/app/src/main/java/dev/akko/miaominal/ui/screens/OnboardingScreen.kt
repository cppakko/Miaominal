package dev.akko.miaominal.ui.screens

import dev.akko.miaominal.ui.theme.UiSpacing
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.wrapContentWidth
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Lock
import androidx.compose.material.icons.outlined.Terminal
import androidx.compose.material.icons.outlined.Widgets
import androidx.compose.material3.Button
import dev.akko.miaominal.ui.components.HintIcon as Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.heading
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import dev.akko.miaominal.R
import dev.akko.miaominal.ui.components.BrandMark

private data class OnboardingStep(
    val titleRes: Int,
    val bodyRes: Int,
    val icon: ImageVector
)

private val OnboardingSteps = listOf(
    OnboardingStep(R.string.onboarding_step1_title, R.string.onboarding_step1_body, Icons.Outlined.Terminal),
    OnboardingStep(R.string.onboarding_step2_title, R.string.onboarding_step2_body, Icons.Outlined.Lock),
    OnboardingStep(R.string.onboarding_step3_title, R.string.onboarding_step3_body, Icons.Outlined.Widgets)
)

@Composable
fun OnboardingScreen(
    onComplete: () -> Unit,
    modifier: Modifier = Modifier,
    onAddHost: (() -> Unit)? = null,
    onImportHosts: (() -> Unit)? = null
) {
    var stepIndex by rememberSaveable { mutableIntStateOf(0) }
    val step = OnboardingSteps[stepIndex]
    val isLastStep = stepIndex == OnboardingSteps.lastIndex

    Surface(modifier = modifier.fillMaxSize(), color = MaterialTheme.colorScheme.surface) {
        Column(
            modifier = Modifier
                .fillMaxSize()
                .safeDrawingPadding()
                .wrapContentWidth()
                .widthIn(max = 600.dp)
                .padding(horizontal = 28.dp, vertical = UiSpacing.sm),
            horizontalAlignment = Alignment.CenterHorizontally
        ) {
            TextButton(onClick = { stepIndex = OnboardingSteps.lastIndex }, modifier = Modifier.align(Alignment.End), enabled = !isLastStep) {
                Text(stringResource(R.string.ux_skip))
            }
            BoxWithConstraints(Modifier.weight(1f).fillMaxWidth()) {
                val compactHeight = maxHeight < 400.dp
                Column(
                    modifier = Modifier.fillMaxSize().verticalScroll(rememberScrollState()),
                    horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.Center
                ) {
                    BrandMark()
                    Spacer(Modifier.height(if (compactHeight) 16.dp else 32.dp))
                    Surface(
                        modifier = Modifier.size(if (compactHeight) 64.dp else 96.dp),
                        shape = CircleShape,
                        color = MaterialTheme.colorScheme.primaryContainer,
                        contentColor = MaterialTheme.colorScheme.onPrimaryContainer
                    ) {
                        Icon(
                            imageVector = step.icon,
                            contentDescription = null,
                            modifier = Modifier.padding(if (compactHeight) 16.dp else 26.dp)
                        )
                    }
                    Spacer(Modifier.height(if (compactHeight) 16.dp else 28.dp))
                    Text(
                        text = stringResource(step.titleRes),
                        modifier = Modifier.semantics { heading() },
                        style = MaterialTheme.typography.headlineSmall,
                        textAlign = TextAlign.Center
                    )
                    Spacer(Modifier.height(12.dp))
                    Text(
                        text = stringResource(step.bodyRes),
                        style = MaterialTheme.typography.bodyLarge,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        textAlign = TextAlign.Center
                    )
                    Spacer(Modifier.height(16.dp))
                }
            }

            Text(stringResource(R.string.ux_step, stepIndex + 1, OnboardingSteps.size), style = MaterialTheme.typography.labelLarge)
            Row(
                horizontalArrangement = Arrangement.spacedBy(UiSpacing.xs),
                verticalAlignment = Alignment.CenterVertically
            ) {
                OnboardingSteps.indices.forEach { index ->
                    Box(
                        modifier = Modifier
                            .size(if (index == stepIndex) 10.dp else 8.dp)
                            .clip(CircleShape)
                            .background(
                                if (index == stepIndex) {
                                    MaterialTheme.colorScheme.primary
                                } else {
                                    MaterialTheme.colorScheme.outlineVariant
                                }
                            )
                    )
                }
            }
            Spacer(Modifier.height(24.dp))
            Button(
                onClick = {
                    if (isLastStep) { onComplete(); onAddHost?.invoke() } else stepIndex += 1
                },
                modifier = Modifier.fillMaxWidth()
            ) {
                Text(
                    stringResource(
                        if (isLastStep && onAddHost != null) R.string.ux_start_add else if (isLastStep) R.string.onboarding_start else R.string.onboarding_next
                    )
                )
            }
            if (isLastStep && onImportHosts != null) TextButton(onClick = { onComplete(); onImportHosts() }) {
                Text(stringResource(R.string.ux_start_import))
            }
            if (stepIndex > 0) TextButton(onClick = { stepIndex -= 1 }) { Text(stringResource(R.string.ux_previous)) }
        }
    }
}

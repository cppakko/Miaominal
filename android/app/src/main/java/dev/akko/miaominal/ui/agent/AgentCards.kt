package dev.akko.miaominal.ui.agent

import dev.akko.miaominal.R
import dev.akko.miaominal.ui.uiString

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.outlined.Block
import androidx.compose.material.icons.outlined.Build
import androidx.compose.material.icons.outlined.ErrorOutline
import androidx.compose.material.icons.outlined.Psychology
import androidx.compose.material.icons.outlined.Schedule
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import dev.akko.miaominal.ui.components.HintIcon as Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.material.icons.outlined.ExpandLess
import androidx.compose.material.icons.outlined.ExpandMore
import dev.akko.miaominal.data.agent.AgentMessage
import dev.akko.miaominal.data.agent.AgentRole
import dev.akko.miaominal.data.agent.ContentBlock
import dev.akko.miaominal.data.agent.InteractionKind
import dev.akko.miaominal.data.agent.InteractionOption
import dev.akko.miaominal.data.agent.InteractionQuestion
import dev.akko.miaominal.data.agent.InteractionRequest
import dev.akko.miaominal.data.agent.PlanStep
import dev.akko.miaominal.data.agent.ToolStatus

const val AgentMessageListTag = "agent-message-list"
const val AgentComposerTag = "agent-composer"
const val AgentInteractionTag = "agent-interaction"
const val AgentSendTag = "agent-send"
const val AgentReconnectTag = "agent-reconnect"

@Composable
fun AgentMessageView(message: AgentMessage, modifier: Modifier = Modifier) {
    when (message.role) {
        AgentRole.User -> BoxWithConstraints(
            modifier = modifier.fillMaxWidth(),
            contentAlignment = Alignment.CenterEnd
        ) {
            Surface(
                shape = RoundedCornerShape(22.dp),
                color = MaterialTheme.colorScheme.surfaceContainer,
                modifier = Modifier.widthIn(max = maxWidth * 0.88f)
            ) {
                Column(
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp),
                    verticalArrangement = Arrangement.spacedBy(6.dp)
                ) {
                    message.blocks.forEach { block ->
                        if (block is ContentBlock.Text) Text(block.text, style = MaterialTheme.typography.bodyLarge)
                        else AgentBlockView(block)
                    }
                }
            }
        }

        AgentRole.System -> Surface(
            shape = RoundedCornerShape(10.dp),
            color = MaterialTheme.colorScheme.surfaceContainerHigh,
            modifier = modifier.fillMaxWidth()
        ) {
            Column(
                modifier = Modifier.padding(10.dp),
                verticalArrangement = Arrangement.spacedBy(6.dp)
            ) {
                message.blocks.forEach { block -> AgentBlockView(block) }
            }
        }

        AgentRole.Assistant -> Column(
            modifier = modifier.fillMaxWidth(),
            verticalArrangement = Arrangement.spacedBy(8.dp)
        ) {
            message.blocks.forEach { block -> AgentBlockView(block) }
            val copyText = message.blocks.mapNotNull {
                (it as? ContentBlock.Text)?.text
            }.joinToString("\n\n")
            if (!message.streaming && copyText.isNotBlank()) AgentCopyButton(copyText)
        }
    }
}

@Composable
fun AgentBlockView(block: ContentBlock, modifier: Modifier = Modifier) {
    when (block) {
        is ContentBlock.Text -> AgentMarkdown(block.text, modifier)

        is ContentBlock.Reasoning -> AgentCollapsibleCard(
            title = uiString(R.string.ui_reasoning),
            icon = { Icon(Icons.Outlined.Psychology, contentDescription = null) },
            defaultExpanded = false,
            modifier = modifier
        ) {
            Text(
                text = block.text,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(10.dp)
            )
        }

        is ContentBlock.ToolCall -> AgentCollapsibleCard(
            title = (block.title ?: block.name) + " · " + toolStatusLabel(block.status),
            icon = { ToolStatusIcon(block.status) },
            defaultExpanded = false,
            modifier = modifier
        ) {
            Column(
                modifier = Modifier.padding(10.dp),
                verticalArrangement = Arrangement.spacedBy(6.dp)
            ) {
                Text(
                    text = block.name,
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
                block.input?.takeIf(String::isNotBlank)?.let { input ->
                    CodeText(input, maxLines = 6)
                }
                block.output?.takeIf(String::isNotBlank)?.let { output ->
                    CodeText(output, maxLines = 18)
                }
                block.error?.takeIf(String::isNotBlank)?.let { error ->
                    Text(
                        text = error,
                        style = MaterialTheme.typography.bodySmall.copy(
                            fontFamily = FontFamily.Monospace
                        ),
                        color = MaterialTheme.colorScheme.error
                    )
                }
            }
        }

        is ContentBlock.FileChanges -> AgentCollapsibleCard(
            title = uiString(R.string.ui_file_changes, block.changes.size),
            icon = { ToolStatusIcon(block.status) },
            defaultExpanded = block.changes.size <= 2,
            modifier = modifier
        ) {
            Column(
                modifier = Modifier.padding(10.dp),
                verticalArrangement = Arrangement.spacedBy(10.dp)
            ) {
                block.changes.forEach { change ->
                    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        Row(
                            horizontalArrangement = Arrangement.spacedBy(8.dp),
                            verticalAlignment = Alignment.CenterVertically
                        ) {
                            Surface(
                                shape = RoundedCornerShape(4.dp),
                                color = MaterialTheme.colorScheme.secondaryContainer
                            ) {
                                Text(
                                    text = change.kind,
                                    style = MaterialTheme.typography.labelSmall,
                                    modifier = Modifier.padding(horizontal = 6.dp, vertical = 2.dp)
                                )
                            }
                            Text(
                                text = change.path,
                                style = MaterialTheme.typography.bodySmall.copy(
                                    fontFamily = FontFamily.Monospace
                                ),
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                                modifier = Modifier.weight(1f)
                            )
                        }
                        change.diff?.takeIf(String::isNotBlank)?.let { diff ->
                            AgentCodeBlock(diff, language = change.path, previewLines = 8, diff = true)
                        }
                    }
                }
            }
        }

        is ContentBlock.Plan -> AgentCollapsibleCard(
            title = uiString(R.string.ui_plan),
            icon = { Icon(Icons.Outlined.Build, contentDescription = null) },
            defaultExpanded = true,
            modifier = modifier
        ) {
            Column(
                modifier = Modifier.padding(10.dp),
                verticalArrangement = Arrangement.spacedBy(6.dp)
            ) {
                block.explanation?.takeIf(String::isNotBlank)?.let { explanation ->
                    Text(explanation, style = MaterialTheme.typography.bodySmall)
                }
                block.steps.forEach { step -> PlanStepRow(step) }
            }
        }

        is ContentBlock.Notice -> AgentNoticeView(block.text)
    }
}

@Composable
private fun PlanStepRow(step: PlanStep) {
    Row(
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.CenterVertically
    ) {
        val (icon, tint) = when (step.status) {
            "completed" -> Icons.Filled.CheckCircle to MaterialTheme.colorScheme.primary
            "inProgress", "in_progress" -> Icons.Outlined.Schedule to MaterialTheme.colorScheme.tertiary
            else -> Icons.Outlined.Schedule to MaterialTheme.colorScheme.onSurfaceVariant
        }
        Icon(icon, contentDescription = null, tint = tint, modifier = Modifier.size(16.dp))
        Text(
            text = step.step,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurface
        )
    }
}

@Composable
private fun toolStatusLabel(status: ToolStatus): String = uiString(when (status) {
    ToolStatus.Pending -> R.string.agent_tool_pending
    ToolStatus.Running -> R.string.agent_tool_running
    ToolStatus.Completed -> R.string.agent_tool_completed
    ToolStatus.Failed -> R.string.agent_tool_failed
    ToolStatus.Declined -> R.string.agent_tool_declined
})

@Composable
private fun ToolStatusIcon(status: ToolStatus) {
    when (status) {
        ToolStatus.Pending -> Icon(
            Icons.Outlined.Schedule,
            contentDescription = null,
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.size(16.dp)
        )

        ToolStatus.Running -> CircularProgressIndicator(
            modifier = Modifier.size(14.dp),
            strokeWidth = 2.dp
        )

        ToolStatus.Completed -> Icon(
            Icons.Filled.CheckCircle,
            contentDescription = null,
            tint = MaterialTheme.colorScheme.primary,
            modifier = Modifier.size(16.dp)
        )

        ToolStatus.Failed -> Icon(
            Icons.Outlined.ErrorOutline,
            contentDescription = null,
            tint = MaterialTheme.colorScheme.error,
            modifier = Modifier.size(16.dp)
        )

        ToolStatus.Declined -> Icon(
            Icons.Outlined.Block,
            contentDescription = null,
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.size(16.dp)
        )
    }
}

@Composable
private fun AgentCollapsibleCard(
    title: String,
    icon: @Composable () -> Unit,
    defaultExpanded: Boolean,
    modifier: Modifier = Modifier,
    content: @Composable () -> Unit
) {
    var expanded by rememberSaveable { mutableStateOf(defaultExpanded) }
    val expandedLabel = uiString(if (expanded) R.string.agent_collapse else R.string.agent_expand)
    Surface(
        shape = RoundedCornerShape(10.dp),
        color = MaterialTheme.colorScheme.surfaceContainer,
        modifier = modifier.fillMaxWidth()
    ) {
        Column {
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically
            ) {
                TextButton(
                    onClick = { expanded = !expanded },
                    modifier = Modifier.weight(1f).semantics { stateDescription = expandedLabel }
                ) {
                    icon()
                    Spacer(Modifier.width(8.dp))
                    Text(
                        text = title,
                        style = MaterialTheme.typography.labelLarge,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite },
                        color = MaterialTheme.colorScheme.onSurface
                    )
                    Spacer(Modifier.weight(1f))
                    Icon(
                        imageVector = if (expanded) Icons.Outlined.ExpandLess
                        else Icons.Outlined.ExpandMore,
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
            }
            AnimatedVisibility(visible = expanded) {
                Column {
                    HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
                    content()
                }
            }
        }
    }
}

@Composable
private fun CodeText(text: String, maxLines: Int) {
    AgentCodeBlock(text, previewLines = maxLines)
}

/** One pending approval or follow-up question. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun AgentInteractionCard(
    request: InteractionRequest,
    onOption: (String) -> Unit,
    onAnswers: (Map<String, List<String>>) -> Unit,
    modifier: Modifier = Modifier,
    submitting: Boolean = false,
    failure: String? = null
) {
    var selections by rememberSaveable(request.id) { mutableStateOf(mapOf<String, List<String>>()) }
    var freeform by rememberSaveable(request.id) { mutableStateOf(mapOf<String, String>()) }

    Surface(
        shape = RoundedCornerShape(12.dp),
        color = MaterialTheme.colorScheme.tertiaryContainer,
        modifier = modifier
            .fillMaxWidth()
            .testTag(AgentInteractionTag)
    ) {
        Column(
            modifier = Modifier.padding(12.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp)
        ) {
            Text(
                text = request.title,
                style = MaterialTheme.typography.titleSmall,
                color = MaterialTheme.colorScheme.onTertiaryContainer
            )
            request.detail?.takeIf(String::isNotBlank)?.let { detail ->
                Text(
                    text = detail,
                    style = MaterialTheme.typography.bodySmall.copy(
                        fontFamily = FontFamily.Monospace
                    ),
                    color = MaterialTheme.colorScheme.onTertiaryContainer
                )
            }
            if (submitting) Text(uiString(R.string.agent_submitting),
                modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite })
            if (failure != null && !submitting) Text(uiString(R.string.agent_submit_failed),
                color = MaterialTheme.colorScheme.error,
                modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite })
            if (request.kind == InteractionKind.Approval && request.questions.isEmpty()) {
                Text(uiString(R.string.agent_allow_scope), style = MaterialTheme.typography.bodySmall)
                FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    request.options.forEach { option ->
                        val primary = option.id in setOf("accept", "once")
                        if (primary) {
                            androidx.compose.material3.Button(
                                onClick = { onOption(option.id) },
                                enabled = !submitting,
                                modifier = Modifier.testTag("agent-option-${option.id}")
                            ) {
                                Text(optionLabel(option))
                            }
                        } else {
                            androidx.compose.material3.OutlinedButton(
                                onClick = { onOption(option.id) },
                                enabled = !submitting,
                                modifier = Modifier.testTag("agent-option-${option.id}")
                            ) {
                                Text(optionLabel(option))
                            }
                        }
                    }
                }
            }
            if (request.questions.isNotEmpty()) {
                request.questions.forEach { question ->
                    AgentQuestionView(
                        question = question,
                        enabled = !submitting,
                        selected = selections[question.id].orEmpty().toMutableList(),
                        freeformValue = freeform[question.id].orEmpty(),
                        onSelectionChange = { selections = selections + (question.id to it.toList()) },
                        onFreeformChange = { freeform = freeform + (question.id to it) }
                    )
                }
                TextButton(
                    onClick = {
                        val answers = request.questions.associate { question ->
                            val values = mutableListOf<String>()
                            values += selections[question.id].orEmpty()
                            freeform[question.id]
                                ?.takeIf(String::isNotBlank)
                                ?.let(values::add)
                            question.id to values
                        }
                        onAnswers(answers)
                    },
                    enabled = !submitting && request.questions.all { question ->
                        selections[question.id].orEmpty().isNotEmpty() || !freeform[question.id].isNullOrBlank()
                    },
                    modifier = Modifier.testTag("agent-answer-submit")
                ) {
                    Text(uiString(R.string.ui_submit_answers))
                }
            }
        }
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun AgentQuestionView(
    question: InteractionQuestion,
    enabled: Boolean = true,
    selected: MutableList<String>,
    freeformValue: String,
    onSelectionChange: (MutableList<String>) -> Unit,
    onFreeformChange: (String) -> Unit
) {
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Text(
            text = question.question,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onTertiaryContainer
        )
        if (question.options.isNotEmpty()) {
            FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                question.options.forEach { option ->
                    val isSelected = selected.contains(option.id)
                    FilterChip(
                        selected = isSelected,
                        enabled = enabled,
                        onClick = {
                            val next = selected.toMutableList()
                            if (question.multiSelect) {
                                if (isSelected) next.remove(option.id) else next.add(option.id)
                            } else {
                                next.clear()
                                next.add(option.id)
                            }
                            onSelectionChange(next)
                        },
                        label = { Text(option.label) }
                    )
                }
            }
        }
        if (question.freeform || question.options.isEmpty()) {
            OutlinedTextField(
                value = freeformValue,
                enabled = enabled,
                onValueChange = onFreeformChange,
                label = { Text(uiString(R.string.ui_answer)) },
                modifier = Modifier
                    .fillMaxWidth()
                    .testTag("agent-answer-${question.id}")
            )
        }
    }
}

/** Localized labels for protocol option ids; unknown ids keep the CLI text. */
internal fun optionLabel(option: InteractionOption): String = when (option.id) {
    "accept" -> uiString(R.string.ui_allow)
    "acceptForSession" -> uiString(R.string.ui_allow_for_this_session)
    "decline" -> uiString(R.string.ui_decline)
    "cancel" -> uiString(R.string.ui_cancel_task)
    "once" -> uiString(R.string.ui_allow_once)
    "always" -> uiString(R.string.ui_always_allow)
    "reject" -> uiString(R.string.ui_decline)
    else -> option.label
}

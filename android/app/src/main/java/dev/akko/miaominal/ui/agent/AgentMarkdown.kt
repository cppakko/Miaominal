package dev.akko.miaominal.ui.agent

import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.TextLinkStyles
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.withLink
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp

/**
 * Minimal, dependency-free Markdown renderer for chat content.
 *
 * Supports the constructs agent replies actually use: fenced code blocks,
 * headings, bullet lists, paragraphs, inline code, bold and italic. Anything
 * else is rendered as plain text, so an unknown construct can never break the
 * conversation view.
 */
internal sealed interface MarkdownBlock {
    data class Paragraph(val text: String) : MarkdownBlock
    data class Heading(val level: Int, val text: String) : MarkdownBlock
    data class Bullet(val items: List<String>) : MarkdownBlock
    data class Code(val content: String, val language: String = "") : MarkdownBlock
    data class Ordered(val items: List<Pair<String, String>>) : MarkdownBlock
    data class Table(val header: List<String>, val rows: List<List<String>>) : MarkdownBlock
}

@Composable
fun AgentMarkdown(
    text: String,
    modifier: Modifier = Modifier
) {
    val blocks = remember(text) { parseMarkdown(text) }
    Column(modifier = modifier, verticalArrangement = Arrangement.spacedBy(6.dp)) {
        blocks.forEach { block ->
            when (block) {
                is MarkdownBlock.Heading -> Text(
                    text = inlineMarkdown(block.text),
                    style = if (block.level <= 2) {
                        MaterialTheme.typography.titleMedium
                    } else {
                        MaterialTheme.typography.titleSmall
                    },
                    color = MaterialTheme.colorScheme.onSurface
                )

                is MarkdownBlock.Paragraph -> Text(
                    text = inlineMarkdown(block.text),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurface
                )

                is MarkdownBlock.Bullet -> Column(
                    verticalArrangement = Arrangement.spacedBy(2.dp)
                ) {
                    block.items.forEach { item ->
                        Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                            Text(
                                text = "•",
                                style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant
                            )
                            Text(
                                text = inlineMarkdown(item),
                                style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.onSurface
                            )
                        }
                    }
                }

                is MarkdownBlock.Code -> AgentCodeBlock(block.content, block.language)
                is MarkdownBlock.Ordered -> Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    block.items.forEach { (number, value) ->
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            Text(number, style = MaterialTheme.typography.bodyMedium)
                            Text(inlineMarkdown(value), style = MaterialTheme.typography.bodyMedium)
                        }
                    }
                }
                is MarkdownBlock.Table -> Column(Modifier.horizontalScroll(rememberScrollState())) {
                    (listOf(block.header) + block.rows).forEachIndexed { index, row ->
                        Surface(color = if (index == 0) MaterialTheme.colorScheme.surfaceContainerHigh else MaterialTheme.colorScheme.surface) {
                            Row {
                                block.header.indices.forEach { column ->
                                    Text(inlineMarkdown(row.getOrElse(column) { "" }),
                                        style = MaterialTheme.typography.bodyMedium,
                                        fontWeight = if (index == 0) FontWeight.SemiBold else FontWeight.Normal,
                                        modifier = Modifier.width(180.dp).padding(8.dp))
                                }
                            }
                        }
                        androidx.compose.material3.HorizontalDivider()
                    }
                }
            }
        }
    }
}

internal fun splitMarkdownCells(line: String): List<String> {
    val cells = mutableListOf<String>()
    val cell = StringBuilder()
    var code = false
    var escaped = false
    val trimmed = line.trim().removePrefix("|").removeSuffix("|")
    for ((position, char) in trimmed.withIndex()) {
        when {
            escaped -> { cell.append(char); escaped = false }
            char == '\\' && trimmed.getOrNull(position + 1) in listOf('|', '\\', '`') -> escaped = true
            char == '`' -> { code = !code; cell.append(char) }
            char == '|' && !code -> { cells += cell.toString().trim(); cell.clear() }
            else -> cell.append(char)
        }
    }
    if (escaped) cell.append('\\')
    cells += cell.toString().trim()
    return cells
}

internal fun parseMarkdown(text: String): List<MarkdownBlock> {
    val result = mutableListOf<MarkdownBlock>()
    val lines = text.lines()
    val ordered = Regex("^(\\d+[.)])\\s+(.+)$")
    val tableDivider = Regex("^:?-{3,}:?$")
    var index = 0
    while (index < lines.size) {
        val line = lines[index].trim()
        when {
            line.isBlank() -> index++
            line.startsWith("```") -> {
                val language = line.removePrefix("```").trim()
                val content = mutableListOf<String>()
                index++
                while (index < lines.size && !lines[index].trimStart().startsWith("```")) content += lines[index++]
                if (index < lines.size) index++
                result += MarkdownBlock.Code(content.joinToString("\n"), language)
            }
            line.startsWith("#") && line.takeWhile { it == '#' }.length in 1..6 &&
                line.dropWhile { it == '#' }.startsWith(" ") -> {
                val level = line.takeWhile { it == '#' }.length
                result += MarkdownBlock.Heading(level, line.drop(level).trim())
                index++
            }
            index + 1 < lines.size && line.contains('|') &&
                splitMarkdownCells(lines[index + 1]).let { it.isNotEmpty() && it.all(tableDivider::matches) } -> {
                val header = splitMarkdownCells(line)
                index += 2
                val rows = mutableListOf<List<String>>()
                while (index < lines.size && lines[index].contains('|') && lines[index].isNotBlank()) {
                    rows += splitMarkdownCells(lines[index++])
                }
                result += MarkdownBlock.Table(header, rows)
            }
            ordered.matches(line) -> {
                val entries = mutableListOf<Pair<String, String>>()
                while (index < lines.size) {
                    val match = ordered.matchEntire(lines[index].trim()) ?: break
                    entries += match.groupValues[1] to match.groupValues[2]
                    index++
                }
                result += MarkdownBlock.Ordered(entries)
            }
            line.startsWith("- ") || line.startsWith("* ") -> {
                val entries = mutableListOf<String>()
                while (index < lines.size && (lines[index].trim().startsWith("- ") || lines[index].trim().startsWith("* "))) {
                    entries += lines[index++].trim().drop(2)
                }
                result += MarkdownBlock.Bullet(entries)
            }
            else -> {
                // Preserve hard line boundaries; streaming partial markup stays readable.
                result += MarkdownBlock.Paragraph(line.removePrefix("> "))
                index++
            }
        }
    }
    return result
}

/** Nested emphasis/code and safe web links. Unknown/incomplete markup stays literal. */
internal fun inlineMarkdown(text: String): AnnotatedString = buildAnnotatedString {
    var index = 0
    while (index < text.length) {
        val remaining = text.substring(index)
        val link = if (remaining.startsWith("[")) Regex("^\\[([^\\]]+)\\]\\((https?://[^\\s)]+)\\)").find(remaining) else null
        when {
            text[index] == '`' -> {
                val end = text.indexOf('`', index + 1)
                if (end > index) {
                    withStyle(SpanStyle(fontFamily = FontFamily.Monospace,
                        background = androidx.compose.ui.graphics.Color(0x22808080))) {
                        append(text.substring(index + 1, end))
                    }
                    index = end + 1
                } else append(text[index++])
            }
            link != null -> {
                withLink(LinkAnnotation.Url(link.groupValues[2],
                    TextLinkStyles(SpanStyle(textDecoration = TextDecoration.Underline)))) { append(link.groupValues[1]) }
                index += link.value.length
            }
            text.startsWith("**", index) -> {
                val end = text.indexOf("**", index + 2)
                if (end > index + 2) {
                    withStyle(SpanStyle(fontWeight = FontWeight.SemiBold)) {
                        append(inlineMarkdown(text.substring(index + 2, end)))
                    }
                    index = end + 2
                } else append(text[index++])
            }
            text[index] == '*' -> {
                val end = text.indexOf('*', index + 1)
                if (end > index + 1) {
                    withStyle(SpanStyle(fontStyle = FontStyle.Italic)) {
                        append(inlineMarkdown(text.substring(index + 1, end)))
                    }
                    index = end + 1
                } else append(text[index++])
            }
            else -> append(text[index++])
        }
    }
}

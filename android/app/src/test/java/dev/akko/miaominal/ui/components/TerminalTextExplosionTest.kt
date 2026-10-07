package dev.akko.miaominal.ui.components

import org.junit.Assert.assertEquals
import org.junit.Test

class TerminalTextExplosionTest {
    @Test
    fun tokenizesShellCommandAndQuotedArgument() {
        val source = "docker ps --format \"{{.Names}} {{.Status}}\" | grep web"

        val tokens = tokenizeTerminalText(source)

        assertEquals(
            listOf(
                "docker",
                "ps",
                "--format",
                "\"{{.Names}} {{.Status}}\"",
                "|",
                "grep",
                "web"
            ),
            tokens.map { it.text }
        )
    }

    @Test
    fun keepsPathsWholeAndSeparatesRedirection() {
        val source = "cat /var/log/nginx/error.log 2>/tmp/errors.txt"

        val tokens = tokenizeTerminalText(source)

        assertEquals(
            listOf("cat", "/var/log/nginx/error.log", "2", ">", "/tmp/errors.txt"),
            tokens.map { it.text }
        )
    }

    @Test
    fun preservesOriginalSpacingForContiguousSelection() {
        val source = "git   status --short"
        val tokens = tokenizeTerminalText(source)

        assertEquals(
            "git   status",
            selectedTerminalText(source, tokens, setOf(0, 1))
        )
        assertEquals(
            "git --short",
            selectedTerminalText(source, tokens, setOf(0, 2))
        )
    }

    @Test
    fun returnsEmptyTextWithoutSelection() {
        val source = "pwd"
        val tokens = tokenizeTerminalText(source)

        assertEquals("", selectedTerminalText(source, tokens, emptySet()))
    }
}

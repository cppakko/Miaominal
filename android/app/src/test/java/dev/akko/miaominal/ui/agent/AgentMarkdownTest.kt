package dev.akko.miaominal.ui.agent

import org.junit.Assert.*
import org.junit.Test

class AgentMarkdownTest {
    @Test fun incompleteStreamingCodePreservesLinesAndLanguage() {
        val result = parseMarkdown("```kotlin\nval x = 1\nprintln(x)")
        assertEquals(MarkdownBlock.Code("val x = 1\nprintln(x)", "kotlin"), result.single())
    }

    @Test fun tableKeepsPipesInsideCodeAndEscapedPipes() {
        val table = parseMarkdown("| Name | Value |\n| --- | :---: |\n| `a|b` | x\\|y |").single() as MarkdownBlock.Table
        assertEquals(listOf("Name", "Value"), table.header)
        assertEquals(listOf("`a|b`", "x|y"), table.rows.single())
    }

    @Test fun numberedListKeepsOriginalNumbers() {
        assertEquals(MarkdownBlock.Ordered(listOf("3." to "three", "4." to "four")),
            parseMarkdown("3. three\n4. four").single())
    }

    @Test fun windowsPathsInsideTablesKeepTheirBackslashes() {
        assertEquals(listOf("file", "C:\\Users\\project"), splitMarkdownCells("| file | C:\\Users\\project |"))
    }

    @Test fun inlineCodeInsideBoldDoesNotLeaveLiteralBackticks() {
        assertEquals("Use runFullTrust here", inlineMarkdown("**Use `runFullTrust` here**").text)
    }

    @Test fun webLinksAreAnnotatedButOtherSchemesRemainLiteral() {
        val link = inlineMarkdown("[docs](https://example.com/docs)")
        assertEquals("docs", link.text)
        assertEquals(1, link.getLinkAnnotations(0, link.length).size)
        assertEquals("[bad](javascript:alert)", inlineMarkdown("[bad](javascript:alert)").text)
    }
}

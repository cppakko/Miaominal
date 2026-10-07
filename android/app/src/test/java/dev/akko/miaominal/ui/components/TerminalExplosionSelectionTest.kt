package dev.akko.miaominal.ui.components

import androidx.compose.ui.state.ToggleableState
import org.junit.Assert.*
import org.junit.Test

class TerminalExplosionSelectionTest {
    @Test fun aMixedDragStartingUnselectedTogglesEachWordOnceAndUndoesTogether() {
        val document = TerminalExplosionSelection("one two three four")
        document.setTokenSelected(1, true)
        document.setTokenSelected(3, true)
        val drag = TerminalExplosionDragSelection(document)
        drag.visit(0)
        drag.visit(3)
        assertEquals("one three", document.resultText())
        val version = document.version
        repeat(30) { drag.visit(3) }
        drag.visit(0)
        assertEquals(version, document.version)
        assertEquals("one three", document.resultText())
        document.undo()
        assertEquals("two four", document.resultText())
        document.undo()
        assertEquals("two", document.resultText())
    }

    @Test fun aMixedDragStartingSelectedAlsoSelectsUnselectedWordsInBothDirections() {
        val document = TerminalExplosionSelection("one two three four")
        document.setTokenSelected(0, true)
        document.setTokenSelected(2, true)
        val drag = TerminalExplosionDragSelection(document)
        drag.visit(2)
        drag.visit(0)
        assertEquals("two", document.resultText())
        drag.visit(3)
        assertEquals("two four", document.resultText())
        assertEquals(2, document.selectedTokenCount)
        document.undo()
        assertEquals("one three", document.resultText())
        document.undo()
        assertEquals("one", document.resultText())
    }

    @Test fun invalidDragVisitsDoNotCreateHistoryAndANewDragCanToggleTheSameWordAgain() {
        val document = TerminalExplosionSelection("one two")
        val first = TerminalExplosionDragSelection(document)
        first.visit(-1)
        first.visit(document.tokens.size)
        assertTrue(document.isEmpty)
        assertFalse(document.canUndo)
        first.visit(0)
        assertEquals("one", document.resultText())
        val second = TerminalExplosionDragSelection(document)
        second.visit(0)
        assertTrue(document.isEmpty)
        document.undo()
        assertEquals("one", document.resultText())
        document.undo()
        assertTrue(document.isEmpty)
        assertFalse(document.canUndo)
    }

    @Test fun selectingAllPreservesIndentationNewlinesAndTrailingWhitespace() {
        val source = "  echo   hello\n\tworld  \n"
        val document = TerminalExplosionSelection(source)
        assertTrue(document.isEmpty)
        assertFalse(document.allSelected)
        assertFalse(document.canUndo)
        assertEquals(0, document.selectedTokenCount)
        assertEquals("", document.resultText())
        document.tokens.indices.forEach { assertEquals(ToggleableState.Off, document.tokenState(it)) }
        document.selectAll()
        assertEquals(source, document.resultText())
        assertTrue(document.allSelected)
        assertEquals(3, document.selectedTokenCount)
        document.clear()
        assertTrue(document.isEmpty)
        assertEquals("", document.resultText())
        document.undo()
        assertEquals(source, document.resultText())
    }

    @Test fun selectingAQuotedWordKeepsItsInternalWhitespaceAndWholeGraphemes() {
        val source = "echo  'a  b👩🏽‍💻 e\u0301' | cat"
        val document = TerminalExplosionSelection(source)
        assertEquals(listOf("echo", "'a  b👩🏽‍💻 e\u0301'", "|", "cat"), document.tokens.map { it.text })
        document.setTokenSelected(1, true)
        assertEquals("'a  b👩🏽‍💻 e\u0301'", document.resultText())
        assertEquals(1, document.selectedTokenCount)
        assertEquals(ToggleableState.On, document.tokenState(1))
        assertEquals(ToggleableState.Off, document.tokenState(0))
        document.setTokenSelected(0, true)
        assertEquals("echo  'a  b👩🏽‍💻 e\u0301'", document.resultText())
        document.undo()
        assertEquals("'a  b👩🏽‍💻 e\u0301'", document.resultText())
    }

    @Test fun disjointSelectionsFollowSourceOrderRatherThanTapOrder() {
        val document = TerminalExplosionSelection("git   status --short\nnext")
        document.setTokenSelected(2, true)
        document.setTokenSelected(0, true)
        assertEquals("git --short", document.resultText())
        document.setTokenSelected(1, true)
        assertEquals("git   status --short", document.resultText())
        document.setTokenSelected(3, true)
        assertEquals("git   status --short\nnext", document.resultText())
        document.setTokenSelected(0, false)
        assertEquals("status --short\nnext", document.resultText())
    }

    @Test fun aWholeDragIsOneUndoStepAndRepeatingAVisitedTokenDoesNotToggleItAgain() {
        val document = TerminalExplosionSelection("one two three")
        document.checkpoint()
        document.setTokenSelected(0, true, record = false)
        document.setTokenSelected(1, true, record = false)
        document.setTokenSelected(0, true, record = false)
        assertEquals("one two", document.resultText())
        document.undo()
        assertEquals("", document.resultText())
        assertFalse(document.canUndo)
    }

    @Test fun naturalLanguageWordBreaksNeverCreateFragmentsOfAnEmoji() {
        val source = "你好👩🏽‍💻世界"
        val document = TerminalExplosionSelection(source)
        assertEquals(source, document.tokens.joinToString("") { it.text })
        assertTrue(document.tokens.any { "👩🏽‍💻" in it.text })
        val index = document.tokens.indexOfFirst { "👩🏽‍💻" in it.text }
        document.setTokenSelected(index, true)
        assertEquals(document.tokens[index].text, document.resultText())
        assertEquals(ToggleableState.On, document.tokenState(index))
        document.undo()
        assertTrue(document.isEmpty)
        document.selectAll()
        document.tokens.indices.forEach { assertEquals(ToggleableState.On, document.tokenState(it)) }
        assertEquals(source, document.resultText())
        assertEquals(listOf("e\u0301", "👩🏽‍💻", "🇨🇳"),
            terminalCharacterTokens("e\u0301 👩🏽‍💻 🇨🇳").map { it.text })
    }

    @Test fun identicalWordsCanBeSelectedIndependentlyAndNoOpEditsDoNotCreateHistory() {
        val document = TerminalExplosionSelection("echo  echo\necho")
        document.setTokenSelected(1, true)
        assertEquals(ToggleableState.Off, document.tokenState(0))
        assertEquals(ToggleableState.On, document.tokenState(1))
        assertEquals(ToggleableState.Off, document.tokenState(2))
        assertEquals("echo", document.resultText())
        val version = document.version
        document.setTokenSelected(1, true)
        document.setTokenSelected(-1, true)
        document.setTokenSelected(document.tokens.size, true)
        assertEquals(version, document.version)
        document.undo()
        assertTrue(document.isEmpty)
        assertFalse(document.canUndo)
    }

    @Test fun undoKeepsOnlyTheLastTwentyWordSelectionEdits() {
        val document = TerminalExplosionSelection((1..30).joinToString(" ") { "word$it" })
        document.tokens.indices.forEach { document.setTokenSelected(it, true) }
        repeat(20) { document.undo() }
        assertEquals(10, document.selectedTokenCount)
        assertEquals((1..10).joinToString(" ") { "word$it" }, document.resultText())
        assertFalse(document.canUndo)
        val version = document.version
        document.undo()
        assertEquals(version, document.version)
        document.tokens.indices.forEach {
            assertEquals(if (it < 10) ToggleableState.On else ToggleableState.Off, document.tokenState(it))
        }
    }

    @Test fun largeSelectionsRemainReversibleAndWhitespaceOnlyInputHasNoActions() {
        val document = TerminalExplosionSelection((1..5000).joinToString(" ") { "word$it" })
        assertEquals(0, document.selectedTokenCount)
        document.selectAll()
        assertEquals(5000, document.selectedTokenCount)
        document.setTokenSelected(2500, false)
        assertEquals(4999, document.selectedTokenCount)
        document.undo()
        assertTrue(document.allSelected)
        val empty = TerminalExplosionSelection(" \t\n\u00a0\u2007")
        assertTrue(empty.tokens.isEmpty())
        assertTrue(empty.isEmpty)
        assertFalse(empty.allSelected)
        assertEquals("", empty.resultText())
    }
}

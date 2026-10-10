package dev.akko.miaominal.ui.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class LicenseDocumentTest {
    @Test
    fun longUnicodeLinesRemainCompleteAndHaveBoundedLayoutWork() {
        val source = "Copyright © 汉字🙂 <owner> & contributors ".repeat(20_000)
        val blocks = licenseTextBlocks(source)
        assertEquals(source, blocks.joinToString(""))
        assertTrue(blocks.size > 1)
        blocks.forEach { block ->
            assertTrue(block.length in 1..2048)
            assertFalse(block.first().isLowSurrogate())
            assertFalse(block.last().isHighSurrogate())
        }
    }

    @Test
    fun originalLineBreaksAndBlankLinesArePreserved() {
        val source = "License\r\nCopyright\r\n\r\nTerms\n\n".repeat(5_000)
        val blocks = licenseTextBlocks(source)
        assertEquals(source, blocks.joinToString(""))
        assertTrue(blocks.all { it.length in 1..2048 })
        assertTrue(licenseTextBlocks("").isEmpty())
        assertEquals(listOf("Short license\n"), licenseTextBlocks("Short license\n"))
    }
}

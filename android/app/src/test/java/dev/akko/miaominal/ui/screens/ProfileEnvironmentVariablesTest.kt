package dev.akko.miaominal.ui.screens

import dev.akko.miaominal.data.SessionEnvironmentVariable
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class ProfileEnvironmentVariablesTest {
    @Test
    fun trimsNamesKeepsValuesAndDropsEmptyRows() {
        val result = normalizeEnvironmentVariables(
            listOf(
                SessionEnvironmentVariable(" APP_ENV ", "staging environment"),
                SessionEnvironmentVariable("", "")
            )
        ).getOrThrow()

        assertEquals(
            listOf(SessionEnvironmentVariable("APP_ENV", "staging environment")),
            result
        )
    }

    @Test
    fun rejectsMissingAndInvalidNames() {
        assertTrue(
            normalizeEnvironmentVariables(
                listOf(SessionEnvironmentVariable("", "has-value"))
            ).isFailure
        )
        assertTrue(
            normalizeEnvironmentVariables(
                listOf(SessionEnvironmentVariable("BAD-NAME", "value"))
            ).isFailure
        )
    }
}

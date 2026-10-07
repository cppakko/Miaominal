package dev.akko.miaominal.ui

import androidx.compose.runtime.staticCompositionLocalOf
import dev.akko.miaominal.data.MiaominalRepository

/**
 * Repository provided at the application root. Screens read data and issue
 * writes through it; tests and Previews provide an in-memory implementation.
 */
val LocalMiaominalRepository = staticCompositionLocalOf<MiaominalRepository> {
    error("MiaominalRepository was not provided")
}

val LocalAgentConfigRepository = staticCompositionLocalOf<dev.akko.miaominal.data.agent.AgentConfigRepository?> { null }

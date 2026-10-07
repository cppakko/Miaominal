package dev.akko.miaominal.ui

import android.content.Context
import android.content.res.Configuration
import androidx.annotation.StringRes
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue

/** Resource access for callbacks and error formatters outside composition. */
internal object UiStrings {
    private lateinit var applicationContext: Context
    private var localizedContext by mutableStateOf<Context?>(null)

    fun initialize(context: Context) {
        applicationContext = context.applicationContext
        updateConfiguration(context)
    }

    fun updateConfiguration(context: Context) {
        // Use an application-owned context so the cached resources never retain
        // an Activity. Snapshot state also refreshes UI callers of uiString().
        localizedContext = applicationContext.createConfigurationContext(
            Configuration(context.resources.configuration)
        )
    }

    fun get(@StringRes id: Int, vararg args: Any): String {
        val context = localizedContext ?: applicationContext
        return if (args.isEmpty()) context.getString(id) else context.getString(id, *args)
    }
}

internal fun uiString(@StringRes id: Int, vararg args: Any): String = UiStrings.get(id, *args)

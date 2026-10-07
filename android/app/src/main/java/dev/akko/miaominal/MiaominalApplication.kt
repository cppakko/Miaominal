package dev.akko.miaominal

import android.app.Application
import dev.akko.miaominal.data.AppContainer

/**
 * Application-scoped startup: the native backend client and the repository are
 * created once per process, before (and independently of) any Activity.
 */
class MiaominalApplication : Application() {
    lateinit var container: AppContainer
        private set

    override fun onCreate() {
        super.onCreate()
        dev.akko.miaominal.ui.UiStrings.initialize(this)
        container = AppContainer(this)
        container.start()
    }
}

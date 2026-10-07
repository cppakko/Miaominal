package dev.akko.miaominal.ui.model

import android.content.SharedPreferences
import androidx.lifecycle.ViewModelStore
import dev.akko.miaominal.data.*
import dev.akko.miaominal.ui.theme.ThemeMode
import kotlinx.coroutines.*
import kotlinx.coroutines.test.*
import org.junit.*
import org.junit.Assert.*

@OptIn(ExperimentalCoroutinesApi::class)
class AppearanceOperationsTest {
    private val store = ViewModelStore()
    @Before fun setup() { Dispatchers.setMain(UnconfinedTestDispatcher()) }
    @After fun cleanup() { store.clear(); Dispatchers.resetMain() }
    private fun model(preferences: TestPreferences) = AppearanceOperations(preferences).also { store.put("appearance", it) }
    private suspend fun settle(model: AppearanceOperations) = withTimeout(3000) { while (model.busy) delay(5) }

    @Test fun backendFailureRetainsConfirmedThemeAndDoesNotPersistTheRequestedChoice() = runBlocking {
        val preferences = TestPreferences()
        val base = InMemoryMiaominalRepository()
        val repository = object : MiaominalRepository by base {
            override suspend fun updateSettings(transform: (BusinessSettings) -> BusinessSettings): Result<Unit> = Result.failure(IllegalStateException("backend save failed"))
        }
        val model = model(preferences)
        model.update(repository, false, nextMode = ThemeMode.Dark); settle(model)
        assertEquals(ThemeMode.Light, model.mode); assertNotNull(model.error)
        assertEquals("Light", preferences.getString("theme_mode", null))
    }

    @Test fun diskFailureRestoresPreferenceMemoryAndBackendThemeBeforeRetry() = runBlocking {
        val preferences = TestPreferences(); preferences.failNext = true
        val base = InMemoryMiaominalRepository()
        val before = base.settings.value
        val model = model(preferences)
        model.update(base, false, nextMode = ThemeMode.Dark); settle(model)
        assertEquals(ThemeMode.Light, model.mode); assertEquals("Light", preferences.getString("theme_mode", null))
        assertEquals(before.themeId, base.settings.value.themeId); assertEquals(before.seedColor, base.settings.value.seedColor)
        assertNotNull(model.error)
        model.update(base, false, nextMode = ThemeMode.Dark); settle(model)
        assertEquals(ThemeMode.Dark, model.mode); assertEquals("dark", base.settings.value.themeId); assertNull(model.error)
    }

    @Test fun dynamicColorSwitchRetainsTheCustomSeedAndDuplicateWritesAreGuarded() = runBlocking {
        val preferences = TestPreferences()
        val base = InMemoryMiaominalRepository()
        val model = model(preferences)
        model.update(base, false, nextSeed = 0xFF123456.toInt()); settle(model)
        val gate = CompletableDeferred<Result<Unit>>()
        var calls = 0
        val repository = object : MiaominalRepository by base {
            override suspend fun updateSettings(transform: (BusinessSettings) -> BusinessSettings): Result<Unit> { calls++; return gate.await() }
        }
        model.update(repository, false, nextMode = ThemeMode.Dark)
        model.update(repository, false, nextMode = ThemeMode.Dark)
        assertEquals(1, calls)
        gate.complete(Result.failure(IllegalStateException("save failed"))); settle(model)
        model.update(base, false, nextDynamic = true); settle(model)
        model.update(base, false, nextDynamic = false); settle(model)
        assertEquals(0xFF123456.toInt(), model.seed)
        assertEquals(0xFF123456.toInt(), preferences.getInt("seed_color", 0))
    }
}

/** Models Android's in-memory preference mutation even when a disk commit fails. */
private class TestPreferences : SharedPreferences {
    private val data = mutableMapOf<String, Any?>("theme_mode" to "Light")
    var failNext = false
    override fun getAll(): MutableMap<String, *> = data
    override fun contains(key: String?) = data.containsKey(key)
    override fun getString(key: String?, defValue: String?) = data[key] as? String ?: defValue
    override fun getStringSet(key: String?, defValues: MutableSet<String>?) = defValues
    override fun getInt(key: String?, defValue: Int) = data[key] as? Int ?: defValue
    override fun getLong(key: String?, defValue: Long) = data[key] as? Long ?: defValue
    override fun getFloat(key: String?, defValue: Float) = data[key] as? Float ?: defValue
    override fun getBoolean(key: String?, defValue: Boolean) = data[key] as? Boolean ?: defValue
    override fun registerOnSharedPreferenceChangeListener(listener: SharedPreferences.OnSharedPreferenceChangeListener?) = Unit
    override fun unregisterOnSharedPreferenceChangeListener(listener: SharedPreferences.OnSharedPreferenceChangeListener?) = Unit
    override fun edit() = object : SharedPreferences.Editor {
        private val pending = mutableMapOf<String, Any?>()
        private fun put(key: String?, value: Any?): SharedPreferences.Editor { pending[key!!] = value; return this }
        override fun putString(key: String?, value: String?) = put(key, value)
        override fun putStringSet(key: String?, values: MutableSet<String>?) = put(key, values)
        override fun putInt(key: String?, value: Int) = put(key, value)
        override fun putLong(key: String?, value: Long) = put(key, value)
        override fun putFloat(key: String?, value: Float) = put(key, value)
        override fun putBoolean(key: String?, value: Boolean) = put(key, value)
        override fun remove(key: String?): SharedPreferences.Editor { pending[key!!] = null; return this }
        override fun clear(): SharedPreferences.Editor { data.clear(); return this }
        override fun commit(): Boolean { data.putAll(pending); val success = !failNext; failNext = false; return success }
        override fun apply() { commit() }
    }
}

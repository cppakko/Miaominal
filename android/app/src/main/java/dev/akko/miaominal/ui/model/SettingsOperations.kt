package dev.akko.miaominal.ui.model

import android.content.ContentResolver
import android.net.Uri
import androidx.compose.runtime.*
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dev.akko.miaominal.data.*
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

enum class VaultForm { Enable, Unlock, Change }
enum class PasswordError { CurrentRequired, PasswordRequired, ConfirmationRequired, Mismatch }

fun validateVaultPassword(form: VaultForm, current: String, password: String, confirmation: String): PasswordError? = when {
    form == VaultForm.Change && current.isBlank() -> PasswordError.CurrentRequired
    password.isBlank() -> PasswordError.PasswordRequired
    form != VaultForm.Unlock && confirmation.isBlank() -> PasswordError.ConfirmationRequired
    form != VaultForm.Unlock && password != confirmation -> PasswordError.Mismatch
    else -> null
}

fun terminalSettingsAnchor(anchor: String?): String? = when (anchor) {
    "appearance-typography" -> "terminal-typography"
    else -> anchor
}

data class RemovedTerminalKey(val value: String, val index: Int)
fun reorderTerminalKey(values: List<String>, value: String, target: Int): List<String> {
    val from = values.indexOf(value)
    if (from < 0 || values.isEmpty()) return values
    val to = target.coerceIn(values.indices)
    if (from == to) return values
    return values.toMutableList().apply { add(to, removeAt(from)) }
}
fun restoreTerminalKey(values: List<String>, removed: RemovedTerminalKey): List<String> =
    if (removed.value in values) values else values.toMutableList().apply {
        add(removed.index.coerceIn(0, size), removed.value)
    }

/** No SavedStateHandle: secrets, URIs and pending work never enter a Bundle. */
class SettingsOperations : ViewModel() {
    var typographyPicker by mutableStateOf<String?>(null)
    var typographyBusy by mutableStateOf(false); private set
    var typographyError by mutableStateOf<Throwable?>(null); private set
    private val typographyWrites = Mutex()
    fun applyFont(repository: MiaominalRepository, kind: String, selected: String) {
        if (typographyBusy) return
        typographyBusy = true; typographyError = null
        viewModelScope.launch {
            try {
                typographyWrites.withLock { repository.updateSettings {
                    if (kind == "primary") it.copy(terminalFont = selected) else it.copy(cjkFallbackFont = selected)
                }.onSuccess { typographyPicker = null }.onFailure { typographyError = it } }
            } catch (failure: Exception) { typographyError = failure }
            finally { typographyBusy = false }
        }
    }
    fun saveFontMetric(repository: MiaominalRepository, size: Float? = null, height: Float? = null) {
        typographyError = null
        viewModelScope.launch {
            try { typographyWrites.withLock { repository.updateSettings {
                it.copy(terminalFontSize = size ?: it.terminalFontSize, terminalLineHeight = height ?: it.terminalLineHeight)
            }.onFailure { typographyError = it } } }
            catch (failure: Exception) { typographyError = failure }
        }
    }
    var vaultForm by mutableStateOf<VaultForm?>(null); private set
    var currentPassword by mutableStateOf("")
    var password by mutableStateOf("")
    var confirmation by mutableStateOf("")
    var passwordError by mutableStateOf<PasswordError?>(null); private set
    var vaultError by mutableStateOf<Throwable?>(null); private set
    var vaultBusy by mutableStateOf(false); private set
    var vaultCompleted by mutableIntStateOf(0); private set
    val passwordDirty get() = currentPassword.isNotEmpty() || password.isNotEmpty() || confirmation.isNotEmpty()
    fun updatePassword(field: Int, value: String) {
        when (field) { 0 -> currentPassword = value; 1 -> password = value; else -> confirmation = value }
        passwordError = null
    }

    fun openVault(form: VaultForm) { clearVaultForm(); vaultForm = form; vaultError = null }
    fun clearVaultForm() {
        vaultForm = null; currentPassword = ""; password = ""; confirmation = ""; passwordError = null
    }
    fun submitVault(repository: MiaominalRepository) {
        val form = vaultForm ?: return
        if (vaultBusy || repository.vault.value.busy) return
        passwordError = validateVaultPassword(form, currentPassword, password, confirmation)
        if (passwordError != null) return
        val old = currentPassword; val next = password
        runVault(repository) { when (form) {
            VaultForm.Enable -> repository.vaultEnable(next)
            VaultForm.Unlock -> repository.vaultUnlock(next)
            VaultForm.Change -> repository.vaultChangePassphrase(old, next)
        }.also { if (it.isSuccess) clearVaultForm() } }
    }
    fun lockVault(repository: MiaominalRepository) = runVault(repository) { repository.vaultLock() }
    fun disableVault(repository: MiaominalRepository) {
        if (repository.vault.value.mode == VaultMode.Unlocked) runVault(repository) { repository.vaultDisable() }
    }
    private fun runVault(repository: MiaominalRepository, operation: suspend () -> Result<Unit>) {
        if (vaultBusy || repository.vault.value.busy) return
        vaultBusy = true; vaultError = null
        viewModelScope.launch {
            try { vaultError = operation().exceptionOrNull(); if (vaultError == null) vaultCompleted++ }
            catch (error: Exception) { vaultError = error }
            finally { vaultBusy = false }
        }
    }

    var importOpen by mutableStateOf(false); private set
    var source by mutableStateOf(ImportSourceModel.OpenSsh)
    var uri: Uri? = null; private set
    var preview by mutableStateOf<ImportPreviewModel?>(null); private set
    var importError by mutableStateOf<Throwable?>(null); private set
    var importReading by mutableStateOf(false); private set
    var importCommitting by mutableStateOf(false); private set
    var previewValid by mutableStateOf(false); private set
    var importedCount by mutableStateOf<Int?>(null); private set
    private var request = 0L

    fun openImport(initialSource: ImportSourceModel) {
        source = initialSource; importOpen = true; importError = null; importedCount = null
    }
    fun chooseImportSource(repository: MiaominalRepository, selected: ImportSourceModel) {
        if (importReading || importCommitting) return
        source = selected
        viewModelScope.launch { try { repository.updateDevicePreferences { it.copy(importSource = selected.name) } }
            catch (failure: Exception) { importError = failure } }
    }
    override fun onCleared() {
        request++; clearVaultForm(); uri = null; preview = null
        super.onCleared()
    }
    fun closeImport(repository: MiaominalRepository) {
        if (importCommitting) return
        request++; importOpen = false; importReading = false
        val id = preview?.previewId?.takeIf { previewValid }
        preview = null; previewValid = false; uri = null; importError = null; importedCount = null
        if (id != null) viewModelScope.launch { repository.discardImport(id) }
    }
    fun readImport(repository: MiaominalRepository, resolver: ContentResolver, selected: Uri) {
        if (!importOpen || importCommitting || importReading) return
        uri = selected
        val selectedSource = source
        loadImport(repository) {
            withContext(Dispatchers.IO) { runCatching {
                val bytes = resolver.openInputStream(selected)?.use(ImportDocuments::readAtMost)
                    ?: throw java.io.IOException("Unable to read file")
                repository.previewImport(selectedSource,
                    selected.lastPathSegment?.substringAfterLast('/') ?: "imported-config", bytes).getOrThrow()
            } }
        }
    }
    /** Generation check deliberately does not cancel native parsing: a late ID must be discarded. */
    internal fun loadImport(repository: MiaominalRepository, read: suspend () -> Result<ImportPreviewModel>) {
        if (!importOpen || importCommitting || importReading) return
        importReading = true; importError = null; importedCount = null
        val generation = ++request
        val previous = preview?.previewId?.takeIf { previewValid }
        previewValid = false
        viewModelScope.launch {
            if (previous != null) repository.discardImport(previous)
            val result = try { read() } catch (failure: Exception) { Result.failure(failure) }
            if (generation != request || !importOpen) {
                result.getOrNull()?.let { repository.discardImport(it.previewId) }
                return@launch
            }
            result.onSuccess { preview = it; previewValid = true }.onFailure { importError = it }
            importReading = false
        }
    }
    fun commitImport(repository: MiaominalRepository) {
        val snapshot = preview ?: return
        if (!previewValid || importReading || importCommitting || snapshot.entries.isEmpty()) return
        importCommitting = true; previewValid = false; importError = null
        viewModelScope.launch {
            try { repository.commitImport(snapshot.previewId)
                .onSuccess { importedCount = it; uri = null }
                .onFailure { importError = it }
            } catch (error: Exception) { importError = error }
            finally { importCommitting = false }
        }
    }
}

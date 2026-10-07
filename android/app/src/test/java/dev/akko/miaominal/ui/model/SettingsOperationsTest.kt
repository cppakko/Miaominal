package dev.akko.miaominal.ui.model

import androidx.lifecycle.ViewModelStore
import dev.akko.miaominal.data.*
import kotlinx.coroutines.*
import kotlinx.coroutines.test.*
import org.junit.*
import org.junit.Assert.*

@OptIn(ExperimentalCoroutinesApi::class)
class SettingsOperationsTest {
    private val store = ViewModelStore()
    private lateinit var operations: SettingsOperations
    @Before fun setup() {
        Dispatchers.setMain(StandardTestDispatcher())
        operations = SettingsOperations(); store.put("settings", operations)
    }
    @After fun cleanup() { store.clear(); Dispatchers.resetMain() }

    @Test fun passphraseValidationLocatesEmptyAndMismatchFieldsWithoutTrimmingSecrets() {
        assertEquals(PasswordError.CurrentRequired, validateVaultPassword(VaultForm.Change, "", "new", "new"))
        assertEquals(PasswordError.PasswordRequired, validateVaultPassword(VaultForm.Enable, "", "", ""))
        assertEquals(PasswordError.ConfirmationRequired, validateVaultPassword(VaultForm.Enable, "", "new", ""))
        assertEquals(PasswordError.Mismatch, validateVaultPassword(VaultForm.Enable, "", " pass ", "pass"))
        assertNull(validateVaultPassword(VaultForm.Enable, "", " pass ", " pass "))
        assertNull(validateVaultPassword(VaultForm.Unlock, "", " pass ", ""))
    }

    @Test fun failedVaultSubmissionKeepsSecretsAndBlocksDoubleSubmissionUntilRetrySucceeds() = runTest {
        val base = InMemoryMiaominalRepository()
        val gate = CompletableDeferred<Result<Unit>>()
        var calls = 0
        val repository = object : MiaominalRepository by base {
            override suspend fun vaultEnable(passphrase: String): Result<Unit> { calls++; return if (calls == 1) gate.await() else base.vaultEnable(passphrase) }
        }
        operations.openVault(VaultForm.Enable); operations.password = " pass "; operations.confirmation = " pass "
        operations.submitVault(repository); operations.submitVault(repository); runCurrent()
        assertEquals(1, calls); assertTrue(operations.vaultBusy)
        gate.complete(Result.failure(IllegalStateException("storage failed"))); runCurrent()
        assertEquals(" pass ", operations.password); assertEquals(" pass ", operations.confirmation)
        assertNotNull(operations.vaultError); assertFalse(operations.vaultBusy)
        operations.submitVault(repository); advanceUntilIdle()
        assertEquals(2, calls); assertEquals(VaultMode.Unlocked, base.vault.value.mode)
        assertNull(operations.vaultForm); assertFalse(operations.passwordDirty)
    }

    @Test fun disablingRequiresUnlockedVaultAndFailureRemainsOutsideThePasswordEditor() = runTest {
        val base = InMemoryMiaominalRepository(vault = VaultStatus(VaultMode.Locked, true))
        operations.disableVault(base); advanceUntilIdle()
        assertNull(operations.vaultError); assertEquals(VaultMode.Locked, base.vault.value.mode)
        base.vaultUnlock("preview-passphrase").getOrThrow()
        val failing = object : MiaominalRepository by base {
            override suspend fun vaultDisable(): Result<Unit> = Result.failure(IllegalStateException("migration failed"))
        }
        operations.disableVault(failing); advanceUntilIdle()
        assertNull(operations.vaultForm); assertEquals("migration failed", operations.vaultError?.message)
        assertEquals(VaultMode.Unlocked, base.vault.value.mode)
    }

    @Test fun changedPassphraseAndDisableKeepHostsInTheFake() = runTest {
        val host = SessionProfile(id = "test", name = "Test", host = "host", port = 22, username = "user")
        val repository = InMemoryMiaominalRepository(hosts = listOf(host))
        repository.vaultEnable("old").getOrThrow()
        operations.openVault(VaultForm.Change); operations.currentPassword = "old"
        operations.password = "new"; operations.confirmation = "new"
        operations.submitVault(repository); advanceUntilIdle()
        repository.vaultLock().getOrThrow()
        assertTrue(repository.vaultUnlock("old").isFailure)
        repository.vaultUnlock("new").getOrThrow()
        operations.disableVault(repository); advanceUntilIdle()
        assertEquals(listOf(host), repository.hosts.value); assertEquals(VaultMode.Disabled, repository.vault.value.mode)
    }

    @Test fun leavingDuringParsingDiscardsLatePreviewAndCannotOverwriteAReopenedFlow() = runTest {
        val base = InMemoryMiaominalRepository()
        val stale = base.previewImport(ImportSourceModel.OpenSsh, "old", byteArrayOf(1)).getOrThrow()
        val gate = CompletableDeferred<Result<ImportPreviewModel>>()
        val discarded = mutableListOf<Long>()
        val repository = object : MiaominalRepository by base {
            override suspend fun discardImport(previewId: Long) { discarded += previewId; base.discardImport(previewId) }
        }
        operations.openImport(ImportSourceModel.OpenSsh); operations.loadImport(repository) { gate.await() }; runCurrent()
        operations.closeImport(repository); operations.openImport(ImportSourceModel.Putty)
        operations.loadImport(repository) { base.previewImport(ImportSourceModel.Putty, "new", byteArrayOf(1)) }; runCurrent()
        val newId = operations.preview!!.previewId
        gate.complete(Result.success(stale)); advanceUntilIdle()
        assertEquals(newId, operations.preview!!.previewId); assertTrue(operations.previewValid)
        assertEquals(listOf(stale.previewId), discarded)
        assertTrue(base.commitImport(stale.previewId).isFailure)
    }

    @Test fun failedCommitConsumesIdAndNeedsNewPreviewBeforeExplicitCommit() = runTest {
        val base = InMemoryMiaominalRepository()
        val gate = CompletableDeferred<Unit>()
        val ids = mutableListOf<Long>()
        val repository = object : MiaominalRepository by base {
            override suspend fun commitImport(previewId: Long): Result<Int> {
                ids += previewId
                if (ids.size == 1) { gate.await(); base.discardImport(previewId); return Result.failure(IllegalStateException("save failed")) }
                return base.commitImport(previewId)
            }
        }
        operations.openImport(ImportSourceModel.OpenSsh)
        operations.loadImport(repository) { base.previewImport(ImportSourceModel.OpenSsh, "first", byteArrayOf(1)) }; runCurrent()
        val first = operations.preview!!
        operations.commitImport(repository); operations.commitImport(repository); operations.closeImport(repository); runCurrent()
        assertTrue(operations.importOpen); assertTrue(operations.importCommitting); assertEquals(1, ids.size)
        gate.complete(Unit); advanceUntilIdle()
        assertEquals(first, operations.preview); assertFalse(operations.previewValid)
        operations.commitImport(repository); runCurrent(); assertEquals(1, ids.size)
        operations.loadImport(repository) { base.previewImport(ImportSourceModel.OpenSsh, "second", byteArrayOf(1)) }; runCurrent()
        assertTrue(operations.previewValid); assertNull(operations.importedCount); assertEquals(1, ids.size)
        operations.commitImport(repository); advanceUntilIdle()
        assertNotEquals(ids[0], ids[1]); assertEquals(1, operations.importedCount)
    }

    @Test fun undoRestoresOnlyRemovedItemAndKeepsSubsequentReorderAndAdditions() {
        val removed = RemovedTerminalKey("TAB", 1)
        assertEquals(listOf("RIGHT", "TAB", "ESC", "LEFT"), restoreTerminalKey(listOf("RIGHT", "ESC", "LEFT"), removed))
        assertEquals(listOf("ESC", "TAB"), restoreTerminalKey(listOf("ESC", "TAB"), removed))
        assertEquals(listOf("ESC", "TAB"), restoreTerminalKey(listOf("ESC"), RemovedTerminalKey("TAB", 99)))
    }

    @Test fun dragAndMenuReorderingKeepAllKeysAndHandleEdgeAndMissingTargets() {
        val original = listOf("ESC", "TAB", "LEFT", "RIGHT")
        val moved = reorderTerminalKey(original, "ESC", 3)
        assertEquals(listOf("TAB", "LEFT", "RIGHT", "ESC"), moved)
        assertEquals(original.toSet(), moved.toSet())
        assertEquals(original, reorderTerminalKey(moved, "ESC", -1))
        assertEquals(original, reorderTerminalKey(original, "missing", 2))
        assertEquals(listOf("ESC"), reorderTerminalKey(listOf("ESC"), "ESC", 99))
        assertEquals(emptyList<String>(), reorderTerminalKey(emptyList(), "ESC", 0))
    }

    @Test fun fontApplyFailureKeepsPickerAndBlocksDuplicateWritesUntilSuccessfulRetry() = runTest {
        val base = InMemoryMiaominalRepository()
        val gate = CompletableDeferred<Result<Unit>>()
        var calls = 0
        val repository = object : MiaominalRepository by base {
            override suspend fun updateSettings(transform: (BusinessSettings) -> BusinessSettings): Result<Unit> {
                calls++; return if (calls == 1) gate.await() else base.updateSettings(transform)
            }
        }
        operations.typographyPicker = "primary"
        operations.applyFont(repository, "primary", "ChosenFont"); operations.applyFont(repository, "primary", "ChosenFont"); runCurrent()
        assertEquals(1, calls); assertTrue(operations.typographyBusy)
        gate.complete(Result.failure(IllegalStateException("font save failed"))); advanceUntilIdle()
        assertEquals("primary", operations.typographyPicker); assertNotNull(operations.typographyError)
        assertEquals("", base.settings.value.terminalFont)
        operations.applyFont(repository, "primary", "ChosenFont"); advanceUntilIdle()
        assertNull(operations.typographyPicker); assertNull(operations.typographyError)
        assertEquals("ChosenFont", base.settings.value.terminalFont)
    }

    @Test fun typographyWritesSerializeAndPreserveIndependentSettings() = runTest {
        val base = InMemoryMiaominalRepository()
        operations.applyFont(base, "primary", "ChosenFont")
        operations.saveFontMetric(base, size = 18.5f)
        operations.saveFontMetric(base, height = 26.5f)
        operations.chooseImportSource(base, ImportSourceModel.SecureCrt)
        advanceUntilIdle()
        assertEquals("ChosenFont", base.settings.value.terminalFont)
        assertEquals(18.5f, base.settings.value.terminalFontSize)
        assertEquals(26.5f, base.settings.value.terminalLineHeight)
        assertEquals("SecureCrt", base.devicePreferences.value.importSource)
    }

    @Test fun legacyTypographyAnchorRedirectsAndOtherAnchorsStayStable() {
        assertEquals("terminal-typography", terminalSettingsAnchor("appearance-typography"))
        assertEquals("terminal-quick", terminalSettingsAnchor("terminal-quick"))
        assertNull(terminalSettingsAnchor(null))
    }
}

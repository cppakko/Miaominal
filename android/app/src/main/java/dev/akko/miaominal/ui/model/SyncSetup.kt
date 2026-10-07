package dev.akko.miaominal.ui.model

import dev.akko.miaominal.data.*
import dev.akko.miaominal.R
import java.net.URI
import kotlinx.coroutines.CancellationException

enum class SyncSetupIssue { GistId, WebDavUrl, Credential }

/** Only non-secret fields should be placed in saved UI state. */
data class SyncConnectionDraft(
    val provider: SyncProviderModel,
    val gistId: String = "",
    val webdavUrl: String = "",
    val webdavUsername: String = "",
    val credential: String = "",
    val removeCredential: Boolean = false
) {
    fun firstIssue(config: SyncConfigModel): SyncSetupIssue? = when {
        provider == SyncProviderModel.Gist && normalizeGistId(gistId) == null -> SyncSetupIssue.GistId
        provider == SyncProviderModel.WebDav && !validWebDavUrl(webdavUrl) -> SyncSetupIssue.WebDavUrl
        provider != SyncProviderModel.None && !removeCredential && credential.isBlank() &&
            !(if (provider == SyncProviderModel.Gist) config.hasGithubToken else config.hasWebdavPassword) -> SyncSetupIssue.Credential
        else -> null
    }

    fun destinationChanged(config: SyncConfigModel) = provider != config.provider || when (provider) {
        SyncProviderModel.Gist -> normalizeGistId(gistId) != config.gistId
        SyncProviderModel.WebDav -> webdavUrl.trim() != config.webdavUrl || webdavUsername.trim() != config.webdavUsername
        SyncProviderModel.None -> false
    }
}

fun normalizeGistId(value: String): String? {
    val text = value.trim()
    if (text.isEmpty()) return "" // First upload creates the Gist.
    if (text.matches(Regex("[a-fA-F0-9]+"))) return text
    return runCatching {
        val uri = URI(text)
        if (!uri.host.equals("gist.github.com", ignoreCase = true) || uri.scheme != "https") return null
        val segments = uri.path.split('/').filter(String::isNotEmpty)
        val id = if (segments.size == 1) segments[0] else segments.getOrNull(1)
        id?.takeIf { it.matches(Regex("[a-fA-F0-9]+")) }
    }.getOrNull()
}

fun validWebDavUrl(value: String): Boolean = runCatching {
    val uri = URI(value.trim())
    (uri.scheme.equals("https", true) || (uri.scheme.equals("http", true) &&
        uri.host?.lowercase() in listOf("localhost", "127.0.0.1", "[::1]", "::1"))) && !uri.host.isNullOrBlank() &&
        uri.rawUserInfo == null && uri.fragment == null
}.getOrDefault(false)

fun syncConnectionReady(config: SyncConfigModel): Boolean = when (config.provider) {
    SyncProviderModel.None -> false
    SyncProviderModel.Gist -> config.hasGithubToken
    SyncProviderModel.WebDav -> validWebDavUrl(config.webdavUrl) && config.hasWebdavPassword
}

/** Disabled is also the backend's resting phase when only automatic sync is off. */
fun syncOverviewTitle(sync: SyncStateModel): Int? = when {
    !sync.loaded -> R.string.sync_mobile_loading
    sync.phase.isRunning -> null // The page displays the current operation/phase.
    sync.pendingConflict != null -> R.string.sync_mobile_conflict_title
    !syncConnectionReady(sync.config) || !sync.config.hasPassphrase -> R.string.sync_mobile_setup_title
    sync.lastFailure != null || sync.status is SyncStatusModel.Failed -> R.string.sync_mobile_last_failed
    sync.vaultLocked -> R.string.ui_paused_vault_locked
    sync.phase == SyncPhaseModel.Idle || sync.phase == SyncPhaseModel.Disabled -> when (sync.status) {
        is SyncStatusModel.Pushed -> R.string.sync_mobile_uploaded
        is SyncStatusModel.Pulled -> R.string.sync_mobile_downloaded
        is SyncStatusModel.UpToDate -> R.string.sync_mobile_up_to_date
        SyncStatusModel.Cancelled -> R.string.ui_sync_cancelled
        else -> R.string.sync_mobile_ready
    }
    else -> null
}

enum class SyncSaveStage { PauseAutomatic, Credential, Configuration }
class SyncSetupSaveException(val stage: SyncSaveStage, val credentialSaved: Boolean, cause: Throwable) :
    Exception(cause.message, cause)

/** Validate before writing. Never activate a new destination using the old credential. */
suspend fun saveSyncConnection(repository: MiaominalRepository, draft: SyncConnectionDraft): Result<Unit> {
    val config = repository.sync.value.config
    if (draft.firstIssue(config) != null) return Result.failure(IllegalArgumentException("Invalid sync setup"))
    val changed = draft.destinationChanged(config)
    var stage = SyncSaveStage.PauseAutomatic
    var credentialSaved = false
    return try {
        if ((changed || draft.removeCredential) && config.autoSyncEnabled) {
            repository.syncSetAutoSync(false).getOrThrow()
        }
        val kind = when (draft.provider) {
            SyncProviderModel.Gist -> SyncCredentialKindModel.GithubToken
            SyncProviderModel.WebDav -> SyncCredentialKindModel.WebdavPassword
            SyncProviderModel.None -> null
        }
        stage = SyncSaveStage.Credential
        if (kind != null && (draft.removeCredential || draft.credential.isNotBlank())) {
            repository.setSyncCredential(kind, if (draft.removeCredential) CredentialUpdate.Clear else {
                CredentialUpdate.Set(if (draft.provider == SyncProviderModel.Gist) draft.credential.trim() else draft.credential)
            }).getOrThrow()
            credentialSaved = true
        }
        stage = SyncSaveStage.Configuration
        repository.updateSyncConfig(SyncConfigEdit(
            provider = draft.provider,
            gistId = if (draft.provider == SyncProviderModel.Gist) normalizeGistId(draft.gistId) else null,
            webdavUrl = if (draft.provider == SyncProviderModel.WebDav) draft.webdavUrl.trim() else null,
            webdavUsername = if (draft.provider == SyncProviderModel.WebDav) draft.webdavUsername.trim() else null,
            autoSyncEnabled = if (changed || draft.removeCredential) false else null,
            webdavUnsafeWriteConsent = if (changed || draft.removeCredential) false else null
        )).getOrThrow()
        Result.success(Unit)
    } catch (error: CancellationException) {
        throw error
    } catch (error: Throwable) {
        Result.failure(SyncSetupSaveException(stage, credentialSaved, error))
    }
}

enum class SyncPassphraseIssue { Empty, Mismatch }
fun syncPassphraseIssue(value: String, confirmation: String): SyncPassphraseIssue? = when {
    value.isBlank() -> SyncPassphraseIssue.Empty
    value != confirmation -> SyncPassphraseIssue.Mismatch
    else -> null
}

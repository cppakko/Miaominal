package dev.akko.miaominal.data

import android.content.Context
import android.os.Build
import android.util.Log
import dev.akko.miaominal.nativebridge.BackendException
import dev.akko.miaominal.nativebridge.LogLevel
import dev.akko.miaominal.nativebridge.PlatformHost

/**
 * Rust-facing platform capabilities implemented by the Android application
 * process. The instance is application-scoped and shared by every backend
 * handle.
 */
class AndroidPlatformHost(context: Context) : PlatformHost {
    private val appContext = context.applicationContext
    val credentialStore = KeystoreCredentialStore(appContext)
    val safDocuments = SafDocuments(appContext)

    override fun `log`(level: LogLevel, message: String) {
        when (level) {
            LogLevel.DEBUG -> Log.d(TAG, message)
            LogLevel.INFO -> Log.i(TAG, message)
            LogLevel.WARN -> Log.w(TAG, message)
            LogLevel.ERROR -> Log.e(TAG, message)
        }
    }

    override fun `deviceLabel`(): String =
        "${Build.MANUFACTURER} ${Build.MODEL} (API ${Build.VERSION.SDK_INT})"

    override fun `probeSecret`(key: String): String? {
        val token = preferences().getString(key, null) ?: return null
        return SecretVault().decrypt(token)
    }

    override fun `credentialGet`(account: String): String? = try {
        credentialStore.get(account)
    } catch (error: CredentialKeyInvalidatedException) {
        throw BackendException.Platform(
            platformFailure = "credential key invalidated: ${error.message}"
        )
    } catch (error: CredentialStoreCorruptionException) {
        throw BackendException.Platform(
            platformFailure = "credential store corrupt: ${error.message}"
        )
    } catch (error: Throwable) {
        throw BackendException.Platform(
            platformFailure = "credential read failed: ${error.javaClass.simpleName}"
        )
    }

    override fun `credentialSet`(account: String, value: String) {
        try {
            credentialStore.set(account, value)
        } catch (error: CredentialStoreCorruptionException) {
            throw BackendException.Platform(
                platformFailure = "credential store corrupt: ${error.message}"
            )
        } catch (error: Throwable) {
            throw BackendException.Platform(
                platformFailure = "credential write failed: ${error.javaClass.simpleName}"
            )
        }
    }

    override fun `credentialDelete`(account: String) {
        try {
            credentialStore.delete(account)
        } catch (error: CredentialStoreCorruptionException) {
            throw BackendException.Platform(
                platformFailure = "credential store corrupt: ${error.message}"
            )
        } catch (error: Throwable) {
            throw BackendException.Platform(
                platformFailure = "credential delete failed: ${error.javaClass.simpleName}"
            )
        }
    }

    /** OSC 52 clipboard writes from a terminal session. */
    override fun `storeClipboard`(text: String) {
        val clipboard = appContext.getSystemService(Context.CLIPBOARD_SERVICE)
            as? android.content.ClipboardManager ?: return
        runCatching {
            clipboard.setPrimaryClip(android.content.ClipData.newPlainText("terminal", text))
        }
    }

    override fun `safList`(
        treeUri: String,
        relativePath: String
    ): List<dev.akko.miaominal.nativebridge.SafEntry> = safDocuments.list(treeUri, relativePath)

    override fun `safStat`(
        treeUri: String,
        relativePath: String
    ): dev.akko.miaominal.nativebridge.SafEntry = safDocuments.stat(treeUri, relativePath)

    override fun `safCreateDirectory`(treeUri: String, relativePath: String) {
        safDocuments.createDirectory(treeUri, relativePath)
    }

    override fun `safDelete`(treeUri: String, relativePath: String) {
        safDocuments.delete(treeUri, relativePath)
    }

    override fun `safOpenFile`(treeUri: String, relativePath: String): Int =
        safDocuments.openFile(treeUri, relativePath)

    override fun `safBeginWrite`(
        treeUri: String,
        relativePath: String
    ): dev.akko.miaominal.nativebridge.SafWriteStart =
        safDocuments.beginWrite(treeUri, relativePath)

    override fun `safFinishWrite`(
        treeUri: String,
        targetRelative: String,
        stagingRelative: String?,
        commit: Boolean
    ) {
        safDocuments.finishWrite(treeUri, targetRelative, stagingRelative, commit)
    }

    fun rememberProbeSecret(key: String, value: String) {
        val token = SecretVault().encrypt(value) ?: return
        preferences().edit().putString(key, token).apply()
    }

    private fun preferences() =
        appContext.getSharedPreferences(PROBE_STORE, Context.MODE_PRIVATE)

    companion object {
        const val TAG = "MiaominalNative"
        private const val PROBE_STORE = "native_bridge_probe"
    }
}

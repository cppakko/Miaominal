package dev.akko.miaominal.data

import android.content.Context
import java.io.File

/**
 * One-time switch to the P2 business data directory.
 *
 * The first P2 launch must start from a clean Rust root: the old development
 * data, the old JSON store and the old dedicated Keystore key are removed
 * before the new root is marked initialized. The marker makes the reset a
 * once-per-install event, so later launches (including upgrades with the same
 * signature) never repeat it.
 */
object FirstSwitchMigration {
    private const val CURRENT_DATA_DIR = "backend-v2"
    private const val LEGACY_DATA_DIR = "backend"
    private const val MARKER_FILE = ".p2-initialized"
    private const val MARKER_CONTENT = "miaominal-android/p2\n"
    private const val LEGACY_STORE_PREFERENCES = "miaominal-store"

    fun runIfNeeded(context: Context) {
        val appContext = context.applicationContext
        val root = File(appContext.noBackupFilesDir, CURRENT_DATA_DIR)
        // The marker must stay outside the Rust business root: any extra file
        // in that directory makes the Rust settings store treat a fresh
        // install as existing app data and skip the first-launch onboarding.
        val marker = File(appContext.noBackupFilesDir, MARKER_FILE)
        val legacyMarker = File(root, MARKER_FILE)
        if (marker.exists()) return
        if (legacyMarker.exists()) {
            marker.writeText(MARKER_CONTENT, Charsets.UTF_8)
            legacyMarker.delete()
            return
        }

        File(appContext.noBackupFilesDir, LEGACY_DATA_DIR).deleteRecursively()
        File(appContext.cacheDir, LEGACY_DATA_DIR).deleteRecursively()
        appContext.getSharedPreferences(LEGACY_STORE_PREFERENCES, Context.MODE_PRIVATE)
            .edit()
            .clear()
            .commit()
        // Old credentials were wrapped by the legacy Keystore alias; deleting
        // the key makes any leftover ciphertext permanently unreadable.
        SecretVault().deleteKey()

        root.mkdirs()
        marker.writeText(MARKER_CONTENT, Charsets.UTF_8)
    }
}

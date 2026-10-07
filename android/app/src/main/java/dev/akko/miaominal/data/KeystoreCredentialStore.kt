package dev.akko.miaominal.data

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.AtomicFile
import android.util.Base64
import org.json.JSONObject
import java.io.File
import java.security.KeyStore
import javax.crypto.AEADBadTagException
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/** Thrown when the Android Keystore key can no longer decrypt stored values. */
class CredentialKeyInvalidatedException(message: String, cause: Throwable? = null) :
    IllegalStateException(message, cause)

/** Thrown when the credential document exists but cannot be read or parsed. */
class CredentialStoreCorruptionException(message: String, cause: Throwable? = null) :
    IllegalStateException(message, cause)

/**
 * Android Keystore credential backend for the native Rust stores.
 *
 * Rust decides the account names and the value format; this store only encrypts
 * the opaque ciphertext with a non-exportable Keystore AES key and persists the
 * result under `noBackupFilesDir`, which Android never backs up or transfers to
 * a new device. Losing the Keystore key therefore makes the ciphertext
 * undecryptable instead of silently readable.
 */
class KeystoreCredentialStore(context: Context) {

    private val appContext = context.applicationContext
    private val lock = Any()
    private val file = File(
        File(appContext.noBackupFilesDir, CREDENTIALS_DIR),
        CREDENTIALS_FILE
    )
    private val atomicFile = AtomicFile(file)

    fun get(account: String): String? = synchronized(lock) {
        val token = readDocument().optString(account, "")
        if (token.isEmpty()) return null
        decrypt(token)
    }

    fun set(account: String, value: String) = synchronized(lock) {
        val document = readDocument()
        document.put(account, encrypt(value))
        writeDocument(document)
    }

    fun delete(account: String) = synchronized(lock) {
        val document = readDocument()
        if (document.has(account)) {
            document.remove(account)
            writeDocument(document)
        }
    }

    /** Delete every stored credential; used by "clear local data". */
    fun clearAll() = synchronized(lock) {
        atomicFile.delete()
        // Remove the wrapping key as well so old ciphertext can never be
        // resurrected from a stale file after this point.
        deleteKey()
    }

    fun accounts(): List<String> = synchronized(lock) {
        readDocument().keys().asSequence().toList()
    }

    /**
     * Read the credential document.
     *
     * Only a missing file is treated as empty storage. An unreadable,
     * unparsable, or zero-byte document raises instead of returning an empty
     * object: a real empty store is written as `{}`, so a truncated file is
     * damage that must never be overwritten (or migrated away from) silently.
     */
    private fun readDocument(): JSONObject {
        if (!file.exists()) return JSONObject()
        val bytes = try {
            atomicFile.readFully()
        } catch (error: Throwable) {
            throw CredentialStoreCorruptionException(
                "credential store is unreadable: ${error.javaClass.simpleName}",
                error
            )
        }
        if (bytes.isEmpty()) {
            throw CredentialStoreCorruptionException("credential store is empty")
        }
        return try {
            JSONObject(String(bytes, Charsets.UTF_8))
        } catch (error: Throwable) {
            throw CredentialStoreCorruptionException(
                "credential store is corrupt: ${error.javaClass.simpleName}",
                error
            )
        }
    }

    /**
     * Atomically replace the credential document.
     *
     * `AtomicFile` writes a sidecar and renames it over the base file, so a
     * crash at any point keeps the previous credential set readable; the code
     * must not delete the only valid copy before the replacement exists.
     */
    private fun writeDocument(document: JSONObject) {
        val directory = file.parentFile ?: return
        directory.mkdirs()
        val stream = atomicFile.startWrite()
        try {
            stream.write(document.toString().toByteArray(Charsets.UTF_8))
            atomicFile.finishWrite(stream)
        } catch (error: Throwable) {
            atomicFile.failWrite(stream)
            throw error
        }
    }

    private fun encrypt(plaintext: String): String {
        val key = secretKeyOrCreate()
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.ENCRYPT_MODE, key)
        val payload = cipher.iv + cipher.doFinal(plaintext.toByteArray(Charsets.UTF_8))
        return Base64.encodeToString(payload, Base64.NO_WRAP)
    }

    private fun decrypt(token: String): String? {
        val key = existingSecretKey()
            ?: throw CredentialKeyInvalidatedException("credential key is missing")
        val payload = try {
            Base64.decode(token, Base64.NO_WRAP)
        } catch (error: IllegalArgumentException) {
            throw CredentialKeyInvalidatedException("stored credential is not valid base64", error)
        }
        if (payload.size <= IV_LENGTH) {
            throw CredentialKeyInvalidatedException("stored credential is truncated")
        }
        return try {
            val cipher = Cipher.getInstance(TRANSFORMATION)
            cipher.init(
                Cipher.DECRYPT_MODE,
                key,
                GCMParameterSpec(TAG_BITS, payload, 0, IV_LENGTH)
            )
            String(
                cipher.doFinal(payload, IV_LENGTH, payload.size - IV_LENGTH),
                Charsets.UTF_8
            )
        } catch (error: AEADBadTagException) {
            throw CredentialKeyInvalidatedException(
                "credential key was invalidated or replaced",
                error
            )
        } catch (error: android.security.keystore.KeyPermanentlyInvalidatedException) {
            throw CredentialKeyInvalidatedException("credential key was permanently invalidated", error)
        }
    }

    private fun existingSecretKey(): SecretKey? = runCatching {
        val keyStore = keyStore()
        (keyStore.getEntry(KEY_ALIAS, null) as? KeyStore.SecretKeyEntry)?.secretKey
    }.getOrNull()

    private fun secretKeyOrCreate(): SecretKey {
        existingSecretKey()?.let { return it }
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, KEYSTORE)
        generator.init(
            KeyGenParameterSpec.Builder(
                KEY_ALIAS,
                KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT
            )
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .setUserAuthenticationRequired(false)
                .build()
        )
        return generator.generateKey()
    }

    fun deleteKey() {
        runCatching { keyStore().deleteEntry(KEY_ALIAS) }
    }

    private fun keyStore(): KeyStore = KeyStore.getInstance(KEYSTORE).apply { load(null) }

    companion object {
        private const val KEYSTORE = "AndroidKeyStore"
        private const val KEY_ALIAS = "miaominal-credentials-v1"
        private const val TRANSFORMATION = "AES/GCM/NoPadding"
        private const val IV_LENGTH = 12
        private const val TAG_BITS = 128
        private const val CREDENTIALS_DIR = "native-credentials"
        private const val CREDENTIALS_FILE = "platform_credentials.json"
    }
}

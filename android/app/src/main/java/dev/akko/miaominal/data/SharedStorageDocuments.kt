package dev.akko.miaominal.data

import android.net.Uri
import android.os.Environment
import android.os.ParcelFileDescriptor
import android.provider.DocumentsContract
import dev.akko.miaominal.nativebridge.BackendException
import dev.akko.miaominal.nativebridge.SafWriteStart
import java.io.File
import java.nio.file.Files
import java.nio.file.SimpleFileVisitor
import java.nio.file.FileVisitResult
import java.nio.file.Path
import java.nio.file.StandardCopyOption
import java.nio.file.attribute.BasicFileAttributes
import java.util.UUID

/** Shared storage uses the same descriptor-based transfer contract as a SAF tree. */
internal class SharedStorageDocuments(private val accessAllowed: () -> Boolean = Environment::isExternalStorageManager) {
    @Suppress("DEPRECATION")
    private val root: File get() = Environment.getExternalStorageDirectory().canonicalFile

    fun rootUri(): String? = if (accessAllowed()) Uri.fromFile(root).toString() else null

    private fun resolve(treeUri: String, relativePath: String): File {
        if (!accessAllowed()) {
            throw BackendException.PermissionDenied(permissionDetail = "Allow all files access to browse shared storage")
        }
        val uri = Uri.parse(treeUri)
        val storageRoot = root
        if (uri.scheme != "file" || !uri.authority.isNullOrEmpty() || uri.path != storageRoot.path) {
            throw BackendException.InvalidArgument(detail = "not the shared storage root: $treeUri")
        }
        if (relativePath.isNotEmpty() && relativePath.split('/').any { it.isEmpty() || it == "." || it == ".." }) {
            throw BackendException.InvalidArgument(detail = "local paths must stay inside shared storage: $relativePath")
        }
        val file = File(storageRoot, relativePath).canonicalFile
        if (file != storageRoot && !file.path.startsWith(storageRoot.path + File.separator)) {
            throw BackendException.InvalidArgument(detail = "local path leaves shared storage: $relativePath")
        }
        return file
    }

    fun document(treeUri: String, relativePath: String): SafDocuments.Document? {
        val file = resolve(treeUri, relativePath)
        if (!file.exists()) return null
        return document(file, relativePath)
    }

    private fun document(file: File, relativePath: String) = SafDocuments.Document(
        name = file.name,
        relativePath = relativePath,
        uri = Uri.fromFile(file),
        documentId = relativePath,
        isDirectory = file.isDirectory,
        size = if (file.isDirectory) null else file.length(),
        modifiedAtMs = file.lastModified().takeIf { it > 0L },
        mimeType = if (file.isDirectory) DocumentsContract.Document.MIME_TYPE_DIR else "application/octet-stream"
    )

    fun listDocuments(treeUri: String, relativePath: String): List<SafDocuments.Document> {
        val directory = resolve(treeUri, relativePath)
        if (!directory.isDirectory) throw storageFailure("not a directory: $relativePath")
        val children = directory.listFiles()
            ?: throw BackendException.PermissionDenied(permissionDetail = "cannot read directory: $relativePath")
        return children.map { document(it, if (relativePath.isEmpty()) it.name else "$relativePath/${it.name}") }
    }

    fun createDirectory(treeUri: String, relativePath: String) {
        val directory = resolve(treeUri, relativePath)
        if (!directory.isDirectory && !directory.mkdirs()) throw storageFailure("cannot create directory: $relativePath")
    }

    fun delete(treeUri: String, relativePath: String) {
        if (relativePath.isEmpty()) throw BackendException.InvalidArgument(detail = "cannot delete the storage root")
        val file = resolve(treeUri, relativePath)
        if (!file.exists()) return
        // Do not follow directory symlinks while cleaning up a transfer.
        Files.walkFileTree(file.toPath(), object : SimpleFileVisitor<Path>() {
            override fun visitFile(path: Path, attrs: BasicFileAttributes): FileVisitResult {
                Files.delete(path)
                return FileVisitResult.CONTINUE
            }
            override fun postVisitDirectory(path: Path, error: java.io.IOException?): FileVisitResult {
                if (error != null) throw error
                Files.delete(path)
                return FileVisitResult.CONTINUE
            }
        })
    }

    fun openFile(treeUri: String, relativePath: String): Int {
        val file = resolve(treeUri, relativePath)
        if (!file.isFile) throw storageFailure("not a file: $relativePath")
        return ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY).detachFd()
    }

    fun beginWrite(treeUri: String, relativePath: String): SafWriteStart {
        if (relativePath.isEmpty()) throw BackendException.InvalidArgument(detail = "a file name is required")
        val target = resolve(treeUri, relativePath)
        if (target.isDirectory) throw storageFailure("cannot replace a directory with a file: $relativePath")
        val parent = relativePath.substringBeforeLast('/', "")
        createDirectory(treeUri, parent)
        val stagingName = ".miaominal-transfer-${UUID.randomUUID()}.part"
        val stagingRelative = if (parent.isEmpty()) stagingName else "$parent/$stagingName"
        val staging = resolve(treeUri, stagingRelative)
        try {
            val descriptor = ParcelFileDescriptor.open(staging,
                ParcelFileDescriptor.MODE_WRITE_ONLY or ParcelFileDescriptor.MODE_CREATE or ParcelFileDescriptor.MODE_TRUNCATE)
            return SafWriteStart(descriptor.detachFd(), stagingRelative)
        } catch (error: Throwable) {
            staging.delete()
            throw error
        }
    }

    fun finishWrite(treeUri: String, targetRelative: String, stagingRelative: String?, commit: Boolean) {
        val target = resolve(treeUri, targetRelative)
        if (stagingRelative == null) return
        val staging = resolve(treeUri, stagingRelative)
        if (staging.parentFile != target.parentFile || !staging.name.startsWith(".miaominal-transfer-") || !staging.name.endsWith(".part")) {
            throw BackendException.InvalidArgument(detail = "invalid staging file: $stagingRelative")
        }
        if (!commit) {
            Files.deleteIfExists(staging.toPath())
            return
        }
        // Same-directory rename installs the completed download atomically, preserving the original on failure.
        try {
            Files.move(staging.toPath(), target.toPath(), StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING)
        } catch (error: Throwable) {
            staging.delete()
            throw error
        }
    }

    private fun storageFailure(message: String) = BackendException.Storage(storageFailure = message)
}

fun isSharedStorageTree(treeUri: String?): Boolean = treeUri?.startsWith("file:") == true

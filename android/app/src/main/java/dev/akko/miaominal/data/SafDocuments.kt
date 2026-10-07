package dev.akko.miaominal.data

import android.content.ContentResolver
import android.content.Context
import android.content.Intent
import android.database.Cursor
import android.net.Uri
import android.provider.DocumentsContract
import dev.akko.miaominal.nativebridge.BackendException
import dev.akko.miaominal.nativebridge.SafEntry
import dev.akko.miaominal.nativebridge.SafEntryKind
import dev.akko.miaominal.nativebridge.SafWriteStart
import java.io.FileNotFoundException

/**
 * SAF document operations for the Rust transfer engine.
 *
 * Rust never guesses a real path: it asks this adapter for directory listings,
 * metadata, document creation and removal, and it only receives file
 * descriptors for file content. Every operation is scoped to a tree URI the
 * user authorized with a persistable URI permission.
 */
/**
 * Display label for an authorized tree, derived without querying the provider
 * (safe to call during composition).
 */
fun safTreeLabel(treeUri: String): String = runCatching {
    val segment = android.net.Uri.parse(treeUri).lastPathSegment ?: return treeUri
    val decoded = java.net.URLDecoder.decode(segment, "UTF-8")
    decoded.substringAfterLast(':').substringAfterLast('/').takeIf { it.isNotBlank() } ?: treeUri
}.getOrDefault(treeUri)

/** One local entry shown in the SAF pane. */
data class SafLocalEntry(
    val name: String,
    val relativePath: String,
    val isDirectory: Boolean,
    val size: Long?,
    val modifiedAtMs: Long?
)

class SafDocuments(context: Context) {

    private val appContext = context.applicationContext
    private val resolver: ContentResolver get() = appContext.contentResolver

    data class Document(
        val name: String,
        val relativePath: String,
        val uri: Uri,
        val documentId: String,
        val isDirectory: Boolean,
        val size: Long?,
        val modifiedAtMs: Long?,
        val mimeType: String
    )

    private class Tree(val uri: Uri) {
        val rootDocumentUri: Uri = DocumentsContract.buildDocumentUriUsingTree(
            uri,
            DocumentsContract.getTreeDocumentId(uri)
        )
    }

    fun list(treeUri: String, relativePath: String): List<SafEntry> {
        val tree = parseTree(treeUri)
        val directory = resolve(tree, relativePath, createDirectories = false)
            ?: throw storageFailure("document not found: $relativePath")
        if (!directory.isDirectory) {
            throw storageFailure("not a directory: $relativePath")
        }
        return children(tree, directory).map { entry ->
            SafEntry(
                name = entry.name,
                relativePath = joinRelative(relativePath, entry.name),
                kind = entry.toKind(),
                size = entry.size?.toULong()
            )
        }
    }

    /** Local-pane listing, retaining document URIs for UI actions. */
    fun listDocuments(treeUri: String, relativePath: String): List<Document> {
        val tree = parseTree(treeUri)
        val directory = resolve(tree, relativePath, createDirectories = false)
            ?: return emptyList()
        return children(tree, directory)
    }

    fun stat(treeUri: String, relativePath: String): SafEntry {
        val tree = parseTree(treeUri)
        val document = resolve(tree, relativePath, createDirectories = false)
            ?: throw storageFailure("document not found: $relativePath")
        return SafEntry(
            name = document.name,
            relativePath = relativePath,
            kind = document.toKind(),
            size = document.size?.toULong()
        )
    }

    fun document(treeUri: String, relativePath: String): Document? {
        val tree = parseTree(treeUri)
        return resolve(tree, relativePath, createDirectories = false)
    }

    fun createDirectory(treeUri: String, relativePath: String) {
        val tree = parseTree(treeUri)
        resolve(tree, relativePath, createDirectories = true)
    }

    fun delete(treeUri: String, relativePath: String) {
        val tree = parseTree(treeUri)
        val document = resolve(tree, relativePath, createDirectories = false) ?: return
        try {
            DocumentsContract.deleteDocument(resolver, document.uri)
        } catch (_: FileNotFoundException) {
            // Already gone: deletion stays idempotent for cleanup paths.
        } catch (error: Throwable) {
            throw failure("delete", relativePath, error)
        }
    }

    /**
     * Detach a read descriptor. Ownership moves to Rust, which duplicates the
     * descriptor and closes this original.
     */
    fun openFile(treeUri: String, relativePath: String): Int {
        val tree = parseTree(treeUri)
        val document = resolve(tree, relativePath, createDirectories = false)
            ?: throw storageFailure("document not found: $relativePath")
        val descriptor = try {
            resolver.openFileDescriptor(document.uri, "r")
        } catch (error: Throwable) {
            throw failure("open", relativePath, error)
        } ?: throw storageFailure("provider returned no descriptor for $relativePath")
        return descriptor.detachFd()
    }

    /**
     * Prepare a document for writing.
     *
     * A missing target is created directly. An existing target is replaced
     * through a staging document: the provider's original file stays untouched
     * until [finishWrite] commits the replacement.
     */
    fun beginWrite(treeUri: String, relativePath: String): SafWriteStart {
        val tree = parseTree(treeUri)
        val name = relativePath.substringAfterLast('/')
        if (name.isBlank()) {
            throw storageFailure("a document name is required to write $relativePath")
        }
        val parent = parentRelative(relativePath)
        val parentDocument = resolve(tree, parent, createDirectories = true)
            ?: throw storageFailure("cannot create the parent directory of $relativePath")
        val existing = findChild(tree, parentDocument, name)
        return try {
            if (existing == null || existing.isDirectory) {
                if (existing?.isDirectory == true) {
                    throw unsupportedFailure("cannot replace the directory $relativePath with a file")
                }
                val created = createDocument(tree, parentDocument, name, MIME_TYPE_BINARY)
                    ?: throw unsupportedFailure("the provider refused to create $relativePath")
                val descriptor = resolver.openFileDescriptor(created.uri, "wt")
                    ?: throw storageFailure("provider returned no descriptor for $relativePath")
                SafWriteStart(fd = descriptor.detachFd(), stagingRelative = null)
            } else {
                val stagingName = "$STAGING_PREFIX${java.util.UUID.randomUUID()}$STAGING_SUFFIX"
                val staging = createDocument(tree, parentDocument, stagingName, MIME_TYPE_BINARY)
                    ?: throw unsupportedFailure(
                        "the provider cannot create a temporary document; save as a new file instead"
                    )
                val descriptor = resolver.openFileDescriptor(staging.uri, "wt")
                    ?: throw storageFailure("provider returned no descriptor for the staging document")
                SafWriteStart(
                    fd = descriptor.detachFd(),
                    stagingRelative = joinRelative(parent, stagingName)
                )
            }
        } catch (error: BackendException) {
            throw error
        } catch (error: Throwable) {
            throw failure("write", relativePath, error)
        }
    }

    /**
     * Commit or discard a write prepared by [beginWrite].
     *
     * Commit with a staging document renames the previous version to a backup,
     * installs the staging document, and only then removes the backup. A
     * provider that cannot rename reports `Unsupported` and the caller must
     * save under a different name.
     */
    fun finishWrite(treeUri: String, targetRelative: String, stagingRelative: String?, commit: Boolean) {
        val tree = parseTree(treeUri)
        if (!commit) {
            val cleanup = stagingRelative ?: targetRelative
            runCatching { delete(treeUri, cleanup) }
            return
        }
        if (stagingRelative == null) {
            return
        }
        val targetParent = parentRelative(targetRelative)
        val parentDocument = resolve(tree, targetParent, createDirectories = false)
            ?: throw storageFailure("the target directory disappeared: $targetParent")
        val target = findChild(tree, parentDocument, name(targetRelative))
            ?: throw storageFailure("the target document disappeared: $targetRelative")
        val staging = resolve(tree, stagingRelative, createDirectories = false)
            ?: throw storageFailure("the staging document disappeared: $stagingRelative")
        val backupName = "$BACKUP_PREFIX${java.util.UUID.randomUUID()}$BACKUP_SUFFIX"
        val renamed = try {
            DocumentsContract.renameDocument(resolver, target.uri, backupName)
        } catch (error: Throwable) {
            removeStagingDocument(staging)
            throw unsupportedFailure(
                "the provider cannot safely replace an existing file (${error.javaClass.simpleName}); save as a new file instead"
            )
        } ?: run {
            removeStagingDocument(staging)
            throw unsupportedFailure(
                "the provider cannot safely replace an existing file; save as a new file instead"
            )
        }
        val installed = try {
            DocumentsContract.renameDocument(resolver, staging.uri, target.name)
        } catch (_: Throwable) {
            null
        }
        if (installed == null) {
            runCatching { DocumentsContract.renameDocument(resolver, renamed, target.name) }
            removeStagingDocument(staging)
            throw storageFailure("failed to install the downloaded file; the original was restored")
        }
        runCatching { DocumentsContract.deleteDocument(resolver, renamed) }
    }

    private fun removeStagingDocument(staging: Document) {
        runCatching { DocumentsContract.deleteDocument(resolver, staging.uri) }
    }

    /** Persist the URI permission granted by a picker result. */
    fun takePersistablePermission(uri: Uri): Boolean = try {
        resolver.takePersistableUriPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION)
        true
    } catch (_: SecurityException) {
        // Some providers only grant a transient permission; the tree stays
        // usable for this process and the UI explains that it is not persisted.
        false
    }

    fun displayName(treeUri: String, relativePath: String): String =
        document(treeUri, relativePath)?.name ?: name(relativePath)

    /** Local-pane listing without the native entry type. */
    fun listLocal(treeUri: String, relativePath: String): List<SafLocalEntry> =
        listDocuments(treeUri, relativePath).map { document ->
            SafLocalEntry(
                name = document.name,
                relativePath = if (relativePath.isBlank()) document.name else "$relativePath/${document.name}",
                isDirectory = document.isDirectory,
                size = document.size,
                modifiedAtMs = document.modifiedAtMs
            )
        }

    private fun parseTree(treeUri: String): Tree {
        val uri = Uri.parse(treeUri)
        if (!DocumentsContract.isTreeUri(uri)) {
            throw BackendException.InvalidArgument(detail = "not a SAF tree URI: $treeUri")
        }
        return Tree(uri)
    }

    private fun resolve(tree: Tree, relativePath: String, createDirectories: Boolean): Document? {
        if (relativePath.isBlank()) {
            return documentOf(tree, tree.rootDocumentUri)
        }
        var parentUri = tree.rootDocumentUri
        var parent: Document? = documentOf(tree, parentUri)
        for (component in relativePath.split('/')) {
            if (component.isBlank() || component == "." || component == "..") {
                throw BackendException.InvalidArgument(
                    detail = "SAF paths must stay inside the authorized tree: $relativePath"
                )
            }
            val existing = parent?.let { findChild(tree, it, component) }
            if (existing != null) {
                parentUri = existing.uri
                parent = existing
                continue
            }
            if (!createDirectories) {
                return null
            }
            val parentDocument = parent ?: documentOf(tree, parentUri) ?: return null
            val created = createDocument(tree, parentDocument, component, MIME_TYPE_DIRECTORY)
                ?: return null
            parentUri = created.uri
            parent = created
        }
        return parent
    }

    private fun documentOf(tree: Tree, documentUri: Uri): Document? {
        val cursor = try {
            resolver.query(
                documentUri,
                DOCUMENT_COLUMNS,
                null,
                null,
                null
            )
        } catch (error: Throwable) {
            throw failure("query", documentUri.toString(), error)
        } ?: return null
        cursor.use {
            if (!it.moveToFirst()) return null
            val documentId = it.stringOrNull(DocumentsContract.Document.COLUMN_DOCUMENT_ID) ?: return null
            val name = it.stringOrNull(DocumentsContract.Document.COLUMN_DISPLAY_NAME) ?: ""
            val mime = it.stringOrNull(DocumentsContract.Document.COLUMN_MIME_TYPE) ?: MIME_TYPE_BINARY
            val size = it.longOrNull(DocumentsContract.Document.COLUMN_SIZE)
            val modified = it.longOrNull(DocumentsContract.Document.COLUMN_LAST_MODIFIED)
            return Document(
                name = name,
                relativePath = "",
                uri = DocumentsContract.buildDocumentUriUsingTree(tree.uri, documentId),
                documentId = documentId,
                isDirectory = mime == MIME_TYPE_DIRECTORY,
                size = size,
                modifiedAtMs = modified,
                mimeType = mime
            )
        }
    }

    private fun children(tree: Tree, directory: Document): List<Document> {
        val childUri = DocumentsContract.buildChildDocumentsUriUsingTree(tree.uri, directory.documentId)
        val cursor = try {
            resolver.query(childUri, DOCUMENT_COLUMNS, null, null, null)
        } catch (error: Throwable) {
            throw failure("list", directory.name, error)
        } ?: throw storageFailure("the provider refused to list ${directory.name}")
        return cursor.use { it.toDocuments(tree, directory) }
    }

    private fun Cursor.toDocuments(tree: Tree, parent: Document): List<Document> {
        val documents = ArrayList<Document>()
        while (moveToNext()) {
            val documentId = stringOrNull(DocumentsContract.Document.COLUMN_DOCUMENT_ID) ?: continue
            val name = stringOrNull(DocumentsContract.Document.COLUMN_DISPLAY_NAME) ?: continue
            val mime = stringOrNull(DocumentsContract.Document.COLUMN_MIME_TYPE) ?: MIME_TYPE_BINARY
            val size = longOrNull(DocumentsContract.Document.COLUMN_SIZE)
            val modified = longOrNull(DocumentsContract.Document.COLUMN_LAST_MODIFIED)
            documents.add(
                Document(
                    name = name,
                    relativePath = joinRelative(parent.relativePath, name),
                    uri = DocumentsContract.buildDocumentUriUsingTree(tree.uri, documentId),
                    documentId = documentId,
                    isDirectory = mime == MIME_TYPE_DIRECTORY,
                    size = size,
                    modifiedAtMs = modified,
                    mimeType = mime
                )
            )
        }
        return documents
    }

    private fun findChild(tree: Tree, parent: Document, name: String): Document? =
        children(tree, parent).firstOrNull { it.name == name }

    private fun createDocument(tree: Tree, parent: Document, name: String, mime: String): Document? {
        val created = DocumentsContract.createDocument(resolver, parent.uri, mime, name) ?: return null
        return documentOf(tree, created)?.copy(relativePath = joinRelative(parent.relativePath, name))
    }

    private fun Document.toKind(): SafEntryKind = when {
        isDirectory -> SafEntryKind.DIRECTORY
        mimeType == MIME_TYPE_DIRECTORY -> SafEntryKind.DIRECTORY
        else -> SafEntryKind.FILE
    }
}

private const val MIME_TYPE_DIRECTORY = DocumentsContract.Document.MIME_TYPE_DIR
private const val MIME_TYPE_BINARY = "application/octet-stream"
private const val STAGING_PREFIX = ".miaominal-transfer-"
private const val STAGING_SUFFIX = ".part"
private const val BACKUP_PREFIX = ".miaominal-backup-"
private const val BACKUP_SUFFIX = ".bak"

private val DOCUMENT_COLUMNS = arrayOf(
    DocumentsContract.Document.COLUMN_DOCUMENT_ID,
    DocumentsContract.Document.COLUMN_DISPLAY_NAME,
    DocumentsContract.Document.COLUMN_MIME_TYPE,
    DocumentsContract.Document.COLUMN_SIZE,
    DocumentsContract.Document.COLUMN_LAST_MODIFIED
)

private fun joinRelative(parent: String, name: String): String =
    if (parent.isBlank()) name else "$parent/$name"

private fun parentRelative(path: String): String =
    path.substringBeforeLast('/', "")

private fun name(path: String): String = path.substringAfterLast('/')

private fun Cursor.stringOrNull(column: String): String? {
    val index = getColumnIndex(column)
    if (index < 0 || isNull(index)) return null
    return getString(index)
}

private fun Cursor.longOrNull(column: String): Long? {
    val index = getColumnIndex(column)
    if (index < 0 || isNull(index)) return null
    return getLong(index)
}

private fun storageFailure(message: String): BackendException =
    BackendException.Storage(storageFailure = message)

private fun unsupportedFailure(message: String): BackendException =
    BackendException.Unsupported(capability = message)

private fun failure(operation: String, path: String, error: Throwable): BackendException = when (error) {
    is BackendException -> error
    is SecurityException -> BackendException.PermissionDenied(
        permissionDetail = "$operation denied for $path: ${error.message}"
    )
    is UnsupportedOperationException, is IllegalStateException -> BackendException.Unsupported(
        capability = "$operation is not supported for $path: ${error.javaClass.simpleName}"
    )
    is android.system.ErrnoException -> if (error.errno == android.system.OsConstants.ENOSPC) {
        BackendException.NoSpace(spaceDetail = "$operation failed for $path: no space left on device")
    } else {
        BackendException.Storage(storageFailure = "$operation failed for $path: ${error.message}")
    }
    else -> BackendException.Storage(storageFailure = "$operation failed for $path: ${error.message}")
}

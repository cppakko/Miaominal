package dev.akko.miaominal.data

import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test

class SftpTransferActionsTest {
    @Test fun retryCreatesNewTaskRetainsFailureAndDeduplicatesActiveReplacement() = runBlocking {
        val repository = InMemoryMiaominalRepository()
        val session = repository.seedSftpSession("host")
        val failedId = session.seedTransfer("file", SftpTransferStateModel.Failed)
        val failed = session.handleFor(failedId)!!
        val request = TransferRequest.UploadFile("host", "content://source", "file", "/home/tester/file")
        repository.transferActions.record(failed, request)
        repository.transferActions.retainFailures("host", session.transfersSnapshot())
        val key = TransferKey(session.connectionId, failedId)
        val retried = repository.transferActions.retry(key, session.connectionId).getOrThrow()
        assertNotEquals(failedId, retried.transferId)
        assertEquals(request, repository.transferActions.request(TransferKey(retried.connectionId, retried.transferId)))
        assertEquals(SftpTransferStateModel.Failed, failed.snapshot()!!.state)
        assertEquals(retried.transferId, repository.transferActions.retry(key, session.connectionId).getOrThrow().transferId)
        assertEquals(1, repository.transferActions.failureHistory("host").size)
        assertTrue(repository.transferActions.failureHistory("another-host").isEmpty())
    }

    @Test fun destinationReplacementRequiresExplicitChoice() = runBlocking {
        val repository = InMemoryMiaominalRepository()
        val session = repository.seedSftpSession("host")
        session.seedTree("/home/tester", listOf(SftpEntryModel("file", "/home/tester/file", SftpEntryKind.File, 12, null, null, null)))
        val failedId = session.seedTransfer("file", SftpTransferStateModel.Failed)
        val key = TransferKey(session.connectionId, failedId)
        repository.transferActions.record(session.handleFor(failedId)!!, TransferRequest.UploadTree("host", "content://tree", "file", "file", "/home/tester/file"))
        assertTrue(repository.transferActions.retry(key, session.connectionId).isFailure)
        assertTrue(repository.transferActions.retry(key, session.connectionId, allowOverwrite = true).isSuccess)
    }

    @Test fun sourcePermissionFailureAndConcurrentTriggerNeverQueueExtraTask() = runBlocking {
        val base = InMemoryMiaominalRepository()
        val session = base.seedSftpSession("host")
        val sourceEntered = CompletableDeferred<Unit>()
        val releaseSource = CompletableDeferred<Unit>()
        val denied = object : MiaominalRepository by base {
            override suspend fun registerUploadFile(uri: String, name: String): Result<LocalUploadHandle> {
                sourceEntered.complete(Unit); releaseSource.await()
                return Result.failure(SecurityException("permission expired"))
            }
        }
        val actions = SftpTransferActions(denied)
        val failedId = session.seedTransfer("file", SftpTransferStateModel.Failed)
        val key = TransferKey(session.connectionId, failedId)
        actions.record(session.handleFor(failedId)!!, TransferRequest.UploadFile("host", "content://expired", "file", "/file"))
        val first = async { actions.retry(key, session.connectionId) }
        sourceEntered.await()
        assertTrue(actions.retry(key, session.connectionId).isFailure)
        releaseSource.complete(Unit)
        assertTrue(first.await().exceptionOrNull() is SecurityException)
        assertEquals(1, session.transfersSnapshot().size)
    }

    @Test fun wrongHostOrMissingRecipeFailsWithoutChangingFiles() = runBlocking {
        val repository = InMemoryMiaominalRepository()
        val session = repository.seedSftpSession("host")
        val id = session.seedTransfer("file", SftpTransferStateModel.Failed)
        val key = TransferKey(session.connectionId, id)
        assertTrue(repository.transferActions.retry(key, session.connectionId).isFailure)
        repository.transferActions.record(session.handleFor(id)!!, TransferRequest.Download("other-host", "/file", "content://tree", "file"))
        assertTrue(repository.transferActions.retry(key, session.connectionId).isFailure)
        assertEquals(1, session.transfersSnapshot().size)
    }

    @Test fun completionCleanupOnlyHidesDoneRecordsAndCloseCountsUnfinishedTasks() {
        val repository = InMemoryMiaominalRepository()
        val session = repository.seedSftpSession("host")
        SftpTransferStateModel.entries.forEach { session.seedTransfer(it.name, it) }
        val snapshots = session.transfersSnapshot()
        assertEquals(3, dev.akko.miaominal.ui.model.unfinishedTransferCount(snapshots))
        repository.transferActions.hideCompleted(snapshots)
        snapshots.forEach { assertEquals(it.state == SftpTransferStateModel.Done,
            repository.transferActions.isHidden(TransferKey(it.connectionId, it.transferId))) }
        assertEquals(6, session.transfersSnapshot().size)
    }
}

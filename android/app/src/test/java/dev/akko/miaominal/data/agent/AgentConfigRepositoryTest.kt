package dev.akko.miaominal.data.agent

import dev.akko.miaominal.data.CredentialUpdate
import kotlinx.coroutines.test.runTest
import org.junit.Assert.*
import org.junit.Test

class AgentConfigRepositoryTest {
    private class MemoryStore : AgentConfigPersistence {
        var document: String? = null
        var readFailure = false
        var writeFailure = false
        override fun read(): String? {
            if (readFailure) error("decryption failed: private-key")
            return document
        }
        override fun write(document: String) {
            if (writeFailure) error("write failed: private-key")
            this.document = document
        }
        override fun delete() { document = null }
    }

    private fun profile(id: String = "p", protocol: AgentApiProtocol = AgentApiProtocol.Responses) =
        AgentConfigProfile(id, "Proxy", protocol, "https://example.test/v1", listOf("one", "two"), "two")

    @Test fun savesReloadsAndKeepsCredentialsOutOfPublicState() = runTest {
        val store = MemoryStore()
        val repo = AgentConfigRepository(store)
        repo.save(profile(), CredentialUpdate.Set("private-key")).getOrThrow()
        repo.remember("host", AgentCli.Codex, AgentConfigSource.Local("p")).getOrThrow()
        val loaded = AgentConfigRepository(store)
        loaded.load().getOrThrow()
        assertTrue(loaded.state.value.profiles.single().hasApiKey)
        assertFalse(loaded.state.value.toString().contains("private-key"))
        assertEquals(AgentConfigSource.Local("p"), loaded.selection("host", AgentCli.Codex).getOrThrow())
        assertEquals(AgentConfigSource.Remote, loaded.selection("host", AgentCli.OpenCode).getOrThrow())
        assertEquals(AgentConfigSource.Remote, loaded.selection("different", AgentCli.Codex).getOrThrow())
        val snapshot = loaded.resolve(AgentConfigSource.Local("p"), AgentCli.Codex).getOrThrow()!!
        assertFalse(snapshot.toString().contains("private-key"))
        assertEquals("[REDACTED]", snapshot.redact("private-key"))
        assertEquals("private-key", snapshot.keyForLaunch())
    }

    @Test fun credentialUpdatesAreAtomicAndFailuresNeverPublishUncommittedChanges() = runTest {
        val store = MemoryStore()
        val repo = AgentConfigRepository(store)
        repo.save(profile(), CredentialUpdate.Set("old")).getOrThrow()
        store.writeFailure = true
        val failed = repo.save(profile().copy(name = "Changed"), CredentialUpdate.Set("new"))
        assertTrue(failed.isFailure)
        assertFalse(failed.exceptionOrNull()!!.message!!.contains("private-key"))
        assertEquals("Proxy", repo.state.value.profiles.single().name)
        assertEquals("old", repo.resolve(AgentConfigSource.Local("p"), AgentCli.Codex).getOrThrow()!!.keyForLaunch())
        store.writeFailure = false
        repo.save(profile().copy(name = "Changed"), CredentialUpdate.Keep).getOrThrow()
        assertEquals("old", repo.resolve(AgentConfigSource.Local("p"), AgentCli.Codex).getOrThrow()!!.keyForLaunch())
        repo.save(profile(), CredentialUpdate.Clear).getOrThrow()
        assertFalse(repo.state.value.profiles.single().hasApiKey)
        assertTrue(repo.resolve(AgentConfigSource.Local("p"), AgentCli.Codex).isFailure)
    }

    @Test fun deletionCannotFallBackToRemoteAuthAndDoesNotMutateRunningSnapshots() = runTest {
        val repo = AgentConfigRepository(MemoryStore())
        repo.save(profile(), CredentialUpdate.Set("old")).getOrThrow()
        repo.remember("host", AgentCli.Codex, AgentConfigSource.Local("p")).getOrThrow()
        val pinned = repo.resolve(AgentConfigSource.Local("p"), AgentCli.Codex).getOrThrow()!!
        repo.save(profile().copy(baseUrl = "https://new.test/v1"), CredentialUpdate.Set("new")).getOrThrow()
        repo.delete("p").getOrThrow()
        assertEquals("https://example.test/v1", pinned.profile.baseUrl)
        assertEquals("old", pinned.keyForLaunch())
        val selected = repo.selection("host", AgentCli.Codex).getOrThrow()
        assertEquals(AgentConfigSource.Local("p"), selected)
        assertTrue(repo.resolve(selected, AgentCli.Codex).isFailure)
        repo.remember("host", AgentCli.Codex, AgentConfigSource.Remote).getOrThrow()
        assertNull(repo.resolve(AgentConfigSource.Remote, AgentCli.Codex).getOrThrow())
    }

    @Test fun protocolAndInputValidationBlockInvalidLaunches() = runTest {
        val repo = AgentConfigRepository(MemoryStore())
        repo.save(profile(protocol = AgentApiProtocol.ChatCompletions), CredentialUpdate.Set("test")).getOrThrow()
        assertTrue(repo.resolve(AgentConfigSource.Local("p"), AgentCli.Codex).isFailure)
        assertTrue(repo.resolve(AgentConfigSource.Local("p"), AgentCli.OpenCode).isSuccess)
        assertTrue(repo.remember("host", AgentCli.Codex, AgentConfigSource.Local("p")).isFailure)
        for (invalid in listOf("file:///tmp/key", "https://user:key@example.test/v1", "https://example.test/#key", "bad")) {
            assertTrue(repo.save(profile().copy(baseUrl = invalid), CredentialUpdate.Keep).isFailure)
        }
        assertTrue(repo.save(profile().copy(defaultModel = "missing"), CredentialUpdate.Keep).isFailure)
        assertTrue(repo.save(profile(), CredentialUpdate.Set("key\ncommand")).isFailure)
        repo.save(profile().copy(models = listOf(" one ", "one", "two")), CredentialUpdate.Set("test")).getOrThrow()
        assertEquals(listOf("one", "two"), repo.state.value.profiles.single().models)
    }

    @Test fun corruptedOrLockedDocumentsCannotBeOverwrittenByAnEmptyState() = runTest {
        val store = MemoryStore().apply { document = "invalid private-key" }
        val repo = AgentConfigRepository(store)
        assertTrue(repo.load().isFailure)
        assertTrue(repo.save(profile(), CredentialUpdate.Set("test")).isFailure)
        assertEquals("invalid private-key", store.document)
        assertFalse(repo.state.value.toString().contains("private-key"))
        repo.clear().getOrThrow()
        repo.save(profile(), CredentialUpdate.Set("test")).getOrThrow()
        store.readFailure = true
        val locked = AgentConfigRepository(store)
        assertTrue(locked.load().isFailure)
        assertTrue(locked.resolve(AgentConfigSource.Local("p"), AgentCli.Codex).isFailure)
    }

    @Test fun resetClearsProfilesAndRememberedSelections() = runTest {
        val store = MemoryStore()
        val repo = AgentConfigRepository(store)
        repo.save(profile(), CredentialUpdate.Set("test")).getOrThrow()
        repo.remember("host", AgentCli.Codex, AgentConfigSource.Local("p")).getOrThrow()
        repo.clearWithCredentials { store.document = null } // application's Keystore clearAll
        assertTrue(repo.state.value.profiles.isEmpty())
        assertEquals(AgentConfigSource.Remote, repo.selection("host", AgentCli.Codex).getOrThrow())
        assertTrue(AgentConfigRepository(store).load().isSuccess)
    }
}

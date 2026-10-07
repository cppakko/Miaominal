package dev.akko.miaominal

import android.app.Application
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.akko.miaominal.data.BackgroundPolicy
import dev.akko.miaominal.data.BusinessSettings
import dev.akko.miaominal.data.InMemoryMiaominalRepository
import dev.akko.miaominal.data.SessionPhase
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * background-policy acceptance: the 30 second grace period closes sessions,
 * app-owned user flows suspend it, keep-alive prevents it, and the service
 * "stop tasks" action closes everything.
 */
@RunWith(AndroidJUnit4::class)
class BackgroundPolicyTest {
    private val application =
        InstrumentationRegistry.getInstrumentation().targetContext.applicationContext as Application

    private fun newPolicy(
        repository: InMemoryMiaominalRepository,
        graceMillis: Long = 150L
    ) = BackgroundPolicy(
        application,
        repository,
        CoroutineScope(SupervisorJob() + Dispatchers.Default),
        graceMillis = graceMillis
    )

    private fun repositoryWithSession(): Pair<InMemoryMiaominalRepository, Long> {
        val repository = InMemoryMiaominalRepository(
            hosts = TestFixtures.hosts(),
            settings = BusinessSettings(onboardingCompleted = true)
        )
        val session = repository.seedSession(TestFixtures.PRIMARY_HOST_ID, SessionPhase.Ready)
        return repository to session.connectionId
    }

    private fun hasActiveSession(repository: InMemoryMiaominalRepository): Boolean =
        repository.sessions.value.any { !it.phase.isTerminal }

    private fun waitUntil(timeoutMillis: Long = 3_000, predicate: () -> Boolean) {
        val deadline = System.currentTimeMillis() + timeoutMillis
        while (System.currentTimeMillis() < deadline) {
            if (predicate()) return
            Thread.sleep(25)
        }
        assertTrue("condition was not met before the timeout", predicate())
    }

    @Test
    fun backgroundingClosesSessionsAfterTheGracePeriod() {
        val (repository, _) = repositoryWithSession()
        val policy = newPolicy(repository)

        policy.simulateBackgroundForTest()

        waitUntil { !hasActiveSession(repository) }
    }

    @Test
    fun anAppOwnedUserFlowSuspendsTheCountdown() {
        val (repository, _) = repositoryWithSession()
        val policy = newPolicy(repository)

        policy.beginUserFlow()
        policy.simulateBackgroundForTest()
        Thread.sleep(600)
        assertTrue(
            "a document picker or biometric flow must not count as leaving the app",
            hasActiveSession(repository)
        )

        policy.endUserFlow()
        waitUntil { !hasActiveSession(repository) }
    }

    @Test
    fun anExternalAppFlowSuspendsTheCountdownUntilTheAppReturns() {
        val (repository, _) = repositoryWithSession()
        val policy = newPolicy(repository)

        policy.beginExternalFlow()
        policy.simulateBackgroundForTest()
        Thread.sleep(600)
        assertTrue(
            "the browser opened from the app must not count as leaving the app",
            hasActiveSession(repository)
        )

        policy.endExternalFlow()
        waitUntil { !hasActiveSession(repository) }
    }

    @Test
    fun keepAlivePreventsTheBackgroundClose() {
        val (repository, _) = repositoryWithSession()
        val policy = newPolicy(repository)

        policy.restoreFromPreferences(true)
        policy.simulateBackgroundForTest()
        Thread.sleep(600)
        assertTrue(
            "background keep-alive must keep sessions open",
            hasActiveSession(repository)
        )
    }

    @Test
    fun stopAllTasksClosesEverySession() {
        val (repository, _) = repositoryWithSession()
        val policy = newPolicy(repository)

        policy.stopAllTasks()

        waitUntil { !hasActiveSession(repository) }
    }
}

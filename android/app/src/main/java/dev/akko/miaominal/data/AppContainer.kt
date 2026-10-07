package dev.akko.miaominal.data

import android.app.Application
import android.content.Context
import dev.akko.miaominal.ConnectionService
import dev.akko.miaominal.data.agent.AgentRepository
import dev.akko.miaominal.data.agent.AgentConfigRepository
import dev.akko.miaominal.data.agent.KeystoreAgentConfigPersistence
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch

/**
 * Application-scoped composition root: one Kotlin credential backend, one
 * native backend client, one repository, the AI agent repository, and the
 * background policy. Created once per process before any Activity, so Compose
 * only consumes it.
 */
class AppContainer(context: Context) {
    private val appContext = context.applicationContext
    private val platformHost = AndroidPlatformHost(appContext)

    val credentialStore: KeystoreCredentialStore = platformHost.credentialStore
    val backendClient: NativeBackendClient = NativeBackendClient.get(appContext)
    private val policyScope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    val agentConfigs = AgentConfigRepository(KeystoreAgentConfigPersistence(credentialStore))

    /** AI chat state and CLI sessions; survives every Activity rebuild. */
    val agentRepository: AgentRepository =
        AgentRepository(appContext, backendClient, policyScope, configs = agentConfigs)

    val repository: MiaominalRepository = NativeMiaominalRepository(
        appContext, backendClient, credentialStore,
        beforeReset = { agentRepository.closeSession() },
        clearDeviceCredentials = { agentConfigs.clearWithCredentials { credentialStore.clearAll() } }
    )

    val backgroundPolicy: BackgroundPolicy = BackgroundPolicy(
        appContext as Application,
        repository,
        policyScope,
        agentSessions = agentRepository
    )

    init {
        // The foreground service observes the same repository instance and uses
        // the policy for its "stop tasks" action.
        ConnectionService.repository = repository
        ConnectionService.policy = backgroundPolicy
        ConnectionService.ensureChannel(appContext)
        // Returning to the foreground runs one automatic check; the Rust side
        // serializes it with the WorkManager entry and with manual actions.
        backgroundPolicy.onForeground = {
            policyScope.launch {
                if (repository.sync.value.config.autoSyncEnabled) {
                    repository.syncAutoCheck()
                }
            }
        }
        policyScope.launch {
            // Register exactly one periodic job while automatic sync is on and
            // cancel it as soon as the setting is turned off. A state that has
            // not been loaded from the backend is ignored, so a cold start
            // (which may have been triggered by the job itself) cannot cancel
            // the persisted work.
            repository.sync.collect { state ->
                SyncScheduler.reconcile(appContext, state)
            }
        }
    }

    fun start() {
        backendClient.start()
        repository.start()
        policyScope.launch { agentConfigs.load(); agentRepository.restore() }
        backgroundPolicy.attach()
        backgroundPolicy.restoreFromPreferences(
            repository.devicePreferences.value.backgroundKeepAlive
        )
    }
}

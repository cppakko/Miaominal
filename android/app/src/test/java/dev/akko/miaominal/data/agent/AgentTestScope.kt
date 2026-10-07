package dev.akko.miaominal.data.agent

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.UnconfinedTestDispatcher

/**
 * Adapter scope that runs eagerly on the test dispatcher so channel events
 * flow synchronously through the adapter and its collectors; the adapters emit
 * through buffered channels, which a standard-dispatch scope would lag behind.
 */
internal fun TestScope.adapterScope(): CoroutineScope =
    CoroutineScope(backgroundScope.coroutineContext + UnconfinedTestDispatcher(testScheduler))

/** Collect an adapter's events into `target` with eager dispatch. */
internal fun TestScope.collectAgentEvents(
    adapter: AgentAdapter,
    target: MutableList<AgentEvent>
) {
    backgroundScope.launch(UnconfinedTestDispatcher(testScheduler)) {
        adapter.events.collect { target += it }
    }
}

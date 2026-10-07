package dev.akko.miaominal.ui.screens

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class ForwardBrowserUrlTest {
    @Test
    fun localRulesUseTheStoredPort() {
        assertEquals(
            "http://127.0.0.1:8080",
            forwardBrowserUrl(ForwardKind.Local, "127.0.0.1", 8080, null)
        )
    }

    @Test
    fun listenerWildcardsFallBackToLoopback() {
        assertEquals(
            "http://127.0.0.1:15432",
            forwardBrowserUrl(ForwardKind.Local, "", 15432, null)
        )
        assertEquals(
            "http://127.0.0.1:15432",
            forwardBrowserUrl(ForwardKind.Local, "0.0.0.0", 15432, null)
        )
        assertEquals(
            "http://127.0.0.1:15432",
            forwardBrowserUrl(ForwardKind.Local, "::", 15432, null)
        )
    }

    @Test
    fun ipv6ListenersAreBracketed() {
        assertEquals(
            "http://[::1]:9000",
            forwardBrowserUrl(ForwardKind.Local, "::1", 9000, null)
        )
    }

    @Test
    fun portZeroFallsBackToTheBoundPort() {
        assertEquals(
            "http://127.0.0.1:43210",
            forwardBrowserUrl(ForwardKind.Local, "127.0.0.1", 0, 43210)
        )
        assertNull(forwardBrowserUrl(ForwardKind.Local, "127.0.0.1", 0, null))
    }

    @Test
    fun remoteRulesHaveNoBrowserTarget() {
        assertNull(forwardBrowserUrl(ForwardKind.Remote, "127.0.0.1", 8080, 8080))
    }
}

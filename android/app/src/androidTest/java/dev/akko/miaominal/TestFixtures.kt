package dev.akko.miaominal

import dev.akko.miaominal.data.AuthMethod
import dev.akko.miaominal.data.ForwardKind
import dev.akko.miaominal.data.HostRuntimeStatus
import dev.akko.miaominal.data.KnownHostRecord
import dev.akko.miaominal.data.ManagedKeyRecord
import dev.akko.miaominal.data.ManagedKeySource
import dev.akko.miaominal.data.PortForwardRule
import dev.akko.miaominal.data.SessionProfile
import dev.akko.miaominal.data.SnippetRecord

object TestFixtures {
    const val PRIMARY_HOST_ID = "host-1"
    const val PRIMARY_HOST_NAME = "edge-sg-prod"
    const val PRIMARY_HOST_ADDRESS = "47.245.42.8"
    const val PRIMARY_HOST_USERNAME = "ubuntu"
    const val PRIMARY_HOST_PORT = 22

    private const val MINUTE = 60_000L
    private const val HOUR = 60 * MINUTE
    private const val DAY = 24 * HOUR

    fun hosts(): List<SessionProfile> {
        val now = System.currentTimeMillis()
        return listOf(
            SessionProfile(
                id = PRIMARY_HOST_ID,
                name = PRIMARY_HOST_NAME,
                group = "生产",
                tags = listOf("中继", "CI"),
                host = PRIMARY_HOST_ADDRESS,
                port = PRIMARY_HOST_PORT,
                username = PRIMARY_HOST_USERNAME,
                authMethod = AuthMethod.Password,
                isFavorite = true,
                lastConnectedAt = now - 2 * MINUTE
            ),
            SessionProfile(
                id = "host-2",
                name = "home-lab",
                group = "家庭",
                tags = listOf("NAS"),
                host = "192.168.1.8",
                port = 2222,
                username = "root",
                authMethod = AuthMethod.KeyFile,
                lastConnectedAt = now - 26 * HOUR
            ),
            SessionProfile(
                id = "host-3",
                name = "dev-box",
                group = "开发",
                host = "10.0.0.24",
                port = 22,
                username = "miaominal"
            )
        )
    }

    fun hostStatus(): Map<String, HostRuntimeStatus> = mapOf(
        PRIMARY_HOST_ID to HostRuntimeStatus(online = true, latencyMillis = 18),
        "host-2" to HostRuntimeStatus(online = true, latencyMillis = 3),
        "host-3" to HostRuntimeStatus(online = false)
    )

    fun snippets(): List<SnippetRecord> = listOf(
        SnippetRecord(
            id = "snippet-1",
            description = "列出当前目录",
            packageName = "日常巡检",
            script = "ls -la"
        ),
        SnippetRecord(
            id = "snippet-2",
            description = "查看运行状态",
            packageName = "日常巡检",
            script = "uptime"
        ),
        SnippetRecord(
            id = "snippet-3",
            description = "查看 Git 状态",
            packageName = "开发",
            script = "git status"
        )
    )

    fun forwardRules(): List<PortForwardRule> = listOf(
        PortForwardRule(
            id = "forward-1",
            profileId = PRIMARY_HOST_ID,
            label = "本地 PostgreSQL",
            kind = ForwardKind.Local,
            listenHost = "127.0.0.1",
            listenPort = 15432,
            targetHost = "127.0.0.1",
            targetPort = 5432,
            enabled = true
        )
    )

    fun managedKeys(): List<ManagedKeyRecord> = listOf(
        ManagedKeyRecord(
            id = "key-1",
            name = "edge-deploy",
            algorithm = "ssh-ed25519",
            publicKey = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAITestFixtureKeyMaterial",
            source = ManagedKeySource.Generated
        )
    )

    fun knownHosts(): List<KnownHostRecord> {
        val now = System.currentTimeMillis()
        return listOf(
            KnownHostRecord(
                host = PRIMARY_HOST_ADDRESS,
                port = PRIMARY_HOST_PORT,
                algorithm = "ssh-ed25519",
                fingerprint = "SHA256:test-fixture-fingerprint",
                addedAt = now - 2 * DAY
            )
        )
    }
}

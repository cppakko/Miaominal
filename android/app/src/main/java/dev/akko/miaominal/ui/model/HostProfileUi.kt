package dev.akko.miaominal.ui.model

import android.content.res.Resources
import androidx.annotation.StringRes
import dev.akko.miaominal.R
import dev.akko.miaominal.data.AuthMethod
import dev.akko.miaominal.data.HostRuntimeStatus
import dev.akko.miaominal.data.ProfileKind
import dev.akko.miaominal.data.SessionProfile
import dev.akko.miaominal.data.SessionEnvironmentVariable
import dev.akko.miaominal.data.ShellType

enum class ProfileKindUi(@param:StringRes val labelRes: Int) {
    Ssh(R.string.kind_ssh),
    Telnet(R.string.kind_telnet),
    Rdp(R.string.kind_rdp);

    fun toDomain(): ProfileKind = when (this) {
        Ssh -> ProfileKind.Ssh
        Telnet -> ProfileKind.Telnet
        Rdp -> ProfileKind.Rdp
    }

    companion object {
        fun fromDomain(kind: ProfileKind): ProfileKindUi = when (kind) {
            ProfileKind.Ssh, ProfileKind.Local -> Ssh
            ProfileKind.Telnet -> Telnet
            ProfileKind.Rdp -> Rdp
        }
    }
}

enum class AuthMethodUi(@param:StringRes val labelRes: Int) {
    Password(R.string.auth_password),
    KeyFile(R.string.auth_key_file),
    ManagedKey(R.string.auth_managed_key),
    Agent(R.string.auth_agent);

    fun toDomain(): AuthMethod = when (this) {
        Password -> AuthMethod.Password
        KeyFile -> AuthMethod.KeyFile
        ManagedKey -> AuthMethod.ManagedKey
        Agent -> AuthMethod.Agent
    }

    companion object {
        fun fromDomain(method: AuthMethod): AuthMethodUi = when (method) {
            AuthMethod.Password, AuthMethod.KeyboardInteractive -> Password
            AuthMethod.KeyFile -> KeyFile
            AuthMethod.ManagedKey -> ManagedKey
            AuthMethod.Agent -> Agent
        }
    }
}

data class HostProfileUi(
    val id: String = "",
    val name: String,
    val address: String,
    val username: String,
    val group: String,
    val tags: List<String> = emptyList(),
    val port: Int = 22,
    val kind: ProfileKindUi = ProfileKindUi.Ssh,
    val authMethod: AuthMethodUi = AuthMethodUi.Password,
    val privateKeyPath: String = "",
    val managedKeyId: String = "",
    val agentIdentity: String = "",
    val startupCommand: String = "",
    val environmentVariables: List<SessionEnvironmentVariable> = emptyList(),
    val shellType: ShellType = ShellType.Posix,
    val isFavorite: Boolean = false,
    val latency: String,
    val lastConnected: String,
    val online: Boolean,
    val entryProxyId: String = "",
    val proxyJumpProfileIds: List<String> = emptyList()
)

fun parseTags(text: String): List<String> = text
    .split(',', '，')
    .map { it.trim() }
    .filter { it.isNotEmpty() }
    .distinct()

fun SessionProfile.toUi(status: HostRuntimeStatus, resources: Resources): HostProfileUi =
    HostProfileUi(
        id = id,
        name = name,
        address = host,
        username = username,
        group = group,
        tags = tags,
        port = port,
        kind = ProfileKindUi.fromDomain(kind),
        authMethod = AuthMethodUi.fromDomain(authMethod),
        privateKeyPath = privateKeyPath,
        managedKeyId = managedKeyId,
        agentIdentity = agentIdentity,
        startupCommand = startupCommand,
        environmentVariables = environmentVariables,
        shellType = shellType,
        isFavorite = isFavorite,
        latency = formatLatency(resources, status.latencyMillis),
        lastConnected = formatLastConnected(resources, status.lastConnectedAt ?: lastConnectedAt),
        online = status.online,
        entryProxyId = entryProxyId,
        proxyJumpProfileIds = proxyJumpProfileIds
    )

fun HostProfileUi.toDomain(): SessionProfile = SessionProfile(
    id = id,
    name = name,
    group = group,
    tags = tags,
    kind = kind.toDomain(),
    host = address,
    port = port,
    username = username,
    authMethod = authMethod.toDomain(),
    privateKeyPath = privateKeyPath,
    managedKeyId = managedKeyId,
    agentIdentity = agentIdentity,
    startupCommand = startupCommand,
    environmentVariables = environmentVariables,
    shellType = shellType,
    isFavorite = isFavorite,
    entryProxyId = entryProxyId,
    proxyJumpProfileIds = proxyJumpProfileIds
)

private fun formatLatency(resources: Resources, latencyMillis: Int?): String =
    if (latencyMillis == null) {
        resources.getString(R.string.host_latency_unknown)
    } else {
        resources.getString(R.string.host_latency_value, latencyMillis)
    }

private fun formatLastConnected(resources: Resources, timestamp: Long?): String {
    if (timestamp == null) return ""
    val elapsed = (System.currentTimeMillis() - timestamp).coerceAtLeast(0)
    val minutes = elapsed / 60_000
    return when {
        minutes < 1 -> resources.getString(R.string.fm_just_now)
        minutes < 60 -> resources.getString(R.string.time_minutes_ago, minutes)
        minutes < 24 * 60 -> resources.getString(R.string.time_hours_ago, minutes / 60)
        minutes < 48 * 60 -> resources.getString(R.string.time_yesterday)
        else -> resources.getString(R.string.time_days_ago, minutes / (24 * 60))
    }
}

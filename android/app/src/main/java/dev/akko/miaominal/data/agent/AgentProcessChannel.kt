package dev.akko.miaominal.data.agent

import dev.akko.miaominal.data.ShellType
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.withTimeout
import kotlinx.coroutines.withContext
import java.io.ByteArrayOutputStream
import java.util.Base64
import java.util.UUID
import java.util.zip.Deflater
import java.util.zip.DeflaterOutputStream

/**
 * Configured processes read credentials from stdin before the protocol begins.
 * No SSH env request, secret argv, remote configuration file or home override is used.
 * Exactly one collector owns the native event stream, just as in the adapters.
 */
internal class AgentProcessChannel(
    private val delegate: AgentChannel,
    private val cli: AgentCli,
    private val shellType: ShellType,
    private val config: AgentConfigSnapshot?
) : AgentChannel by delegate {
    private val ready = CompletableDeferred<Unit>()
    private val marker = "MIAOMINAL_READY_${UUID.randomUUID().toString().replace("-", "")}"
    private val prelude = ByteArrayOutputStream()
    private val stderr = ByteArrayOutputStream()
    private var discardingStderrLine = false
    private var configuredId: Long = -1

    suspend fun launch(cwd: String): Long {
        if (config == null) return delegate.openExec(AgentCommands.launch(cliCommand(), cwd, shellType))
        val id = delegate.openExec(bootstrapCommand(cwd))
        configuredId = id
        try {
            // JSON is a single physical line; read never interprets shell metacharacters.
            delegate.writeStdin(id, bootstrapInput(cwd))
        } catch (error: Throwable) {
            withContext(NonCancellable) { closeChannel(id) }
            if (error is CancellationException) throw error
            throw IllegalStateException("Could not deliver the AI configuration.")
        }
        return id
    }

    suspend fun awaitReady(timeoutMs: Long) {
        if (config == null) return
        try { withTimeout(timeoutMs) { ready.await() } }
        catch (error: Throwable) {
            withContext(NonCancellable) { closeChannel(configuredId) }
            if (error is kotlinx.coroutines.CancellationException && error !is kotlinx.coroutines.TimeoutCancellationException) throw error
            throw IllegalStateException("AI configuration startup failed. Check the remote CLI and shell version.")
        }
    }

    override suspend fun closeChannel(channelId: Long) {
        if (config != null && channelId == configuredId &&
            shellType in listOf(ShellType.Posix, ShellType.Fish)) {
            runCatching { delegate.signal(channelId, "TERM") }
        }
        delegate.closeChannel(channelId)
    }

    override suspend fun close() {
        withContext(NonCancellable) {
            if (configuredId >= 0) runCatching { closeChannel(configuredId) }
            delegate.close()
        }
    }

    override fun events(): Flow<AgentChannelEventModel> = if (config == null) delegate.events() else flow {
        delegate.events().collect { event ->
            when {
                event is AgentChannelEventModel.Stdout && event.channelId == configuredId && !ready.isCompleted -> {
                    prelude.write(event.data)
                    val bytes = prelude.toByteArray()
                    val newline = bytes.indexOf('\n'.code.toByte())
                    if (newline >= 0) {
                        val line = String(bytes, 0, newline, Charsets.UTF_8).trimEnd('\r')
                        if (line != marker) {
                            ready.completeExceptionally(IllegalStateException("Unexpected AI startup output."))
                            emit(AgentChannelEventModel.ExecFailed(configuredId, "Unexpected AI startup output."))
                        } else {
                            ready.complete(Unit)
                            if (newline + 1 < bytes.size) emit(event.copy(data = bytes.copyOfRange(newline + 1, bytes.size)))
                        }
                        prelude.reset()
                    } else if (bytes.size > 65536) {
                        ready.completeExceptionally(IllegalStateException("AI startup output is too large."))
                        prelude.reset()
                    }
                }
                event is AgentChannelEventModel.Stderr && event.channelId == configuredId -> {
                    var chunk = event.data
                    if (discardingStderrLine) {
                        val end = chunk.indexOf('\n'.code.toByte())
                        if (end < 0) return@collect
                        discardingStderrLine = false
                        chunk = chunk.copyOfRange(end + 1, chunk.size)
                    }
                    stderr.write(chunk)
                    val bytes = stderr.toByteArray()
                    val newline = bytes.indexOfLast { it == '\n'.code.toByte() }
                    val safeEnd = if (newline >= 0) newline + 1 else 0
                    if (safeEnd > 0) {
                        val safe = config.redact(String(bytes, 0, safeEnd, Charsets.UTF_8))
                        emit(event.copy(data = safe.toByteArray(Charsets.UTF_8)))
                        stderr.reset()
                        stderr.write(bytes, safeEnd, bytes.size - safeEnd)
                    }
                    if (stderr.size() > 65536) {
                        // Drop the entire oversized line rather than disclose a
                        // credential prefix split at a truncation boundary.
                        stderr.reset()
                        discardingStderrLine = true
                    }
                }
                event is AgentChannelEventModel.ExecExited || event is AgentChannelEventModel.ChannelClosed ||
                    event is AgentChannelEventModel.ExecFailed || event is AgentChannelEventModel.Error ||
                    event is AgentChannelEventModel.Closed -> {
                    if (stderr.size() > 0) {
                        emit(AgentChannelEventModel.Stderr(configuredId, config.redact(stderr.toString("UTF-8")).toByteArray()))
                        stderr.reset()
                    }
                    ready.completeExceptionally(IllegalStateException("AI process exited during startup."))
                    emit(when (event) {
                        is AgentChannelEventModel.ExecFailed -> event.copy(message = config.redact(event.message))
                        is AgentChannelEventModel.Error -> event.copy(message = config.redact(event.message))
                        else -> event
                    })
                }
                else -> emit(event)
            }
        }
    }

    internal fun bootstrapCommand(cwd: String): String {
        check(config != null)
        require(cwd.none(Char::isISOControl)) { "The working directory contains a control character." }
        return when (shellType) {
            ShellType.Posix, ShellType.Fish -> {
                val script = "IFS= read -r $KEY_ENV || exit 70\n" +
                    "IFS= read -r OPENCODE_CONFIG_CONTENT || exit 70\n" +
                    "IFS= read -r miaominal_command || exit 70\n" +
                    "IFS= read -r miaominal_unused || exit 70\n" +
                    "export $KEY_ENV OPENCODE_CONFIG_CONTENT\n" +
                    "printf '%s\\n' '$marker'\n" + "exec sh -c \"\$miaominal_command\""
                // Fish and POSIX both understand a double-quoted, escaped argument.
                "sh -c " + doubleQuote(script)
            }
            ShellType.PowerShell, ShellType.Cmd -> {
                // Compress only our fixed program, never configuration data.
                // This also stays below CMD's 8191-character command limit.
                val bytes = ByteArrayOutputStream()
                val deflater = Deflater(Deflater.DEFAULT_COMPRESSION, true)
                try { DeflaterOutputStream(bytes, deflater).use { it.write(windowsScript().toByteArray(Charsets.UTF_8)) } }
                finally { deflater.end() }
                val compressed = Base64.getEncoder().encodeToString(bytes.toByteArray())
                val script = "\$miaominal_marker='$marker';" +
                    "\$m=[IO.MemoryStream]::new([Convert]::FromBase64String('$compressed'));" +
                    "\$d=[IO.Compression.DeflateStream]::new(\$m,[IO.Compression.CompressionMode]::Decompress);" +
                    "\$r=[IO.StreamReader]::new(\$d,[Text.Encoding]::UTF8);" +
                    "try { & ([scriptblock]::Create(\$r.ReadToEnd())) } finally { \$r.Dispose(); \$d.Dispose(); \$m.Dispose() }"
                if (shellType == ShellType.PowerShell) {
                    // Invoking a nested powershell.exe through a PowerShell
                    // host can activate mini-shell input/output serialization.
                    // Execute our fixed program in the SSH host instead.
                    script
                } else "powershell.exe -NoLogo -NoProfile -NonInteractive -Command \"$script\""
            }
        }
    }

    internal fun bootstrapInput(cwd: String): ByteArray {
        val windows = shellType == ShellType.PowerShell || shellType == ShellType.Cmd
        val invocation = if (windows) JsonCodec().stringify(arguments().map(::windowsQuote)) else
            AgentCommands.launch("exec " + posixInvocation(), cwd, ShellType.Posix)
        return (requireNotNull(config).keyForLaunch() + "\n" + openCodeConfig() + "\n" +
            invocation + "\n" + (if (windows) cwd else "") + "\n").toByteArray(Charsets.UTF_8)
    }

    internal fun arguments(): List<String> {
        val profile = requireNotNull(config).profile
        if (cli == AgentCli.OpenCode) return listOf("acp")
        val json = JsonCodec()
        return listOf("app-server", "-c", "model_provider=${json.stringify(PROVIDER)}",
            "-c", "model=${json.stringify(profile.defaultModel)}", "-c",
            "model_providers.$PROVIDER={name=\"Miaominal\",base_url=${json.stringify(profile.baseUrl)}," +
                "env_key=\"$KEY_ENV\",wire_api=\"responses\",requires_openai_auth=false}")
    }

    private fun posixInvocation() = (listOf(cli.id) + arguments()).joinToString(" ", transform = ::singleQuote)
    private fun cliCommand() = if (cli == AgentCli.Codex) "codex app-server" else "opencode acp"

    internal fun openCodeConfig(): String {
        val profile = config?.profile ?: return "{}"
        if (cli != AgentCli.OpenCode) return "{}"
        return JsonCodec().stringify(mapOf(
            "model" to "$PROVIDER/${profile.defaultModel}",
            "provider" to mapOf(PROVIDER to mapOf(
                "name" to profile.name,
                "npm" to if (profile.protocol == AgentApiProtocol.Responses) "@ai-sdk/openai" else "@ai-sdk/openai-compatible",
                "options" to mapOf("baseURL" to profile.baseUrl, "apiKey" to "{env:$KEY_ENV}"),
                "models" to profile.models.associateWith { mapOf("name" to it) }
            ))
        ))
    }

    private fun windowsScript(): String {
        // .NET reads only the four bootstrap lines, then pumps raw UTF-8 protocol
        // bytes. PowerShell's buffered text/native pipeline is never involved.
        return """
            ${'$'}ErrorActionPreference = 'Stop'
            ${'$'}ProgressPreference = 'SilentlyContinue'
            ${'$'}inputStream = [Console]::OpenStandardInput()
            Add-Type -TypeDefinition @'
            using System.IO;
            using System.Threading.Tasks;
            public static class MiaominalStdioPump {
                public static Task Start(Stream source, Stream target) {
                    return Task.Factory.StartNew(() => {
                        var buffer = new byte[8192];
                        int count;
                        while ((count = source.Read(buffer, 0, buffer.Length)) > 0) {
                            target.Write(buffer, 0, count);
                            target.Flush();
                        }
                    }, TaskCreationOptions.LongRunning);
                }
            }
            '@
            function Read-BootstrapLine {
                ${'$'}bytes = New-Object 'System.Collections.Generic.List[byte]'
                while ((${ '$' }b = ${'$'}inputStream.ReadByte()) -ne -1) {
                    if (${ '$' }b -eq 10) { return [Text.Encoding]::UTF8.GetString(${'$'}bytes.ToArray()) }
                    if (${'$'}bytes.Count -ge 65536) { throw 'AI bootstrap input is too large.' }
                    ${'$'}bytes.Add([byte]${ '$' }b)
                }
                throw 'AI bootstrap input ended.'
            }
            function Stop-AgentProcess(${'$'}process) {
                ${'$'}killInfo = New-Object Diagnostics.ProcessStartInfo
                ${'$'}killInfo.FileName = 'taskkill.exe'
                ${'$'}killInfo.Arguments = '/PID ' + ${'$'}process.Id + ' /T /F'
                ${'$'}killInfo.UseShellExecute = ${'$'}false
                ${'$'}killInfo.CreateNoWindow = ${'$'}true
                ${'$'}killInfo.RedirectStandardOutput = ${'$'}true
                ${'$'}killInfo.RedirectStandardError = ${'$'}true
                try {
                    ${'$'}killer = [Diagnostics.Process]::Start(${'$'}killInfo)
                    try { [void]${'$'}killer.WaitForExit(5000) } finally { ${'$'}killer.Dispose() }
                } catch { }
                if (-not ${'$'}process.HasExited) { ${'$'}process.Kill() }
                ${'$'}process.WaitForExit()
            }
            ${'$'}p = ${'$'}null
            ${'$'}started = ${'$'}false
            try {
                ${'$'}secret = Read-BootstrapLine
                ${'$'}inline = Read-BootstrapLine
                ${'$'}arguments = Read-BootstrapLine | ConvertFrom-Json
                ${'$'}directory = Read-BootstrapLine
                ${'$'}si = New-Object Diagnostics.ProcessStartInfo
                ${'$'}exe = Get-Command '${cli.id}.exe' -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
                if (${'$'}exe) {
                    ${'$'}si.FileName = ${'$'}exe.Source
                } else {
                    ${'$'}shim = Get-Command '${cli.id}' -ErrorAction Stop | Select-Object -First 1
                    ${'$'}root = Split-Path -Parent ${'$'}shim.Source
                    ${'$'}candidates = @()
                    if ('${cli.id}' -eq 'opencode') {
                        ${'$'}candidates += Join-Path ${'$'}root 'node_modules/opencode-ai/bin/opencode.exe'
                    } else {
                        ${'$'}arch = if (${'$'}env:PROCESSOR_ARCHITECTURE -eq 'ARM64') { 'aarch64' } else { 'x86_64' }
                        ${'$'}pkg = if (${'$'}arch -eq 'aarch64') { 'codex-win32-arm64' } else { 'codex-win32-x64' }
                        ${'$'}tail = 'vendor/' + ${'$'}arch + '-pc-windows-msvc/codex/codex.exe'
                        ${'$'}candidates += Join-Path ${'$'}root ('node_modules/@openai/codex/' + ${'$'}tail)
                        ${'$'}candidates += Join-Path ${'$'}root ('node_modules/@openai/codex/node_modules/@openai/' + ${'$'}pkg + '/' + ${'$'}tail)
                        ${'$'}candidates += Join-Path ${'$'}root ('node_modules/@openai/' + ${'$'}pkg + '/' + ${'$'}tail)
                    }
                    ${'$'}si.FileName = ${'$'}candidates | Where-Object { Test-Path -LiteralPath ${'$'}_ -PathType Leaf } | Select-Object -First 1
                    if (-not ${'$'}si.FileName) { throw 'The native CLI executable was not found.' }
                }
                ${'$'}si.Arguments = (${'$'}arguments -join ' ')
                ${'$'}si.UseShellExecute = ${'$'}false
                ${'$'}si.CreateNoWindow = ${'$'}true
                ${'$'}si.RedirectStandardInput = ${'$'}true
                ${'$'}si.RedirectStandardOutput = ${'$'}true
                ${'$'}si.RedirectStandardError = ${'$'}true
                ${'$'}si.EnvironmentVariables['$KEY_ENV'] = ${'$'}secret
                ${'$'}si.EnvironmentVariables['OPENCODE_CONFIG_CONTENT'] = ${'$'}inline
                if (${'$'}directory) { ${'$'}si.WorkingDirectory = ${'$'}directory }
                ${'$'}p = New-Object Diagnostics.Process
                ${'$'}p.StartInfo = ${'$'}si
                if (-not ${'$'}p.Start()) { throw 'AI process could not start.' }
                ${'$'}started = ${'$'}true
                ${'$'}secret = ${'$'}null
                ${'$'}inline = ${'$'}null
                ${'$'}outputStream = [Console]::OpenStandardOutput()
                ${'$'}ack = [Text.Encoding]::UTF8.GetBytes(${'$'}miaominal_marker + [char]10)
                ${'$'}outputStream.Write(${'$'}ack, 0, ${'$'}ack.Length)
                ${'$'}outputStream.Flush()
                ${'$'}stdoutTask = [MiaominalStdioPump]::Start(${'$'}p.StandardOutput.BaseStream, ${'$'}outputStream)
                ${'$'}stderrTask = [MiaominalStdioPump]::Start(${'$'}p.StandardError.BaseStream, [Console]::OpenStandardError())
                ${'$'}stdinTask = [MiaominalStdioPump]::Start(${'$'}inputStream, ${'$'}p.StandardInput.BaseStream)
                while (-not ${'$'}p.WaitForExit(100)) {
                    if (${'$'}stdinTask.IsCompleted) { break }
                }
                if (-not ${'$'}p.HasExited) {
                    Stop-AgentProcess ${'$'}p
                }
                [void]${'$'}stdoutTask.GetAwaiter().GetResult()
                [void]${'$'}stderrTask.GetAwaiter().GetResult()
                ${'$'}code = ${'$'}p.ExitCode
            } catch {
                [Console]::Error.WriteLine('AI startup failed. Check the CLI installation and configuration.')
                ${'$'}code = 70
            } finally {
                if (${'$'}p) {
                    if (${'$'}started -and -not ${'$'}p.HasExited) { Stop-AgentProcess ${'$'}p }
                    ${'$'}p.Dispose()
                }
            }
            exit ${'$'}code
        """.trimIndent().lineSequence().joinToString("\n") { it.trim() }
    }

    companion object {
        const val PROVIDER = "miaominal_phone"
        const val KEY_ENV = "MIAOMINAL_AI_API_KEY"
        private fun singleQuote(text: String) = "'" + text.replace("'", "'\\''") + "'"
        private fun doubleQuote(text: String) = "\"" + text.replace("\\", "\\\\")
            .replace("\"", "\\\"").replace("$", "\\$").replace("`", "\\`") + "\""
        internal fun windowsQuote(text: String): String = buildString {
            append('"')
            var slashes = 0
            for (c in text) {
                when (c) {
                    '\\' -> slashes++
                    '"' -> { repeat(slashes * 2 + 1) { append('\\') }; append(c); slashes = 0 }
                    else -> { repeat(slashes) { append('\\') }; append(c); slashes = 0 }
                }
            }
            repeat(slashes * 2) { append('\\') }
            append('"')
        }
    }
}

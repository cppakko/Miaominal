package dev.akko.miaominal.data.agent

import com.sun.net.httpserver.HttpServer
import dev.akko.miaominal.data.ShellType
import org.junit.Assert.*
import org.junit.Assume.assumeTrue
import org.junit.Test
import java.io.File
import java.net.InetSocketAddress
import java.nio.file.Files
import java.util.concurrent.CompletableFuture
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit

/** Runs the generated bootstrap with a fake native CLI and a loopback API. */
class AgentProcessIntegrationTest {
    private data class Request(val path: String, val auth: String, val body: String)

    @Test fun windowsShellsDeliverAllThreeApiConfigurationsWithoutFilesOrSecretArguments() {
        assumeTrue(System.getProperty("os.name").orEmpty().startsWith("Windows"))
        verify(listOf(ShellType.PowerShell, ShellType.Cmd))
    }

    @Test fun posixBootstrapDeliversTheSameContracts() {
        assumeTrue(System.getProperty("os.name").orEmpty().startsWith("Windows"))
        val posixShell = findPosixShell()
        assumeTrue("Put sh.exe or Git on PATH, or set MIAOMINAL_TEST_POSIX_SHELL to sh.exe", posixShell != null)
        verify(listOf(ShellType.Posix, ShellType.Fish), requireNotNull(posixShell))
    }

    private fun findPosixShell(): File? {
        // An explicit executable path also supports portable/custom Git installations.
        System.getenv("MIAOMINAL_TEST_POSIX_SHELL")?.takeIf(String::isNotBlank)?.let { path ->
            val shell = File(path).absoluteFile
            require(shell.isFile) { "MIAOMINAL_TEST_POSIX_SHELL must point to an existing sh.exe" }
            return shell
        }
        val path = System.getenv().entries.firstOrNull { it.key.equals("PATH", ignoreCase = true) }
            ?.value.orEmpty()
        val directories = path.split(File.pathSeparatorChar)
            .map { it.trim().trim('"') }.filter(String::isNotBlank).map(::File)
        val candidates = directories.map { File(it, "sh.exe") } + directories
            .filter { File(it, "git.exe").isFile }
            .map { File(it.parentFile, "usr/bin/sh.exe") }
        return candidates.firstOrNull(File::isFile)?.absoluteFile
    }

    private fun verify(shells: List<ShellType>, posixShell: File? = findPosixShell()) {
        val directory = Files.createTempDirectory("miaominal-agent-cli-").toFile()
        val requests = LinkedBlockingQueue<Request>()
        val server = HttpServer.create(InetSocketAddress("127.0.0.1", 0), 0)
        server.createContext("/") { exchange ->
            requests.add(Request(exchange.requestURI.path, exchange.requestHeaders.getFirst("Authorization"),
                exchange.requestBody.readBytes().toString(Charsets.UTF_8)))
            val bytes = "{}".toByteArray()
            exchange.sendResponseHeaders(200, bytes.size.toLong())
            exchange.responseBody.use { it.write(bytes) }
        }
        server.start()
        try {
            val source = directory.resolve("FakeCli.cs")
            source.writeText(javaClass.getResource("/agent/FakeCli.cs")!!.readText(), Charsets.UTF_8)
            val binary = directory.resolve("codex.exe")
            val compile = ProcessBuilder("powershell.exe", "-NoProfile", "-NonInteractive", "-Command",
                "\$ErrorActionPreference='Stop'; Add-Type -Path '${source.path.replace("'", "''")}' " +
                    "-OutputAssembly '${binary.path.replace("'", "''")}' -OutputType ConsoleApplication " +
                    "-ReferencedAssemblies System.dll,System.Core.dll,System.Web.Extensions.dll")
                .redirectErrorStream(true).start()
            assertTrue("Fixture compilation timed out", compile.waitFor(30, TimeUnit.SECONDS))
            assertEquals(compile.inputStream.bufferedReader().readText(), 0, compile.exitValue())
            binary.copyTo(directory.resolve("opencode.exe"))
            val preserved = directory.resolve("auth.json").apply { writeText("original fixture credentials") }
            val model = "custom/\$name%TEMP%'\"\\tail"
            val key = "fixture-\$key%TEMP%'\"token"
            val matrix = listOf(Triple(AgentCli.Codex, AgentApiProtocol.Responses, false),
                Triple(AgentCli.OpenCode, AgentApiProtocol.Responses, false),
                Triple(AgentCli.OpenCode, AgentApiProtocol.ChatCompletions, false))
            for (shell in shells) for ((cli, protocol, hold) in matrix +
                if (shell in listOf(ShellType.PowerShell, ShellType.Cmd))
                    listOf(Triple(AgentCli.Codex, AgentApiProtocol.Responses, true)) else emptyList()) {
                val snapshot = AgentConfigSnapshot(AgentConfigProfile("p", "Fixture", protocol,
                    "http://127.0.0.1:${server.address.port}/v1", listOf(model), model, true), key)
                val channel = AgentProcessChannel(FakeAgentChannel(), cli, shell, snapshot)
                val command = channel.bootstrapCommand(directory.path.replace('\\', '/'))
                // The local fixture environment carries only our fixed command,
                // never credentials/configuration. This avoids Java's nested
                // Windows argv quoting and temporary executable script files.
                // Production still sends its command through SSH exec.
                val script = directory.resolve("${shell.name}.launch.sh")
                val windows = shell in listOf(ShellType.PowerShell, ShellType.Cmd)
                if (!windows) script.writeText(command, Charsets.UTF_8)
                val invocation = when (shell) {
                    ShellType.Cmd -> listOf("cmd.exe", "/d", "/c", "%MIAOMINAL_FIXTURE_COMMAND%")
                    ShellType.PowerShell -> listOf("powershell.exe", "-NoProfile", "-NonInteractive", "-Command",
                        "& ([scriptblock]::Create(\$env:MIAOMINAL_FIXTURE_COMMAND))")
                    else -> listOf(requireNotNull(posixShell).path, script.path)
                }
                val builder = ProcessBuilder(invocation).directory(directory)
                if (windows) builder.environment()["MIAOMINAL_FIXTURE_COMMAND"] = command
                val pathKeys = builder.environment().keys.filter { it.equals("PATH", ignoreCase = true) }
                val pathValue = pathKeys.firstOrNull()?.let { builder.environment()[it] }.orEmpty()
                pathKeys.forEach { builder.environment().remove(it) }
                builder.environment()["Path"] = listOfNotNull(directory.path, posixShell?.parent, pathValue)
                    .filter(String::isNotEmpty).joinToString(File.pathSeparator)
                val process = builder.start()
                try {
                    val reader = process.inputStream.bufferedReader(Charsets.UTF_8)
                    process.outputStream.write(channel.bootstrapInput(directory.path.replace('\\', '/')))
                    process.outputStream.flush()
                    val ack = CompletableFuture.supplyAsync { reader.readLine() }.get(20, TimeUnit.SECONDS)
                    if (ack?.startsWith("MIAOMINAL_READY_") != true) {
                        val diagnostic = if (ack == null && process.waitFor(5, TimeUnit.SECONDS))
                            process.errorStream.bufferedReader().readText() else "stdout=$ack"
                        val exit = if (process.isAlive) "running" else process.exitValue().toString()
                        fail("$shell/$cli/$protocol/hold=$hold bootstrap did not acknowledge: exit=$exit, $diagnostic; " +
                            "command=${command.take(160)}")
                    }
                    process.outputStream.write("{\"id\":1,\"method\":\"fixture\",\"hold\":$hold}\n".toByteArray())
                    process.outputStream.flush()
                    val reply = CompletableFuture.supplyAsync { reader.readLine() }.get(20, TimeUnit.SECONDS)
                    if (reply == null) {
                        assertTrue("$shell/$cli output closed while process remained live", process.waitFor(5, TimeUnit.SECONDS))
                        fail("$shell/$cli produced no protocol output: exit=${process.exitValue()}, " +
                            process.errorStream.bufferedReader().readText())
                    }
                    val parsed = JsonCodec().parse(reply!!)
                    assertNotNull("$shell/$cli invalid protocol output: $reply", parsed)
                    requireNotNull(parsed)
                    assertEquals(model, parsed.mapAt("result")!!.stringAt("model"))
                    assertFalse(parsed.mapAt("result")!!.listAt("arguments").toString().contains(key))
                    val request = requests.poll(10, TimeUnit.SECONDS)!!
                    assertEquals("/v1/" + if (protocol == AgentApiProtocol.Responses) "responses" else "chat/completions", request.path)
                    assertEquals("Bearer $key", request.auth)
                    assertEquals(model, JsonCodec().parse(request.body)!!.stringAt("model"))
                    if (hold) process.outputStream.close()
                    assertTrue("$shell/$cli did not stop after stdin closed", process.waitFor(10, TimeUnit.SECONDS))
                    if (!hold) assertEquals(process.errorStream.bufferedReader().readText(), 0, process.exitValue())
                    val childPid = parsed.mapAt("result")!!.longAt("processId")!!
                    assertFalse("$shell/$cli left the child process running", ProcessHandle.of(childPid).map { it.isAlive }.orElse(false))
                    assertEquals("original fixture credentials", preserved.readText())
                } finally {
                    if (process.isAlive) {
                        val pid = Process::class.java.getMethod("pid").invoke(process)!!.toString()
                        val killer = ProcessBuilder("taskkill.exe", "/PID", pid, "/T", "/F")
                            .redirectErrorStream(true).start()
                        killer.waitFor(10, TimeUnit.SECONDS)
                    }
                    process.destroyForcibly()
                }
            }
            val expected = setOf("FakeCli.cs", "codex.exe", "opencode.exe", "auth.json") +
                shells.filter { it in listOf(ShellType.Posix, ShellType.Fish) }.map { "${it.name}.launch.sh" }
            assertEquals(expected, directory.list()!!.toSet())
        } finally {
            server.stop(0)
            directory.deleteRecursively()
        }
    }
}

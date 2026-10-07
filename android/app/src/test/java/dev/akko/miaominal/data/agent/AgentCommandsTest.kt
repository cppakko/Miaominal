package dev.akko.miaominal.data.agent

import dev.akko.miaominal.data.ShellType
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Test

class AgentCommandsTest {
    @Test
    fun buildsStartupCommandsForEveryShell() {
        assertEquals(
            "cd '/srv/my project' && codex app-server",
            AgentCommands.cdAndRun("/srv/my project", "codex app-server", ShellType.Posix)
        )
        assertEquals(
            "cd '/srv/my project'; and opencode acp",
            AgentCommands.cdAndRun("/srv/my project", "opencode acp", ShellType.Fish)
        )
        assertEquals(
            "Set-Location -LiteralPath 'C:\\Users\\O''Brien\\My Project'; " +
                "if (\$?) { codex app-server }",
            AgentCommands.cdAndRun(
                "C:\\Users\\O'Brien\\My Project",
                "codex app-server",
                ShellType.PowerShell
            )
        )
        assertEquals(
            "cd /d \"C:\\Work & Tools\\100%\" && opencode acp",
            AgentCommands.cdAndRun(
                "C:\\Work & Tools\\100%",
                "opencode acp",
                ShellType.Cmd
            )
        )
    }

    @Test
    fun launchesWithoutChangingDirectoryWhenNoWorkspaceIsChosen() {
        assertEquals(
            "codex app-server",
            AgentCommands.launch("codex app-server", null, ShellType.Posix)
        )
        assertEquals(
            "opencode acp",
            AgentCommands.launch("opencode acp", "  ", ShellType.PowerShell)
        )
        assertEquals(
            "cd /srv/project && codex app-server",
            AgentCommands.launch("codex app-server", "/srv/project", ShellType.Posix)
        )
    }

    @Test
    fun rejectsControlCharactersAndCmdQuotes() {
        assertThrows(IllegalArgumentException::class.java) {
            AgentCommands.cdAndRun("C:\\bad\npath", "codex app-server", ShellType.PowerShell)
        }
        assertThrows(IllegalArgumentException::class.java) {
            AgentCommands.cdAndRun("C:\\bad\"path", "codex app-server", ShellType.Cmd)
        }
        assertThrows(IllegalArgumentException::class.java) {
            AgentCommands.cdAndRun("C:\\%TEMP%\\work", "codex app-server", ShellType.Cmd)
        }
    }

    @Test
    fun convertsWindowsPathsBetweenSftpAndCliForms() {
        assertEquals(
            "/C:/Users/tester/My Project",
            AgentCommands.toSftpPath("C:\\Users\\tester\\My Project", ShellType.PowerShell)
        )
        assertEquals(
            "C:\\Users\\tester\\My Project",
            AgentCommands.fromSftpPath("/C:/Users/tester/My Project", ShellType.PowerShell)
        )
        assertEquals(
            "/srv/project",
            AgentCommands.fromSftpPath("/srv/project", ShellType.Posix)
        )
    }
}

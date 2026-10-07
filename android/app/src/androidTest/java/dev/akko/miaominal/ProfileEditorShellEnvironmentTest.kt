package dev.akko.miaominal

import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextInput
import dev.akko.miaominal.data.InMemoryMiaominalRepository
import dev.akko.miaominal.data.ShellType
import dev.akko.miaominal.ui.LocalMiaominalRepository
import dev.akko.miaominal.ui.model.HostProfileUi
import dev.akko.miaominal.ui.screens.ProfileEditorScreen
import dev.akko.miaominal.ui.screens.ProfileEnvironmentAddTag
import dev.akko.miaominal.ui.screens.ProfileSaveTag
import dev.akko.miaominal.ui.screens.profileEnvironmentNameTag
import dev.akko.miaominal.ui.screens.profileEnvironmentValueTag
import dev.akko.miaominal.ui.screens.profileShellOptionTag
import dev.akko.miaominal.ui.theme.MiaominalTheme
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test

class ProfileEditorShellEnvironmentTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun savesShellTypeAndEnvironmentVariables() {
        val repository = InMemoryMiaominalRepository()
        var saved: HostProfileUi? = null
        compose.setContent {
            CompositionLocalProvider(LocalMiaominalRepository provides repository) {
                MiaominalTheme {
                    ProfileEditorScreen(
                        initial = HostProfileUi(
                            id = "host-1",
                            name = "Windows host",
                            address = "windows.example",
                            username = "tester",
                            group = "",
                            latency = "--",
                            lastConnected = "",
                            online = false
                        ),
                        availableGroups = emptyList(),
                        onSave = { profile, _ ->
                            saved = profile
                            Result.success(Unit)
                        },
                        onDismiss = {}
                    )
                }
            }
        }

        compose.onNodeWithTag(profileShellOptionTag(ShellType.PowerShell))
            .performScrollTo()
            .performClick()
        compose.onNodeWithTag(ProfileEnvironmentAddTag).performClick()
        compose.onNodeWithTag(profileEnvironmentNameTag(0)).performTextInput("APP_ENV")
        compose.onNodeWithTag(profileEnvironmentValueTag(0)).performTextInput("mobile")
        compose.onNodeWithTag(ProfileSaveTag).performClick()
        compose.waitUntil(timeoutMillis = 5_000) { saved != null }

        assertEquals(ShellType.PowerShell, saved?.shellType)
        assertEquals("APP_ENV", saved?.environmentVariables?.single()?.name)
        assertEquals("mobile", saved?.environmentVariables?.single()?.value)
    }
}

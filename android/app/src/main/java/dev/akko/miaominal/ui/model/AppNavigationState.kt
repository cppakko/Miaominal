package dev.akko.miaominal.ui.model

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.saveable.listSaver

/** Presentation state only. Live sessions and drafts remain owned by their repositories. */
class AppNavigationState {
    var selectedName by mutableStateOf("Hosts")
    var editorForNewHost by mutableStateOf(false)
    var editorProfileId by mutableStateOf<String?>(null)
    var selectedToolName by mutableStateOf<String?>(null)
    var selectedSettingName by mutableStateOf<String?>(null)
    var agentSheetHostId by mutableStateOf<String?>(null)
    var agentChatVisible by mutableStateOf(false)
    var pendingClose by mutableStateOf<String?>(null)

    companion object {
        val Saver = listSaver<AppNavigationState, String>(save = {
            listOf(it.selectedName, it.editorForNewHost.toString(), it.editorProfileId.orEmpty(),
                it.selectedToolName.orEmpty(), it.selectedSettingName.orEmpty(),
                it.agentSheetHostId.orEmpty(), it.agentChatVisible.toString(), it.pendingClose.orEmpty())
        }, restore = { values -> AppNavigationState().apply {
            selectedName = values[0]; editorForNewHost = values[1].toBoolean()
            editorProfileId = values[2].ifEmpty { null }; selectedToolName = values[3].ifEmpty { null }
            selectedSettingName = values[4].ifEmpty { null }; agentSheetHostId = values[5].ifEmpty { null }
            agentChatVisible = values[6].toBoolean(); pendingClose = values[7].ifEmpty { null }
        } })
    }
}

package dev.akko.miaominal.ui.screens

import androidx.compose.foundation.clickable
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.relocation.BringIntoViewRequester
import androidx.compose.foundation.relocation.bringIntoViewRequester
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.*
import androidx.compose.ui.unit.dp
import dev.akko.miaominal.R
import dev.akko.miaominal.data.VaultMode
import dev.akko.miaominal.ui.LocalMiaominalRepository
import dev.akko.miaominal.ui.components.SectionTitle
import dev.akko.miaominal.ui.model.*
import kotlinx.coroutines.launch

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun MobileVaultSettingsPage(operations: SettingsOperations, modifier: Modifier, focusAnchor: String?) {
    val repository = LocalMiaominalRepository.current
    val status by repository.vault.collectAsState()
    val settings by repository.settings.collectAsState()
    val scope = rememberCoroutineScope()
    var disableConfirm by rememberSaveable { mutableStateOf(false) }
    var discardConfirm by rememberSaveable { mutableStateOf(false) }
    var autoLockOpen by rememberSaveable { mutableStateOf(false) }
    var settingsError by rememberSaveable { mutableStateOf<String?>(null) }
    val busy = operations.vaultBusy || status.busy
    val snackbar = remember { SnackbarHostState() }
    val completedMessage = stringResource(R.string.settings_operation_complete)
    var lastCompletion by rememberSaveable { mutableIntStateOf(operations.vaultCompleted) }
    LaunchedEffect(operations.vaultCompleted) {
        if (operations.vaultCompleted > lastCompletion) { lastCompletion = operations.vaultCompleted; snackbar.showSnackbar(completedMessage) }
    }
    val list = rememberLazyListState()
    val options = listOf(0 to R.string.desktop_vault_lock_off, 5 to R.string.vault_lock_5_min,
        15 to R.string.vault_lock_15_min, 60 to R.string.desktop_vault_lock_1_hour, 1440 to R.string.desktop_vault_lock_1_day)
    SettingsAnchorScroll(list, focusAnchor)
    Box(modifier) {
    LazyColumn(state = list, modifier = Modifier.fillMaxSize().testTag("vault-settings-list"), contentPadding = PaddingValues(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        item(key = "vault-status") { SectionTitle(stringResource(R.string.desktop_vault_status)) }
        item { SettingsDetailCard {
            Text(stringResource(when (status.mode) {
                VaultMode.Disabled -> R.string.desktop_vault_disabled
                VaultMode.Locked -> R.string.desktop_vault_locked
                VaultMode.Unlocked -> R.string.desktop_vault_unlocked
            }), style = MaterialTheme.typography.titleLarge)
            if (busy) LinearProgressIndicator(Modifier.fillMaxWidth())
            val error = operations.vaultError?.let(::describeSyncError) ?: status.error ?: settingsError
            error?.let { SettingsErrorText(it) }
            when (status.mode) {
                VaultMode.Disabled -> Button(enabled = !busy, onClick = { operations.openVault(VaultForm.Enable) }) { Text(stringResource(R.string.desktop_vault_enable)) }
                VaultMode.Locked -> Button(enabled = !busy, onClick = { operations.openVault(VaultForm.Unlock) }) { Text(stringResource(R.string.vault_unlock)) }
                VaultMode.Unlocked -> {
                    Button(enabled = !busy, onClick = { operations.lockVault(repository) }) { Text(stringResource(R.string.vault_lock_now)) }
                    OutlinedButton(enabled = !busy, onClick = { operations.openVault(VaultForm.Change) }) { Text(stringResource(R.string.settings_change_password)) }
                }
            }
        } }
        if (status.mode != VaultMode.Disabled) {
            item(key = "vault-auto-lock") { SectionTitle(stringResource(R.string.vault_auto_lock)) }
            item { ListItem(
                headlineContent = { Text(stringResource(R.string.vault_auto_lock)) },
                supportingContent = { Text(stringResource(options.firstOrNull { it.first == settings.vaultAutoLockMinutes }?.second ?: R.string.desktop_vault_lock_off)) },
                modifier = Modifier.clickable(enabled = !busy) { autoLockOpen = true }) }
            item { TextButton(enabled = !busy && status.mode == VaultMode.Unlocked, onClick = { disableConfirm = true }) { Text(stringResource(R.string.desktop_vault_disable)) } }
        }
    }
    SnackbarHost(snackbar, Modifier.align(androidx.compose.ui.Alignment.BottomCenter))
    }
    operations.vaultForm?.let { form ->
        val close = { if (!busy) { if (operations.passwordDirty) discardConfirm = true else operations.clearVaultForm() }; Unit }
        SettingsEditorPanel(title = stringResource(when (form) {
            VaultForm.Enable -> R.string.desktop_vault_enable
            VaultForm.Unlock -> R.string.vault_unlock
            VaultForm.Change -> R.string.settings_change_password
        }), busy = busy, onDismiss = close) {
            var visible by rememberSaveable { mutableStateOf(false) }
            val focus = LocalFocusManager.current
            val focusRequests = remember(form) { List(3) { FocusRequester() } }
            val errorBring = remember { BringIntoViewRequester() }
            LaunchedEffect(operations.vaultError) { if (operations.vaultError != null) errorBring.bringIntoView() }
            LaunchedEffect(form) { focusRequests[if (form == VaultForm.Change) 0 else 1].requestFocus() }
            LaunchedEffect(operations.passwordError) {
                operations.passwordError?.let { focusRequests[when (it) {
                    PasswordError.CurrentRequired -> 0
                    PasswordError.PasswordRequired -> 1
                    else -> 2
                }].requestFocus() }
            }
            val submit = { operations.submitVault(repository) }
            val fields = buildList {
                if (form == VaultForm.Change) add(Triple(R.string.settings_current_password, operations.currentPassword, 0))
                add(Triple(if (form == VaultForm.Unlock) R.string.desktop_vault_passphrase else R.string.settings_new_password, operations.password, 1))
                if (form != VaultForm.Unlock) add(Triple(R.string.desktop_vault_confirm_passphrase, operations.confirmation, 2))
            }
            fields.forEachIndexed { index, (label, value, field) ->
                val fieldError = operations.passwordError?.takeIf { when (it) {
                    PasswordError.CurrentRequired -> field == 0
                    PasswordError.PasswordRequired -> field == 1
                    PasswordError.ConfirmationRequired, PasswordError.Mismatch -> field == 2
                } }
                OutlinedTextField(value = value, onValueChange = { operations.updatePassword(field, it) }, enabled = !busy, label = { Text(stringResource(label)) }, singleLine = true,
                    isError = fieldError != null,
                    supportingText = { fieldError?.let { Text(stringResource(if (it == PasswordError.Mismatch) R.string.ui_the_master_passwords_do_not_match else R.string.settings_password_required)) } },
                    visualTransformation = if (visible) VisualTransformation.None else PasswordVisualTransformation(),
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, imeAction = if (index == fields.lastIndex) ImeAction.Done else ImeAction.Next),
                    keyboardActions = KeyboardActions(onNext = { focus.moveFocus(androidx.compose.ui.focus.FocusDirection.Next) }, onDone = { submit() }), modifier = Modifier.fillMaxWidth().focusRequester(focusRequests[field]))
            }
            SettingsToggleRow(title = stringResource(R.string.settings_show_password), checked = visible, onCheckedChange = { visible = it })
            Box(Modifier.bringIntoViewRequester(errorBring)) { operations.vaultError?.let { SettingsErrorText(describeSyncError(it)) } }
            Button(onClick = submit, enabled = !busy, modifier = Modifier.fillMaxWidth()) {
                Text(stringResource(when (form) { VaultForm.Enable -> R.string.desktop_vault_enable; VaultForm.Unlock -> R.string.vault_unlock; VaultForm.Change -> R.string.settings_change_password }))
            }
        }
    }
    if (discardConfirm) AlertDialog(onDismissRequest = { discardConfirm = false }, title = { Text(stringResource(R.string.settings_discard_title)) },
        confirmButton = { TextButton(onClick = { discardConfirm = false; operations.clearVaultForm() }) { Text(stringResource(R.string.settings_discard)) } },
        dismissButton = { TextButton(onClick = { discardConfirm = false }) { Text(stringResource(R.string.tool_action_cancel)) } })
    if (disableConfirm) AlertDialog(onDismissRequest = { disableConfirm = false }, title = { Text(stringResource(R.string.desktop_vault_disable)) },
        text = { Text(stringResource(R.string.settings_disable_vault_explanation)) },
        confirmButton = { TextButton(onClick = { disableConfirm = false; operations.disableVault(repository) }) { Text(stringResource(R.string.desktop_vault_disable)) } },
        dismissButton = { TextButton(onClick = { disableConfirm = false }) { Text(stringResource(R.string.tool_action_cancel)) } })
    if (autoLockOpen) ModalBottomSheet(onDismissRequest = { autoLockOpen = false }) {
        Text(stringResource(R.string.vault_auto_lock), style = MaterialTheme.typography.titleLarge, modifier = Modifier.padding(16.dp))
        options.forEach { (minutes, label) -> ListItem(headlineContent = { Text(stringResource(label)) },
            leadingContent = { RadioButton(selected = settings.vaultAutoLockMinutes == minutes, onClick = null) },
            modifier = Modifier.selectable(selected = settings.vaultAutoLockMinutes == minutes, role = Role.RadioButton, onClick = { autoLockOpen = false; scope.launch { repository.updateSettings { it.copy(vaultAutoLockMinutes = minutes) }.onFailure { settingsError = describeSyncError(it) } } })) }
    }
}

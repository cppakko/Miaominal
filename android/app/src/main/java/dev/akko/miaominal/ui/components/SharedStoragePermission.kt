package dev.akko.miaominal.ui.components

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Environment
import android.provider.Settings
import android.widget.Toast
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import dev.akko.miaominal.R

/** Android grants all files access from a settings page, rather than a runtime dialog. */
@Composable
fun rememberAllFilesAccessRequest(onReturn: () -> Unit = {}): () -> Unit {
    val context = LocalContext.current
    val launcher = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { onReturn() }
    return {
        try {
            launcher.launch(Intent(Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION,
                Uri.parse("package:${context.packageName}")))
        } catch (_: ActivityNotFoundException) {
            try {
                launcher.launch(Intent(Settings.ACTION_MANAGE_ALL_FILES_ACCESS_PERMISSION))
            } catch (_: ActivityNotFoundException) {
                Toast.makeText(context, R.string.sftp_storage_settings_unavailable, Toast.LENGTH_LONG).show()
                onReturn()
            }
        }
    }
}

@Composable
fun StartupStoragePermission(enabled: Boolean) {
    val context = LocalContext.current
    var visible by rememberSaveable { mutableStateOf(false) }
    val requestAccess = rememberAllFilesAccessRequest()
    LaunchedEffect(enabled) {
        if (!enabled || Environment.isExternalStorageManager()) return@LaunchedEffect
        val preferences = context.getSharedPreferences("permissions", Context.MODE_PRIVATE)
        if (!preferences.getBoolean("storage_asked", false)) {
            preferences.edit().putBoolean("storage_asked", true).apply()
            visible = true
        }
    }
    if (visible) AlertDialog(
        onDismissRequest = { visible = false },
        title = { Text(stringResource(R.string.sftp_storage_permission_title)) },
        text = { Text(stringResource(R.string.sftp_storage_permission_body)) },
        confirmButton = { TextButton(onClick = { visible = false; requestAccess() }) {
            Text(stringResource(R.string.sftp_storage_allow))
        } },
        dismissButton = { TextButton(onClick = { visible = false }) {
            Text(stringResource(R.string.sftp_storage_later))
        } }
    )
}

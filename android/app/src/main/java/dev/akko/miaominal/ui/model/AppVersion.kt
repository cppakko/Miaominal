package dev.akko.miaominal.ui.model

import android.content.Context
import android.content.pm.PackageManager
import android.os.Build

/**
 * Version name straight from the package manager.
 *
 * The product UI must never show a hardcoded milestone string: the APK version
 * comes from `android/version.properties`, independently of the Rust backend.
 */
fun appVersionName(context: Context): String = runCatching {
    val manager = context.packageManager
    val info = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        manager.getPackageInfo(context.packageName, PackageManager.PackageInfoFlags.of(0))
    } else {
        @Suppress("DEPRECATION")
        manager.getPackageInfo(context.packageName, 0)
    }
    info.versionName
}.getOrNull().orEmpty()

import miaominal.build.VerifyReleaseSigningTask
import java.util.Properties

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
}

val requestedAbis = (findProperty("miaominalAbis") as String?)
    ?.split(',', ';')
    ?.map(String::trim)
    ?.filter(String::isNotEmpty)
    ?.takeIf { it.isNotEmpty() }
    ?: listOf("arm64-v8a")


// `-PmiaominalTestBuildType=release` switches the instrumentation target to the
// R8-optimized release variant so the release smoke test runs against it.
// Regular runs keep testing the debug variant.
val requestedTestBuildType = (findProperty("miaominalTestBuildType") as String?)
    ?.trim()
    ?.takeIf(String::isNotEmpty)

// Android releases have their own version and monotonic install code. Neither
// depends on the bundled Rust workspace version.
val mobileVersionProperties = Properties().apply {
    load(providers.fileContents(rootProject.layout.projectDirectory.file("version.properties")).asText.get().reader())
}
val mobileVersionName = mobileVersionProperties.getProperty("versionName")?.trim()
    ?.takeIf { it.isNotEmpty() }
    ?: error("android/version.properties must define versionName")
val mobileVersionCode = mobileVersionProperties.getProperty("versionCode")?.trim()?.toIntOrNull()
    ?.takeIf { it in 1..2_100_000_000 }
    ?: error("android/version.properties versionCode must be an integer in 1..2100000000")

// Release signing material comes from the environment (CI secrets) or from a
// local, gitignored `android/keystore.properties`. Nothing here is committed,
// and an unconfigured release build fails instead of falling back to the
// debug signing key.
val signingProperties = Properties().apply {
    val local = File(projectDir.parentFile, "keystore.properties")
    if (local.isFile) {
        local.inputStream().use { load(it) }
    }
}

fun signingValue(environmentName: String, propertyName: String): String? =
    providers.environmentVariable(environmentName).orNull?.takeIf(String::isNotBlank)
        ?: signingProperties.getProperty(propertyName)?.takeIf(String::isNotBlank)

val releaseStoreFile: File? = signingValue(
    "MIAOMINAL_ANDROID_KEYSTORE_FILE",
    "storeFile",
)?.let { path ->
    File(path).let { file ->
        if (file.isAbsolute) file else projectDir.parentFile.resolve(file)
    }
}
val releaseStorePassword = signingValue(
    "MIAOMINAL_ANDROID_KEYSTORE_PASSWORD",
    "storePassword",
)
val releaseKeyAlias = signingValue("MIAOMINAL_ANDROID_KEY_ALIAS", "keyAlias")
val releaseKeyPassword = signingValue(
    "MIAOMINAL_ANDROID_KEY_PASSWORD",
    "keyPassword",
)
val hasReleaseSigning = releaseStoreFile?.isFile == true &&
    !releaseStorePassword.isNullOrBlank() &&
    !releaseKeyAlias.isNullOrBlank() &&
    !releaseKeyPassword.isNullOrBlank()

android {
    namespace = "dev.akko.miaominal"
    compileSdk {
        version = release(37) {
            minorApiLevel = 1
        }
    }

    defaultConfig {
        applicationId = "dev.akko.miaominal"
        minSdk = 31
        targetSdk = 37
        versionCode = mobileVersionCode
        versionName = mobileVersionName

        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"

        ndk {
            requestedAbis.forEach { abiFilters.add(it) }
        }
    }

    signingConfigs {
        if (hasReleaseSigning) {
            create("release") {
                storeFile = releaseStoreFile
                storePassword = releaseStorePassword
                keyAlias = releaseKeyAlias
                keyPassword = releaseKeyPassword
            }
        }
    }

    if (requestedTestBuildType != null) {
        require(requestedTestBuildType in listOf("debug", "release")) {
            "miaominalTestBuildType must be 'debug' or 'release', got '$requestedTestBuildType'"
        }
        testBuildType = requestedTestBuildType
    }
    buildTypes {
        release {
            signingConfig = signingConfigs.findByName("release")
            optimization {
                // R8 code and resource optimization.
                enable = true
            }
            ndk {
                // Keep a symbolized copy of the packaged native libraries.
                debugSymbolLevel = "FULL"
            }
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_11
        targetCompatibility = JavaVersion.VERSION_11
    }
    buildFeatures {
        compose = true
    }
}

val verifyReleaseSigning = tasks.register<VerifyReleaseSigningTask>("verifyReleaseSigning") {
    group = "verification"
    description =
        "Fails release builds when no release signing configuration is available."
    signingConfigured.set(hasReleaseSigning)
    failureMessage.set(
        "Release signing is not configured. Provide android/keystore.properties " +
            "(storeFile, storePassword, keyAlias, keyPassword) or the " +
            "MIAOMINAL_ANDROID_KEYSTORE_FILE / _KEYSTORE_PASSWORD / _KEY_ALIAS / " +
            "_KEY_PASSWORD environment variables. Release builds never fall back " +
            "to the debug signing key."
    )
}

// `preReleaseBuild` runs for every release packaging/install task; wiring the
// check to it makes unsigned release builds fail before packaging starts.
tasks.matching { it.name == "preReleaseBuild" }.configureEach {
    dependsOn(verifyReleaseSigning)
}

dependencies {
    implementation(project(":native"))
    implementation(platform(libs.androidx.compose.bom))
    implementation(libs.androidx.activity.compose)
    implementation("androidx.appcompat:appcompat:1.7.1")
    implementation(libs.androidx.compose.material3)
    implementation("androidx.compose.material3.adaptive:adaptive:1.2.0")
    implementation("androidx.window:window:1.4.0")
    implementation("androidx.customview:customview:1.2.0")
    implementation(libs.androidx.compose.material3.windowSizeClass)
    implementation(libs.androidx.compose.material.icons.extended)
    implementation(libs.androidx.compose.ui)
    implementation(libs.androidx.compose.ui.graphics)
    implementation(libs.androidx.compose.ui.tooling.preview)
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.lifecycle.runtime.ktx)
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.11.0")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.11.0")
    implementation(libs.androidx.material)
    implementation(libs.androidx.work.runtime.ktx)
    implementation(libs.moshi)
    implementation(libs.moshi.kotlin)
    testImplementation(libs.junit)
    testImplementation(libs.kotlinx.coroutines.test)
    androidTestImplementation(platform(libs.androidx.compose.bom))
    androidTestImplementation(libs.androidx.compose.ui.test.junit4)
    androidTestImplementation(libs.androidx.espresso.core)
    androidTestImplementation(libs.androidx.junit)
    debugImplementation(libs.androidx.compose.ui.test.manifest)
    debugImplementation(libs.androidx.compose.ui.tooling)
}

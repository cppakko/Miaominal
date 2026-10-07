import miaominal.build.CargoNdkBuildTask
import miaominal.build.UniffiBindgenTask

plugins {
    alias(libs.plugins.android.library)
}

// Native bridge toolchain baseline.
val ndkVersionValue = "30.0.16248370"
val bridgeCargoPackage = "miaominal-android"
val bindgenCargoPackage = "miaominal-uniffi-bindgen"
val bridgeLibraryName = "miaominal_android"
val minSdkValue = 31

val supportedAbis = mapOf(
    "arm64-v8a" to "aarch64-linux-android",
    "x86_64" to "x86_64-linux-android",
)

val requestedAbis = (findProperty("miaominalAbis") as String?)
    ?.split(',', ';')
    ?.map(String::trim)
    ?.filter(String::isNotEmpty)
    ?.takeIf { it.isNotEmpty() }
    ?: listOf("arm64-v8a")
requestedAbis.forEach { abi ->
    require(supportedAbis.containsKey(abi)) {
        "Unsupported ABI '$abi'; supported values: ${supportedAbis.keys.joinToString(", ")}"
    }
}

val repositoryRoot: File = rootProject.projectDir.parentFile
val cargoManifest = File(repositoryRoot, "Cargo.toml")
val ndkHomeProvider = providers.environmentVariable("ANDROID_NDK_HOME")
    .orElse(providers.environmentVariable("ANDROID_NDK_ROOT"))
    .orElse(
        androidComponents.sdkComponents.sdkDirectory
            .map { it.dir("ndk/$ndkVersionValue").asFile.absolutePath }
    )

android {
    namespace = "dev.akko.miaominal.nativebridge"
    compileSdk {
        version = release(37) {
            minorApiLevel = 1
        }
    }
    ndkVersion = ndkVersionValue
    defaultConfig {
        minSdk = minSdkValue
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_11
        targetCompatibility = JavaVersion.VERSION_11
    }
    buildFeatures {
        buildConfig = false
    }
}

val bindgenAbi = requestedAbis.first()

fun registerRustBuild(taskName: String, isRelease: Boolean): TaskProvider<CargoNdkBuildTask> {
    val outputDir = "rustJniLibs/" + if (isRelease) "release" else "debug"
    return tasks.register<CargoNdkBuildTask>(taskName) {
        group = "build"
        description =
            "Compiles the Miaominal native bridge (${if (isRelease) "release" else "debug"}, " +
                "${requestedAbis.joinToString()})"
        abis.set(requestedAbis)
        release.set(isRelease)
        cargoPackage.set(bridgeCargoPackage)
        ndkHome.set(ndkHomeProvider)
        manifestPath.set(cargoManifest.absolutePath)
        minSdk.set(minSdkValue)
        outputDirectory.set(layout.buildDirectory.dir(outputDir))
    }
}

fun registerBindgen(
    taskName: String,
    isRelease: Boolean,
    rustTask: TaskProvider<CargoNdkBuildTask>,
): TaskProvider<UniffiBindgenTask> {
    val outputDir = "generated/uniffi/" + if (isRelease) "release" else "debug"
    return tasks.register<UniffiBindgenTask>(taskName) {
        group = "build"
        description =
            "Generates Kotlin UniFFI bindings from the ${if (isRelease) "release" else "debug"} library"
        libraryFile.set(
            rustTask.flatMap { task ->
                task.outputDirectory.file("$bindgenAbi/lib$bridgeLibraryName.so")
            }
        )
        uniffiConfig.set(File(repositoryRoot, "crates/miaominal-android/uniffi.toml"))
        cargoLock.set(File(repositoryRoot, "Cargo.lock"))
        bindgenSources.from(
            fileTree(File(repositoryRoot, "crates/miaominal-uniffi-bindgen")) {
                include("**/*.rs")
                include("**/Cargo.toml")
            }
        )
        manifestPath.set(cargoManifest.absolutePath)
        bindgenPackage.set(bindgenCargoPackage)
        outputDirectory.set(layout.buildDirectory.dir(outputDir))
    }
}

val buildRustDebug = registerRustBuild("buildRustDebug", isRelease = false)
val buildRustRelease = registerRustBuild("buildRustRelease", isRelease = true)
val generateUniffiDebug =
    registerBindgen("generateUniffiDebug", isRelease = false, buildRustDebug)
val generateUniffiRelease =
    registerBindgen("generateUniffiRelease", isRelease = true, buildRustRelease)

androidComponents {
    onVariants { variant ->
        val isRelease = variant.buildType == "release"
        val rustTask = if (isRelease) buildRustRelease else buildRustDebug
        val bindgenTask = if (isRelease) generateUniffiRelease else generateUniffiDebug
        variant.sources.kotlin?.addGeneratedSourceDirectory(
            bindgenTask,
            UniffiBindgenTask::outputDirectory,
        )
        variant.sources.jniLibs?.addGeneratedSourceDirectory(
            rustTask,
            CargoNdkBuildTask::outputDirectory,
        )
    }
}

dependencies {
    api(libs.androidx.annotation)
    api(libs.kotlinx.coroutines.core)
    api("net.java.dev.jna:jna:5.19.1@aar")
}

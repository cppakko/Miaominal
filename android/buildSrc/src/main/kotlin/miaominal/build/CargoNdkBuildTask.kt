package miaominal.build

import org.gradle.api.DefaultTask
import org.gradle.api.file.DirectoryProperty
import org.gradle.api.provider.ListProperty
import org.gradle.api.provider.Property
import org.gradle.api.tasks.Input
import org.gradle.api.tasks.OutputDirectory
import org.gradle.api.tasks.TaskAction
import org.gradle.process.ExecOperations
import java.io.File
import javax.inject.Inject

/**
 * Compiles the Miaominal Rust bridge for the selected Android ABIs with `cargo ndk`.
 *
 * Cargo is the authoritative incremental checker, and its fingerprint includes the
 * Rust toolchain, features, profiles and `Cargo.lock`; Gradle cannot model all of
 * those. The task therefore never reports up-to-date and always re-runs `cargo`,
 * which is cheap when nothing changed but guarantees that a stale `.so` is never
 * paired with newer sources or bindings.
 */
abstract class CargoNdkBuildTask : DefaultTask() {
    init {
        outputs.upToDateWhen { false }
    }
    @get:Input
    abstract val abis: ListProperty<String>

    @get:Input
    abstract val release: Property<Boolean>

    @get:Input
    abstract val cargoPackage: Property<String>

    @get:Input
    abstract val ndkHome: Property<String>

    @get:Input
    abstract val manifestPath: Property<String>

    @get:Input
    abstract val minSdk: Property<Int>

    @get:OutputDirectory
    abstract val outputDirectory: DirectoryProperty

    @get:Inject
    abstract val execOperations: ExecOperations

    @TaskAction
    fun buildNativeLibrary() {
        val output = outputDirectory.get().asFile
        output.deleteRecursively()
        val manifest = File(manifestPath.get())
        execOperations.exec {
            workingDir = manifest.parentFile
            commandLine(
                buildList {
                    add("cargo")
                    add("ndk")
                    abis.get().forEach { abi ->
                        add("-t")
                        add(abi)
                    }
                    add("-P")
                    add(minSdk.get().toString())
                    add("-o")
                    add(output.absolutePath)
                    add("build")
                    if (release.get()) {
                        // The dedicated profile inherits release but keeps line
                        // tables so the delivered native library can be symbolized.
                        add("--profile")
                        add("android-release")
                    }
                    add("--locked")
                    add("--manifest-path")
                    add(manifest.absolutePath)
                    add("-p")
                    add(cargoPackage.get())
                }
            )
            environment("ANDROID_NDK_HOME", ndkHome.get())
        }
    }
}

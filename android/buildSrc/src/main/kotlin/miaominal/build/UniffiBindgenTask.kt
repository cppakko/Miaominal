package miaominal.build

import org.gradle.api.DefaultTask
import org.gradle.api.file.ConfigurableFileCollection
import org.gradle.api.file.DirectoryProperty
import org.gradle.api.file.RegularFileProperty
import org.gradle.api.provider.Property
import org.gradle.api.tasks.Input
import org.gradle.api.tasks.InputFile
import org.gradle.api.tasks.InputFiles
import org.gradle.api.tasks.OutputDirectory
import org.gradle.api.tasks.PathSensitive
import org.gradle.api.tasks.PathSensitivity
import org.gradle.api.tasks.TaskAction
import org.gradle.process.ExecOperations
import java.io.File
import javax.inject.Inject

/**
 * Runs the host-side `miaominal-uniffi-bindgen` against a freshly built native library.
 *
 * The task is up-to-date only when the library content *and* every input that can
 * change the generated Kotlin (crate `uniffi.toml`, the generator sources and
 * `Cargo.lock`) are unchanged, so edits to the Kotlin package name, Android
 * options or record generation flags always regenerate the bindings.
 */
abstract class UniffiBindgenTask : DefaultTask() {
    @get:InputFile
    abstract val libraryFile: RegularFileProperty

    @get:InputFile
    abstract val uniffiConfig: RegularFileProperty

    @get:InputFile
    abstract val cargoLock: RegularFileProperty

    @get:InputFiles
    @get:PathSensitive(PathSensitivity.RELATIVE)
    abstract val bindgenSources: ConfigurableFileCollection

    @get:Input
    abstract val manifestPath: Property<String>

    @get:Input
    abstract val bindgenPackage: Property<String>

    @get:OutputDirectory
    abstract val outputDirectory: DirectoryProperty

    @get:Inject
    abstract val execOperations: ExecOperations

    @TaskAction
    fun generateBindings() {
        val output = outputDirectory.get().asFile
        output.deleteRecursively()
        val manifest = File(manifestPath.get())
        execOperations.exec {
            workingDir = manifest.parentFile
            commandLine(
                "cargo",
                "run",
                "--locked",
                "--manifest-path",
                manifest.absolutePath,
                "-p",
                bindgenPackage.get(),
                "--",
                "generate",
                "--library",
                libraryFile.get().asFile.absolutePath,
                "--language",
                "kotlin",
                "--out-dir",
                output.absolutePath,
                "--no-format",
            )
        }
    }
}

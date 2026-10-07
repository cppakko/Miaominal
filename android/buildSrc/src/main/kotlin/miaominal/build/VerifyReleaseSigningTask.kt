package miaominal.build

import org.gradle.api.DefaultTask
import org.gradle.api.GradleException
import org.gradle.api.provider.Property
import org.gradle.api.tasks.Input
import org.gradle.api.tasks.TaskAction

/**
 * Fails a release build when no release signing configuration is available.
 *
 * The plan requires that release builds fail explicitly instead of silently
 * falling back to the debug signing key.
 */
abstract class VerifyReleaseSigningTask : DefaultTask() {
    @get:Input
    abstract val signingConfigured: Property<Boolean>

    @get:Input
    abstract val failureMessage: Property<String>

    @TaskAction
    fun verify() {
        if (!signingConfigured.get()) {
            throw GradleException(failureMessage.get())
        }
    }
}

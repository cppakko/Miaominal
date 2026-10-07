package dev.akko.miaominal.ui.theme

import androidx.compose.animation.core.CubicBezierEasing
import androidx.compose.animation.core.tween
import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * Pins the motion tokens to the published Material Design 3 values
 * (`md.sys.motion.easing.*` / `md.sys.motion.duration.*`) and to the Android
 * system surface transition parameters, so page transitions cannot drift away
 * from the spec unnoticed.
 */
class MiaominalMotionTest {
    @Test
    fun easingTokensMatchThePublishedControlPoints() {
        assertEquals(CubicBezierEasing(0.3f, 0f, 1f, 1f), MiaominalMotion.StandardAccelerate)
        assertEquals(CubicBezierEasing(0f, 0f, 0f, 1f), MiaominalMotion.StandardDecelerate)
        // Predictive back design guidance: cubic-bezier(.1, .1, 0, 1).
        assertEquals(CubicBezierEasing(0.1f, 0.1f, 0f, 1f), MiaominalMotion.SystemSurfaceEasing)
    }

    @Test
    fun durationTokensMatchTheMd3Scale() {
        assertEquals(100, MiaominalMotion.DurationShort2)
        assertEquals(300, MiaominalMotion.DurationMedium2)
        assertEquals(350, MiaominalMotion.DurationMedium3)
        assertEquals(450, MiaominalMotion.DurationLong1)
    }

    @Test
    fun fadeThroughLeavesBeforeTheIncomingContentAppears() {
        assertEquals(90, MiaominalMotion.FadeThroughOutMillis)
        assertEquals(210, MiaominalMotion.FadeThroughInMillis)
        // The two phases add up to the medium duration, so a fade through
        // finishes on the same clock as a surface transition would.
        assertEquals(
            MiaominalMotion.DurationMedium2,
            MiaominalMotion.FadeThroughOutMillis + MiaominalMotion.FadeThroughInMillis
        )
    }

    @Test
    fun surfaceTransitionSwapsAtTheThirtyFivePercentThreshold() {
        assertEquals(0.35f, MiaominalMotion.SystemSurfaceFadeThreshold)
        assertEquals(350, MiaominalMotion.SystemSurfaceDurationMillis)
        assertEquals(122, MiaominalMotion.SystemSurfaceFadeOutMillis)
        assertEquals(228, MiaominalMotion.SystemSurfaceFadeInMillis)
        assertEquals(
            MiaominalMotion.SystemSurfaceDurationMillis,
            MiaominalMotion.SystemSurfaceFadeOutMillis + MiaominalMotion.SystemSurfaceFadeInMillis
        )
        assertEquals(0.9f, MiaominalMotion.SystemSurfaceExitScale)
        assertEquals(1.1f, MiaominalMotion.SystemSurfaceEnterScale)
    }

    @Test
    fun surfaceSpecsUseTheSystemInterpolator() {
        val systemEasing = CubicBezierEasing(0.1f, 0.1f, 0f, 1f)
        assertEquals(
            tween<Float>(MiaominalMotion.SystemSurfaceDurationMillis, easing = systemEasing),
            MiaominalMotion.surfaceScale<Float>()
        )
        assertEquals(
            tween<Float>(MiaominalMotion.SystemSurfaceFadeOutMillis, easing = systemEasing),
            MiaominalMotion.surfaceFadeOut<Float>()
        )
        assertEquals(
            tween<Float>(
                MiaominalMotion.SystemSurfaceFadeInMillis,
                delayMillis = MiaominalMotion.SystemSurfaceFadeOutMillis,
                easing = systemEasing
            ),
            MiaominalMotion.surfaceFadeIn<Float>()
        )
    }

    @Test
    fun fadeThroughFadesUseAccelerateThenDelayedDecelerate() {
        assertEquals(
            tween<Float>(MiaominalMotion.FadeThroughOutMillis, easing = CubicBezierEasing(0.3f, 0f, 1f, 1f)),
            MiaominalMotion.fadeThroughOut<Float>()
        )
        assertEquals(
            tween<Float>(
                MiaominalMotion.FadeThroughInMillis,
                delayMillis = MiaominalMotion.FadeThroughOutMillis,
                easing = CubicBezierEasing(0f, 0f, 0f, 1f)
            ),
            MiaominalMotion.fadeThroughIn<Float>()
        )
    }

    @Test
    fun chromeCrossFadeRunsBothHalvesTogether() {
        assertEquals(
            tween<Float>(MiaominalMotion.DurationShort4, easing = CubicBezierEasing(0f, 0f, 0f, 1f)),
            MiaominalMotion.chromeFadeIn<Float>()
        )
        assertEquals(
            tween<Float>(MiaominalMotion.DurationShort4, easing = CubicBezierEasing(0.3f, 0f, 1f, 1f)),
            MiaominalMotion.chromeFadeOut<Float>()
        )
    }
}

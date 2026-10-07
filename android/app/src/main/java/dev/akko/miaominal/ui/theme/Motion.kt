package dev.akko.miaominal.ui.theme

import androidx.compose.animation.ContentTransform
import androidx.compose.animation.core.CubicBezierEasing
import androidx.compose.animation.core.Easing
import androidx.compose.animation.core.FiniteAnimationSpec
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.animation.togetherWith

/**
 * Motion tokens shared by every page transition.
 *
 * Full-screen page changes follow the Android system surface transition used by
 * predictive back and cross-activity animations: the outgoing surface scales
 * down to 90% and fades out over the first 35% of the animation, then the
 * incoming surface fades in while settling from 110% back to 100%. The two
 * surfaces are therefore never translucent at the same time.
 *
 * Top-level destinations keep the Material 3 fade through, and the easing and
 * duration tokens are the published `md.sys.motion.easing.*` /
 * `md.sys.motion.duration.*` values.
 */
object MiaominalMotion {
    // Easing tokens (`md.sys.motion.easing.*`).
    val StandardAccelerate: Easing = CubicBezierEasing(0.3f, 0f, 1f, 1f)
    val StandardDecelerate: Easing = CubicBezierEasing(0f, 0f, 0f, 1f)

    /**
     * Interpolator of the system surface transition, taken from the predictive
     * back design guidance (`cubic-bezier(.1, .1, 0, 1)`), which matches the
     * interpolator SystemUI uses.
     */
    val SystemSurfaceEasing: Easing = CubicBezierEasing(0.1f, 0.1f, 0f, 1f)

    // Duration tokens in milliseconds (`md.sys.motion.duration.*`).
    const val DurationShort2 = 100
    const val DurationShort4 = 200
    const val DurationMedium2 = 300
    const val DurationMedium3 = 350
    const val DurationLong1 = 450

    /** Total duration of one full-screen surface transition. */
    const val SystemSurfaceDurationMillis = DurationMedium3

    /**
     * Progress at which the outgoing surface has faded out and the incoming one
     * starts fading in. Neither surface is visible at this point, which is what
     * keeps the shared top bar from ever blending into itself.
     */
    const val SystemSurfaceFadeThreshold = 0.35f

    /** Scale the outgoing surface shrinks to, and the incoming one starts at. */
    const val SystemSurfaceExitScale = 0.9f
    const val SystemSurfaceEnterScale = 1.1f

    /**
     * Fade-through split from the MD3 spec: the outgoing content fades out over
     * the first 90ms and the incoming content fades in over the following
     * 210ms.
     */
    const val FadeThroughOutMillis = 90
    const val FadeThroughInMillis = 210

    /** Milliseconds the outgoing surface needs to reach the fade threshold. */
    val SystemSurfaceFadeOutMillis: Int
        get() = (SystemSurfaceDurationMillis * SystemSurfaceFadeThreshold).toInt()

    /** Milliseconds left for the incoming surface to fade in. */
    val SystemSurfaceFadeInMillis: Int
        get() = SystemSurfaceDurationMillis - SystemSurfaceFadeOutMillis

    /** Scale spec shared by both halves of the surface transition. */
    fun <T> surfaceScale(): FiniteAnimationSpec<T> =
        tween(SystemSurfaceDurationMillis, easing = SystemSurfaceEasing)

    /** Fade-out half: the outgoing surface is gone at the fade threshold. */
    fun <T> surfaceFadeOut(): FiniteAnimationSpec<T> =
        tween(SystemSurfaceFadeOutMillis, easing = SystemSurfaceEasing)

    /** Fade-in half: the incoming surface appears once the outgoing one has left. */
    fun <T> surfaceFadeIn(): FiniteAnimationSpec<T> =
        tween(SystemSurfaceFadeInMillis, delayMillis = SystemSurfaceFadeOutMillis, easing = SystemSurfaceEasing)

    /** Fade-through exit: a short fade on the accelerate curve. */
    fun <T> fadeThroughOut(): FiniteAnimationSpec<T> =
        tween(FadeThroughOutMillis, easing = StandardAccelerate)

    /** Fade-through entry: starts once the outgoing content has left. */
    fun <T> fadeThroughIn(): FiniteAnimationSpec<T> =
        tween(FadeThroughInMillis, delayMillis = FadeThroughOutMillis, easing = StandardDecelerate)

    /**
     * Chrome crossfade for persistent bars that swap their contents (the hosts
     * search capsule against the plain top bar). Both halves run at the same
     * time so the bar never disappears mid-swap.
     */
    fun <T> chromeFadeIn(): FiniteAnimationSpec<T> =
        tween(DurationShort4, easing = StandardDecelerate)

    fun <T> chromeFadeOut(): FiniteAnimationSpec<T> =
        tween(DurationShort4, easing = StandardAccelerate)

    /**
     * Android system surface transition for a full-screen page change: the
     * outgoing page shrinks and leaves, then the incoming page grows into place.
     */
    fun surfaceTransition(): ContentTransform = (
        scaleIn(
            animationSpec = surfaceScale(),
            initialScale = SystemSurfaceEnterScale
        ) + fadeIn(surfaceFadeIn())
        ) togetherWith (
        scaleOut(
            animationSpec = surfaceScale(),
            targetScale = SystemSurfaceExitScale
        ) + fadeOut(surfaceFadeOut())
        )
}

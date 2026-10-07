package dev.akko.miaominal.ui.components

import android.view.View
import android.os.Bundle
import androidx.core.view.accessibility.AccessibilityNodeInfoCompat
import androidx.customview.widget.ExploreByTouchHelper
import dev.akko.miaominal.R

/** Line nodes use the already merged frame; accessibility never issues additional render requests. */
internal class TerminalAccessibility(private val terminal: TerminalSurfaceView) : ExploreByTouchHelper(terminal) {
    override fun getVirtualViewAt(x: Float, y: Float): Int = terminal.accessibleRows()
        .firstOrNull { terminal.accessibleRowBounds(it).contains(x.toInt(), y.toInt()) } ?: INVALID_ID

    override fun getVisibleVirtualViews(virtualViewIds: MutableList<Int>) {
        virtualViewIds.addAll(terminal.accessibleRows())
    }

    override fun onPopulateNodeForVirtualView(id: Int, node: AccessibilityNodeInfoCompat) {
        node.text = terminal.accessibleRowText(id).ifBlank { " " }
        node.className = "android.widget.TextView"
        node.setBoundsInParent(terminal.accessibleRowBounds(id))
        node.addAction(AccessibilityNodeInfoCompat.AccessibilityActionCompat(
            AccessibilityNodeInfoCompat.ACTION_COPY, terminal.context.getString(R.string.ux_copy_line)))
    }

    override fun onPerformActionForVirtualView(id: Int, action: Int, arguments: Bundle?): Boolean =
        if (action == AccessibilityNodeInfoCompat.ACTION_COPY) { terminal.copyAccessibleRow(id); true } else false

    override fun onPopulateNodeForHost(node: AccessibilityNodeInfoCompat) {
        super.onPopulateNodeForHost(node)
        node.contentDescription = terminal.context.getString(R.string.ux_read_terminal)
        node.isScrollable = true
        node.addAction(AccessibilityNodeInfoCompat.AccessibilityActionCompat(
            AccessibilityNodeInfoCompat.ACTION_SCROLL_BACKWARD, terminal.context.getString(R.string.ux_previous_page)))
        node.addAction(AccessibilityNodeInfoCompat.AccessibilityActionCompat(
            AccessibilityNodeInfoCompat.ACTION_SCROLL_FORWARD, terminal.context.getString(R.string.ux_next_page)))
        node.addAction(AccessibilityNodeInfoCompat.AccessibilityActionCompat(
            R.id.terminal_accessibility_latest, terminal.context.getString(R.string.ux_latest)))
    }

    override fun performAccessibilityAction(host: View, action: Int, arguments: Bundle?): Boolean = when (action) {
        AccessibilityNodeInfoCompat.ACTION_SCROLL_BACKWARD -> { terminal.accessibleScroll(true); true }
        AccessibilityNodeInfoCompat.ACTION_SCROLL_FORWARD -> { terminal.accessibleScroll(false); true }
        R.id.terminal_accessibility_latest -> { terminal.session?.scrollTo(0); true }
        else -> super.performAccessibilityAction(host, action, arguments)
    }
}

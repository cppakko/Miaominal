package dev.akko.miaominal.ui.components

import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsAnimationCompat
import androidx.core.view.WindowInsetsCompat
import android.os.SystemClock
import dev.akko.miaominal.ui.model.terminalRowText

import android.content.ClipboardManager
import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Rect
import android.graphics.RenderNode
import android.graphics.Typeface
import android.text.InputType
import android.util.AttributeSet
import android.view.ActionMode
import android.view.KeyEvent
import android.view.Menu
import android.view.MenuItem
import android.view.MotionEvent
import android.view.View
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection
import dev.akko.miaominal.R
import dev.akko.miaominal.data.TerminalCellFlags
import dev.akko.miaominal.data.TerminalCellModel
import dev.akko.miaominal.data.TerminalFrameModel
import dev.akko.miaominal.data.TerminalModesModel
import dev.akko.miaominal.data.TerminalRowModel
import dev.akko.miaominal.data.TerminalSelectionKind
import dev.akko.miaominal.data.TerminalSelectionModel
import dev.akko.miaominal.data.TerminalSessionHandle
import dev.akko.miaominal.ui.model.TerminalGeometryPolicy
import dev.akko.miaominal.ui.model.TerminalGridSize
import dev.akko.miaominal.ui.model.TerminalScrollAccumulator
import dev.akko.miaominal.ui.model.terminalViewport
import java.nio.charset.StandardCharsets
import kotlin.math.abs
import kotlin.math.ceil
import kotlin.math.floor

/**
 * Grid renderer plus terminal input for one session.
 *
 * The view receives sparse render frames (full or changed rows), paints the
 * cell grid with monospace metrics, reports geometry changes so the session can
 * resize the remote PTY, and forwards IME/hardware input. Text is never turned
 * into a log list: only the current grid is stored.
 */
class TerminalSurfaceView @JvmOverloads constructor(
    context: Context,
    attrs: AttributeSet? = null
) : View(context, attrs) {

    var session: TerminalSessionHandle? = null
        set(value) {
            if (field === value) return
            field = value
            geometryPolicy.resetSession()
            rows.clear()
            rowNodes.clear()
            columns = 0
            lines = 0
            composingText = null
            invalidate()
        }

    /**
     * Input is only forwarded while the session is writable. The flag is
     * updated by the screen from the session phase, and every send is wrapped
     * defensively so a disconnect racing the UI cannot crash the app.
     */
    var inputEnabled: Boolean = true
        set(value) {
            if (field == value) return
            field = value
            if (!value) {
                composingText = null
                invalidate()
            }
        }

    /** Called when the layout/font metrics imply a new terminal size. */
    var onGeometryChanged: ((columns: Int, rows: Int) -> Unit)? = null

    /** Called with the selected text when the user picks "explode" in the selection menu. */
    var onExplodeSelection: ((String) -> Unit)? = null

    private val paint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        typeface = Typeface.MONOSPACE
        textSize = 14f * resources.displayMetrics.scaledDensity
    }
    private val backgroundPaint = Paint()
    private val geometryPolicy = TerminalGeometryPolicy()
    private val imeAnimations = mutableSetOf<WindowInsetsAnimationCompat>()

    private val terminalAccessibility by lazy { TerminalAccessibility(this) }
    private var accessibilityNotificationPending = false
    private var lastAccessibilityNotification = 0L
    private val notifyAccessibleContent = Runnable {
        accessibilityNotificationPending = false
        lastAccessibilityNotification = SystemClock.uptimeMillis()
        terminalAccessibility.invalidateRoot()
    }

    init {
        ViewCompat.setAccessibilityDelegate(this, terminalAccessibility)
        // Focus is required for both input paths: hardware key events are
        // dispatched to the focused view, and the IME attaches to the focused
        // text editor. Without these flags `requestFocus()` is a no-op.
        isFocusable = true
        isFocusableInTouchMode = true

        // Inset the grid from the view edges so text never touches the screen
        // border. `onDraw` still fills the whole view with the terminal
        // background, so the padding area stays seamless.
        val inset = (8f * resources.displayMetrics.density).toInt()
        setPadding(inset, inset, inset, inset)
        ViewCompat.setWindowInsetsAnimationCallback(this,
            object : WindowInsetsAnimationCompat.Callback(DISPATCH_MODE_CONTINUE_ON_SUBTREE) {
                override fun onPrepare(animation: WindowInsetsAnimationCompat) {
                    if (animation.typeMask and WindowInsetsCompat.Type.ime() != 0 && imeAnimations.add(animation)) {
                        geometryPolicy.beginImeAnimation()
                        removeCallbacks(settleGeometry)
                    }
                }

                override fun onProgress(insets: WindowInsetsCompat,
                    runningAnimations: MutableList<WindowInsetsAnimationCompat>): WindowInsetsCompat = insets

                override fun onEnd(animation: WindowInsetsAnimationCompat) {
                    if (!imeAnimations.remove(animation)) return
                    geometryPolicy.endImeAnimation()
                    if (!geometryPolicy.animating) {
                        removeCallbacks(settleGeometry)
                        // Compose's imePadding must finish laying out before measuring.
                        postOnAnimation(settleGeometry)
                    }
                }
            })
    }

    private var cellWidth = 1f
    private var cellHeight = 1f
    private var baseline = 0f

    private fun viewport() = terminalViewport(lines, cellHeight,
        (height - paddingTop - paddingBottom).toFloat(), cursorLine, displayOffset, modes.alternateScreen)

    private fun visibleRowCount(): Int =
        minOf(lines, floor((height - paddingTop - paddingBottom) / cellHeight).toInt())
            .coerceAtLeast(0)

    /**
     * Fallback for layouts without IME animation callbacks (manual window resize,
     * font changes). An initial grid is immediate; subsequent changes settle first.
     */
    private val settleGeometry = Runnable {
        reportGeometryIfChanged()
    }

    private var columns = 0
    private var lines = 0
    private var rows: MutableList<TerminalRowModel?> = mutableListOf()
    /** Recorded in row-local coordinates, so height animation only translates them. */
    private val rowNodes = mutableListOf<RenderNode?>()
    private var defaultFg = Color.WHITE
    private var defaultBg = Color.BLACK
    private var cursorVisible = false
    private var cursorLine = 0
    private var cursorColumn = 0
    private var selection: TerminalSelectionModel? = null
    private var displayOffset = 0
    private var modes = TerminalModesModel(
        bracketedPaste = false,
        appCursor = false,
        appKeypad = false,
        focusEvents = false,
        mouseProtocol = 0,
        mouseEncoding = 0,
        alternateScreen = false
    )
    private var composingText: String? = null

    private var selectionAnchor: Pair<Int, Int>? = null
    private var selecting = false
    /** Bumped whenever a new selection starts, so a stale ActionMode cannot clear it. */
    private var selectionGeneration = 0
    private var activeActionMode: ActionMode? = null
    private var downX = 0f
    private var downY = 0f
    private var lastDragY = 0f
    private val scrollMotion = TerminalScrollAccumulator()
    private var longPressScheduled = false

    private val longPress = Runnable {
        if (!longPressScheduled) return@Runnable
        val anchor = selectionAnchor ?: return@Runnable
        selecting = true
        selectionGeneration += 1
        // Select the word under the finger: a plain point would leave the
        // selection empty (invisible) until the user drags, and the backend
        // collapses selections that start outside the grid.
        session?.startSelection(anchor.first, anchor.second, SIDE_LEFT, TerminalSelectionKind.SEMANTIC)
        invalidate()
    }

    fun applyFrame(frame: TerminalFrameModel) {
        val backgroundChanged = defaultBg != frame.defaultBg
        val geometryChanged = columns != frame.columns || rows.size != frame.lines
        // Scroll frames change row positions, but most cell runs are unchanged.
        // Retain only recordings from the previous grid, including duplicate rows.
        val reusable = if (!backgroundChanged && !geometryChanged &&
            (frame.full || displayOffset != frame.displayOffset || frame.rows.size > rows.size / 2)) {
            buildMap<List<TerminalCellModel>, RenderNode> {
                rows.forEachIndexed { index, row ->
                    val node = rowNodes.getOrNull(index)
                    if (row != null && node != null && node.hasDisplayList()) put(row.cells, node)
                }
            }
        } else null
        columns = frame.columns
        lines = frame.lines
        defaultFg = frame.defaultFg
        defaultBg = frame.defaultBg
        cursorVisible = frame.cursorVisible
        cursorLine = frame.cursorLine
        cursorColumn = frame.cursorColumn
        selection = frame.selection
        displayOffset = frame.displayOffset
        modes = frame.modes
        if (frame.full || geometryChanged) {
            rows = MutableList(frame.lines) { null }
            rowNodes.clear()
            repeat(frame.lines) { rowNodes.add(null) }
        } else if (backgroundChanged) {
            clearRowRecordings()
        }
        frame.rows.forEach { row ->
            if (row.row in rows.indices && rows[row.row] != row) {
                rows[row.row] = row
                rowNodes[row.row] = reusable?.get(row.cells)
            }
        }
        if (!accessibilityNotificationPending && frame.rows.isNotEmpty() &&
            (context.getSystemService(Context.ACCESSIBILITY_SERVICE) as android.view.accessibility.AccessibilityManager).isEnabled) {
            accessibilityNotificationPending = true
            postDelayed(notifyAccessibleContent, (1000L - (SystemClock.uptimeMillis() - lastAccessibilityNotification)).coerceAtLeast(0L))
        }
        invalidate()
    }

    internal fun accessibleRows(): List<Int> {
        val viewport = viewport()
        return (viewport.firstRow until viewport.endRowExclusive).filter { rows.getOrNull(it) != null }
    }
    internal fun accessibleRowText(id: Int): String = rows.getOrNull(id)?.let(::terminalRowText).orEmpty()
    internal fun accessibleRowBounds(id: Int): Rect {
        val top = paddingTop + viewport().rowTop(id)
        return Rect(paddingLeft, floor(top).toInt().coerceAtLeast(paddingTop), width - paddingRight,
            ceil(top + cellHeight).toInt().coerceAtMost(height - paddingBottom))
    }
    internal fun copyAccessibleRow(id: Int) {
        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as android.content.ClipboardManager
        clipboard.setPrimaryClip(android.content.ClipData.newPlainText("terminal", accessibleRowText(id)))
    }
    internal fun accessibleScroll(previous: Boolean) { session?.scroll(if (previous) visibleRowCount() else -visibleRowCount()) }
    override fun dispatchHoverEvent(event: android.view.MotionEvent): Boolean =
        terminalAccessibility.dispatchHoverEvent(event) || super.dispatchHoverEvent(event)

    /** Re-report the grid implied by the current layout (after switching sessions). */
    fun notifyGeometry() {
        reportGeometryIfChanged(force = true)
    }

    /** Reports the current grid unless the backend already knows it. */
    private fun reportGeometryIfChanged(force: Boolean = false) {
        if (width <= 0 || height <= 0) return
        val callback = onGeometryChanged ?: return
        val (columns, lines) = terminalSize()
        val size = geometryPolicy.takeReport(TerminalGridSize(columns, lines), force) ?: return
        callback(size.columns, size.rows)
    }

    private fun sendBytes(bytes: ByteArray) {
        val session = if (inputEnabled) session else return
        runCatching { session?.writeBytes(bytes) }
    }

    private fun sendKey(
        key: String,
        keyChar: String?,
        control: Boolean,
        alt: Boolean,
        shift: Boolean,
        phase: Int
    ): Boolean {
        val session = if (inputEnabled) session else return false
        return runCatching { session?.handleKey(key, keyChar, control, alt, shift, phase) }
            .getOrNull() ?: false
    }

    private fun sendPaste(text: String) {
        val session = if (inputEnabled) session else return
        runCatching { session?.paste(text) }
    }

    fun setTextSizePx(sizePx: Float) {
        if (abs(paint.textSize - sizePx) < 0.5f) return
        paint.textSize = sizePx
        updateMetrics()
        clearRowRecordings()
        invalidate()
        // A different cell size changes how many columns and lines fit, and the
        // view keeps its pixel size, so onSizeChanged never fires: report the new
        // grid explicitly or the backend keeps resizing to the old one.
        scheduleGeometry()
    }

    private fun updateMetrics() {
        cellWidth = ceil(paint.measureText("M")).coerceAtLeast(1f)
        val metrics = paint.fontMetrics
        cellHeight = ceil(metrics.descent - metrics.ascent).coerceAtLeast(1f)
        baseline = -metrics.ascent
    }

    private fun terminalSize(): Pair<Int, Int> {
        if (cellWidth <= 1f || cellHeight <= 1f) updateMetrics()
        val visibleColumns = floor((width - paddingLeft - paddingRight) / cellWidth).toInt()
        val visibleLines = floor((height - paddingTop - paddingBottom) / cellHeight).toInt()
        return visibleColumns.coerceAtLeast(2) to visibleLines.coerceAtLeast(1)
    }

    override fun onSizeChanged(w: Int, h: Int, oldw: Int, oldh: Int) {
        super.onSizeChanged(w, h, oldw, oldh)
        if (geometryPolicy.needsInitialReport) reportGeometryIfChanged()
        scheduleGeometry()
    }

    private fun scheduleGeometry() {
        removeCallbacks(settleGeometry)
        if (!geometryPolicy.animating) postDelayed(settleGeometry, GEOMETRY_SETTLE_MILLIS)
    }

    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        clearRowRecordings()
        invalidate()
        scheduleGeometry()
    }

    override fun onWindowVisibilityChanged(visibility: Int) {
        super.onWindowVisibilityChanged(visibility)
        if (visibility == VISIBLE) {
            // Android can release hardware recordings while the window is hidden.
            // Restore from the retained grid on the first draw, even for an idle session.
            clearRowRecordings()
            invalidate()
        }
    }

    override fun onDetachedFromWindow() {
        removeCallbacks(notifyAccessibleContent)
        accessibilityNotificationPending = false
        removeCallbacks(settleGeometry)
        imeAnimations.clear()
        geometryPolicy.detach()
        super.onDetachedFromWindow()
    }

    override fun onDraw(canvas: Canvas) {
        canvas.drawColor(defaultBg)
        if (cellHeight <= 1f) updateMetrics()
        val viewport = viewport()
        val gridSave = canvas.save()
        canvas.clipRect(paddingLeft, paddingTop, width - paddingRight, height - paddingBottom)
        for (rowIndex in viewport.firstRow until viewport.endRowExclusive) {
            val row = rows.getOrNull(rowIndex) ?: continue
            val top = paddingTop + viewport.rowTop(rowIndex)
            val save = canvas.save()
            canvas.translate(paddingLeft.toFloat(), top)
            if (canvas.isHardwareAccelerated) {
                val node = rowNodes[rowIndex]?.takeIf { it.hasDisplayList() } ?: RenderNode("terminal-row").also {
                    it.setPosition(0, 0, ceil(row.cells.size * cellWidth).toInt(), ceil(cellHeight).toInt())
                    it.setClipToBounds(false)
                    val recording = it.beginRecording()
                    try { drawRow(recording, row) } finally { it.endRecording() }
                    rowNodes[rowIndex] = it
                }
                canvas.drawRenderNode(node)
            } else {
                // Bitmap screenshots / software canvases cannot draw RenderNodes.
                drawRow(canvas, row)
            }
            canvas.restoreToCount(save)
        }
        drawCursor(canvas)
        drawComposing(canvas)
        canvas.restoreToCount(gridSave)
    }

    private fun clearRowRecordings() {
        rowNodes.indices.forEach { rowNodes[it] = null }
    }

    private fun drawRow(canvas: Canvas, row: TerminalRowModel) {
        // Background runs first so glyphs sit on top.
        var column = 0
        while (column < row.cells.size) {
            val cell = row.cells[column]
            if (cell.bg == defaultBg || cell.flags and TerminalCellFlags.SPACER != 0) {
                column++
                continue
            }
            var span = 1
            while (column + span < row.cells.size &&
                row.cells[column + span].bg == cell.bg &&
                row.cells[column + span].flags and TerminalCellFlags.SPACER == 0
            ) {
                span++
            }
            backgroundPaint.color = cell.bg
            canvas.drawRect(
                column * cellWidth,
                0f,
                (column + span) * cellWidth,
                cellHeight,
                backgroundPaint
            )
            column += span
        }
        for ((columnIndex, cell) in row.cells.withIndex()) {
            if (cell.flags and TerminalCellFlags.SPACER != 0) continue
            val decorated = cell.flags and (TerminalCellFlags.UNDERLINE or TerminalCellFlags.STRIKETHROUGH) != 0
            if (cell.text.isBlank() && !decorated) continue
            paint.color = cell.fg
            paint.isFakeBoldText = cell.flags and TerminalCellFlags.BOLD != 0
            paint.textSkewX = if (cell.flags and TerminalCellFlags.ITALIC != 0) -0.25f else 0f
            paint.alpha = if (cell.flags and TerminalCellFlags.DIM != 0) 160 else 255
            if (cell.flags and TerminalCellFlags.SEARCH_CURRENT != 0) {
                paint.color = Color.BLACK
            }
            val x = columnIndex * cellWidth
            if (!cell.text.isBlank()) canvas.drawText(cell.text, x, baseline, paint)
            if (cell.flags and TerminalCellFlags.UNDERLINE != 0) {
                canvas.drawRect(x, baseline + 2f, x + cellWidth, baseline + 4f, paint)
            }
            if (cell.flags and TerminalCellFlags.STRIKETHROUGH != 0) {
                canvas.drawRect(x, baseline - cellHeight / 3f, x + cellWidth, baseline - cellHeight / 3f + 2f, paint)
            }
            paint.alpha = 255
            paint.isFakeBoldText = false
            paint.textSkewX = 0f
        }
    }

    private fun drawCursor(canvas: Canvas) {
        if (!hasFocus() || !cursorVisible) return
        val viewport = viewport()
        if (cursorLine !in viewport.firstRow until viewport.endRowExclusive) return
        val left = paddingLeft + cursorColumn * cellWidth
        val top = paddingTop + viewport.rowTop(cursorLine)
        paint.color = if (modes.appCursor) 0xFF7CCFA5.toInt() else 0xFFE8EAED.toInt()
        canvas.drawRect(left, top + cellHeight - 3f, left + cellWidth, top + cellHeight, paint)
    }

    private fun drawComposing(canvas: Canvas) {
        val composing = composingText ?: return
        if (composing.isEmpty()) return
        val viewport = viewport()
        if (cursorLine !in viewport.firstRow until viewport.endRowExclusive) return
        paint.color = Color.WHITE
        paint.alpha = 220
        canvas.drawText(
            composing,
            paddingLeft + cursorColumn * cellWidth,
            paddingTop + viewport.rowTop(cursorLine) + baseline,
            paint
        )
        paint.alpha = 255
    }

    // ------------------------------------------------------------------
    // Input
    // ------------------------------------------------------------------

    private fun cellAt(x: Float, y: Float): Pair<Int, Int> {
        val frameRow = viewport().rowAt(y - paddingTop)
        val column = floor((x - paddingLeft) / cellWidth).toInt().coerceAtLeast(0)
        // Touch positions are view rows and the backend expects buffer lines
        // (negative values are scrollback), so shift by the row drawn at the top
        // and by the current scroll offset. Everything is clamped to the grid the
        // backend actually has: a stale grid (font change, keyboard animation)
        // can be shorter than the view, and a line outside the grid makes the
        // selection collapse to nothing.
        val line = frameRow - displayOffset
        return line to column.coerceIn(0, (columns - 1).coerceAtLeast(0))
    }

    /** Selection bounds in view coordinates, used to anchor the floating toolbar. */
    private fun selectionBoundsInView(): Rect? {
        val range = selection ?: return null
        val viewport = viewport()
        if (viewport.endRowExclusive <= viewport.firstRow) return null
        val topRow = range.startLine.coerceIn(viewport.firstRow, viewport.endRowExclusive - 1)
        val bottomRow = range.endLine.coerceIn(viewport.firstRow, viewport.endRowExclusive - 1)
        return Rect(
            (paddingLeft + range.startColumn * cellWidth).toInt(),
            (paddingTop + viewport.rowTop(topRow)).toInt().coerceAtLeast(paddingTop),
            (paddingLeft + (range.endColumn + 1) * cellWidth).toInt(),
            (paddingTop + viewport.rowTop(bottomRow + 1)).toInt().coerceAtMost(height - paddingBottom)
        )
    }

    override fun onTouchEvent(event: MotionEvent): Boolean {
        val session = session ?: return false
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                downX = event.x
                downY = event.y
                lastDragY = event.y
                scrollMotion.reset()
                selectionAnchor = cellAt(event.x, event.y)
                selecting = false
                longPressScheduled = true
                postDelayed(longPress, 450)
                return true
            }
            MotionEvent.ACTION_MOVE -> {
                if (selecting) {
                    val (row, column) = cellAt(event.x, event.y)
                    session.updateSelection(row, column, SIDE_RIGHT)
                    invalidate()
                } else {
                    val totalDelta = event.y - downY
                    if (abs(totalDelta) > cellHeight) {
                        longPressScheduled = false
                        removeCallbacks(longPress)
                    }
                    val deltaY = event.y - lastDragY
                    val lineDelta = scrollMotion.move(deltaY, cellHeight)
                    lastDragY = event.y
                    if (lineDelta != 0) {
                        session.scroll(lineDelta)
                    }
                }
                return true
            }
            MotionEvent.ACTION_CANCEL, MotionEvent.ACTION_UP -> {
                longPressScheduled = false
                removeCallbacks(longPress)
                if (selecting) {
                    startSelectionActionMode()
                } else if (abs(event.x - downX) < cellWidth && abs(event.y - downY) < cellHeight) {
                    session.clearSelection()
                    // The selection is gone: dismiss the floating toolbar too.
                    activeActionMode?.finish()
                    requestFocus()
                    showSoftInput()
                }
                selecting = false
                return true
            }
        }
        return super.onTouchEvent(event)
    }

    private fun showSoftInput() {
        val manager = context.getSystemService(Context.INPUT_METHOD_SERVICE) as? android.view.inputmethod.InputMethodManager
        manager?.showSoftInput(this, android.view.inputmethod.InputMethodManager.SHOW_IMPLICIT)
    }
    override fun onCheckIsTextEditor(): Boolean = true

    override fun onCreateInputConnection(outAttrs: EditorInfo): InputConnection {
        outAttrs.inputType = InputType.TYPE_CLASS_TEXT or
            InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS or
            InputType.TYPE_TEXT_FLAG_MULTI_LINE
        outAttrs.imeOptions = EditorInfo.IME_FLAG_NO_FULLSCREEN
        return TerminalInputConnection()
    }

    override fun onFocusChanged(gainFocus: Boolean, direction: Int, previouslyFocusedRect: android.graphics.Rect?) {
        super.onFocusChanged(gainFocus, direction, previouslyFocusedRect)
        session?.focused = gainFocus
        invalidate()
    }

    /** Type text without pressing Enter; used for snippets. */
    fun typeText(text: String) {
        if (text.isEmpty() || !inputEnabled) return
        requestFocus()
        showSoftInput()
        sendBytes(text.toByteArray(StandardCharsets.UTF_8))
    }

    fun copySelection(): String? = runCatching { session?.selectionText() }.getOrNull()

    fun pasteFromClipboard() {
        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as? ClipboardManager ?: return
        val text = clipboard.primaryClip?.getItemAt(0)?.coerceToText(context)?.toString() ?: return
        if (text.isNotEmpty()) sendPaste(text)
    }

    override fun onKeyDown(keyCode: Int, event: KeyEvent): Boolean {
        if (handleHardwareKey(event, phase = 0)) return true
        return super.onKeyDown(keyCode, event)
    }

    override fun onKeyUp(keyCode: Int, event: KeyEvent): Boolean {
        if (handleHardwareKey(event, phase = 2)) return true
        return super.onKeyUp(keyCode, event)
    }

    /** Returns true when the event was encoded and sent to the remote. */
    fun handleHardwareKey(event: KeyEvent, phase: Int): Boolean {
        if (session == null || !inputEnabled) return false
        val control = event.isCtrlPressed
        val alt = event.isAltPressed
        val shift = event.isShiftPressed
        val named = namedKeyFor(event.keyCode)
        val keyChar = if (!control && !alt && event.unicodeChar != 0) {
            String(Character.toChars(event.unicodeChar))
        } else {
            null
        }
        if (named != null) {
            return sendKey(named, keyChar, control, alt, shift, phase)
        }
        if (phase == 2) return false
        if (!keyChar.isNullOrEmpty() && !keyChar[0].isISOControl()) {
            sendBytes(keyChar.toByteArray(StandardCharsets.UTF_8))
            return true
        }
        return false
    }

    private fun namedKeyFor(keyCode: Int): String? = when (keyCode) {
        KeyEvent.KEYCODE_DPAD_UP -> "up"
        KeyEvent.KEYCODE_DPAD_DOWN -> "down"
        KeyEvent.KEYCODE_DPAD_LEFT -> "left"
        KeyEvent.KEYCODE_DPAD_RIGHT -> "right"
        KeyEvent.KEYCODE_ENTER, KeyEvent.KEYCODE_NUMPAD_ENTER -> "enter"
        KeyEvent.KEYCODE_TAB -> "tab"
        KeyEvent.KEYCODE_ESCAPE -> "escape"
        KeyEvent.KEYCODE_DEL -> "backspace"
        KeyEvent.KEYCODE_FORWARD_DEL -> "delete"
        KeyEvent.KEYCODE_MOVE_HOME -> "home"
        KeyEvent.KEYCODE_MOVE_END -> "end"
        KeyEvent.KEYCODE_PAGE_UP -> "pageup"
        KeyEvent.KEYCODE_PAGE_DOWN -> "pagedown"
        KeyEvent.KEYCODE_F1 -> "f1"
        KeyEvent.KEYCODE_F2 -> "f2"
        KeyEvent.KEYCODE_F3 -> "f3"
        KeyEvent.KEYCODE_F4 -> "f4"
        KeyEvent.KEYCODE_F5 -> "f5"
        KeyEvent.KEYCODE_F6 -> "f6"
        KeyEvent.KEYCODE_F7 -> "f7"
        KeyEvent.KEYCODE_F8 -> "f8"
        KeyEvent.KEYCODE_F9 -> "f9"
        KeyEvent.KEYCODE_F10 -> "f10"
        KeyEvent.KEYCODE_F11 -> "f11"
        KeyEvent.KEYCODE_F12 -> "f12"
        else -> {
            // Ctrl/Alt combinations need the logical key name even for letters.
            if (eventKeyIsCharacter(keyCode)) {
                (eventKeyChar(keyCode) ?: return null).toString()
            } else {
                null
            }
        }
    }

    private fun eventKeyIsCharacter(keyCode: Int): Boolean =
        keyCode in KeyEvent.KEYCODE_A..KeyEvent.KEYCODE_Z ||
            keyCode in KeyEvent.KEYCODE_0..KeyEvent.KEYCODE_9

    private fun eventKeyChar(keyCode: Int): Char? = when {
        keyCode in KeyEvent.KEYCODE_A..KeyEvent.KEYCODE_Z -> ('a' + (keyCode - KeyEvent.KEYCODE_A))
        keyCode in KeyEvent.KEYCODE_0..KeyEvent.KEYCODE_9 -> ('0' + (keyCode - KeyEvent.KEYCODE_0))
        else -> null
    }

    private fun startSelectionActionMode() {
        // A window only has one action mode: starting a new one finishes the
        // previous one, and its onDestroyActionMode runs *after* the new
        // selection was started. Remember which selection this mode owns so the
        // stale callback cannot clear it.
        val generation = selectionGeneration
        val callback = object : ActionMode.Callback2() {
            override fun onCreateActionMode(mode: ActionMode?, menu: Menu?): Boolean {
                menu?.add(Menu.NONE, MENU_COPY, 1, android.R.string.copy)
                menu?.add(Menu.NONE, MENU_PASTE, 2, android.R.string.paste)
                menu?.add(Menu.NONE, MENU_EXPLODE, 3, context.getString(R.string.terminal_explosion_action))
                return true
            }

            override fun onPrepareActionMode(mode: ActionMode?, menu: Menu?): Boolean = false

            /**
             * Anchors the floating toolbar at the selection. Without this the
             * platform default reports the whole view as the content rect, which
             * is why the copy/paste popup showed up in the wrong place.
             */
            override fun onGetContentRect(mode: ActionMode?, view: View?, outRect: Rect?) {
                val rect = outRect ?: return
                val bounds = selectionBoundsInView()
                    ?: Rect(0, 0, view?.width ?: width, view?.height ?: height)
                rect.set(bounds)
            }

            override fun onActionItemClicked(mode: ActionMode?, item: MenuItem?): Boolean {
                when (item?.itemId) {
                    MENU_COPY -> copySelection()?.let { text ->
                        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as? ClipboardManager
                        clipboard?.setPrimaryClip(android.content.ClipData.newPlainText("terminal", text))
                    }
                    MENU_PASTE -> pasteFromClipboard()
                    MENU_EXPLODE -> copySelection()?.takeIf { it.isNotBlank() }?.let { text ->
                        onExplodeSelection?.invoke(text)
                    }
                    else -> return false
                }
                mode?.finish()
                return true
            }

            override fun onDestroyActionMode(mode: ActionMode?) {
                if (activeActionMode === mode) {
                    activeActionMode = null
                }
                // Only clear the selection this mode was created for: a newer
                // long press may already have started another one.
                if (selectionGeneration == generation) {
                    runCatching { session?.clearSelection() }
                }
                invalidate()
            }
        }
        activeActionMode = startActionMode(callback, ActionMode.TYPE_FLOATING)
    }

    private inner class TerminalInputConnection :
        BaseInputConnection(this@TerminalSurfaceView, true) {

        override fun commitText(text: CharSequence?, newCursorPosition: Int): Boolean {
            composingText = null
            val value = text?.toString().orEmpty()
            if (value.isNotEmpty()) {
                sendBytes(value.toByteArray(StandardCharsets.UTF_8))
            }
            invalidate()
            return true
        }

        override fun setComposingText(text: CharSequence?, newCursorPosition: Int): Boolean {
            composingText = text?.toString()
            invalidate()
            return true
        }

        override fun finishComposingText(): Boolean {
            composingText = null
            invalidate()
            return true
        }

        override fun deleteSurroundingText(beforeLength: Int, afterLength: Int): Boolean {
            if (!inputEnabled) return true
            repeat(beforeLength.coerceAtMost(64)) {
                sendKey("backspace", null, false, false, false, 0)
            }
            return true
        }

        override fun sendKeyEvent(event: KeyEvent): Boolean {
            return handleHardwareKey(event, if (event.action == KeyEvent.ACTION_UP) 2 else 0)
        }
    }

    companion object {
        private const val SIDE_LEFT = 0
        private const val SIDE_RIGHT = 1
        private const val MENU_COPY = 1
        private const val MENU_PASTE = 2
        private const val MENU_EXPLODE = 3

        /** How long the size must stay unchanged before the grid is reported again. */
        private const val GEOMETRY_SETTLE_MILLIS = 160L
    }
}

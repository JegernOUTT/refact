package com.smallcloud.refactai.code_lens

import com.intellij.openapi.Disposable
import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.editor.Editor
import com.intellij.openapi.editor.EditorCustomElementRenderer
import com.intellij.openapi.editor.Inlay
import com.intellij.openapi.editor.event.EditorMouseEvent
import com.intellij.openapi.editor.event.EditorMouseListener
import com.intellij.openapi.editor.event.EditorMouseMotionListener
import com.intellij.openapi.editor.markup.TextAttributes
import com.intellij.util.ui.UIUtil
import com.smallcloud.refactai.Resources
import com.smallcloud.refactai.modes.diff.renderer.RenderHelper
import java.awt.Cursor
import java.awt.Graphics
import java.awt.Rectangle
import java.awt.event.MouseEvent

private enum class CodeLensLabelStyle { Normal, Underlined }

class CodeLensInlayRenderer(
    private val editor: Editor,
    private val lenses: List<CodeLensRenderData>
) : EditorCustomElementRenderer, EditorMouseListener, EditorMouseMotionListener, Disposable {
    private var inlay: Inlay<*>? = null
    private val xBounds = mutableListOf<Pair<Int, Int>>()
    private val styles = lenses.map { CodeLensLabelStyle.Normal }.toMutableList()
    private val defaultCursor = Cursor.getPredefinedCursor(Cursor.TEXT_CURSOR)

    init {
        editor.addEditorMouseListener(this)
        editor.addEditorMouseMotionListener(this)
    }

    override fun calcWidthInPixels(inlay: Inlay<*>): Int {
        this.inlay = inlay
        val metrics = editor.contentComponent.getFontMetrics(RenderHelper.getFont(editor, false))
        val separatorWidth = metrics.stringWidth(" | ")
        val labelsWidth = lenses.sumOf { metrics.stringWidth(labelText(it.label)) }
        return labelsWidth + separatorWidth * (lenses.size - 1).coerceAtLeast(0)
    }

    override fun calcHeightInPixels(inlay: Inlay<*>): Int = editor.lineHeight

    override fun paint(
        inlay: Inlay<*>,
        g: Graphics,
        targetRegion: Rectangle,
        textAttributes: TextAttributes
    ) {
        this.inlay = inlay
        val font = RenderHelper.getFont(editor, false)
        g.font = font.deriveFont(font.size2D - 1)
        xBounds.clear()

        var xOffset = Resources.Icons.LOGO_12x12.iconWidth + 4
        Resources.Icons.LOGO_12x12.paintIcon(editor.contentComponent, g, targetRegion.x, targetRegion.y)
        val separatorWidth = g.fontMetrics.stringWidth(" | ")

        lenses.forEachIndexed { idx, lens ->
            val text = labelText(lens.label)
            g.color = if (styles[idx] == CodeLensLabelStyle.Normal) RenderHelper.color else RenderHelper.underlineColor
            g.drawString(text, targetRegion.x + xOffset, targetRegion.y + editor.ascent)
            val width = g.fontMetrics.stringWidth(text)
            xBounds.add(xOffset to xOffset + width)
            xOffset += width
            if (idx != lenses.lastIndex) {
                g.color = RenderHelper.color
                g.drawString(" | ", targetRegion.x + xOffset, targetRegion.y + editor.ascent)
                xOffset += separatorWidth
            }
        }
    }

    override fun mouseClicked(event: EditorMouseEvent) {
        if (event.inlay !== inlay) return
        val idx = insideBlockIndex(event.mouseEvent)
        if (idx == -1) return
        event.consume()
        ApplicationManager.getApplication().invokeLater {
            lenses.getOrNull(idx)?.action?.actionPerformed()
        }
        UIUtil.setCursor(editor.contentComponent, defaultCursor)
    }

    override fun mouseMoved(event: EditorMouseEvent) {
        val idx = if (event.inlay === inlay) insideBlockIndex(event.mouseEvent) else -1
        UIUtil.setCursor(
            editor.contentComponent,
            if (idx == -1) defaultCursor else Cursor.getPredefinedCursor(Cursor.HAND_CURSOR)
        )
        styles.indices.forEach { styles[it] = if (it == idx) CodeLensLabelStyle.Underlined else CodeLensLabelStyle.Normal }
        inlay?.update()
    }

    override fun dispose() {
        UIUtil.setCursor(editor.contentComponent, defaultCursor)
        editor.removeEditorMouseListener(this)
        editor.removeEditorMouseMotionListener(this)
    }

    private fun insideBlockIndex(mouseEvent: MouseEvent): Int {
        val bounds = inlay?.bounds ?: return -1
        val point = mouseEvent.point
        if (point.y < bounds.y || point.y > bounds.y + bounds.height) return -1
        val localX = point.x - bounds.x
        return xBounds.indexOfFirst { localX >= it.first && localX <= it.second }
    }

    private fun labelText(label: String): String = label.ifBlank { "Refact.ai" }
}

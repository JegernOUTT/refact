package com.smallcloud.refactai.modes

import com.intellij.openapi.editor.Editor
import com.intellij.codeInsight.completion.CompletionUtil.DUMMY_IDENTIFIER
import com.intellij.openapi.Disposable
import com.intellij.openapi.actionSystem.DataContext
import com.intellij.openapi.editor.Caret
import com.intellij.openapi.editor.event.CaretEvent
import com.intellij.openapi.editor.event.DocumentEvent
import com.intellij.openapi.editor.ex.EditorEx
import com.jetbrains.rd.util.getOrCreate
import com.smallcloud.refactai.listeners.GlobalCaretListener
import com.smallcloud.refactai.listeners.GlobalFocusListener
import com.smallcloud.refactai.modes.diff.DiffMode
import com.smallcloud.refactai.modes.diff.DiffModeWithSideEffects
import java.lang.System.currentTimeMillis
import java.lang.System.identityHashCode


enum class ModeType {
    Diff,
    DiffWithSideEffects,
}

class ModeProvider(
    private val editor: Editor,
    private val modes: MutableMap<ModeType, Mode> = mutableMapOf(
        ModeType.Diff to DiffMode()
    ),
    private var activeMode: Mode? = null,
) : Disposable {

    fun modeInActiveState(): Boolean = activeMode?.isInActiveState() == true

    fun isDiffMode(): Boolean {
        val mode = activeMode ?: return false
        return mode === modes[ModeType.Diff] || mode === modes[ModeType.DiffWithSideEffects]
    }

    fun beforeDocumentChangeNonBulk(event: DocumentEvent?, editor: Editor) {
        if (event?.newFragment.toString() == DUMMY_IDENTIFIER) return
        activeMode?.beforeDocumentChangeNonBulk(DocumentEventExtra(event, editor, currentTimeMillis()))
    }

    fun onCaretChange(event: CaretEvent) {
        activeMode?.onCaretChange(event)
    }

    fun focusGained() {}

    fun focusLost() {}

    fun onTabPressed(editor: Editor, caret: Caret?, dataContext: DataContext) {
        activeMode?.onTabPressed(editor, caret, dataContext)
    }

    fun onEscPressed(editor: Editor, caret: Caret?, dataContext: DataContext) {
        activeMode?.onEscPressed(editor, caret, dataContext)
    }

    override fun dispose() {
    }

    fun switchMode(newMode: ModeType) {
        if (activeMode == modes[newMode]) return
        activeMode?.cleanup(editor)
        activeMode = modes[newMode]
    }

    fun exitMode(mode: Mode) {
        if (activeMode === mode) activeMode = null
    }

    fun removeSideEffects() {
        activeMode?.cleanup(editor)
        modes.remove(ModeType.DiffWithSideEffects)
        activeMode = null
    }

    fun addSideEffects(onTab: (Editor, Caret?, DataContext) -> Unit, onEsc: (Editor, Caret?, DataContext) -> Unit): DiffModeWithSideEffects {
        fun <T> handleFn(fn: T): T {
            removeSideEffects()
            return fn
        }
        val mode = DiffModeWithSideEffects(handleFn(onTab), handleFn(onEsc))
        modes.set(ModeType.DiffWithSideEffects, mode)
        this.switchMode(ModeType.DiffWithSideEffects)
        return mode
    }



    fun getDiffMode(): DiffMode = (modes[ModeType.Diff] as DiffMode?)!!

    companion object {
        private const val MAX_EDITORS: Int = 8
        private var modeProviders: LinkedHashMap<Int, ModeProvider> = linkedMapOf()
        private var providersToTs: LinkedHashMap<Int, Long> = linkedMapOf()

        fun getOrCreateModeProvider(editor: Editor): ModeProvider {
            val hashId = identityHashCode(editor)
            if (modeProviders.size > MAX_EDITORS) {
                val toRemove = providersToTs.minByOrNull { it.value }?.key
                providersToTs.remove(toRemove)
                modeProviders.remove(toRemove)
            }
            return modeProviders.getOrCreate(hashId) {
                val modeProvider = ModeProvider(editor)
                providersToTs[hashId] = currentTimeMillis()
                editor.caretModel.addCaretListener(GlobalCaretListener())
                (editor as? EditorEx)?.let {
                    try {
                        it.addFocusListener(GlobalFocusListener(), modeProvider)
                    } catch (_: UnsupportedOperationException) {
                    }
                }
                modeProvider
            }
        }

        fun findModeProvider(editor: Editor): ModeProvider? = modeProviders[identityHashCode(editor)]
    }
}

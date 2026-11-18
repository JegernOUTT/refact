package com.smallcloud.refactai.listeners


import com.intellij.codeInsight.hint.HintManagerImpl.ActionToIgnore
import com.intellij.openapi.actionSystem.DataContext
import com.intellij.openapi.diagnostic.Logger
import com.intellij.openapi.editor.Caret
import com.intellij.openapi.editor.Editor
import com.intellij.openapi.editor.actionSystem.EditorAction
import com.intellij.openapi.editor.actionSystem.EditorWriteActionHandler
import com.smallcloud.refactai.Resources
import com.smallcloud.refactai.modes.ModeProvider

class CancelPressedAction :
    EditorAction(RejectDiffHandler()),
    ActionToIgnore {
    val ACTION_ID = "CancelPressedAction"

    init {
        this.templatePresentation.icon = Resources.Icons.LOGO_RED_16x16
    }

    class RejectDiffHandler : EditorWriteActionHandler() {
        override fun executeWriteAction(editor: Editor, caret: Caret?, dataContext: DataContext) {
            Logger.getInstance("CancelPressedAction").debug("executeWriteAction")
            ModeProvider.getOrCreateModeProvider(editor).onEscPressed(editor, caret, dataContext)
        }

        override fun isEnabledForCaret(
            editor: Editor,
            caret: Caret,
            dataContext: DataContext
        ): Boolean {
            return isRefactDiffActive(editor)
        }
    }
}

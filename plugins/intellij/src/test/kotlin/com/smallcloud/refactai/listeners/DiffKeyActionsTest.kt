package com.smallcloud.refactai.listeners

import com.intellij.openapi.actionSystem.ActionManager
import com.intellij.openapi.actionSystem.AnAction
import com.intellij.openapi.actionSystem.AnActionEvent
import com.intellij.openapi.actionSystem.CommonDataKeys
import com.intellij.openapi.actionSystem.DataContext
import com.intellij.openapi.actionSystem.impl.SimpleDataContext
import com.intellij.testFramework.fixtures.BasePlatformTestCase
import com.smallcloud.refactai.modes.ModeProvider
import com.smallcloud.refactai.modes.diff.DiffModeWithSideEffects
import org.junit.Test

class DiffKeyActionsTest : BasePlatformTestCase() {
    private class OtherAction : AnAction() {
        override fun actionPerformed(e: AnActionEvent) {}
    }

    private fun editorContext(): DataContext =
        SimpleDataContext.getSimpleContext(CommonDataKeys.EDITOR, myFixture.editor)

    @Test
    fun testPromotionIsEmptyWithoutActiveDiff() {
        val tab = TabPressedAction()
        val actions = listOf(OtherAction(), tab)

        assertEquals(emptyList<AnAction>(), diffActionsToPromote(actions, false, TabPressedAction::class.java))
        assertEquals(listOf(tab), diffActionsToPromote(actions, true, TabPressedAction::class.java))
    }

    @Test
    fun testRefactDoesNotOverrideThePlatformInlineCompletionAction() {
        val actionManager = ActionManager.getInstance()

        assertTrue(actionManager.getAction("TabPressedAction") is TabPressedAction)
        assertTrue(actionManager.getAction("CancelPressedAction") is CancelPressedAction)
        val platformInsert = actionManager.getAction("InsertInlineCompletionAction")
        assertFalse(platformInsert is TabPressedAction)
    }

    @Test
    fun testTabAndEscAreInactiveOutsideRefactDiff() {
        myFixture.configureByText("a.txt", "hello<caret>")
        val editor = myFixture.editor
        val caret = editor.caretModel.currentCaret

        assertFalse(isRefactDiffActive(editor))
        assertFalse(TabPressedAction.ApproveDiffHandler().isEnabled(editor, caret, editorContext()))
        assertFalse(CancelPressedAction.RejectDiffHandler().isEnabled(editor, caret, editorContext()))
        assertEquals(emptyList<AnAction>(), AcceptActionsPromoter().promote(mutableListOf(OtherAction(), TabPressedAction()), editorContext()))
        assertEquals(emptyList<AnAction>(), CancelActionsPromoter().promote(mutableListOf(OtherAction(), CancelPressedAction()), editorContext()))
    }

    @Test
    fun testTabAndEscArePromotedDuringRefactDiffAndReleasedAfterExit() {
        myFixture.configureByText("a.txt", "hello<caret>")
        val editor = myFixture.editor
        val provider = ModeProvider.getOrCreateModeProvider(editor)
        val mode: DiffModeWithSideEffects = provider.addSideEffects({ _, _, _ -> }, { _, _, _ -> })
        editor.selectionModel.setSelection(0, editor.document.textLength)
        mode.actionPerformed(editor, "world")
        val caret = editor.caretModel.currentCaret
        val tab = TabPressedAction()
        val esc = CancelPressedAction()

        assertTrue(isRefactDiffActive(editor))
        assertTrue(TabPressedAction.ApproveDiffHandler().isEnabled(editor, caret, editorContext()))
        assertTrue(CancelPressedAction.RejectDiffHandler().isEnabled(editor, caret, editorContext()))
        assertEquals(listOf(tab), AcceptActionsPromoter().promote(mutableListOf(OtherAction(), tab), editorContext()))
        assertEquals(listOf(esc), CancelActionsPromoter().promote(mutableListOf(OtherAction(), esc), editorContext()))

        provider.onEscPressed(editor, caret, editorContext())

        assertFalse(isRefactDiffActive(editor))
        assertFalse(provider.isDiffMode())
        assertEquals(emptyList<AnAction>(), AcceptActionsPromoter().promote(mutableListOf(OtherAction(), tab), editorContext()))
    }

    @Test
    fun testTabAcceptAppliesDiffAndEndsRefactDiff() {
        myFixture.configureByText("a.txt", "hello<caret>")
        val editor = myFixture.editor
        val provider = ModeProvider.getOrCreateModeProvider(editor)
        var tabCallbacks = 0
        val mode: DiffModeWithSideEffects = provider.addSideEffects({ _, _, _ -> tabCallbacks++ }, { _, _, _ -> })
        editor.selectionModel.setSelection(0, editor.document.textLength)
        mode.actionPerformed(editor, "world")
        val tab = TabPressedAction()
        val esc = CancelPressedAction()

        assertTrue(isRefactDiffActive(editor))
        assertTrue(provider.isDiffMode())

        myFixture.performEditorAction("TabPressedAction")

        val caret = editor.caretModel.currentCaret
        assertEquals("world", editor.document.text)
        assertEquals(1, tabCallbacks)
        assertFalse(isRefactDiffActive(editor))
        assertFalse(provider.isDiffMode())
        assertFalse(TabPressedAction.ApproveDiffHandler().isEnabled(editor, caret, editorContext()))
        assertFalse(CancelPressedAction.RejectDiffHandler().isEnabled(editor, caret, editorContext()))
        assertEquals(emptyList<AnAction>(), AcceptActionsPromoter().promote(mutableListOf(OtherAction(), tab), editorContext()))
        assertEquals(emptyList<AnAction>(), CancelActionsPromoter().promote(mutableListOf(OtherAction(), esc), editorContext()))
    }
}

package com.smallcloud.refactai.listeners

import com.intellij.openapi.actionSystem.ActionPromoter
import com.intellij.openapi.actionSystem.AnAction
import com.intellij.openapi.actionSystem.CommonDataKeys
import com.intellij.openapi.actionSystem.DataContext

internal fun diffActionsToPromote(
    actions: List<AnAction>,
    diffActive: Boolean,
    actionClass: Class<out AnAction>
): List<AnAction> {
    if (!diffActive) return emptyList()
    return actions.filter { actionClass.isInstance(it) }
}

internal fun promoteForActiveDiff(
    actions: List<AnAction>,
    context: DataContext,
    actionClass: Class<out AnAction>
): List<AnAction> {
    val editor = CommonDataKeys.EDITOR.getData(context) ?: return emptyList()
    return diffActionsToPromote(actions, isRefactDiffActive(editor), actionClass)
}

class AcceptActionsPromoter : ActionPromoter {
    override fun promote(actions: MutableList<out AnAction>, context: DataContext): List<AnAction> =
        promoteForActiveDiff(actions, context, TabPressedAction::class.java)
}

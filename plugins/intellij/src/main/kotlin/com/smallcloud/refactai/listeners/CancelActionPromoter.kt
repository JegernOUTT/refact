package com.smallcloud.refactai.listeners

import com.intellij.openapi.actionSystem.ActionPromoter
import com.intellij.openapi.actionSystem.AnAction
import com.intellij.openapi.actionSystem.DataContext

class CancelActionsPromoter : ActionPromoter {
    override fun promote(actions: MutableList<out AnAction>, context: DataContext): List<AnAction> =
        promoteForActiveDiff(actions, context, CancelPressedAction::class.java)
}

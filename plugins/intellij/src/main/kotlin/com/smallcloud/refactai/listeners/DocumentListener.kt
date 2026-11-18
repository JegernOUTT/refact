package com.smallcloud.refactai.listeners

import com.intellij.openapi.Disposable
import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.diagnostic.Logger
import com.intellij.openapi.editor.Document
import com.intellij.openapi.editor.Editor
import com.intellij.openapi.editor.EditorFactory
import com.intellij.openapi.editor.event.BulkAwareDocumentListener
import com.intellij.openapi.editor.event.DocumentEvent
import com.smallcloud.refactai.modes.ModeProvider


class DocumentListener : BulkAwareDocumentListener, Disposable {
    override fun beforeDocumentChangeNonBulk(event: DocumentEvent) {
        Logger.getInstance("DocumentListener").debug("beforeDocumentChangeNonBulk")
        val editor = getActiveEditor(event.document) ?: return
        ModeProvider.findModeProvider(editor)?.beforeDocumentChangeNonBulk(event, editor)
    }

    private fun getActiveEditor(document: Document): Editor? {
        if (!ApplicationManager.getApplication().isDispatchThread) {
            return null
        }
        return EditorFactory.getInstance().getEditors(document).firstOrNull()
    }

    override fun dispose() {}
}

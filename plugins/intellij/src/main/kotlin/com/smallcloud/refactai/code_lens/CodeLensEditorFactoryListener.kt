package com.smallcloud.refactai.code_lens

import com.intellij.openapi.components.service
import com.intellij.openapi.editor.event.EditorFactoryEvent
import com.intellij.openapi.editor.event.EditorFactoryListener

class CodeLensEditorFactoryListener : EditorFactoryListener {
    override fun editorCreated(event: EditorFactoryEvent) {
        val project = event.editor.project ?: return
        project.service<CodeLensInlayService>().editorCreated(event.editor)
    }

    override fun editorReleased(event: EditorFactoryEvent) {
        val project = event.editor.project ?: return
        project.service<CodeLensInlayService>().editorReleased(event.editor)
    }
}

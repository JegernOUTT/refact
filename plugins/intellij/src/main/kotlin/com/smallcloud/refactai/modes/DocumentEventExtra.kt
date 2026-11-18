package com.smallcloud.refactai.modes

import com.intellij.openapi.editor.Editor
import com.intellij.openapi.editor.event.DocumentEvent

data class DocumentEventExtra(
    val event: DocumentEvent?,
    val editor: Editor,
    val ts: Long,
)

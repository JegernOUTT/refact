package com.smallcloud.refactai.code_lens

import com.intellij.openapi.editor.Editor
import com.intellij.openapi.util.TextRange
import com.smallcloud.refactai.struct.ChatMessage

data class CodeLensItem(
    val range: TextRange,
    val label: String,
    val action: CodeLensAction
)

data class CodeLensCommandConfig(
    val key: String,
    val label: String,
    val messages: Array<ChatMessage>,
    val sendImmediately: Boolean,
    val openNewTab: Boolean
)

data class CodeLensRenderData(
    val line1: Int,
    val line2: Int,
    val label: String,
    val action: CodeLensAction
)

fun makeIdForProvider(commandKey: String): String = "refactai.codelens.$commandKey"

internal fun Editor.lineRange(line1: Int, line2: Int): TextRange {
    val startOffset = document.getLineStartOffset(line1)
    val endOffset = document.getLineEndOffset(line2)
    return TextRange(startOffset, endOffset)
}

package com.smallcloud.refactai.code_lens

import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.editor.Editor
import com.intellij.openapi.editor.LogicalPosition
import com.intellij.openapi.fileEditor.FileDocumentManager
import com.intellij.openapi.roots.ProjectRootManager
import com.intellij.openapi.wm.ToolWindowManager
import com.smallcloud.refactai.panes.RefactAIToolboxPaneFactory
import com.smallcloud.refactai.struct.ChatMessage
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.io.path.relativeTo

class CodeLensAction(
    private val editor: Editor,
    private val line1: Int,
    private val line2: Int,
    private val messages: Array<ChatMessage>,
    private val sendImmediately: Boolean,
    private val openNewTab: Boolean
) {
    private val isActionRunning = AtomicBoolean(false)

    private fun replaceVariablesInText(
        text: String,
        relativePath: String,
        cursor: Int?,
        codeSelection: String
    ): String {
        return text
            .replace("%CURRENT_FILE%", relativePath)
            .replace("%CURSOR_LINE%", cursor?.plus(1)?.toString() ?: "")
            .replace("%CODE_SELECTION%", codeSelection)
            .replace("%PROMPT_EXPLORATION_TOOLS%", "")
    }

    private fun formatMultipleMessagesForCodeLens(
        messages: Array<ChatMessage>,
        relativePath: String,
        cursor: Int?,
        text: String
    ): Array<ChatMessage> {
        return messages.map { message ->
            if (message.role == "user") {
                message.copy(content = replaceVariablesInText(message.content, relativePath, cursor, text))
            } else {
                message
            }
        }.toTypedArray()
    }

    private fun formatMessages(): Array<ChatMessage> {
        val lineCount = editor.document.lineCount
        if (lineCount == 0) return emptyArray()
        val safeLine1 = line1.coerceIn(0, lineCount - 1)
        val safeLine2 = line2.coerceIn(safeLine1, lineCount - 1)
        val pos1 = LogicalPosition(safeLine1, 0)
        val text = editor.document.text.slice(
            editor.logicalPositionToOffset(pos1) until editor.document.getLineEndOffset(safeLine2)
        )
        val file = FileDocumentManager.getInstance().getFile(editor.document)
        val filePath = file?.toNioPath() ?: return formatMultipleMessagesForCodeLens(messages, "", safeLine1, text)
        val relativePath = editor.project?.let { project ->
            ProjectRootManager.getInstance(project).contentRoots.mapNotNull { root ->
                runCatching { filePath.relativeTo(root.toNioPath()) }.getOrNull()
            }.minByOrNull { it.toString().length }
        }

        return formatMultipleMessagesForCodeLens(
            messages,
            relativePath?.toString() ?: filePath.toString(),
            safeLine1,
            text
        )
    }

    fun actionPerformed() {
        val project = editor.project ?: return
        val chat = ToolWindowManager.getInstance(project).getToolWindow("Refact")

        chat?.activate {
            RefactAIToolboxPaneFactory.chat?.requestFocus()
            RefactAIToolboxPaneFactory.chat?.executeCodeLensCommand(formatMessages(), sendImmediately, openNewTab)
        }

        if (messages.isEmpty() && isActionRunning.compareAndSet(false, true)) {
            ApplicationManager.getApplication().invokeLater {
                try {
                    val lineCount = editor.document.lineCount
                    if (lineCount == 0) return@invokeLater
                    val safeLine1 = line1.coerceIn(0, lineCount - 1)
                    val safeLine2 = line2.coerceIn(safeLine1, lineCount - 1)
                    val intendedStart = editor.document.getLineStartOffset(safeLine1)
                    val intendedEnd = editor.document.getLineEndOffset(safeLine2)
                    editor.selectionModel.setSelection(intendedStart, intendedEnd)
                } finally {
                    isActionRunning.set(false)
                }
            }
        }
    }
}

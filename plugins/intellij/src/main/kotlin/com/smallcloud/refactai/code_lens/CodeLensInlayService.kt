package com.smallcloud.refactai.code_lens

import com.intellij.openapi.Disposable
import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.application.invokeLater
import com.intellij.openapi.components.service
import com.intellij.openapi.diagnostic.Logger
import com.intellij.openapi.editor.Editor
import com.intellij.openapi.editor.EditorFactory
import com.intellij.openapi.editor.Inlay
import com.intellij.openapi.fileEditor.FileDocumentManager
import com.intellij.openapi.project.Project
import com.intellij.openapi.util.Disposer
import com.smallcloud.refactai.lsp.LSPProcessHolder.Companion.getInstance
import com.smallcloud.refactai.lsp.LSPProcessHolderChangedNotifier
import java.util.concurrent.ConcurrentHashMap

class CodeLensInlayService(private val project: Project) : Disposable {
    private val logger = Logger.getInstance(CodeLensInlayService::class.java)
    private val editorInlays = ConcurrentHashMap<Editor, MutableList<Inlay<*>>>()

    init {
        project.messageBus.connect(this).subscribe(LSPProcessHolderChangedNotifier.TOPIC, object : LSPProcessHolderChangedNotifier {
            override fun lspIsActive(isActive: Boolean) {
                if (isActive) refreshAllEditors()
            }
        })
    }

    fun editorCreated(editor: Editor) {
        if (editor.project != project) return
        refreshEditor(editor)
    }

    fun editorReleased(editor: Editor) {
        clearEditor(editor)
    }

    fun refreshAllEditors() {
        invokeLater {
            EditorFactory.getInstance().allEditors
                .filter { it.project == project }
                .forEach { refreshEditor(it) }
        }
    }

    fun refreshEditor(editor: Editor) {
        if (editor.project != project) return
        if (FileDocumentManager.getInstance().getFile(editor.document) == null) return
        ApplicationManager.getApplication().executeOnPooledThread {
            val lenses = loadCodeLenses(editor)
            invokeLater {
                if (editor.isDisposed || editor.project != project) return@invokeLater
                render(editor, lenses)
            }
        }
    }

    private fun loadCodeLenses(editor: Editor): List<CodeLensItem> {
        val lsp = getInstance(project) ?: return emptyList()
        if (!lsp.isWorking || lsp.baseUrlOrNull() == null) {
            lsp.ensureStartedAsync("code-lens-inlay")
            return emptyList()
        }
        return try {
            CodeLensParser.codeLens(editor, lsp.getCachedCustomization() ?: lsp.fetchCustomization())
        } catch (error: Exception) {
            logger.debug("Failed to load code lenses", error)
            emptyList()
        }
    }

    private fun render(editor: Editor, lenses: List<CodeLensItem>) {
        clearEditor(editor)
        if (lenses.isEmpty()) return
        val grouped = lenses.groupBy { it.range.startOffset }
        val inlays = mutableListOf<Inlay<*>>()
        grouped.toSortedMap().forEach { (offset, items) ->
            val renderData = items.map { item ->
                val position = editor.offsetToLogicalPosition(item.range.startOffset)
                val line2 = editor.offsetToLogicalPosition(item.range.endOffset).line
                CodeLensRenderData(position.line, line2, item.label, item.action)
            }
            val renderer = CodeLensInlayRenderer(editor, renderData)
            val inlay = editor.inlayModel.addBlockElement(offset, false, true, 10, renderer) ?: run {
                renderer.dispose()
                return@forEach
            }
            Disposer.register(this, inlay)
            Disposer.register(inlay, renderer)
            inlays.add(inlay)
        }
        if (inlays.isNotEmpty()) editorInlays[editor] = inlays
    }

    private fun clearEditor(editor: Editor) {
        editorInlays.remove(editor)?.forEach { inlay ->
            if (inlay.isValid) Disposer.dispose(inlay)
        }
    }

    override fun dispose() {
        editorInlays.keys.toList().forEach { clearEditor(it) }
    }
}

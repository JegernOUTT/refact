package com.smallcloud.refactai.lsp

import com.google.gson.Gson
import com.intellij.openapi.editor.Editor
import com.intellij.openapi.fileEditor.FileDocumentManager
import com.intellij.openapi.project.Project
import com.intellij.openapi.project.modules
import com.intellij.openapi.roots.ModuleRootManager
import com.intellij.openapi.roots.ProjectRootManager
import com.intellij.openapi.vfs.VirtualFile
import com.intellij.util.application
import com.smallcloud.refactai.io.ConnectionStatus
import com.smallcloud.refactai.io.HttpStatusException
import java.io.File
import java.net.URI
import java.nio.file.Paths
import com.smallcloud.refactai.io.InferenceGlobalContext.Companion.instance as InferenceGlobalContext
import com.smallcloud.refactai.lsp.LSPProcessHolder.Companion.getInstance as getLSPProcessHolder

fun findRoots(paths: List<String>): List<String> {
    val sortedPaths = paths.map { Paths.get(it).normalize() }.sortedBy { it.nameCount }

    val roots = mutableListOf<java.nio.file.Path>()

    for (path in sortedPaths) {
        if (roots.none { path.startsWith(it) }) {
            roots.add(path)
        }
    }
    return roots.map { it.toString() }
}

fun lspProjectInitialize(lsp: LSPProcessHolder, project: Project) {
    val projectRootManager = ProjectRootManager.getInstance(project)
    val projectRoots = projectRootManager.contentRoots.mapNotNull { root ->
        root.path.takeIf { root.isInLocalFileSystem && it.isNotBlank() }
    }.ifEmpty {
        val listOfFiles: MutableList<String> = mutableListOf<String>().also { list ->
            project.basePath?.let { list.add(it) }
        }
        application.runReadAction {
            project.modules.forEach { module ->
                val rootManager = ModuleRootManager.getInstance(module)
                rootManager.fileIndex.iterateContent { vfile ->
                    if (vfile.isInLocalFileSystem &&
                        (rootManager.fileIndex.isInContent(vfile) ||
                            rootManager.fileIndex.isInSourceContent(vfile) ||
                            rootManager.fileIndex.isInTestSourceContent(vfile))
                    ) {
                        listOfFiles.add(vfile.toNioPath().toString())
                    }
                    true
                }
            }
        }
        findRoots(listOfFiles)
    }.ifEmpty { listOfNotNull(project.basePath) }
        .map { path -> runCatching { File(path).canonicalPath }.getOrElse { path } }
    val baseUrl = lsp.baseUrlOrNull() ?: throw IllegalStateException("LSP project initialize requires an attached worker")
    val url = baseUrl.resolve("v1/lsp-initialize")
    val data = Gson().toJson(
        mapOf(
            "project_roots" to projectRoots.map { File(it).toURI().toString() },
        )
    )

    InferenceGlobalContext.connection.post(url, data, dataReceiveEnded = {
        InferenceGlobalContext.status = ConnectionStatus.CONNECTED
        InferenceGlobalContext.lastErrorMsg = null
    }, failedDataReceiveEnded = {
        InferenceGlobalContext.status = ConnectionStatus.ERROR
        if (it != null) {
            InferenceGlobalContext.lastErrorMsg = it.message
        }
    }).join().get()
}

internal fun isRecoverableHttpStatus(error: Throwable?): Boolean {
    val statusCode = when (error) {
        is HttpStatusException -> error.statusCode
        is DaemonHttpStatusException -> error.statusCode
        else -> null
    }
    if (statusCode != null) return statusCode in 500..599
    return error?.cause?.let { isRecoverableHttpStatus(it) } ?: false
}

private fun shouldWakeAndRetry(error: Throwable?): Boolean {
    return isRecoverableHttpStatus(error)
}

private fun sleepBeforeWakeRetry(attempt: Int) {
    Thread.sleep((attempt * 100L).coerceAtMost(300L))
}

private fun <T> withWakeRetry(project: Project, startReason: String, block: () -> T?): T? {
    val lsp = getLSPProcessHolder(project)
    repeat(3) { attempt ->
        try {
            return block()
        } catch (error: Exception) {
            if (attempt < 2 && shouldWakeAndRetry(error)) {
                if (!lsp.wakeWorkerForRetry("$startReason-retry-${attempt + 1}")) return null
                sleepBeforeWakeRetry(attempt + 1)
            } else {
                throw error
            }
        }
    }
    return null
}

private fun getLspBaseUrl(project: Project, startReason: String): URI? {
    val lsp = getLSPProcessHolder(project)
    val baseUrl = lsp.baseUrlOrNull()
    if (baseUrl == null) {
        lsp.ensureStartedAsync(startReason)
        return null
    }
    return baseUrl
}

fun lspDocumentDidChanged(project: Project, docUrl: String, text: String) {
    val baseUrl = getLspBaseUrl(project, "document-changed") ?: return
    val url = baseUrl.resolve("v1/lsp-did-changed")
    val data = Gson().toJson(
        mapOf(
            "uri" to docUrl,
            "text" to text,
        )
    )

    InferenceGlobalContext.connection.post(url, data, dataReceiveEnded = {
        InferenceGlobalContext.status = ConnectionStatus.CONNECTED
        InferenceGlobalContext.lastErrorMsg = null
    }, failedDataReceiveEnded = {
        InferenceGlobalContext.status = ConnectionStatus.ERROR
        if (it != null) {
            InferenceGlobalContext.lastErrorMsg = it.message
        }
    })
}

private fun getVirtualFile(editor: Editor): VirtualFile? {
    return FileDocumentManager.getInstance().getFile(editor.document)
}

fun lspSetActiveDocument(editor: Editor) {
    val project = editor.project ?: return
    val vFile = getVirtualFile(editor) ?: return
    if (!vFile.exists()) return

    val baseUrl = getLspBaseUrl(project, "active-document-changed") ?: return
    val url = baseUrl.resolve("v1/lsp-set-active-document")
    val data = Gson().toJson(
        mapOf(
            "uri" to vFile.url,
        )
    )

    InferenceGlobalContext.connection.post(url, data, dataReceiveEnded = {
        InferenceGlobalContext.status = ConnectionStatus.CONNECTED
        InferenceGlobalContext.lastErrorMsg = null
    }, failedDataReceiveEnded = {
        InferenceGlobalContext.status = ConnectionStatus.ERROR
        if (it != null) {
            InferenceGlobalContext.lastErrorMsg = it.message
        }
    })
}


fun lspGetCodeLens(editor: Editor): String {
    val project = editor.project ?: return ""
    val virtualFile = getVirtualFile(editor) ?: return ""
    return try {
        withWakeRetry(project, "code-lens-request") {
            val baseUrl = getLspBaseUrl(project, "code-lens-request") ?: return@withWakeRetry ""
            val url = baseUrl.resolve("v1/code-lens")
            val data = Gson().toJson(
                mapOf(
                    "uri" to virtualFile.url,
                )
            )
            InferenceGlobalContext.connection.post(url, data, dataReceiveEnded = {
                InferenceGlobalContext.status = ConnectionStatus.CONNECTED
                InferenceGlobalContext.lastErrorMsg = null
            }, failedDataReceiveEnded = {
                InferenceGlobalContext.status = ConnectionStatus.ERROR
                if (it != null) {
                    InferenceGlobalContext.lastErrorMsg = it.message
                }
            }).get()?.get() as? String ?: ""
        } ?: ""
    } catch (_: Exception) {
        ""
    }
}

fun lspGetCommitMessage(project: Project, diff: String, currentMessage: String): String {
    val lsp = getLSPProcessHolder(project)
    if (!lsp.isWorking) {
        lsp.ensureStartedAsync("commit-message-request")
        return ""
    }

    val baseUrl = lsp.baseUrlOrNull() ?: return ""

    val url = baseUrl.resolve("v1/commit-message-from-diff")
    val requestBody = mutableMapOf<String, String>("diff" to diff)
    if (currentMessage.isNotBlank()) {
        requestBody["text"] = currentMessage
    }
    val data = Gson().toJson(requestBody)
    return try {
        InferenceGlobalContext.connection.post(url, data, dataReceiveEnded = {
            InferenceGlobalContext.status = ConnectionStatus.CONNECTED
            InferenceGlobalContext.lastErrorMsg = null
        }, failedDataReceiveEnded = {
            InferenceGlobalContext.status = ConnectionStatus.ERROR
            if (it != null) {
                InferenceGlobalContext.lastErrorMsg = it.message
            }
        }).get()?.get() as? String ?: ""
    } catch (_: Exception) {
        ""
    }
}

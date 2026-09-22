package com.smallcloud.refactai.io

import com.google.gson.Gson
import com.google.gson.JsonObject
import com.intellij.openapi.Disposable
import com.intellij.openapi.diagnostic.Logger
import com.intellij.openapi.project.Project
import com.intellij.util.concurrency.AppExecutorUtil
import com.smallcloud.refactai.lsp.LSPProcessHolder
import com.smallcloud.refactai.lsp.LSPProcessHolderChangedNotifier
import com.smallcloud.refactai.notifications.emitChat
import java.io.BufferedReader
import java.io.InputStreamReader
import java.net.HttpURLConnection
import java.util.concurrent.Future
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong
import com.smallcloud.refactai.notifications.DecisionScope
import com.smallcloud.refactai.notifications.DecisionWire
import com.smallcloud.refactai.notifications.ProjectDecisionNotifications

class NotificationSSEClient(private val project: Project) : Disposable {
    private val logger = Logger.getInstance(NotificationSSEClient::class.java)
    private val gson = Gson()
    private val isRunning = AtomicBoolean(false)
    private val generation = AtomicLong(0)
    @Volatile private var activeConnection: HttpURLConnection? = null
    private var connectionFuture: Future<*>? = null
    private val reconnectDelayMs = 5000L

    fun start() {
        if (isRunning.getAndSet(true)) return

        project.messageBus.connect(this).subscribe(
            LSPProcessHolderChangedNotifier.TOPIC,
            object : LSPProcessHolderChangedNotifier {
                override fun lspIsActive(isActive: Boolean) {
                    if (isActive) {
                        reconnect()
                    } else {
                        disconnect()
                    }
                }
            }
        )

        reconnect()
    }

    @Synchronized private fun reconnect() {
        disconnect()
        val token = generation.get()
        connectionFuture = AppExecutorUtil.getAppExecutorService().submit {
            while (isRunning.get() && generation.get() == token && !Thread.currentThread().isInterrupted) {
                try {
                    connect(token)
                } catch (e: InterruptedException) {
                    Thread.currentThread().interrupt()
                    break
                } catch (e: Exception) {
                    logger.debug("SSE connection error: ${e.message}")
                }
                if (isRunning.get()) {
                    try {
                        Thread.sleep(reconnectDelayMs)
                    } catch (e: InterruptedException) {
                        Thread.currentThread().interrupt()
                        break
                    }
                }
            }
        }
    }

    private fun connect(token: Long) {
        val lspHolder = LSPProcessHolder.getInstance(project) ?: return
        val baseUrl = lspHolder.baseUrlOrNull()
        if (baseUrl == null) {
            lspHolder.ensureStartedAsync("sidebar-sse-connect")
            return
        }

        val sseUrl = baseUrl.resolve("v1/sidebar/subscribe")
        val decisions = ProjectDecisionNotifications.getInstance(project)
        val scope = decisions.currentScope() ?: return
        if (scope.endpointIdentity != com.smallcloud.refactai.notifications.decisionEndpointIdentity(baseUrl.toString())) return
        logger.info("Connecting to SSE: $sseUrl")

        val connection = (sseUrl.toURL().openConnection() as HttpURLConnection).apply {
            requestMethod = "GET"
            setRequestProperty("Accept", "text/event-stream")
            setRequestProperty("Cache-Control", "no-cache")
            connectTimeout = 10000
            readTimeout = 0
        }

        try {
            synchronized(this) {
                if (generation.get() != token) return
                activeConnection = connection
            }
            connection.connect()
            if (connection.responseCode != 200) {
                logger.warn("SSE connection failed: ${connection.responseCode}")
                return
            }

            BufferedReader(InputStreamReader(connection.inputStream, Charsets.UTF_8)).use { reader ->
                var currentLine: String? = reader.readLine()
                while (isRunning.get() && generation.get() == token && currentLine != null) {
                    if (currentLine.isNotEmpty() && !currentLine.startsWith(":")) {
                        if (currentLine.startsWith("data: ")) {
                            val data = currentLine.substring(6)
                            processEvent(data, scope, token)
                        }
                    }
                    currentLine = reader.readLine()
                }
            }
        } finally {
            connection.disconnect()
            synchronized(this) { if (activeConnection === connection) activeConnection = null }
        }
    }

    private fun processEvent(data: String, scope: DecisionScope, token: Long) {
        try {
            if (!isRunning.get() || generation.get() != token) return
            val decisions = ProjectDecisionNotifications.getInstance(project)
            if (decisions.currentScope() != scope) return
            val json = gson.fromJson(data, JsonObject::class.java)
            val category = json.get("category")?.asString ?: return

            if (category == "notification") {
                val type = json.get("type")?.asString ?: return
                when (type) {
                    "task_done" -> {
                        val chatId = json.get("chat_id")?.asString ?: return
                        val summary = json.get("summary")?.asString ?: "Task completed"
                        emitChat(project, escapeHtml(summary), chatId)
                    }
                    "ask_questions" -> {
                        val payload = DecisionWire.ask(DecisionWire.nativePayload(json)) ?: return
                        decisions.ask(payload, scope) { isRunning.get() && generation.get() == token }
                    }
                    "decision_resolved" -> {
                        val payload = DecisionWire.resolved(DecisionWire.nativePayload(json)) ?: return
                        decisions.resolve(payload, scope) { isRunning.get() && generation.get() == token }
                    }
                }
            }
        } catch (e: Exception) {
            logger.debug("Failed to parse SSE event: ${e.message}")
        }
    }

    private fun escapeHtml(text: String): String {
        return text
            .replace("&", "&amp;")
            .replace("<", "&lt;")
            .replace(">", "&gt;")
    }

    @Synchronized private fun disconnect() {
        generation.incrementAndGet()
        activeConnection?.disconnect()
        activeConnection = null
        connectionFuture?.cancel(true)
        connectionFuture = null
    }

    override fun dispose() {
        isRunning.set(false)
        disconnect()
    }
}

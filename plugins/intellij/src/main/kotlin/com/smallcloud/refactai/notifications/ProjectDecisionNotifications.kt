package com.smallcloud.refactai.notifications

import com.intellij.openapi.Disposable
import com.intellij.openapi.components.Service
import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.project.Project
import com.intellij.notification.Notification
import com.intellij.notification.NotificationAction
import com.intellij.notification.NotificationGroupManager
import com.intellij.notification.NotificationType
import com.intellij.openapi.wm.ToolWindowManager
import com.smallcloud.refactai.Resources
import com.smallcloud.refactai.lsp.LSPProcessHolder
import com.smallcloud.refactai.panes.RefactAIToolboxPaneFactory
import com.smallcloud.refactai.panes.sharedchat.Events

/** One registry for all panes and SSE connections of this project; EDT owns its visible view. */
@Service(Service.Level.PROJECT)
class ProjectDecisionNotifications(private val project: Project) : Disposable {
    private val registry = DecisionRegistry()
    private var bound: DecisionScope? = null
    @Volatile private var disposed = false
    private val notifications = linkedMapOf<String, Notification>()

    init {
        project.messageBus.connect(this).subscribe(
            com.smallcloud.refactai.lsp.LSPProcessHolderChangedNotifier.TOPIC,
            object : com.smallcloud.refactai.lsp.LSPProcessHolderChangedNotifier {
                override fun lspIsActive(isActive: Boolean) {
                    if (isActive) ApplicationManager.getApplication().invokeLater {
                        if (!disposed) currentScope()?.let { prepare(it) }
                    }
                }
            }
        )
    }

    fun currentScope(): DecisionScope? {
        if (project.isDisposed) return null
        val url = LSPProcessHolder.getInstance(project)?.baseUrlOrNull() ?: return null
        val roots = ApplicationManager.getApplication().runReadAction<List<String>?> {
            com.smallcloud.refactai.panes.sharedchat.Editor(project).getCurrentProject().workspaceRoots
        }
        return DecisionScope(roots?.firstOrNull() ?: "", decisionEndpointIdentity(url.toString()) ?: return null)
    }

    private fun prepare(source: DecisionScope): Boolean {
        if (disposed || project.isDisposed || currentScope() != source) return false
        if (bound != source) {
            notifications.values.forEach { it.expire() }
            notifications.clear()
            registry.bind(source)
            bound = source
        }
        return true
    }

    fun ask(payload: Events.AskQuestionsPayload, source: DecisionScope, isCurrent: () -> Boolean = { true }) {
        ApplicationManager.getApplication().invokeLater {
            if (!isCurrent() || !prepare(source)) return@invokeLater
            val outcome = registry.announce(payload, source)
            if (!outcome.announce) return@invokeLater
            val count = payload.questions.size
            val notification = NotificationGroupManager.getInstance().getNotificationGroup("Refact AI Notification Group")
                .createNotification(Resources.titleStr, "AI needs $count ${if (count == 1) "question" else "questions"} answered to continue", NotificationType.INFORMATION)
            notification.addAction(NotificationAction.createSimple("Open Chat") {
                // Navigation and balloon dismissal are not a native resolution.
                if (!disposed && currentScope() == source && registry.contains(outcome.key)) {
                    ToolWindowManager.getInstance(project).getToolWindow("Refact")?.activate {
                        if (!disposed && currentScope() == source) {
                            RefactAIToolboxPaneFactory.chatForProject(project)?.let {
                                it.requestFocus(); it.switchToThread(payload.chatId)
                            }
                        }
                    }
                }
                notification.expire()
            })
            notifications[outcome.key] = notification
            notifications.keys.filter { !registry.contains(it) }.forEach { notifications.remove(it)?.expire() }
            notification.notify(project)
        }
    }

    fun resolve(payload: Events.DecisionResolvedPayload, source: DecisionScope, isCurrent: () -> Boolean = { true }) {
        ApplicationManager.getApplication().invokeLater {
            if (!isCurrent() || !prepare(source)) return@invokeLater
            if (registry.resolve(payload, source)) notifications.remove(DecisionRegistry.key(payload.chatId, payload.toolCallId))?.expire()
        }
    }

    override fun dispose() {
        disposed = true
        notifications.values.forEach { it.expire() }
        notifications.clear()
    }

    companion object {
        fun getInstance(project: Project): ProjectDecisionNotifications = project.getService(ProjectDecisionNotifications::class.java)
    }
}

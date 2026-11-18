package com.smallcloud.refactai.io

import com.intellij.openapi.Disposable
import com.intellij.openapi.application.ApplicationManager
import com.intellij.util.messages.MessageBus
import com.smallcloud.refactai.settings.AppSettingsState.Companion.instance as AppSettingsState

class InferenceGlobalContext : Disposable {
    private val messageBus: MessageBus = ApplicationManager.getApplication().messageBus
    var connection: AsyncConnection = AsyncConnection()

    var status: ConnectionStatus = ConnectionStatus.CONNECTED
        set(newStatus) {
            if (field == newStatus) return
            field = newStatus
            ApplicationManager.getApplication()
                    .messageBus
                    .syncPublisher(ConnectionChangedNotifier.TOPIC)
                    .statusChanged(field)
        }
    var lastErrorMsg: String? = null
        set(newMsg) {
            if (field == newMsg) return
            field = newMsg
            ApplicationManager.getApplication()
                    .messageBus
                    .syncPublisher(ConnectionChangedNotifier.TOPIC)
                    .lastErrorMsgChanged(field)
        }

    var inferenceUri: String?
        get() {
            return AppSettingsState.userInferenceUri
        }
        set(newInferenceUrl) {
            if (newInferenceUrl == inferenceUri) return
            messageBus
                    .syncPublisher(InferenceGlobalContextChangedNotifier.TOPIC)
                    .userInferenceUriChanged(newInferenceUrl)
        }

    var refactBinaryPath: String?
        get() = AppSettingsState.refactBinaryPath
        set(newPath) {
            val normalized = newPath?.trim()?.ifEmpty { null }
            if (normalized == refactBinaryPath) return
            messageBus
                .syncPublisher(InferenceGlobalContextChangedNotifier.TOPIC)
                .refactBinaryPathChanged(normalized)
        }

    var developerModeEnabled: Boolean
        get() = AppSettingsState.developerModeEnabled
        set(newValue) {
            if (newValue == developerModeEnabled) return
            messageBus
                    .syncPublisher(InferenceGlobalContextChangedNotifier.TOPIC)
                    .developerModeEnabledChanged(newValue)
        }

    var stagingVersion: String
        get() = AppSettingsState.stagingVersion
        set(newStr) {
            if (newStr == AppSettingsState.stagingVersion) return
            AppSettingsState.stagingVersion = newStr
        }

    var astIsEnabled: Boolean
        get() = AppSettingsState.astIsEnabled
        set(newValue) {
            if (newValue == astIsEnabled) return
            messageBus
                   .syncPublisher(InferenceGlobalContextChangedNotifier.TOPIC)
                   .astFlagChanged(newValue)
        }

    var astFileLimit: Int
        get() { return AppSettingsState.astFileLimit }
        set(newValue) {
            if (newValue == astFileLimit) return
            messageBus
                  .syncPublisher(InferenceGlobalContextChangedNotifier.TOPIC)
                  .astFileLimitChanged(newValue)
        }

    var codegraphIsEnabled: Boolean
        get() = AppSettingsState.codegraphIsEnabled
        set(newValue) {
            if (newValue == codegraphIsEnabled) return
            messageBus
                .syncPublisher(InferenceGlobalContextChangedNotifier.TOPIC)
                .codegraphFlagChanged(newValue)
        }

    var codegraphFileLimit: Int
        get() { return AppSettingsState.codegraphFileLimit }
        set(newValue) {
            if (newValue == codegraphFileLimit) return
            messageBus
                .syncPublisher(InferenceGlobalContextChangedNotifier.TOPIC)
                .codegraphFileLimitChanged(newValue)
        }

    var insecureSSL: Boolean
        get() = AppSettingsState.insecureSSL
        set(newValue) {
            if (newValue == insecureSSL) return
            messageBus
               .syncPublisher(InferenceGlobalContextChangedNotifier.TOPIC)
               .insecureSSLChanged(newValue)
        }

    var experimentalLspFlagEnabled: Boolean
        get() { return AppSettingsState.experimentalLspFlagEnabled }
        set(newValue) {
            if (newValue == experimentalLspFlagEnabled) return
            messageBus
              .syncPublisher(InferenceGlobalContextChangedNotifier.TOPIC)
              .experimentalLspFlagEnabledChanged(newValue)
        }

    var httpHost: String
        get() = AppSettingsState.httpHost
        set(newValue) {
            val normalized = newValue.trim().ifEmpty { "0.0.0.0" }
            if (normalized == httpHost) return
            messageBus
                .syncPublisher(InferenceGlobalContextChangedNotifier.TOPIC)
                .httpHostChanged(normalized)
        }

    var browserHost: String
        get() = AppSettingsState.browserHost
        set(newValue) {
            val normalized = newValue.trim()
            if (normalized == browserHost) return
            messageBus
                .syncPublisher(InferenceGlobalContextChangedNotifier.TOPIC)
                .browserHostChanged(normalized)
        }

    var xDebugLSPPort: Int?
        get() { return AppSettingsState.xDebugLSPPort }
        set(newValue) {
            if (newValue == AppSettingsState.xDebugLSPPort) return
            messageBus
                .syncPublisher(InferenceGlobalContextChangedNotifier.TOPIC)
                .xDebugLSPPortChanged(newValue)
        }

    override fun dispose() {}

    companion object {
        @JvmStatic
        val instance: InferenceGlobalContext
            get() = ApplicationManager.getApplication().getService(InferenceGlobalContext::class.java)
    }
}

package com.smallcloud.refactai.code_lens

import com.google.gson.Gson
import com.google.gson.JsonObject
import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.editor.Editor
import com.smallcloud.refactai.lsp.lspGetCodeLens
import com.smallcloud.refactai.struct.ChatMessage
import kotlin.math.max

object CodeLensParser {
    private val gson = Gson()

    fun commandConfigs(customization: JsonObject?): List<CodeLensCommandConfig> {
        if (customization == null || !customization.has("code_lens")) return emptyList()
        val allCodeLenses = customization.get("code_lens")
        if (allCodeLenses == null || !allCodeLenses.isJsonObject) return emptyList()

        return allCodeLenses.asJsonObject.entrySet().mapNotNull { entry ->
            val value = entry.value?.takeIf { it.isJsonObject }?.asJsonObject ?: return@mapNotNull null
            val messagesJson = value.get("messages")
            val messages = if (messagesJson != null && messagesJson.isJsonArray) {
                messagesJson.asJsonArray.mapNotNull {
                    try {
                        gson.fromJson(it.asJsonObject, ChatMessage::class.java)
                    } catch (_: Exception) {
                        null
                    }
                }.toTypedArray()
            } else {
                emptyArray()
            }
            val hasUserMessage = messages.any { it.role == "user" }
            if (messages.isNotEmpty() && !hasUserMessage) return@mapNotNull null

            CodeLensCommandConfig(
                key = entry.key,
                label = value.get("label")?.asString ?: return@mapNotNull null,
                messages = messages,
                sendImmediately = value.get("auto_submit")?.asBoolean ?: false,
                openNewTab = value.get("new_tab")?.asBoolean ?: true
            )
        }
    }

    fun codeLens(editor: Editor, customization: JsonObject?): List<CodeLensItem> {
        val configs = commandConfigs(customization)
        if (configs.isEmpty()) return emptyList()

        val codeLensJson = try {
            gson.fromJson(lspGetCodeLens(editor), JsonObject::class.java)
        } catch (_: Exception) {
            return emptyList()
        } ?: return emptyList()

        val codeLenses = codeLensJson.get("code_lens")
        if (codeLenses == null || !codeLenses.isJsonArray) return emptyList()

        val lineCount = ApplicationManager.getApplication().runReadAction<Int> { editor.document.lineCount }
        if (lineCount == 0) return emptyList()

        val result = mutableListOf<CodeLensItem>()
        for (codeLens in codeLenses.asJsonArray) {
            val obj = try {
                codeLens.asJsonObject
            } catch (_: Exception) {
                continue
            }
            var line1 = max(obj.get("line1")?.asInt?.minus(1) ?: continue, 0).coerceAtMost(lineCount - 1)
            var line2 = max(obj.get("line2")?.asInt?.minus(1) ?: continue, 0).coerceAtMost(lineCount - 1)
            if (line2 < line1) {
                val tmp = line1
                line1 = line2
                line2 = tmp
            }

            val range = ApplicationManager.getApplication().runReadAction<com.intellij.openapi.util.TextRange> {
                editor.lineRange(line1, line2)
            }
            configs.forEach { config ->
                result.add(
                    CodeLensItem(
                        range = range,
                        label = config.label,
                        action = CodeLensAction(
                            editor,
                            line1,
                            line2,
                            config.messages,
                            config.sendImmediately,
                            config.openNewTab
                        )
                    )
                )
            }
        }
        return result
    }
}

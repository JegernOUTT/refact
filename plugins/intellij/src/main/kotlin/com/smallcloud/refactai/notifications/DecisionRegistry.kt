package com.smallcloud.refactai.notifications

import com.google.gson.JsonElement
import com.google.gson.JsonObject
import com.smallcloud.refactai.panes.sharedchat.Events

data class DecisionScope(val projectRoot: String, val endpointIdentity: String)

/** Mirrors GUI apiUrl.ts for HTTP engine roots, preserving daemon project proxy paths. */
fun decisionEndpointIdentity(raw: String): String? = runCatching {
    val uri = java.net.URI(raw.trim()).normalize()
    val scheme = uri.scheme?.lowercase(java.util.Locale.ROOT) ?: return null
    if (scheme != "http" && scheme != "https") return null
    val host = uri.host?.lowercase(java.util.Locale.ROOT) ?: return null
    val port = uri.port.takeUnless { it == -1 || (scheme == "http" && it == 80) || (scheme == "https" && it == 443) }
    val endpoints = setOf("ast-file-symbols", "ast-status", "at-command-completion", "at-command-preview",
        "caps", "chat", "chats", "checkpoints-preview", "checkpoints-restore", "codegraph-status",
        "code-intel/overview", "code-intel/graph", "code-intel/communities", "code-intel/dead-code",
        "code-intel/health", "code-intel/git-risk", "code-intel/duplication", "code-intel/pr-blast",
        "code-intel/security-scan", "code-completion", "code-lens", "customization", "docker-container-action",
        "docker-container-list", "files", "git", "git-commit", "graceful-shutdown", "integration-get",
        "integration-save", "integrations", "knowledge", "knowledge-graph", "links", "ping", "performance",
        "bug-report", "rag-status", "task", "tasks", "tools", "trajectories", "vecdb-search", "voice", "worktrees")
    val segments = (uri.rawPath ?: "").split('/')
    val cut = segments.indices.reversed().firstOrNull { i ->
        segments[i] == "v1" && (i == segments.lastIndex || segments[i + 1].isEmpty() ||
            segments[i + 1] in endpoints || segments.drop(i + 1).filter { it.isNotEmpty() }.joinToString("/") in endpoints)
    }
    val path = (if (cut == null) segments else segments.take(cut)).joinToString("/").trimEnd('/')
    "$scheme://${uri.rawUserInfo?.let { "$it@" } ?: ""}$host${port?.let { ":$it" } ?: ""}$path"
}.getOrNull()

/** Wire validation is deliberately tolerant of extra fields, but never coerces types. */
object DecisionWire {
    private val origins = setOf("native_notification_stream", "native_chat_event", "native_snapshot")
    private val resolutions = setOf("answered", "cancelled", "resolved_elsewhere")
    private fun string(obj: JsonObject, key: String): String? = obj.get(key)?.let {
        if (it.isJsonPrimitive && it.asJsonPrimitive.isString) it.asString else null
    }
    private fun metadata(obj: JsonObject): Boolean {
        if (obj.has("origin") && string(obj, "origin") !in origins) return false
        if (obj.has("askedAtMs")) {
            val n = obj.get("askedAtMs")
            if (!n.isJsonPrimitive || !n.asJsonPrimitive.isNumber) return false
            val value = n.asDouble
            if (!value.isFinite() || value < 0 || value > 9007199254740991.0 || value % 1.0 != 0.0) return false
        }
        if (obj.has("scope")) {
            val scope = obj.get("scope")
            if (!scope.isJsonObject || string(scope.asJsonObject, "projectRoot") == null ||
                string(scope.asJsonObject, "endpointIdentity").isNullOrEmpty()) return false
        }
        return !string(obj, "chatId").isNullOrEmpty() && !string(obj, "toolCallId").isNullOrEmpty()
    }
    private fun scope(obj: JsonObject): DecisionScope? = obj.get("scope")?.asJsonObject?.let {
        DecisionScope(string(it, "projectRoot")!!, string(it, "endpointIdentity")!!)
    }
    fun ask(value: JsonElement?): Events.AskQuestionsPayload? = runCatching {
        val obj = value?.takeIf { it.isJsonObject }?.asJsonObject ?: return null
        if (!metadata(obj)) return null
        val qs = obj.get("questions")?.takeIf { it.isJsonArray }?.asJsonArray ?: return null
        if (qs.size() == 0) return null
        val questions = qs.map { q ->
            if (!q.isJsonObject) return null
            val item = q.asJsonObject
            val id = string(item, "id")?.takeIf { it.isNotEmpty() } ?: return null
            val type = string(item, "type")?.takeIf { it.isNotEmpty() } ?: return null
            val text = string(item, "text") ?: return null
            val options = if (item.has("options")) {
                val opts = item.get("options").takeIf { it.isJsonArray }?.asJsonArray ?: return null
                opts.map { if (it.isJsonPrimitive && it.asJsonPrimitive.isString) it.asString else return null }
            } else null
            Events.QuestionItem(id, type, text, options)
        }
        Events.AskQuestionsPayload(string(obj, "chatId")!!, string(obj, "toolCallId")!!,
            questions, obj.get("askedAtMs")?.asLong, scope(obj), string(obj, "origin"))
    }.getOrNull()
    fun resolved(value: JsonElement?): Events.DecisionResolvedPayload? = runCatching {
        val obj = value?.takeIf { it.isJsonObject }?.asJsonObject ?: return null
        if (!metadata(obj) || string(obj, "resolution") !in resolutions) return null
        Events.DecisionResolvedPayload(string(obj, "chatId")!!, string(obj, "toolCallId")!!,
            string(obj, "resolution")!!, obj.get("askedAtMs")?.asLong, scope(obj), string(obj, "origin"))
    }.getOrNull()
    /** Only correlated native notifications enter the bridge; legacy anonymous asks are ignored. */
    fun nativePayload(obj: JsonObject): JsonObject = JsonObject().apply {
        listOf("chat_id" to "chatId", "tool_call_id" to "toolCallId", "asked_at_ms" to "askedAtMs",
            "questions" to "questions", "resolution" to "resolution").forEach { (from, to) ->
            obj.get(from)?.let { add(to, it) }
        }
        addProperty("origin", "native_notification_stream")
    }
}

/** Pure project/endpoint-scoped host view. Nothing here answers a native decision. */
class DecisionRegistry {
    data class Entry(val digest: String, val askedAtMs: Long?)
    data class Outcome(val announce: Boolean, val reason: String, val key: String)
    private var scope: DecisionScope? = null
    private val outstanding = linkedMapOf<String, Entry>()
    private val resolved = linkedMapOf<String, Entry>()
    @Synchronized fun bind(next: DecisionScope) {
        if (scope != next) { outstanding.clear(); resolved.clear(); scope = next }
    }
    @Synchronized fun count(): Int = outstanding.size
    @Synchronized fun tombstoneCount(): Int = resolved.size
    @Synchronized fun contains(key: String): Boolean = outstanding.containsKey(key)
    @Synchronized fun announce(payload: Events.AskQuestionsPayload, source: DecisionScope): Outcome {
        val key = key(payload.chatId, payload.toolCallId)
        if (scope != source || (payload.scope != null && payload.scope != source)) return Outcome(false, "foreign_scope", key)
        val entry = Entry(digest(payload.questions), payload.askedAtMs)
        outstanding[key]?.let { known ->
            if ((known.askedAtMs != null && (entry.askedAtMs == null || entry.askedAtMs < known.askedAtMs)) || known == entry)
                return Outcome(false, "duplicate", key)
            outstanding[key] = entry
            return Outcome(false, "superseded_content", key)
        }
        resolved[key]?.let { old ->
            if (old.askedAtMs == null || entry.askedAtMs == null || entry.askedAtMs <= old.askedAtMs)
                return Outcome(false, "already_resolved", key)
        }
        outstanding[key] = entry
        trim(outstanding)
        resolved.remove(key)
        return Outcome(true, "new", key)
    }
    @Synchronized fun resolve(payload: Events.DecisionResolvedPayload, source: DecisionScope): Boolean {
        if (scope != source || (payload.scope != null && payload.scope != source) ||
            payload.resolution !in setOf("answered", "cancelled", "resolved_elsewhere")) return false
        val key = key(payload.chatId, payload.toolCallId)
        val known = outstanding[key]
        // Without a native incarnation, an unknown terminal cannot certify replay identity.
        if (known == null && payload.askedAtMs == null) return false
        if (known?.askedAtMs != null && known.askedAtMs != payload.askedAtMs) return false
        outstanding.remove(key)
        val entry = known ?: Entry("", payload.askedAtMs)
        val old = resolved[key]
        if (old == null || (old.askedAtMs != null && (entry.askedAtMs == null || entry.askedAtMs >= old.askedAtMs))) {
            resolved.remove(key); resolved[key] = entry; trim(resolved)
        }
        return known != null
    }
    private fun trim(map: LinkedHashMap<String, Entry>) { while (map.size > 200) map.remove(map.keys.first()) }
    companion object {
        fun key(chatId: String, toolCallId: String) = "$chatId\u001f$toolCallId"
        fun digest(questions: List<Events.QuestionItem>): String {
            val canonical = questions.joinToString("\u001e") {
                listOf(it.id, it.type, it.text, (it.options ?: emptyList()).joinToString("\u0001")).joinToString("\u001f")
            }
            var hash = 0x811c9dc5.toInt()
            for (char in canonical) {
                hash = (hash xor (char.code and 255)) * 0x01000193
                if (char.code ushr 8 != 0) hash = (hash xor (char.code ushr 8)) * 0x01000193
            }
            return Integer.toUnsignedString(hash, 16).padStart(8, '0')
        }
    }
}

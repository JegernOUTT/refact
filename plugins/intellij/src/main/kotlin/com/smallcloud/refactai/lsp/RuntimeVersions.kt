package com.smallcloud.refactai.lsp

import com.google.gson.JsonParser
import com.google.gson.TypeAdapter
import com.google.gson.stream.JsonReader
import com.google.gson.stream.JsonWriter

data class RuntimeVersionObservation(
    val state: String = "observed",
    val version: String? = null,
    val commit: String? = null,
    val source: String,
    val observedAtMs: Long = System.currentTimeMillis(),
) {
    fun lastObserved() = copy(state = "last_observed")
}

data class RuntimeVersions(
    val plugin: RuntimeVersionObservation? = null,
    val selected: RuntimeVersionObservation? = null,
    val installed: RuntimeVersionObservation? = null,
    val staged: RuntimeVersionObservation? = null,
    val daemon: RuntimeVersionObservation? = null,
    val worker: RuntimeVersionObservation? = null,
    val requestedVersion: String? = null,
) {
    fun lastObserved() = copy(
        selected = selected?.lastObserved(),
        daemon = daemon?.lastObserved(),
        worker = worker?.lastObserved(),
    )
}

internal fun runtimeVersionString(value: String?): String? = value
    ?.takeIf { it.length <= 160 && it.none { char -> char.isISOControl() } }
    ?.trim()?.takeIf { it.isNotEmpty() }

internal fun initialRuntimeVersions(pluginVersion: String?, requestedVersion: String?) = RuntimeVersions(
    plugin = runtimeVersionString(pluginVersion)?.let {
        RuntimeVersionObservation(version = it, source = "plugin")
    },
    requestedVersion = runtimeVersionString(requestedVersion),
)

internal fun workerVersionObservation(json: String): RuntimeVersionObservation? {
    val root = runCatching { JsonParser.parseString(json) }.getOrNull()
        ?.takeIf { it.isJsonObject }?.asJsonObject ?: return null
    fun string(name: String): String? = root.get(name)
        ?.takeIf { it.isJsonPrimitive && it.asJsonPrimitive.isString }
        ?.asString?.let(::runtimeVersionString)
    val version = string("version")
    val commit = string("commit")
    if (version == null && commit == null) return null
    return RuntimeVersionObservation(version = version, commit = commit, source = "worker")
}

class RuntimeVersionsAdapter : TypeAdapter<RuntimeVersions>() {
    override fun write(writer: JsonWriter, value: RuntimeVersions?) {
        val previous = writer.serializeNulls
        writer.serializeNulls = true
        try {
            if (value == null) {
                writer.nullValue()
                return
            }
            writer.beginObject()
            fun observation(name: String, observation: RuntimeVersionObservation?) {
                writer.name(name)
                if (observation == null) {
                    writer.nullValue()
                    return
                }
                writer.beginObject()
                writer.name("state").value(observation.state)
                writer.name("version").value(observation.version)
                writer.name("commit").value(observation.commit)
                writer.name("source").value(observation.source)
                writer.name("observedAtMs").value(observation.observedAtMs)
                writer.endObject()
            }
            observation("plugin", value.plugin)
            observation("selected", value.selected)
            observation("installed", value.installed)
            observation("staged", value.staged)
            observation("daemon", value.daemon)
            observation("worker", value.worker)
            writer.name("requestedVersion").value(value.requestedVersion)
            writer.endObject()
        } finally {
            writer.serializeNulls = previous
        }
    }

    override fun read(reader: JsonReader): RuntimeVersions? {
        reader.skipValue()
        return null
    }
}

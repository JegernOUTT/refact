package com.smallcloud.refactai.lsp

import com.google.gson.Gson
import com.google.gson.annotations.SerializedName
import com.intellij.execution.configurations.GeneralCommandLine
import com.intellij.openapi.util.SystemInfo
import com.smallcloud.refactai.Resources
import com.smallcloud.refactai.io.InferenceGlobalContext.Companion.instance as InferenceGlobalContext
import java.io.File
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URI
import java.nio.charset.StandardCharsets
import java.util.concurrent.TimeUnit

const val DEFAULT_REFACT_DAEMON_PORT = 8488

interface RefactDaemonClient {
    fun status(): DaemonStatus
    fun ensureDaemon(binPath: String): DaemonStatus
    fun openProject(root: String, settings: LSPConfig): DaemonProject
}

data class DaemonStatus(
    val pid: Int = 0,
    val version: String = "",
    val port: Int = DEFAULT_REFACT_DAEMON_PORT,
    @SerializedName("started_at_ms") val startedAtMs: Long = 0,
    @SerializedName("uptime_secs") val uptimeSecs: Long = 0,
    val workers: Long = 0,
    val authToken: String? = null,
)

data class DaemonProject(
    val projectId: String,
    val baseUrl: URI,
    val daemon: DaemonStatus,
)

class HttpRefactDaemonClient(
    private val portProvider: () -> Int = { InferenceGlobalContext.xDebugLSPPort?.takeIf { it > 0 } ?: DEFAULT_REFACT_DAEMON_PORT },
    private val pluginVersionProvider: () -> String = { Resources.version },
) : RefactDaemonClient {
    private val gson = Gson()

    override fun status(): DaemonStatus {
        val preferredPort = portProvider()
        val diskInfo = readDaemonInfoFromDisk()
        val ports = listOfNotNull(preferredPort, diskInfo?.port)
            .distinct()
        var lastError: Throwable? = null
        for (port in ports) {
            try {
                val body = request(URI("http://127.0.0.1:$port/daemon/v1/status"), "GET", null, null)
                val parsed = gson.fromJson(body, DaemonStatusWire::class.java)
                val resolvedPort = parsed.port ?: port
                return DaemonStatus(
                    pid = parsed.pid ?: 0,
                    version = parsed.version.orEmpty(),
                    port = resolvedPort,
                    startedAtMs = parsed.startedAtMs ?: 0,
                    uptimeSecs = parsed.uptimeSecs ?: 0,
                    workers = parsed.workers ?: 0,
                    authToken = diskInfo?.takeIf { it.port == resolvedPort }?.authToken,
                )
            } catch (error: Throwable) {
                lastError = error
            }
        }
        throw IOException("daemon status request failed", lastError)
    }

    override fun ensureDaemon(binPath: String): DaemonStatus {
        val current = runCatching { status() }.getOrNull()
        if (current != null) {
            if (!versionIsOlder(current.version, pluginVersionProvider())) {
                return current
            }
            shutdown(current, "upgrade")
            waitUntilDown()
        }
        spawnDaemon(binPath)
        return pollStatus()
    }

    override fun openProject(root: String, settings: LSPConfig): DaemonProject {
        val daemon = status()
        val payload = gson.toJson(
            mapOf(
                "root" to root,
                "client_kind" to "jetbrains",
                "settings" to settings.toOpenProjectSettings(),
            )
        )
        val body = request(
            URI("http://127.0.0.1:${daemon.port}/daemon/v1/projects/open"),
            "POST",
            payload,
            daemon.authToken,
        )
        val parsed = gson.fromJson(body, OpenProjectResponseWire::class.java)
        val projectId = parsed.projectId ?: throw IOException("daemon project open response missing project_id")
        return DaemonProject(
            projectId = projectId,
            baseUrl = URI("http://127.0.0.1:${daemon.port}/p/$projectId/"),
            daemon = daemon,
        )
    }

    private fun pollStatus(): DaemonStatus {
        val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(15)
        var lastError: Throwable? = null
        while (System.nanoTime() < deadline) {
            runCatching { status() }
                .onSuccess { return it }
                .onFailure { lastError = it }
            Thread.sleep(200)
        }
        throw IOException("daemon did not become ready before timeout", lastError)
    }

    private fun waitUntilDown() {
        val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(15)
        while (System.nanoTime() < deadline) {
            if (runCatching { status() }.isFailure) return
            Thread.sleep(200)
        }
        throw IOException("daemon did not shut down before timeout")
    }

    private fun shutdown(status: DaemonStatus, reason: String) {
        request(
            URI("http://127.0.0.1:${status.port}/daemon/v1/shutdown"),
            "POST",
            gson.toJson(mapOf("reason" to reason)),
            status.authToken,
        )
    }

    private fun spawnDaemon(binPath: String) {
        val commands = daemonCommandCandidates(binPath)
        var lastError: Throwable? = null
        for (command in commands) {
            try {
                val process = command.withRedirectErrorStream(true).createProcess()
                runCatching { process.outputStream.close() }
                runCatching { process.inputStream.close() }
                runCatching { process.errorStream.close() }
                return
            } catch (error: Throwable) {
                lastError = error
            }
        }
        throw IOException("failed to spawn refact daemon", lastError)
    }

    private fun daemonCommandCandidates(binPath: String): List<GeneralCommandLine> {
        return when {
            SystemInfo.isWindows -> listOf(
                GeneralCommandLine(listOf("cmd", "/c", "start", "refact-daemon", "/b", binPath, "daemon")),
                GeneralCommandLine(listOf(binPath, "daemon")),
            )
            SystemInfo.isLinux -> listOf(
                GeneralCommandLine(listOf("setsid", binPath, "daemon")),
                GeneralCommandLine(listOf("nohup", binPath, "daemon")),
                GeneralCommandLine(listOf(binPath, "daemon")),
            )
            else -> listOf(
                GeneralCommandLine(listOf("nohup", binPath, "daemon")),
                GeneralCommandLine(listOf(binPath, "daemon")),
            )
        }
    }

    private fun request(uri: URI, method: String, body: String?, token: String?): String {
        val connection = uri.toURL().openConnection() as HttpURLConnection
        connection.requestMethod = method
        connection.connectTimeout = 2_000
        connection.readTimeout = 5_000
        connection.setRequestProperty("Accept", "application/json")
        if (!token.isNullOrBlank()) {
            connection.setRequestProperty("Authorization", "Bearer $token")
        }
        if (body != null) {
            connection.doOutput = true
            connection.setRequestProperty("Content-Type", "application/json")
            val bytes = body.toByteArray(StandardCharsets.UTF_8)
            connection.setRequestProperty("Content-Length", bytes.size.toString())
            connection.outputStream.use { it.write(bytes) }
        }
        val responseBody = if (connection.responseCode in 200..299) {
            connection.inputStream.use { it.readBytes().toString(StandardCharsets.UTF_8) }
        } else {
            val errorBody = connection.errorStream?.use { it.readBytes().toString(StandardCharsets.UTF_8) }.orEmpty()
            throw IOException("$method $uri failed with HTTP ${connection.responseCode}: $errorBody")
        }
        connection.disconnect()
        return responseBody
    }

    private fun readDaemonInfoFromDisk(): DaemonInfoWire? {
        val path = File(System.getProperty("user.home"), ".cache/refact/daemon/daemon.json")
        if (!path.isFile) return null
        return runCatching {
            gson.fromJson(path.readText(), DaemonInfoWire::class.java)
        }.getOrNull()
    }
}

private data class DaemonStatusWire(
    val pid: Int? = null,
    val version: String? = null,
    val port: Int? = null,
    @SerializedName("started_at_ms") val startedAtMs: Long? = null,
    @SerializedName("uptime_secs") val uptimeSecs: Long? = null,
    val workers: Long? = null,
)

private data class DaemonInfoWire(
    val port: Int = DEFAULT_REFACT_DAEMON_PORT,
    @SerializedName("auth_token") val authToken: String? = null,
)

private data class OpenProjectResponseWire(
    @SerializedName("project_id") val projectId: String? = null,
)

fun versionIsOlder(current: String, mine: String): Boolean {
    if (current.isBlank() || mine.isBlank()) return false
    val currentParts = parseVersion(current)
    val mineParts = parseVersion(mine)
    for (index in 0..2) {
        val diff = currentParts[index].compareTo(mineParts[index])
        if (diff != 0) return diff < 0
    }
    return false
}

private fun parseVersion(version: String): List<Int> {
    val first = version.split(Regex("[^0-9.]"))
        .firstOrNull { it.isNotBlank() }
        .orEmpty()
    val parts = first.split('.').map { it.toIntOrNull() ?: 0 }
    return listOf(
        parts.getOrElse(0) { 0 },
        parts.getOrElse(1) { 0 },
        parts.getOrElse(2) { 0 },
    )
}

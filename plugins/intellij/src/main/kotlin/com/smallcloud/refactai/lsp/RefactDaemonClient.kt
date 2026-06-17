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
private const val DAEMON_STARTUP_HEALTH_TIMEOUT_SECONDS = 10L

interface RefactDaemonClient {
    fun status(): DaemonStatus
    fun ensureDaemon(binPath: String): DaemonStatus
    fun openProject(root: String, settings: LSPConfig): DaemonProject
    fun detachProject(project: DaemonProject)
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

internal enum class DaemonSpawnOs {
    Windows,
    Linux,
    Other,
}

internal data class DaemonSpawnCommand(val argv: List<String>)

internal fun daemonCommandCandidates(binPath: String, os: DaemonSpawnOs): List<DaemonSpawnCommand> {
    return when (os) {
        DaemonSpawnOs.Windows -> listOf(
            DaemonSpawnCommand(listOf(binPath, "daemon")),
        )
        DaemonSpawnOs.Linux -> listOf(
            DaemonSpawnCommand(listOf("setsid", binPath, "daemon")),
            DaemonSpawnCommand(listOf("nohup", binPath, "daemon")),
            DaemonSpawnCommand(listOf(binPath, "daemon")),
        )
        DaemonSpawnOs.Other -> listOf(
            DaemonSpawnCommand(listOf("nohup", binPath, "daemon")),
            DaemonSpawnCommand(listOf(binPath, "daemon")),
        )
    }
}

internal fun currentDaemonSpawnOs(): DaemonSpawnOs {
    return when {
        SystemInfo.isWindows -> DaemonSpawnOs.Windows
        SystemInfo.isLinux -> DaemonSpawnOs.Linux
        else -> DaemonSpawnOs.Other
    }
}

internal fun spawnDaemonCandidateUntilHealthy(
    commands: List<DaemonSpawnCommand>,
    spawnCandidate: (DaemonSpawnCommand) -> Unit,
    pollCandidate: () -> DaemonStatus,
): DaemonStatus {
    var lastError: Throwable? = null
    for (command in commands) {
        try {
            spawnCandidate(command)
            return pollCandidate()
        } catch (error: Throwable) {
            lastError = error
        }
    }
    throw IOException("failed to spawn refact daemon", lastError)
}

internal fun ensureDaemonWithHealthGate(
    status: () -> DaemonStatus,
    pluginVersion: String,
    commands: List<DaemonSpawnCommand>,
    spawnCandidate: (DaemonSpawnCommand) -> Unit,
    pollCandidate: (String, Int?) -> DaemonStatus,
    shutdown: (DaemonStatus, String) -> Unit,
    waitUntilDown: (DaemonStatus, String) -> DaemonStatus?,
): DaemonStatus {
    val current = runCatching { status() }.getOrNull()
    if (current != null) {
        if (!versionIsOlder(current.version, pluginVersion)) {
            return current
        }
        shutdown(current, "upgrade")
        val compatible = waitUntilDown(current, pluginVersion)
        if (compatible != null) return compatible
    }
    return spawnDaemonCandidateUntilHealthy(commands, spawnCandidate) {
        pollCandidate(pluginVersion, current?.pid)
    }
}

internal fun spawnedDaemonStatusAccepted(status: DaemonStatus, pluginVersion: String, rejectedPid: Int?): Boolean {
    if (versionIsOlder(status.version, pluginVersion)) return false
    if (rejectedPid != null && status.pid == rejectedPid) return false
    return true
}

internal fun daemonUpgradeWaitSatisfied(
    oldDaemon: DaemonStatus,
    discovered: DaemonStatus?,
    pluginVersion: String,
): Boolean {
    if (discovered == null) return false
    if (versionIsOlder(discovered.version, pluginVersion)) return false
    return discovered.pid != oldDaemon.pid || discovered.port != oldDaemon.port
}

internal fun daemonUpgradeWaitFinished(
    oldDaemon: DaemonStatus,
    discovered: DaemonStatus?,
    oldEndpointStatus: DaemonStatus?,
    pluginVersion: String,
): Boolean {
    if (daemonUpgradeWaitSatisfied(oldDaemon, discovered, pluginVersion)) return true
    return oldEndpointStatus == null || oldEndpointStatus.pid != oldDaemon.pid
}

private data class DaemonEndpoint(
    val port: Int,
    val authToken: String? = null,
)

class HttpRefactDaemonClient(
    private val portProvider: () -> Int = { InferenceGlobalContext.xDebugLSPPort?.takeIf { it > 0 } ?: DEFAULT_REFACT_DAEMON_PORT },
    private val pluginVersionProvider: () -> String = { Resources.version },
) : RefactDaemonClient {
    private val gson = Gson()

    override fun status(): DaemonStatus {
        val pluginVersion = pluginVersionProvider()
        var olderStatus: DaemonStatus? = null
        var lastError: Throwable? = null
        for (endpoint in daemonEndpointCandidates()) {
            try {
                val status = statusForEndpoint(endpoint)
                if (!versionIsOlder(status.version, pluginVersion)) return status
                if (olderStatus == null) olderStatus = status
            } catch (error: Throwable) {
                lastError = error
            }
        }
        if (olderStatus != null) return olderStatus
        throw IOException("daemon status request failed", lastError)
    }

    override fun ensureDaemon(binPath: String): DaemonStatus {
        return ensureDaemonWithHealthGate(
            status = { status() },
            pluginVersion = pluginVersionProvider(),
            commands = daemonCommandCandidates(binPath, currentDaemonSpawnOs()),
            spawnCandidate = { spawnDaemonProcess(it) },
            pollCandidate = { expectedVersion, rejectedPid -> pollStatus(expectedVersion, rejectedPid) },
            shutdown = { current, reason -> shutdown(current, reason) },
            waitUntilDown = { current, expectedVersion -> waitUntilDown(current, expectedVersion) },
        )
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

    override fun detachProject(project: DaemonProject) {}

    private fun pollStatus(expectedVersion: String, rejectedPid: Int?): DaemonStatus {
        val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(DAEMON_STARTUP_HEALTH_TIMEOUT_SECONDS)
        var lastError: Throwable? = null
        while (System.nanoTime() < deadline) {
            runCatching { status() }
                .onSuccess {
                    if (spawnedDaemonStatusAccepted(it, expectedVersion, rejectedPid)) return it
                    lastError = IOException("daemon status still reports pid=${it.pid} version=${it.version}")
                }
                .onFailure { lastError = it }
            Thread.sleep(200)
        }
        throw IOException("daemon did not become ready before timeout", lastError)
    }

    private fun waitUntilDown(oldDaemon: DaemonStatus, pluginVersion: String): DaemonStatus? {
        val oldEndpoint = DaemonEndpoint(oldDaemon.port, oldDaemon.authToken)
        val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(15)
        while (System.nanoTime() < deadline) {
            val discovered = runCatching { status() }.getOrNull()
            if (daemonUpgradeWaitSatisfied(oldDaemon, discovered, pluginVersion)) return discovered
            val oldStatus = runCatching { statusForEndpoint(oldEndpoint) }.getOrNull()
            if (daemonUpgradeWaitFinished(oldDaemon, discovered, oldStatus, pluginVersion)) return null
            Thread.sleep(200)
        }
        throw IOException("daemon pid=${oldDaemon.pid} on port ${oldDaemon.port} did not shut down before timeout")
    }

    private fun shutdown(status: DaemonStatus, reason: String) {
        request(
            URI("http://127.0.0.1:${status.port}/daemon/v1/shutdown"),
            "POST",
            gson.toJson(mapOf("reason" to reason)),
            status.authToken,
        )
    }

    private fun spawnDaemonProcess(command: DaemonSpawnCommand) {
        val process = GeneralCommandLine(command.argv).withRedirectErrorStream(true).createProcess()
        runCatching { process.outputStream.close() }
        runCatching { process.inputStream.close() }
        runCatching { process.errorStream.close() }
    }

    private fun daemonEndpointCandidates(): List<DaemonEndpoint> {
        val preferredPort = normalizeDaemonPort(portProvider())
        val diskInfo = readDaemonInfoFromDisk()
        val diskPort = diskInfo?.port?.takeIf { it > 0 }
        val diskToken = diskInfo?.authToken?.trim()?.takeIf { it.isNotEmpty() }
        val endpoints = mutableListOf(
            DaemonEndpoint(preferredPort, if (diskPort == preferredPort) diskToken else null)
        )
        if (diskPort != null && diskPort != preferredPort) {
            endpoints.add(DaemonEndpoint(diskPort, diskToken))
        }
        return endpoints
    }

    private fun statusForEndpoint(endpoint: DaemonEndpoint): DaemonStatus {
        val body = request(
            URI("http://127.0.0.1:${endpoint.port}/daemon/v1/status"),
            "GET",
            null,
            endpoint.authToken,
        )
        val parsed = gson.fromJson(body, DaemonStatusWire::class.java)
        return DaemonStatus(
            pid = parsed.pid ?: 0,
            version = parsed.version.orEmpty(),
            port = normalizeDaemonPort(parsed.port ?: endpoint.port),
            startedAtMs = parsed.startedAtMs ?: 0,
            uptimeSecs = parsed.uptimeSecs ?: 0,
            workers = parsed.workers ?: 0,
            authToken = endpoint.authToken,
        )
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

private fun normalizeDaemonPort(port: Int): Int {
    return if (port > 0) port else DEFAULT_REFACT_DAEMON_PORT
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
    return compareRefactVersions(current, mine) < 0
}

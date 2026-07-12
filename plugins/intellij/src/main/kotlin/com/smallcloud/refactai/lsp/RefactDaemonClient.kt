package com.smallcloud.refactai.lsp

import com.google.gson.Gson
import com.google.gson.annotations.SerializedName
import com.intellij.execution.configurations.GeneralCommandLine
import com.intellij.openapi.diagnostic.Logger
import com.intellij.openapi.util.SystemInfo
import com.smallcloud.refactai.Resources
import com.smallcloud.refactai.io.InferenceGlobalContext.Companion.instance as InferenceGlobalContext
import java.io.File
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URI
import java.net.URLEncoder
import java.nio.charset.StandardCharsets
import java.util.concurrent.TimeUnit
import kotlin.concurrent.thread

const val DEFAULT_REFACT_DAEMON_PORT = 8488
private const val DAEMON_STARTUP_HEALTH_TIMEOUT_SECONDS = 30L
private const val DAEMON_SHUTDOWN_TIMEOUT_SECONDS = 15L
private const val DAEMON_CONNECT_TIMEOUT_MS = 2_000
private const val DAEMON_READ_TIMEOUT_MS = 5_000
private const val DAEMON_OPEN_PROJECT_READ_TIMEOUT_MS = 130_000
private const val DAEMON_POLL_INTERVAL_MS = 200L
private const val DAEMON_SPAWN_EXIT_LOG_WAIT_MS = 500L

interface RefactDaemonClient {
    fun status(): DaemonStatus
    fun ensureDaemon(binPath: String): DaemonStatus = ensureDaemon(binPath, null)
    fun ensureDaemon(binPath: String, requiredVersion: String?): DaemonStatus
    fun openProject(root: String, settings: LSPConfig, daemon: DaemonStatus): DaemonProject
    fun detachProject(project: DaemonProject)
}

data class DaemonStatus(
    val pid: Int = 0,
    val version: String = "",
    val port: Int = DEFAULT_REFACT_DAEMON_PORT,
    val executableSha256: String? = null,
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
    expectedVersion: String,
    expectedExecutableSha256: String?,
    commands: List<DaemonSpawnCommand>,
    spawnCandidate: (DaemonSpawnCommand) -> Unit,
    pollCandidate: (String, Int?, String?) -> DaemonStatus,
    shutdown: (DaemonStatus, String) -> Unit,
    waitUntilDown: (DaemonStatus, String, String?) -> DaemonStatus?,
): DaemonStatus {
    val current = runCatching { status() }.getOrNull()
    if (current != null) {
        if (daemonStatusMatchesExpected(current, expectedVersion, expectedExecutableSha256)) {
            return current
        }
        shutdown(current, "upgrade")
        val compatible = waitUntilDown(current, expectedVersion, expectedExecutableSha256)
        if (compatible != null) return compatible
    }
    return spawnDaemonCandidateUntilHealthy(commands, spawnCandidate) {
        pollCandidate(expectedVersion, current?.pid, expectedExecutableSha256)
    }
}

internal fun spawnedDaemonStatusAccepted(
    status: DaemonStatus,
    expectedVersion: String,
    rejectedPid: Int?,
    expectedExecutableSha256: String?,
): Boolean {
    if (!daemonStatusMatchesExpected(status, expectedVersion, expectedExecutableSha256)) return false
    if (rejectedPid != null && status.pid == rejectedPid) return false
    return true
}

internal fun daemonStatusMatchesExpected(
    status: DaemonStatus,
    expectedVersion: String,
    expectedExecutableSha256: String?,
): Boolean {
    if (versionIsOlder(status.version, expectedVersion)) return false
    if (status.version.trim() != expectedVersion.trim()) return true
    val expected = expectedExecutableSha256?.trim()?.takeIf { it.isNotEmpty() } ?: return true
    val actual = status.executableSha256?.trim()?.takeIf { it.isNotEmpty() } ?: return true
    return actual == expected
}

internal fun shouldVerifyDaemonExecutableHash(status: DaemonStatus, expectedVersion: String): Boolean {
    return status.version.trim() == expectedVersion.trim() && !status.executableSha256.isNullOrBlank()
}

internal object RefactBinaryHashCache {
    private data class Entry(val size: Long, val mtimeMs: Long, val sha256: String)

    private val entries = java.util.concurrent.ConcurrentHashMap<String, Entry>()

    fun sha256OrNull(binPath: java.nio.file.Path): String? {
        return runCatching {
            val normalized = binPath.toAbsolutePath().normalize()
            val attributes = java.nio.file.Files.readAttributes(
                normalized,
                java.nio.file.attribute.BasicFileAttributes::class.java,
            )
            val key = normalized.toString()
            val size = attributes.size()
            val mtimeMs = attributes.lastModifiedTime().toMillis()
            val cached = entries[key]
            if (cached != null && cached.size == size && cached.mtimeMs == mtimeMs) {
                cached.sha256
            } else {
                sha256File(normalized).also { entries[key] = Entry(size, mtimeMs, it) }
            }
        }.getOrNull()
    }
}

internal fun daemonUpgradeWaitSatisfied(
    oldDaemon: DaemonStatus,
    discovered: DaemonStatus?,
    expectedVersion: String,
    expectedExecutableSha256: String?,
): Boolean {
    if (discovered == null) return false
    if (!daemonStatusMatchesExpected(discovered, expectedVersion, expectedExecutableSha256)) return false
    return discovered.pid != oldDaemon.pid || discovered.port != oldDaemon.port
}

internal fun daemonUpgradeWaitFinished(
    oldDaemon: DaemonStatus,
    discovered: DaemonStatus?,
    oldEndpointStatus: DaemonStatus?,
    expectedVersion: String,
    expectedExecutableSha256: String?,
): Boolean {
    if (daemonUpgradeWaitSatisfied(oldDaemon, discovered, expectedVersion, expectedExecutableSha256)) return true
    return oldEndpointStatus == null || oldEndpointStatus.pid != oldDaemon.pid
}

private data class DaemonEndpoint(
    val port: Int,
    val authToken: String? = null,
)

private data class DaemonRequestTimeouts(
    val connectTimeoutMs: Int = DAEMON_CONNECT_TIMEOUT_MS,
    val readTimeoutMs: Int = DAEMON_READ_TIMEOUT_MS,
)

private data class DaemonResponse(
    val body: String,
    val authToken: String?,
)

internal class DaemonHttpStatusException(
    val statusCode: Int,
    val responseBody: String,
    val uri: URI,
    val method: String,
) : IOException("$method $uri failed with HTTP $statusCode: $responseBody")

class HttpRefactDaemonClient(
    private val portProvider: () -> Int = { InferenceGlobalContext.xDebugLSPPort?.takeIf { it > 0 } ?: DEFAULT_REFACT_DAEMON_PORT },
    private val pluginVersionProvider: () -> String = { Resources.version },
) : RefactDaemonClient {
    private val gson = Gson()
    private val logger = Logger.getInstance(HttpRefactDaemonClient::class.java)

    override fun status(): DaemonStatus {
        return statusWithDeadline(null)
    }

    private fun statusWithDeadline(deadlineNanos: Long?): DaemonStatus {
        val pluginVersion = pluginVersionProvider()
        var olderStatus: DaemonStatus? = null
        var lastError: Throwable? = null
        var lastHttpStatusError: DaemonHttpStatusException? = null
        for (endpoint in daemonEndpointCandidates()) {
            val timeouts = timeoutsForDeadline(DAEMON_READ_TIMEOUT_MS, deadlineNanos) ?: break
            try {
                val status = statusForEndpoint(endpoint, timeouts)
                if (!versionIsOlder(status.version, pluginVersion)) return status
                if (olderStatus == null) olderStatus = status
            } catch (error: Throwable) {
                if (error is DaemonHttpStatusException) lastHttpStatusError = error
                lastError = error
            }
        }
        if (olderStatus != null) return olderStatus
        if (lastHttpStatusError != null) throw lastHttpStatusError
        throw IOException("daemon status request failed", lastError)
    }

    override fun ensureDaemon(binPath: String, requiredVersion: String?): DaemonStatus {
        val expectedVersion = requiredVersion?.trim()?.takeIf { it.isNotEmpty() } ?: pluginVersionProvider()
        val expectedExecutableSha256 = RefactBinaryHashCache.sha256OrNull(java.nio.file.Path.of(binPath))
        return ensureDaemonWithHealthGate(
            status = { status() },
            expectedVersion = expectedVersion,
            expectedExecutableSha256 = expectedExecutableSha256,
            commands = daemonCommandCandidates(binPath, currentDaemonSpawnOs()),
            spawnCandidate = { spawnDaemonProcess(it) },
            pollCandidate = { expectedVersion, rejectedPid, expectedHash ->
                pollStatus(expectedVersion, rejectedPid, expectedHash)
            },
            shutdown = { current, reason -> shutdown(current, reason) },
            waitUntilDown = { current, expectedVersion, expectedHash ->
                waitUntilDown(current, expectedVersion, expectedHash)
            },
        )
    }

    override fun openProject(root: String, settings: LSPConfig, daemon: DaemonStatus): DaemonProject {
        val payload = gson.toJson(
            mapOf(
                "root" to root,
                "client_kind" to "jetbrains",
                "settings" to settings.toOpenProjectSettings(),
            )
        )
        val response = authenticatedRequest(
            URI("http://127.0.0.1:${daemon.port}/daemon/v1/projects/open"),
            "POST",
            payload,
            daemon.port,
            daemon.authToken,
            DaemonRequestTimeouts(readTimeoutMs = DAEMON_OPEN_PROJECT_READ_TIMEOUT_MS),
        )
        val parsed = gson.fromJson(response.body, OpenProjectResponseWire::class.java)
        val projectId = parsed.projectId ?: throw IOException("daemon project open response missing project_id")
        val encodedProjectId = encodePathSegment(projectId)
        return DaemonProject(
            projectId = projectId,
            baseUrl = URI("http://127.0.0.1:${daemon.port}/p/$encodedProjectId/"),
            daemon = daemon.copy(authToken = response.authToken),
        )
    }

    override fun detachProject(project: DaemonProject) {}

    private fun pollStatus(
        expectedVersion: String,
        rejectedPid: Int?,
        expectedExecutableSha256: String?,
    ): DaemonStatus {
        val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(DAEMON_STARTUP_HEALTH_TIMEOUT_SECONDS)
        var lastError: Throwable? = null
        while (System.nanoTime() < deadline) {
            runCatching { statusWithDeadline(deadline) }
                .onSuccess {
                    if (spawnedDaemonStatusAccepted(it, expectedVersion, rejectedPid, expectedExecutableSha256)) return it
                    lastError = IOException("daemon status still reports pid=${it.pid} version=${it.version}")
                }
                .onFailure { lastError = it }
            sleepUntilNextPoll(deadline)
        }
        throw IOException("daemon did not become ready before timeout", lastError)
    }

    private fun waitUntilDown(
        oldDaemon: DaemonStatus,
        expectedVersion: String,
        expectedExecutableSha256: String?,
    ): DaemonStatus? {
        val oldEndpoint = DaemonEndpoint(oldDaemon.port, oldDaemon.authToken)
        val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(DAEMON_SHUTDOWN_TIMEOUT_SECONDS)
        while (System.nanoTime() < deadline) {
            val discovered = runCatching { statusWithDeadline(deadline) }.getOrNull()
            if (daemonUpgradeWaitSatisfied(oldDaemon, discovered, expectedVersion, expectedExecutableSha256)) return discovered
            val oldTimeouts = timeoutsForDeadline(DAEMON_READ_TIMEOUT_MS, deadline)
            val oldStatus = oldTimeouts?.let { runCatching { statusForEndpoint(oldEndpoint, it) }.getOrNull() }
            if (daemonUpgradeWaitFinished(oldDaemon, discovered, oldStatus, expectedVersion, expectedExecutableSha256)) return null
            sleepUntilNextPoll(deadline)
        }
        throw IOException("daemon pid=${oldDaemon.pid} on port ${oldDaemon.port} did not shut down before timeout")
    }

    private fun shutdown(status: DaemonStatus, reason: String) {
        authenticatedRequest(
            URI("http://127.0.0.1:${status.port}/daemon/v1/shutdown"),
            "POST",
            gson.toJson(mapOf("reason" to reason)),
            status.port,
            status.authToken,
        )
    }

    private fun spawnDaemonProcess(command: DaemonSpawnCommand) {
        val process = GeneralCommandLine(command.argv).createProcess()
        runCatching { process.outputStream.close() }
        pipeProcessOutput(command, "stdout", process.inputStream)
        pipeProcessOutput(command, "stderr", process.errorStream)
        if (process.waitFor(DAEMON_SPAWN_EXIT_LOG_WAIT_MS, TimeUnit.MILLISECONDS)) {
            val exitCode = process.exitValue()
            logger.warn("Refact daemon command exited during startup argv=${command.argv} exit=$exitCode")
            if (exitCode != 0) throw IOException("refact daemon command exited with code $exitCode")
        }
    }

    private fun pipeProcessOutput(command: DaemonSpawnCommand, streamName: String, stream: java.io.InputStream) {
        thread(start = true, isDaemon = true, name = "refact-daemon-$streamName") {
            runCatching {
                stream.bufferedReader(StandardCharsets.UTF_8).useLines { lines ->
                    lines.forEach { line ->
                        logger.warn("Refact daemon $streamName argv=${command.argv}: $line")
                    }
                }
            }
        }
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

    private fun statusForEndpoint(endpoint: DaemonEndpoint, timeouts: DaemonRequestTimeouts): DaemonStatus {
        val body = request(
            URI("http://127.0.0.1:${endpoint.port}/daemon/v1/status"),
            "GET",
            null,
            endpoint.authToken,
            timeouts,
        )
        val parsed = gson.fromJson(body, DaemonStatusWire::class.java)
            ?: throw IOException("daemon status response is empty")
        val version = parsed.version?.trim()?.takeIf { it.isNotEmpty() }
            ?: throw IOException("daemon status response missing version")
        val statusPort = parsed.port?.takeIf { it > 0 } ?: endpoint.port
        return DaemonStatus(
            pid = parsed.pid ?: 0,
            version = version,
            port = statusPort,
            executableSha256 = parsed.executableSha256,
            startedAtMs = parsed.startedAtMs ?: 0,
            uptimeSecs = parsed.uptimeSecs ?: 0,
            workers = parsed.workers ?: 0,
            authToken = endpoint.authToken,
        )
    }

    private fun authenticatedRequest(
        uri: URI,
        method: String,
        body: String?,
        daemonPort: Int,
        token: String?,
        timeouts: DaemonRequestTimeouts = DaemonRequestTimeouts(),
    ): DaemonResponse {
        try {
            return DaemonResponse(request(uri, method, body, token, timeouts), token)
        } catch (error: DaemonHttpStatusException) {
            if (error.statusCode != HttpURLConnection.HTTP_UNAUTHORIZED) throw error
            val refreshedToken = readAuthTokenForPort(daemonPort) ?: throw error
            return DaemonResponse(request(uri, method, body, refreshedToken, timeouts), refreshedToken)
        }
    }

    private fun request(
        uri: URI,
        method: String,
        body: String?,
        token: String?,
        timeouts: DaemonRequestTimeouts = DaemonRequestTimeouts(),
    ): String {
        val connection = uri.toURL().openConnection() as HttpURLConnection
        try {
            connection.requestMethod = method
            connection.connectTimeout = timeouts.connectTimeoutMs
            connection.readTimeout = timeouts.readTimeoutMs
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
            val statusCode = connection.responseCode
            return if (statusCode in 200..299) {
                connection.inputStream.use { it.readBytes().toString(StandardCharsets.UTF_8) }
            } else {
                val errorBody = connection.errorStream?.use { it.readBytes().toString(StandardCharsets.UTF_8) }.orEmpty()
                throw DaemonHttpStatusException(statusCode, errorBody, uri, method)
            }
        } finally {
            connection.disconnect()
        }
    }

    private fun readAuthTokenForPort(port: Int): String? {
        val diskInfo = readDaemonInfoFromDisk() ?: return null
        if (diskInfo.port != port) return null
        return diskInfo.authToken?.trim()?.takeIf { it.isNotEmpty() }
    }

    private fun readDaemonInfoFromDisk(): DaemonInfoWire? {
        val path = File(System.getProperty("user.home"), ".cache/refact/daemon/daemon.json")
        if (!path.isFile) return null
        return runCatching {
            gson.fromJson(path.readText(), DaemonInfoWire::class.java)
        }.getOrNull()
    }
}

private fun timeoutsForDeadline(readTimeoutMs: Int, deadlineNanos: Long?): DaemonRequestTimeouts? {
    if (deadlineNanos == null) return DaemonRequestTimeouts(readTimeoutMs = readTimeoutMs)
    val remaining = remainingMillis(deadlineNanos)
    if (remaining <= 0) return null
    val bounded = remaining.coerceAtMost(Int.MAX_VALUE.toLong()).toInt().coerceAtLeast(1)
    return DaemonRequestTimeouts(
        connectTimeoutMs = minOf(DAEMON_CONNECT_TIMEOUT_MS, bounded),
        readTimeoutMs = minOf(readTimeoutMs, bounded),
    )
}

private fun sleepUntilNextPoll(deadlineNanos: Long) {
    val remaining = remainingMillis(deadlineNanos)
    if (remaining <= 0) return
    Thread.sleep(minOf(DAEMON_POLL_INTERVAL_MS, remaining))
}

private fun remainingMillis(deadlineNanos: Long): Long {
    val remainingNanos = deadlineNanos - System.nanoTime()
    if (remainingNanos <= 0) return 0
    return TimeUnit.NANOSECONDS.toMillis(remainingNanos).coerceAtLeast(1)
}

private fun encodePathSegment(value: String): String {
    return URLEncoder.encode(value, StandardCharsets.UTF_8).replace("+", "%20")
}

private fun normalizeDaemonPort(port: Int): Int {
    return if (port > 0) port else DEFAULT_REFACT_DAEMON_PORT
}

private data class DaemonStatusWire(
    val pid: Int? = null,
    val version: String? = null,
    val port: Int? = null,
    @SerializedName("executable_sha256") val executableSha256: String? = null,
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

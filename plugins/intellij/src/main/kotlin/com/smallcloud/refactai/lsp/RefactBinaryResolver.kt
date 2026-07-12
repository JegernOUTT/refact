package com.smallcloud.refactai.lsp

import com.intellij.openapi.util.SystemInfo
import java.io.File
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URI
import java.nio.ByteBuffer
import java.nio.channels.FileChannel
import java.nio.file.AtomicMoveNotSupportedException
import java.nio.file.FileAlreadyExistsException
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.StandardCopyOption
import java.nio.file.StandardOpenOption
import java.security.MessageDigest
import java.time.Instant
import java.util.Comparator
import java.util.concurrent.TimeUnit
import java.util.zip.GZIPInputStream
import java.util.zip.ZipInputStream

internal const val REFACT_RELEASE_BASE_URL = "https://github.com/JegernOUTT/refact/releases/download"
internal const val REFACT_RELEASES_API_URL = "https://api.github.com/repos/JegernOUTT/refact/releases?per_page=100"

private const val INSTALL_LOCK_NAME = ".install.lock"
private const val INSTALL_LOCK_RETRY_MS = 100L
private const val INSTALL_LOCK_TIMEOUT_MS = 120_000L
private const val INSTALL_LOCK_STALE_MS = 15 * 60_000L
private const val DOWNLOAD_CONNECT_TIMEOUT_MS = 15_000
private const val DOWNLOAD_READ_TIMEOUT_MS = 60_000

internal data class RefactReleaseAsset(
    val target: String,
    val archiveName: String,
    val archiveUrl: String,
    val sha256Url: String,
)

internal data class ResolvedRefactBinary(
    val path: String,
    val version: String?,
    val fallbackFromVersion: String? = null,
)

internal class DownloadFailedException(val statusCode: Int, message: String) : IOException(message)

internal data class RefactBinaryResolverOptions(
    val explicitPath: String? = null,
    val bundledDir: Path? = null,
    val minVersion: String,
    val pinnedVersion: String,
    val cacheDir: Path,
    val pathEnv: String = System.getenv("PATH").orEmpty(),
    val homeDir: Path = Path.of(System.getProperty("user.home")),
    val osName: String = System.getProperty("os.name"),
    val arch: String = System.getProperty("os.arch"),
    val versionReader: (Path) -> String? = ::readRefactVersion,
    val onDownloadStart: () -> Unit = {},
    val downloader: (URI, Path) -> Unit = ::downloadFile,
    val releaseVersionsProvider: () -> List<String> = ::fetchPublishedEngineVersions,
    val onFallbackVersion: (pinned: String, chosen: String) -> Unit = { _, _ -> },
    val extractor: (Path, Path, Boolean) -> Unit = ::extractArchive,
    val chmod: (Path) -> Unit = ::makeExecutable,
    val installLockRetryMs: Long = INSTALL_LOCK_RETRY_MS,
    val installLockTimeoutMs: Long = INSTALL_LOCK_TIMEOUT_MS,
    val installLockStaleMs: Long = INSTALL_LOCK_STALE_MS,
    val installLockNowMs: () -> Long = System::currentTimeMillis,
)

internal object RefactBinaryResolver {
    fun resolve(options: RefactBinaryResolverOptions): String {
        return resolveDetailed(options).path
    }

    fun resolveDetailed(options: RefactBinaryResolverOptions): ResolvedRefactBinary {
        return resolveLocalDetailedOrNull(options) ?: downloadRefactBinary(options)
    }

    fun resolveLocalOrNull(options: RefactBinaryResolverOptions): String? {
        return resolveLocalDetailedOrNull(options)?.path
    }

    fun resolveLocalDetailedOrNull(options: RefactBinaryResolverOptions): ResolvedRefactBinary? {
        val explicit = options.explicitPath?.trim()?.takeIf { it.isNotEmpty() }
        if (explicit != null) {
            val path = Path.of(explicit).toAbsolutePath().normalize()
            val version = extractRefactVersion(runCatching { options.versionReader(path) }.getOrNull())
            return ResolvedRefactBinary(path.toString(), version)
        }

        for (candidate in bundledRefactCandidates(options.bundledDir, options.osName, options.arch)) {
            compatibleRefactBinaryVersion(candidate, options.minVersion, options.versionReader)?.let {
                return ResolvedRefactBinary(candidate.toString(), it)
            }
        }

        for (candidate in systemRefactCandidates(options.pathEnv, options.homeDir, options.osName)) {
            compatibleRefactBinaryVersion(candidate, options.minVersion, options.versionReader)?.let {
                return ResolvedRefactBinary(candidate.toString(), it)
            }
        }

        return null
    }
}

internal fun refactBinaryName(osName: String = System.getProperty("os.name")): String {
    return if (osName.lowercase().contains("win")) "refact.exe" else "refact"
}

internal fun sharedRefactBinaryPath(homeDir: Path, osName: String = System.getProperty("os.name")): Path {
    return homeDir.resolve(".refact").resolve("bin").resolve(refactBinaryName(osName)).toAbsolutePath().normalize()
}

internal fun refactReleaseTarget(osName: String = System.getProperty("os.name"), arch: String = System.getProperty("os.arch")): String {
    val os = osName.lowercase()
    val normalizedArch = when (arch.lowercase()) {
        "amd64", "x86_64" -> "x86_64"
        "x86", "i386", "i686" -> "i686"
        "aarch64", "arm64" -> "aarch64"
        else -> arch.lowercase()
    }
    return when {
        os.contains("win") && normalizedArch == "x86_64" -> "x86_64-pc-windows-msvc"
        os.contains("win") && normalizedArch == "i686" -> "i686-pc-windows-msvc"
        os.contains("win") && normalizedArch == "aarch64" -> "aarch64-pc-windows-msvc"
        os.contains("linux") && normalizedArch == "x86_64" -> "x86_64-unknown-linux-gnu"
        os.contains("linux") && normalizedArch == "aarch64" -> "aarch64-unknown-linux-gnu"
        os.contains("mac") && normalizedArch == "x86_64" -> "x86_64-apple-darwin"
        os.contains("mac") && normalizedArch == "aarch64" -> "aarch64-apple-darwin"
        else -> throw IllegalArgumentException("unsupported Refact release target for $osName/$arch")
    }
}

internal fun refactReleaseAsset(version: String, target: String, osName: String = System.getProperty("os.name")): RefactReleaseAsset {
    val extension = if (osName.lowercase().contains("win")) "zip" else "tar.gz"
    val archiveName = "refact-$version-$target.$extension"
    val archiveUrl = "$REFACT_RELEASE_BASE_URL/engine/v$version/$archiveName"
    return RefactReleaseAsset(
        target = target,
        archiveName = archiveName,
        archiveUrl = archiveUrl,
        sha256Url = "$archiveUrl.sha256",
    )
}

internal fun extractRefactVersion(output: String?): String? {
    val text = output?.trim()?.takeIf { it.isNotEmpty() } ?: return null
    Regex("""(?:^|\s)refact\s+([0-9]+(?:\.[0-9]+){0,2}(?:[-+][0-9A-Za-z._-]+)?)""", RegexOption.IGNORE_CASE)
        .find(text)
        ?.groupValues
        ?.getOrNull(1)
        ?.let { return it }
    return Regex("""([0-9]+(?:\.[0-9]+){1,2}(?:[-+][0-9A-Za-z._-]+)?)""")
        .find(text)
        ?.groupValues
        ?.getOrNull(1)
}

internal fun compareRefactVersions(left: String?, right: String?): Int {
    val leftParts = parseVersion(left)
    val rightParts = parseVersion(right)
    for (index in 0..2) {
        val diff = leftParts.core[index] - rightParts.core[index]
        if (diff != 0) return if (diff > 0) 1 else -1
    }
    return comparePrerelease(leftParts.prerelease, rightParts.prerelease)
}

private data class ParsedVersion(
    val core: List<Int>,
    val prerelease: List<String>,
)

private fun parseVersion(version: String?): ParsedVersion {
    val match = Regex("""(\d+)(?:\.(\d+))?(?:\.(\d+))?(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?""")
        .find(version.orEmpty().trim())
        ?: return ParsedVersion(listOf(0, 0, 0), emptyList())
    return ParsedVersion(
        core = listOf(
            match.groupValues.getOrNull(1).orEmpty().toIntOrNull() ?: 0,
            match.groupValues.getOrNull(2).orEmpty().toIntOrNull() ?: 0,
            match.groupValues.getOrNull(3).orEmpty().toIntOrNull() ?: 0,
        ),
        prerelease = match.groupValues.getOrNull(4)
            ?.takeIf { it.isNotEmpty() }
            ?.split('.')
            ?.filter { it.isNotEmpty() }
            ?: emptyList(),
    )
}

private fun comparePrerelease(left: List<String>, right: List<String>): Int {
    if (left.isEmpty() && right.isEmpty()) return 0
    if (left.isEmpty()) return 1
    if (right.isEmpty()) return -1
    val length = maxOf(left.size, right.size)
    for (index in 0 until length) {
        val leftPart = left.getOrNull(index) ?: return -1
        val rightPart = right.getOrNull(index) ?: return 1
        val diff = comparePrereleaseIdentifier(leftPart, rightPart)
        if (diff != 0) return diff
    }
    return 0
}

private fun comparePrereleaseIdentifier(left: String, right: String): Int {
    val leftNumeric = left.all { it.isDigit() }
    val rightNumeric = right.all { it.isDigit() }
    if (leftNumeric && rightNumeric) {
        return left.toLong().compareTo(right.toLong()).coerceSign()
    }
    if (leftNumeric) return -1
    if (rightNumeric) return 1
    return left.compareTo(right).coerceSign()
}

private fun Int.coerceSign(): Int = when {
    this > 0 -> 1
    this < 0 -> -1
    else -> 0
}

private fun systemRefactCandidates(pathEnv: String, homeDir: Path, osName: String): List<Path> {
    val binaryName = refactBinaryName(osName)
    val candidates = mutableListOf(sharedRefactBinaryPath(homeDir, osName))
    candidates.addAll(pathEnv.split(pathSeparator(osName))
        .asSequence()
        .filter { it.isNotBlank() }
        .map { Path.of(it, binaryName).toAbsolutePath().normalize() }
        .toList())
    return candidates.distinctBy { it.toString() }
}

private fun bundledRefactCandidates(bundledDir: Path?, osName: String, arch: String): List<Path> {
    val root = bundledDir?.toAbsolutePath()?.normalize() ?: return emptyList()
    val target = runCatching { refactReleaseTarget(osName, arch) }.getOrNull() ?: return emptyList()
    val binaryName = refactBinaryName(osName)
    val distName = "dist-$target"
    return listOf(
        root.resolve("bin").resolve(distName).resolve(binaryName),
        root.resolve("src").resolve("main").resolve("resources").resolve("bin").resolve(distName).resolve(binaryName),
        root.resolve("src").resolve("main").resolve("resources").resolve("bin").resolve(target).resolve(binaryName),
    ).map { it.toAbsolutePath().normalize() }
        .distinctBy { it.toString() }
}

private fun pathSeparator(osName: String): String {
    return if (osName.lowercase().contains("win")) ";" else File.pathSeparator
}

private fun compatibleRefactBinaryVersion(
    binPath: Path,
    minVersion: String,
    versionReader: (Path) -> String?,
): String? {
    if (!Files.isRegularFile(binPath)) return null
    val version = extractRefactVersion(runCatching { versionReader(binPath) }.getOrNull()) ?: return null
    return version.takeIf { compareRefactVersions(it, minVersion) >= 0 }
}

private fun isCompatibleRefactBinary(binPath: Path, minVersion: String, versionReader: (Path) -> String?): Boolean {
    return compatibleRefactBinaryVersion(binPath, minVersion, versionReader) != null
}

private fun downloadRefactBinary(options: RefactBinaryResolverOptions): ResolvedRefactBinary {
    val target = refactReleaseTarget(options.osName, options.arch)
    val binaryName = refactBinaryName(options.osName)
    val sharedBinPath = sharedRefactBinaryPath(options.homeDir, options.osName)
    return withSharedInstallLock(
        sharedBinPath,
        options.installLockRetryMs,
        options.installLockTimeoutMs,
        options.installLockStaleMs,
        options.installLockNowMs,
    ) {
        compatibleRefactBinaryVersion(sharedBinPath, options.minVersion, options.versionReader)
            ?.let { return@withSharedInstallLock ResolvedRefactBinary(sharedBinPath.toString(), it) }

        try {
            val actual = downloadRefactVersionToSharedPath(
                options,
                options.pinnedVersion,
                options.minVersion,
                target,
                binaryName,
                sharedBinPath,
            )
            ResolvedRefactBinary(sharedBinPath.toString(), actual)
        } catch (error: DownloadFailedException) {
            if (error.statusCode != HttpURLConnection.HTTP_NOT_FOUND) throw error
            resolveLatestPublishedRefactBinary(options, target, binaryName, sharedBinPath, error)
        }
    }
}

private fun resolveLatestPublishedRefactBinary(
    options: RefactBinaryResolverOptions,
    target: String,
    binaryName: String,
    sharedBinPath: Path,
    pinnedError: DownloadFailedException,
): ResolvedRefactBinary {
    val versions = runCatching { options.releaseVersionsProvider() }.getOrElse { throw pinnedError }
    val latest = versions
        .map { it.trim() }
        .filter { it.isNotEmpty() }
        .maxWithOrNull { left, right -> compareRefactVersions(left, right) }
        ?: throw pinnedError
    if (compareRefactVersions(latest, options.pinnedVersion) == 0) throw pinnedError
    options.onFallbackVersion(options.pinnedVersion, latest)

    compatibleRefactBinaryVersion(sharedBinPath, latest, options.versionReader)?.let {
        return ResolvedRefactBinary(sharedBinPath.toString(), it, options.pinnedVersion)
    }

    val actual = downloadRefactVersionToSharedPath(options, latest, latest, target, binaryName, sharedBinPath)
    return ResolvedRefactBinary(sharedBinPath.toString(), actual, options.pinnedVersion)
}

private fun downloadRefactVersionToSharedPath(
    options: RefactBinaryResolverOptions,
    version: String,
    minAcceptableVersion: String,
    target: String,
    binaryName: String,
    sharedBinPath: Path,
): String {
    val asset = refactReleaseAsset(version, target, options.osName)
    val tmpDir = options.cacheDir.resolve("tmp-${ProcessHandle.current().pid()}-${System.nanoTime()}")
    val archivePath = tmpDir.resolve(asset.archiveName)
    val shaPath = tmpDir.resolve("${asset.archiveName}.sha256")
    val extractDir = tmpDir.resolve("extract")
    Files.createDirectories(extractDir)
    try {
        options.onDownloadStart()
        options.downloader(URI(asset.archiveUrl), archivePath)
        options.downloader(URI(asset.sha256Url), shaPath)
        verifySha256(archivePath, shaPath)
        options.extractor(archivePath, extractDir, options.osName.lowercase().contains("win"))
        val extractedBin = extractDir.resolve(binaryName)
        if (!Files.isRegularFile(extractedBin)) {
            throw IOException("downloaded Refact archive did not contain $binaryName")
        }
        if (!isWindowsOs(options.osName)) {
            options.chmod(extractedBin)
        }
        promoteSharedBinary(extractedBin, sharedBinPath, options)
        return compatibleRefactBinaryVersion(sharedBinPath, minAcceptableVersion, options.versionReader)
            ?: throw IOException("downloaded Refact binary is older than $minAcceptableVersion")
    } finally {
        deleteRecursively(tmpDir)
    }
}

internal fun fetchPublishedEngineVersions(): List<String> {
    val connection = URI(REFACT_RELEASES_API_URL).toURL().openConnection() as HttpURLConnection
    try {
        connection.connectTimeout = DOWNLOAD_CONNECT_TIMEOUT_MS
        connection.readTimeout = DOWNLOAD_READ_TIMEOUT_MS
        connection.setRequestProperty("User-Agent", "refact-jetbrains")
        connection.setRequestProperty("Accept", "application/vnd.github+json")
        val status = connection.responseCode
        if (status != 200) {
            throw IOException("release list request failed $status")
        }
        val body = connection.inputStream.use { it.readBytes().toString(Charsets.UTF_8) }
        return parsePublishedEngineVersions(body)
    } finally {
        connection.disconnect()
    }
}

internal fun parsePublishedEngineVersions(json: String): List<String> {
    val root = runCatching { com.google.gson.JsonParser.parseString(json) }.getOrNull() ?: return emptyList()
    if (!root.isJsonArray) return emptyList()
    return root.asJsonArray.mapNotNull { element ->
        runCatching { element.asJsonObject.get("tag_name")?.asString }.getOrNull()
    }.filter { it.startsWith("engine/v") }
        .map { it.removePrefix("engine/v") }
        .filter { it.isNotBlank() }
}


private fun <T> withSharedInstallLock(
    sharedBinPath: Path,
    retryMs: Long,
    timeoutMs: Long,
    staleMs: Long,
    nowMs: () -> Long,
    block: () -> T,
): T {
    val sharedDir = sharedBinPath.parent ?: throw IOException("shared Refact binary path has no parent: $sharedBinPath")
    Files.createDirectories(sharedDir)
    val lockPath = sharedDir.resolve(INSTALL_LOCK_NAME)
    val channel = acquireInstallLock(lockPath, maxOf(10L, retryMs), maxOf(10L, timeoutMs), maxOf(10L, staleMs), nowMs)
    val lockText = writeInstallLockMetadata(channel, nowMs())
    try {
        return block()
    } finally {
        try {
            channel.close()
        } finally {
            releaseInstallLock(lockPath, lockText)
        }
    }
}

private fun acquireInstallLock(
    lockPath: Path,
    retryMs: Long,
    timeoutMs: Long,
    staleMs: Long,
    nowMs: () -> Long,
): FileChannel {
    val startedAt = nowMs()
    while (true) {
        try {
            return FileChannel.open(lockPath, StandardOpenOption.CREATE_NEW, StandardOpenOption.WRITE)
        } catch (error: FileAlreadyExistsException) {
            if (breakStaleInstallLock(lockPath, staleMs, nowMs())) {
                continue
            }
            val elapsedMs = nowMs() - startedAt
            if (elapsedMs >= timeoutMs) {
                throw IOException("timed out waiting for Refact install lock at $lockPath", error)
            }
            try {
                Thread.sleep(minOf(retryMs, maxOf(10L, timeoutMs - elapsedMs)))
            } catch (interrupted: InterruptedException) {
                Thread.currentThread().interrupt()
                throw IOException("interrupted waiting for Refact install lock at $lockPath", interrupted)
            }
        }
    }
}

private data class InstallLockMetadata(
    val pid: Long?,
    val timestampMs: Long?,
)

private fun writeInstallLockMetadata(channel: FileChannel, nowMs: Long): String {
    val content = "pid=${ProcessHandle.current().pid()}\ntimestamp_ms=$nowMs\n".toByteArray(Charsets.UTF_8)
    channel.write(ByteBuffer.wrap(content))
    channel.force(true)
    return content.toString(Charsets.UTF_8)
}

private fun releaseInstallLock(lockPath: Path, lockText: String) {
    runCatching {
        if (Files.exists(lockPath) && Files.readString(lockPath) == lockText) {
            Files.deleteIfExists(lockPath)
        }
    }
}

private fun breakStaleInstallLock(lockPath: Path, staleMs: Long, nowMs: Long): Boolean {
    val lockText = runCatching { Files.readString(lockPath) }.getOrNull()
    val metadata = lockText?.let { parseInstallLockMetadata(it) }
    val stale = if (metadata == null) {
        lockFileIsOlderThan(lockPath, nowMs, staleMs)
    } else {
        metadata.timestampMs?.let { nowMs - it >= staleMs } == true ||
            metadata.pid?.let { !processIsAlive(it) } == true ||
            (metadata.timestampMs == null && lockFileIsOlderThan(lockPath, nowMs, staleMs))
    }
    if (!stale) return false
    if (lockText != null && Files.exists(lockPath) && runCatching { Files.readString(lockPath) }.getOrNull() != lockText) {
        return false
    }
    return runCatching { Files.deleteIfExists(lockPath) }.getOrDefault(false)
}

private fun parseInstallLockMetadata(text: String): InstallLockMetadata? {
    val lines = text.lines().map { it.trim() }.filter { it.isNotEmpty() }
    if (lines.isEmpty()) return null
    var pid: Long? = null
    var timestampMs: Long? = null
    for (line in lines) {
        val separator = line.indexOf('=')
        if (separator <= 0) continue
        val key = line.substring(0, separator).trim().lowercase()
        val value = line.substring(separator + 1).trim()
        when (key) {
            "pid", "owner_pid" -> pid = value.toLongOrNull()
            "timestamp", "timestamp_ms", "created_at", "created_at_ms" -> timestampMs = parseLockTimestampMs(value)
        }
    }
    if (pid == null) {
        pid = lines.firstOrNull()?.toLongOrNull()
    }
    if (timestampMs == null) {
        timestampMs = lines.getOrNull(1)?.let { parseLockTimestampMs(it) }
    }
    return if (pid != null || timestampMs != null) InstallLockMetadata(pid, timestampMs) else null
}

private fun parseLockTimestampMs(value: String): Long? {
    return value.toLongOrNull() ?: runCatching { Instant.parse(value).toEpochMilli() }.getOrNull()
}

private fun processIsAlive(pid: Long): Boolean {
    if (pid <= 0) return false
    return runCatching { ProcessHandle.of(pid).map { it.isAlive }.orElse(false) }.getOrDefault(false)
}

private fun lockFileIsOlderThan(lockPath: Path, nowMs: Long, staleMs: Long): Boolean {
    val modifiedMs = runCatching { Files.getLastModifiedTime(lockPath).toMillis() }.getOrNull() ?: return false
    return nowMs - modifiedMs >= staleMs
}

private fun promoteSharedBinary(extractedBin: Path, sharedBinPath: Path, options: RefactBinaryResolverOptions) {
    val sharedDir = sharedBinPath.parent ?: throw IOException("shared Refact binary path has no parent: $sharedBinPath")
    Files.createDirectories(sharedDir)
    val tmpTarget = sharedDir.resolve(".${sharedBinPath.fileName}.tmp.${ProcessHandle.current().pid()}.${System.nanoTime()}")
    try {
        Files.copy(extractedBin, tmpTarget, StandardCopyOption.REPLACE_EXISTING)
        if (!isWindowsOs(options.osName)) {
            options.chmod(tmpTarget)
        }
        moveReplacingWithAtomicFallback(tmpTarget, sharedBinPath)
        if (!isWindowsOs(options.osName)) {
            options.chmod(sharedBinPath)
        }
    } finally {
        Files.deleteIfExists(tmpTarget)
    }
}

internal fun moveReplacingWithAtomicFallback(
    source: Path,
    target: Path,
    move: (Path, Path, Boolean) -> Path = { from, to, atomic ->
        if (atomic) {
            Files.move(
                from,
                to,
                StandardCopyOption.REPLACE_EXISTING,
                StandardCopyOption.ATOMIC_MOVE,
            )
        } else {
            Files.move(from, to, StandardCopyOption.REPLACE_EXISTING)
        }
    },
) {
    try {
        move(source, target, true)
    } catch (_: AtomicMoveNotSupportedException) {
        move(source, target, false)
    }
}

private fun isWindowsOs(osName: String): Boolean {
    return osName.lowercase().contains("win")
}

private fun readRefactVersion(binPath: Path): String? {
    return try {
        val process = ProcessBuilder(binPath.toString(), "--version")
            .redirectErrorStream(true)
            .start()
        if (!process.waitFor(5, TimeUnit.SECONDS)) {
            process.destroyForcibly()
            return null
        }
        if (process.exitValue() != 0) return null
        process.inputStream.bufferedReader().readText()
    } catch (_: Exception) {
        null
    }
}

private fun downloadFile(url: URI, destPath: Path) {
    Files.createDirectories(destPath.parent)
    var current = url
    repeat(5) {
        val connection = current.toURL().openConnection() as HttpURLConnection
        try {
            connection.instanceFollowRedirects = true
            connection.connectTimeout = DOWNLOAD_CONNECT_TIMEOUT_MS
            connection.readTimeout = DOWNLOAD_READ_TIMEOUT_MS
            connection.setRequestProperty("User-Agent", "refact-jetbrains")
            val status = connection.responseCode
            if (status in 300..399) {
                val location = connection.getHeaderField("Location")
                if (!location.isNullOrBlank()) {
                    current = current.resolve(location)
                    return@repeat
                }
            }
            if (status != 200) {
                throw DownloadFailedException(status, "download failed $status $current")
            }
            connection.inputStream.use { input ->
                Files.copy(input, destPath, StandardCopyOption.REPLACE_EXISTING)
            }
            return
        } finally {
            connection.disconnect()
        }
    }
    throw IOException("too many redirects for $url")
}

private fun verifySha256(archivePath: Path, shaPath: Path) {
    val expected = Regex("[a-fA-F0-9]{64}")
        .find(Files.readString(shaPath))
        ?.value
        ?.lowercase()
        ?: throw IOException("sha256 sidecar did not contain a checksum")
    val actual = sha256File(archivePath)
    if (actual != expected) {
        throw IOException("sha256 mismatch for ${archivePath.fileName}")
    }
}

internal fun sha256File(filePath: Path): String {
    val digest = MessageDigest.getInstance("SHA-256")
    Files.newInputStream(filePath).use { input ->
        val buffer = ByteArray(8192)
        while (true) {
            val read = input.read(buffer)
            if (read < 0) break
            digest.update(buffer, 0, read)
        }
    }
    return digest.digest().joinToString("") { "%02x".format(it) }
}

internal fun extractArchive(archivePath: Path, destDir: Path, isWindows: Boolean) {
    if (isWindows) {
        extractZip(archivePath, destDir)
        return
    }
    extractTarGz(archivePath, destDir)
}

private data class TarEntry(
    val target: Path,
    val type: Char,
    val dataStart: Int,
    val size: Int,
)

private fun extractTarGz(archivePath: Path, destDir: Path) {
    Files.createDirectories(destDir)
    val archive = GZIPInputStream(Files.newInputStream(archivePath)).use { it.readBytes() }
    for (entry in tarEntries(archive, destDir)) {
        if (entry.type == '5') {
            val parent = entry.target.parent ?: throw IOException("archive entry has no parent: ${entry.target}")
            Files.createDirectories(parent)
            assertRealPathInside(parent, destDir)
            assertNotSymlink(entry.target)
            Files.createDirectories(entry.target)
            assertRealPathInside(entry.target, destDir)
        } else {
            val parent = entry.target.parent ?: throw IOException("archive entry has no parent: ${entry.target}")
            Files.createDirectories(parent)
            assertRealPathInside(parent, destDir)
            assertNotSymlink(entry.target)
            Files.write(entry.target, archive.copyOfRange(entry.dataStart, entry.dataStart + entry.size))
            assertRealPathInside(entry.target, destDir)
        }
    }
}

private fun tarEntries(archive: ByteArray, destDir: Path): List<TarEntry> {
    val entries = mutableListOf<TarEntry>()
    var offset = 0
    while (offset + 512 <= archive.size) {
        val header = archive.copyOfRange(offset, offset + 512)
        if (header.all { it.toInt() == 0 }) break
        val name = tarString(header, 0, 100)
        val prefix = tarString(header, 345, 155)
        val entryName = if (prefix.isEmpty()) name else "$prefix/$name"
        val size = tarOctal(header, 124, 12)
        val type = header[156].toInt().takeIf { it != 0 }?.toChar() ?: '0'
        val dataStart = offset + 512
        val target = safeArchiveEntryTarget(destDir, entryName)
        when (type) {
            '0', '5' -> entries.add(TarEntry(target, type, dataStart, size))
            'x', 'g' -> {}
            else -> throw IOException("unsupported tar entry type $type for $entryName")
        }
        offset = dataStart + size + tarPadding(size)
    }
    return entries
}

private fun tarString(header: ByteArray, start: Int, length: Int): String {
    val end = (start until start + length).firstOrNull { header[it].toInt() == 0 } ?: (start + length)
    return header.copyOfRange(start, end).toString(Charsets.UTF_8).trim()
}

private fun tarOctal(header: ByteArray, start: Int, length: Int): Int {
    val value = tarString(header, start, length).trim()
    return value.takeIf { it.isNotEmpty() }?.toInt(8) ?: 0
}

private fun tarPadding(size: Int): Int = (512 - (size % 512)) % 512

private fun safeArchiveEntryTarget(destDir: Path, entryName: String): Path {
    if (entryName.isEmpty() || entryName.startsWith('/') || entryName.startsWith('\\') || Regex("^[A-Za-z]:").containsMatchIn(entryName)) {
        throw IOException("archive entry escapes target directory: $entryName")
    }
    val parts = entryName.split(Regex("[\\\\/]+"))
        .filter { it.isNotEmpty() && it != "." }
    if (parts.isEmpty() || parts.any { it == ".." }) {
        throw IOException("archive entry escapes target directory: $entryName")
    }
    val root = destDir.toAbsolutePath().normalize()
    val target = parts.fold(root) { current, part -> current.resolve(part) }.normalize()
    if (!target.startsWith(root)) {
        throw IOException("archive entry escapes target directory: $entryName")
    }
    return target
}

private fun extractZip(archivePath: Path, destDir: Path) {
    ZipInputStream(Files.newInputStream(archivePath)).use { zip ->
        while (true) {
            val entry = zip.nextEntry ?: break
            val target = safeArchiveEntryTarget(destDir, entry.name)
            if (entry.isDirectory) {
                Files.createDirectories(target.parent)
                assertRealPathInside(target.parent, destDir)
                assertNotSymlink(target)
                Files.createDirectories(target)
                assertRealPathInside(target, destDir)
            } else {
                Files.createDirectories(target.parent)
                assertRealPathInside(target.parent, destDir)
                assertNotSymlink(target)
                Files.copy(zip, target, StandardCopyOption.REPLACE_EXISTING)
                assertRealPathInside(target, destDir)
            }
            zip.closeEntry()
        }
    }
}

private fun assertRealPathInside(target: Path, destDir: Path) {
    val root = destDir.toRealPath()
    val realTarget = target.toRealPath()
    if (!realTarget.startsWith(root)) {
        throw IOException("archive entry escapes target directory: $target")
    }
}

private fun assertNotSymlink(target: Path) {
    if (Files.isSymbolicLink(target)) {
        throw IOException("archive entry targets a symlink: $target")
    }
}

private fun makeExecutable(binPath: Path) {
    if (!SystemInfo.isWindows) {
        binPath.toFile().setExecutable(true, false)
    }
}

private fun deleteRecursively(root: Path) {
    if (!Files.exists(root)) return
    Files.walk(root).use { paths ->
        paths.sorted(Comparator.reverseOrder()).forEach { path -> Files.deleteIfExists(path) }
    }
}

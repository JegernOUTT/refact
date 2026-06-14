package com.smallcloud.refactai.lsp

import com.intellij.openapi.util.SystemInfo
import java.io.File
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URI
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.StandardCopyOption
import java.security.MessageDigest
import java.util.Comparator
import java.util.concurrent.TimeUnit
import java.util.zip.ZipInputStream

internal const val REFACT_RELEASE_BASE_URL = "https://github.com/JegernOUTT/refact/releases/download"

internal data class RefactReleaseAsset(
    val target: String,
    val archiveName: String,
    val archiveUrl: String,
    val sha256Url: String,
)

internal data class RefactBinaryResolverOptions(
    val explicitPath: String? = null,
    val minVersion: String,
    val pinnedVersion: String,
    val cacheDir: Path,
    val pathEnv: String = System.getenv("PATH").orEmpty(),
    val homeDir: Path = Path.of(System.getProperty("user.home")),
    val osName: String = System.getProperty("os.name"),
    val arch: String = System.getProperty("os.arch"),
    val versionReader: (Path) -> String? = ::readRefactVersion,
    val downloader: (URI, Path) -> Unit = ::downloadFile,
    val extractor: (Path, Path, Boolean) -> Unit = ::extractArchive,
    val chmod: (Path) -> Unit = ::makeExecutable,
)

internal object RefactBinaryResolver {
    fun resolve(options: RefactBinaryResolverOptions): String {
        val explicit = options.explicitPath?.trim()?.takeIf { it.isNotEmpty() }
        if (explicit != null) {
            return Path.of(explicit).toAbsolutePath().normalize().toString()
        }

        for (candidate in systemRefactCandidates(options.pathEnv, options.homeDir, options.osName)) {
            if (isCompatibleRefactBinary(candidate, options.minVersion, options.versionReader)) {
                return candidate.toString()
            }
        }

        return downloadPinnedRefactBinary(options)
    }
}

internal fun refactBinaryName(osName: String = System.getProperty("os.name")): String {
    return if (osName.lowercase().contains("win")) "refact.exe" else "refact"
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
        val diff = leftParts[index] - rightParts[index]
        if (diff != 0) return if (diff > 0) 1 else -1
    }
    return 0
}

private fun parseVersion(version: String?): List<Int> {
    val parts = version.orEmpty()
        .trim()
        .split(Regex("[.\\-+_\\s]"))
        .map { part -> Regex("^\\d+").find(part)?.value?.toIntOrNull() ?: 0 }
    return listOf(
        parts.getOrElse(0) { 0 },
        parts.getOrElse(1) { 0 },
        parts.getOrElse(2) { 0 },
    )
}

private fun systemRefactCandidates(pathEnv: String, homeDir: Path, osName: String): List<Path> {
    val binaryName = refactBinaryName(osName)
    val candidates = pathEnv.split(File.pathSeparator)
        .asSequence()
        .filter { it.isNotBlank() }
        .map { Path.of(it, binaryName).toAbsolutePath().normalize() }
        .toMutableList()
    candidates.add(homeDir.resolve(".refact").resolve("bin").resolve(binaryName).toAbsolutePath().normalize())
    return candidates.distinctBy { it.toString() }
}

private fun isCompatibleRefactBinary(binPath: Path, minVersion: String, versionReader: (Path) -> String?): Boolean {
    if (!Files.isRegularFile(binPath)) return false
    val version = extractRefactVersion(versionReader(binPath)) ?: return false
    return compareRefactVersions(version, minVersion) >= 0
}

private fun downloadPinnedRefactBinary(options: RefactBinaryResolverOptions): String {
    val target = refactReleaseTarget(options.osName, options.arch)
    val binaryName = refactBinaryName(options.osName)
    val targetDir = options.cacheDir.resolve(options.pinnedVersion).resolve(target)
    val binPath = targetDir.resolve(binaryName)
    if (isCompatibleRefactBinary(binPath, options.minVersion, options.versionReader)) {
        return binPath.toString()
    }

    val asset = refactReleaseAsset(options.pinnedVersion, target, options.osName)
    val tmpDir = options.cacheDir.resolve("tmp-${ProcessHandle.current().pid()}-${System.nanoTime()}")
    val archivePath = tmpDir.resolve(asset.archiveName)
    val shaPath = tmpDir.resolve("${asset.archiveName}.sha256")
    val extractDir = tmpDir.resolve("extract")
    Files.createDirectories(extractDir)
    try {
        options.downloader(URI(asset.archiveUrl), archivePath)
        options.downloader(URI(asset.sha256Url), shaPath)
        verifySha256(archivePath, shaPath)
        options.extractor(archivePath, extractDir, options.osName.lowercase().contains("win"))
        val extractedBin = extractDir.resolve(binaryName)
        if (!Files.isRegularFile(extractedBin)) {
            throw IOException("downloaded Refact archive did not contain $binaryName")
        }
        options.chmod(extractedBin)
        deleteRecursively(targetDir)
        Files.createDirectories(targetDir.parent)
        Files.move(extractDir, targetDir, StandardCopyOption.REPLACE_EXISTING)
        options.chmod(binPath)
        if (!isCompatibleRefactBinary(binPath, options.minVersion, options.versionReader)) {
            throw IOException("downloaded Refact binary is older than ${options.minVersion}")
        }
        return binPath.toString()
    } finally {
        deleteRecursively(tmpDir)
    }
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
        connection.instanceFollowRedirects = true
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
            throw IOException("download failed $status $current")
        }
        connection.inputStream.use { input ->
            Files.copy(input, destPath, StandardCopyOption.REPLACE_EXISTING)
        }
        return
    }
    throw IOException("too many redirects for $url")
}

private fun verifySha256(archivePath: Path, shaPath: Path) {
    val expected = Regex("[a-fA-F0-9]{64}")
        .find(Files.readString(shaPath))
        ?.value
        ?.lowercase()
        ?: throw IOException("sha256 sidecar did not contain a checksum")
    val actual = sha256(archivePath)
    if (actual != expected) {
        throw IOException("sha256 mismatch for ${archivePath.fileName}")
    }
}

private fun sha256(filePath: Path): String {
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

private fun extractArchive(archivePath: Path, destDir: Path, isWindows: Boolean) {
    if (isWindows) {
        extractZip(archivePath, destDir)
        return
    }
    val process = ProcessBuilder("tar", "-xzf", archivePath.toString(), "-C", destDir.toString())
        .redirectErrorStream(true)
        .start()
    val output = process.inputStream.bufferedReader().readText()
    if (!process.waitFor(120, TimeUnit.SECONDS)) {
        process.destroyForcibly()
        throw IOException("tar extraction timed out")
    }
    if (process.exitValue() != 0) {
        throw IOException("tar extraction failed: $output")
    }
}

private fun extractZip(archivePath: Path, destDir: Path) {
    ZipInputStream(Files.newInputStream(archivePath)).use { zip ->
        while (true) {
            val entry = zip.nextEntry ?: break
            val target = destDir.resolve(entry.name).normalize()
            if (!target.startsWith(destDir)) {
                throw IOException("zip entry escapes target directory: ${entry.name}")
            }
            if (entry.isDirectory) {
                Files.createDirectories(target)
            } else {
                Files.createDirectories(target.parent)
                Files.copy(zip, target, StandardCopyOption.REPLACE_EXISTING)
            }
            zip.closeEntry()
        }
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

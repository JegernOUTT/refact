package com.smallcloud.refactai.lsp

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.ByteArrayOutputStream
import java.nio.file.Files
import java.nio.file.Path
import java.io.IOException
import java.security.MessageDigest
import java.util.Comparator
import java.util.zip.GZIPOutputStream

class RefactDaemonClientTest {
    @Test
    fun versionComparisonDetectsOlderDaemon() {
        assertTrue(versionIsOlder("8.1.0", "8.2.0"))
        assertTrue(versionIsOlder("8.1.0-alpha", "8.1.1"))
        assertTrue(versionIsOlder("8.1.0-alpha.1", "8.1.0"))
        assertFalse(versionIsOlder("8.2.0", "8.1.0"))
        assertFalse(versionIsOlder("8.1.0", "8.1.0-alpha.1"))
        assertFalse(versionIsOlder("", "8.1.0"))
        assertEquals(-1, compareRefactVersions("8.1.0-alpha.2", "8.1.0-alpha.10"))
        assertEquals(-1, compareRefactVersions("8.1.0-alpha.1", "8.1.0-beta.1"))
        assertEquals("8.1.0", extractRefactVersion("refact 8.1.0\n"))
    }

    @Test
    fun releaseAssetUrlUsesStableContract() {
        val asset = refactReleaseAsset("8.1.0", "aarch64-pc-windows-msvc", "Windows 11")

        assertEquals("aarch64-pc-windows-msvc", asset.target)
        assertEquals("refact-8.1.0-aarch64-pc-windows-msvc.zip", asset.archiveName)
        assertEquals(
            "https://github.com/JegernOUTT/refact/releases/download/engine/v8.1.0/refact-8.1.0-aarch64-pc-windows-msvc.zip",
            asset.archiveUrl,
        )
        assertEquals(
            "https://github.com/JegernOUTT/refact/releases/download/engine/v8.1.0/refact-8.1.0-aarch64-pc-windows-msvc.zip.sha256",
            asset.sha256Url,
        )
    }

    @Test
    fun binaryResolverHonorsExplicitOverrideWithoutVersionCheck() {
        val root = Files.createTempDirectory("refact-binary-resolver-explicit")
        val explicit = root.resolve("custom").resolve("refact")

        try {
            val resolved = RefactBinaryResolver.resolve(
                RefactBinaryResolverOptions(
                    explicitPath = explicit.toString(),
                    minVersion = "9.0.0",
                    pinnedVersion = "9.0.0",
                    cacheDir = root.resolve("cache"),
                    pathEnv = "",
                    homeDir = root.resolve("home"),
                    versionReader = { null },
                )
            )

            assertEquals(explicit.toAbsolutePath().normalize().toString(), resolved)
        } finally {
            root.deleteRecursively()
        }
    }

    @Test
    fun binaryResolverSkipsOldPathBinaryAndUsesHomeBinary() {
        val root = Files.createTempDirectory("refact-binary-resolver-home")
        val pathDir = root.resolve("path-bin")
        val homeDir = root.resolve("home")
        val pathRefact = pathDir.resolve("refact")
        val homeRefact = homeDir.resolve(".refact").resolve("bin").resolve("refact")

        try {
            Files.createDirectories(pathRefact.parent)
            Files.createDirectories(homeRefact.parent)
            Files.writeString(pathRefact, "")
            Files.writeString(homeRefact, "")

            val resolved = RefactBinaryResolver.resolve(
                RefactBinaryResolverOptions(
                    minVersion = "8.1.0",
                    pinnedVersion = "8.1.0",
                    cacheDir = root.resolve("cache"),
                    pathEnv = pathDir.toString(),
                    homeDir = homeDir,
                    osName = "Linux",
                    arch = "amd64",
                    versionReader = { path -> if (path == pathRefact) "refact 8.0.0" else "refact 8.1.0" },
                )
            )

            assertEquals(homeRefact.toAbsolutePath().normalize().toString(), resolved)
        } finally {
            root.deleteRecursively()
        }
    }

    @Test
    fun binaryResolverDownloadsPinnedArchiveWhenSystemBinariesAreOld() {
        val root = Files.createTempDirectory("refact-binary-resolver-download")
        val pathDir = root.resolve("path-bin")
        val homeDir = root.resolve("home")
        val pathRefact = pathDir.resolve("refact")
        val homeRefact = homeDir.resolve(".refact").resolve("bin").resolve("refact")
        val downloads = mutableListOf<String>()

        try {
            Files.createDirectories(pathRefact.parent)
            Files.createDirectories(homeRefact.parent)
            Files.writeString(pathRefact, "")
            Files.writeString(homeRefact, "")
            val resolved = RefactBinaryResolver.resolve(
                RefactBinaryResolverOptions(
                    minVersion = "8.1.0",
                    pinnedVersion = "8.1.0",
                    cacheDir = root.resolve("cache"),
                    pathEnv = pathDir.toString(),
                    homeDir = homeDir,
                    osName = "Linux",
                    arch = "amd64",
                    versionReader = { path -> if (path.toString().contains("cache")) "refact 8.1.0" else "refact 7.9.0" },
                    downloader = { uri, dest ->
                        downloads.add(uri.toString())
                        Files.createDirectories(dest.parent)
                        if (uri.toString().endsWith(".sha256")) {
                            val archive = dest.parent.resolve("refact-8.1.0-x86_64-unknown-linux-gnu.tar.gz")
                            Files.writeString(dest, "${sha256(archive)}  archive\n")
                        } else {
                            Files.writeString(dest, "archive")
                        }
                    },
                    extractor = { _, dest, _ -> Files.writeString(dest.resolve("refact"), "") },
                    chmod = {},
                )
            )

            assertEquals(root.resolve("cache").resolve("8.1.0").resolve("x86_64-unknown-linux-gnu").resolve("refact").toString(), resolved)
            assertEquals(
                listOf(
                    "https://github.com/JegernOUTT/refact/releases/download/engine/v8.1.0/refact-8.1.0-x86_64-unknown-linux-gnu.tar.gz",
                    "https://github.com/JegernOUTT/refact/releases/download/engine/v8.1.0/refact-8.1.0-x86_64-unknown-linux-gnu.tar.gz.sha256",
                ),
                downloads,
            )
        } finally {
            root.deleteRecursively()
        }
    }

    @Test
    fun tarTraversalEntryIsRejected() {
        val root = Files.createTempDirectory("refact-binary-resolver-tar-slip")
        try {
            val archive = root.resolve("evil.tar.gz")
            val dest = root.resolve("dest")
            Files.createDirectories(dest)
            Files.write(archive, tarGzEntry("../evil", "oops".toByteArray()))

            val error = runCatching { extractArchive(archive, dest, false) }.exceptionOrNull()

            assertTrue(error is IOException)
            assertEquals("archive entry escapes target directory: ../evil", error?.message)
            assertFalse(Files.exists(root.resolve("evil")))
        } finally {
            root.deleteRecursively()
        }
    }

    @Test
    fun windowsDaemonCommandUsesPlainBinaryWithPathSpaces() {
        val bin = "C:\\Program Files\\Refact\\refact.exe"

        val commands = daemonCommandCandidates(bin, DaemonSpawnOs.Windows)

        assertEquals(listOf(DaemonSpawnCommand(listOf(bin, "daemon"))), commands)
        assertFalse(commands.any { it.argv.contains("cmd") || it.argv.contains("start") })
    }

    @Test
    fun candidateSpawnsButNeverHealthyContinuesFallback() {
        val commands = listOf(
            DaemonSpawnCommand(listOf("first", "daemon")),
            DaemonSpawnCommand(listOf("second", "daemon")),
        )
        val spawned = mutableListOf<List<String>>()
        var polls = 0

        val status = spawnDaemonCandidateUntilHealthy(
            commands = commands,
            spawnCandidate = { spawned.add(it.argv) },
            pollCandidate = {
                polls += 1
                if (polls == 1) throw IOException("not ready")
                DaemonStatus(pid = 22, version = "9.0.0")
            },
        )

        assertEquals(DaemonStatus(pid = 22, version = "9.0.0"), status)
        assertEquals(listOf(commands[0].argv, commands[1].argv), spawned)
        assertEquals(2, polls)
    }

    @Test
    fun candidateHealthyIsAccepted() {
        val commands = listOf(
            DaemonSpawnCommand(listOf("first", "daemon")),
            DaemonSpawnCommand(listOf("second", "daemon")),
        )
        val spawned = mutableListOf<List<String>>()

        val status = spawnDaemonCandidateUntilHealthy(
            commands = commands,
            spawnCandidate = { spawned.add(it.argv) },
            pollCandidate = { DaemonStatus(pid = 11, version = "9.0.0") },
        )

        assertEquals(DaemonStatus(pid = 11, version = "9.0.0"), status)
        assertEquals(listOf(commands[0].argv), spawned)
    }

    @Test
    fun intellijPluginDoesNotContainDirectCustomizationSpawn() {
        val sourceRoot = Path.of("src/main/kotlin/com/smallcloud/refactai")
        val forbidden = listOf("--print-" + "customization", "getCustomization" + "Directly", "fetchCustomization" + "Directly")
        val matches = Files.walk(sourceRoot).use { paths ->
            paths.filter { Files.isRegularFile(it) }
                .filter { it.toString().endsWith(".kt") }
                .flatMap { path ->
                    val text = Files.readString(path)
                    forbidden.filter { text.contains(it) }.map { "$path contains $it" }.stream()
                }
                .toList()
        }

        assertEquals(emptyList<String>(), matches)
    }

    @Test
    fun daemonAlreadyRunningDoesNotSpawn() {
        val current = DaemonStatus(pid = 33, version = "9.0.0")
        var spawns = 0
        var polls = 0
        var shutdowns = 0
        var waitUntilDowns = 0

        val status = ensureDaemonWithHealthGate(
            status = { current },
            pluginVersion = "8.0.0",
            commands = listOf(DaemonSpawnCommand(listOf("refact-lsp", "daemon"))),
            spawnCandidate = { spawns += 1 },
            pollCandidate = { _, _ ->
                polls += 1
                DaemonStatus(pid = 44, version = "9.0.0")
            },
            shutdown = { _, _ -> shutdowns += 1 },
            waitUntilDown = { waitUntilDowns += 1 },
        )

        assertEquals(current, status)
        assertEquals(0, spawns)
        assertEquals(0, polls)
        assertEquals(0, shutdowns)
        assertEquals(0, waitUntilDowns)
    }

    @Test
    fun upgradePollReceivesExpectedVersionAndRejectedOldPid() {
        val current = DaemonStatus(pid = 33, version = "8.1.0")
        var spawns = 0
        var shutdowns = 0
        var waits = 0

        val status = ensureDaemonWithHealthGate(
            status = { current },
            pluginVersion = "8.2.0",
            commands = listOf(DaemonSpawnCommand(listOf("refact", "daemon"))),
            spawnCandidate = { spawns += 1 },
            pollCandidate = { expectedVersion, rejectedPid ->
                assertEquals("8.2.0", expectedVersion)
                assertEquals(33, rejectedPid)
                DaemonStatus(pid = 44, version = "8.2.0")
            },
            shutdown = { _, _ -> shutdowns += 1 },
            waitUntilDown = { waits += 1 },
        )

        assertEquals(DaemonStatus(pid = 44, version = "8.2.0"), status)
        assertEquals(1, spawns)
        assertEquals(1, shutdowns)
        assertEquals(1, waits)
        assertFalse(spawnedDaemonStatusAccepted(DaemonStatus(pid = 33, version = "8.2.0"), "8.2.0", 33))
        assertFalse(spawnedDaemonStatusAccepted(DaemonStatus(pid = 44, version = "8.1.0"), "8.2.0", 33))
        assertTrue(spawnedDaemonStatusAccepted(DaemonStatus(pid = 44, version = "8.2.0"), "8.2.0", 33))
    }
}

private fun Path.deleteRecursively() {
    if (!Files.exists(this)) return
    Files.walk(this).use { paths ->
        paths.sorted(Comparator.reverseOrder()).forEach { Files.deleteIfExists(it) }
    }
}

private fun sha256(path: Path): String {
    val digest = MessageDigest.getInstance("SHA-256")
    digest.update(Files.readAllBytes(path))
    return digest.digest().joinToString("") { "%02x".format(it) }
}

private fun tarGzEntry(name: String, data: ByteArray): ByteArray {
    val raw = ByteArrayOutputStream()
    raw.write(tarHeader(name, data.size, '0'))
    raw.write(data)
    raw.write(ByteArray((512 - (data.size % 512)) % 512))
    raw.write(ByteArray(1024))
    val compressed = ByteArrayOutputStream()
    GZIPOutputStream(compressed).use { it.write(raw.toByteArray()) }
    return compressed.toByteArray()
}

private fun tarHeader(name: String, size: Int, type: Char): ByteArray {
    val header = ByteArray(512)
    writeTarField(header, 0, 100, name)
    writeTarField(header, 100, 8, "0000777")
    writeTarField(header, 108, 8, "0000000")
    writeTarField(header, 116, 8, "0000000")
    writeTarField(header, 124, 12, size.toString(8).padStart(11, '0'))
    writeTarField(header, 136, 12, "00000000000")
    for (index in 148 until 156) header[index] = 32
    header[156] = type.code.toByte()
    writeTarField(header, 257, 6, "ustar")
    writeTarField(header, 263, 2, "00")
    val checksum = header.sumOf { it.toInt() and 0xff }
    writeTarField(header, 148, 8, checksum.toString(8).padStart(6, '0'))
    header[154] = 0
    header[155] = 32
    return header
}

private fun writeTarField(header: ByteArray, start: Int, length: Int, value: String) {
    val bytes = value.toByteArray(Charsets.UTF_8)
    val count = minOf(bytes.size, length)
    bytes.copyInto(header, start, 0, count)
}

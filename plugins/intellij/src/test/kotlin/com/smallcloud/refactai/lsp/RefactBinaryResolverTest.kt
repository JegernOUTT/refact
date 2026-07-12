package com.smallcloud.refactai.lsp

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.net.URI
import java.nio.file.AtomicMoveNotSupportedException
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.StandardCopyOption
import java.security.MessageDigest
import java.util.Comparator

class RefactBinaryResolverTest {
    @Test
    fun explicitBinaryWinsOverBundledSystemAndDownload() {
        val root = Files.createTempDirectory("refact-binary-resolver-explicit-precedence")
        val explicit = root.resolve("explicit").resolve("refact")
        val bundled = root.resolve("plugin").resolve("bin").resolve("dist-x86_64-unknown-linux-gnu").resolve("refact")
        val shared = sharedRefactBinaryPath(root.resolve("home"), "Linux")
        var downloadStarts = 0
        try {
            writeBinary(bundled)
            writeBinary(shared)

            val resolved = RefactBinaryResolver.resolve(
                options(
                    root = root,
                    explicitPath = explicit.toString(),
                    bundledDir = root.resolve("plugin"),
                    onDownloadStart = { downloadStarts++ },
                    downloader = { _, _ -> throw AssertionError("download should not run") },
                )
            )

            assertEquals(explicit.toAbsolutePath().normalize().toString(), resolved)
            assertEquals(0, downloadStarts)
        } finally {
            root.deleteRecursively()
        }
    }

    @Test
    fun bundledBinaryWinsOverSystemAndDownload() {
        val root = Files.createTempDirectory("refact-binary-resolver-bundled-precedence")
        val bundled = root.resolve("plugin").resolve("bin").resolve("dist-x86_64-unknown-linux-gnu").resolve("refact")
        val shared = sharedRefactBinaryPath(root.resolve("home"), "Linux")
        val pathDir = root.resolve("path-bin")
        var downloadStarts = 0
        try {
            writeBinary(bundled)
            writeBinary(shared)
            writeBinary(pathDir.resolve("refact"))

            val resolved = RefactBinaryResolver.resolve(
                options(
                    root = root,
                    bundledDir = root.resolve("plugin"),
                    pathEnv = pathDir.toString(),
                    onDownloadStart = { downloadStarts++ },
                    downloader = { _, _ -> throw AssertionError("download should not run") },
                )
            )

            assertEquals(bundled.toAbsolutePath().normalize().toString(), resolved)
            assertEquals(0, downloadStarts)
        } finally {
            root.deleteRecursively()
        }
    }

    @Test
    fun absentBundledBinaryFallsThroughToSystemBinary() {
        val root = Files.createTempDirectory("refact-binary-resolver-absent-bundle")
        val shared = sharedRefactBinaryPath(root.resolve("home"), "Linux")
        var downloadStarts = 0
        try {
            writeBinary(shared)

            val resolved = RefactBinaryResolver.resolve(
                options(
                    root = root,
                    bundledDir = root.resolve("plugin"),
                    onDownloadStart = { downloadStarts++ },
                    downloader = { _, _ -> throw AssertionError("download should not run") },
                )
            )

            assertEquals(shared.toAbsolutePath().normalize().toString(), resolved)
            assertEquals(0, downloadStarts)
        } finally {
            root.deleteRecursively()
        }
    }

    @Test
    fun pathBinaryWinsOverDownloadWhenSharedIsOld() {
        val root = Files.createTempDirectory("refact-binary-resolver-path-precedence")
        val shared = sharedRefactBinaryPath(root.resolve("home"), "Linux")
        val pathBinary = root.resolve("path-bin").resolve("refact")
        var downloadStarts = 0
        try {
            writeBinary(shared, "old-binary")
            writeBinary(pathBinary, "path-binary")

            val resolved = RefactBinaryResolver.resolve(
                options(
                    root = root,
                    pathEnv = pathBinary.parent.toString(),
                    versionReader = { path -> if (path == pathBinary) "refact 8.1.0" else "refact 8.0.0" },
                    onDownloadStart = { downloadStarts++ },
                    downloader = { _, _ -> throw AssertionError("download should not run") },
                )
            )

            assertEquals(pathBinary.toAbsolutePath().normalize().toString(), resolved)
            assertEquals(0, downloadStarts)
        } finally {
            root.deleteRecursively()
        }
    }

    @Test
    fun sharedBinaryFoundInsideInstallLockDoesNotFireDownloadStart() {
        val root = Files.createTempDirectory("refact-binary-resolver-lock-recheck")
        val shared = sharedRefactBinaryPath(root.resolve("home"), "Linux")
        var sharedVersionReads = 0
        var downloadStarts = 0
        try {
            writeBinary(shared)

            val resolved = RefactBinaryResolver.resolve(
                options(
                    root = root,
                    versionReader = { path ->
                        if (path == shared && sharedVersionReads++ == 0) "refact 8.0.0" else "refact 8.1.0"
                    },
                    onDownloadStart = { downloadStarts++ },
                    downloader = { _, _ -> throw AssertionError("download should not run") },
                )
            )

            assertEquals(shared.toAbsolutePath().normalize().toString(), resolved)
            assertEquals(0, downloadStarts)
            assertEquals(2, sharedVersionReads)
        } finally {
            root.deleteRecursively()
        }
    }

    @Test
    fun downloadsOnlyWhenNoLocalCompatibleBinaryExists() {
        val root = Files.createTempDirectory("refact-binary-resolver-download-only")
        val shared = sharedRefactBinaryPath(root.resolve("home"), "Linux")
        val downloads = mutableListOf<String>()
        val events = mutableListOf<String>()
        try {
            writeBinary(shared, "old-binary")

            val resolved = RefactBinaryResolver.resolve(
                options(
                    root = root,
                    bundledDir = root.resolve("plugin"),
                    versionReader = { path ->
                        if (Files.isRegularFile(path) && Files.readString(path) == "new-binary") {
                            "refact 8.1.0"
                        } else {
                            "refact 8.0.0"
                        }
                    },
                    onDownloadStart = { events.add("download-start") },
                    downloader = { uri, dest ->
                        events.add("download:${uri.toString().substringAfterLast('/')}")
                        downloads.add(uri.toString())
                        Files.createDirectories(dest.parent)
                        if (uri.toString().endsWith(".sha256")) {
                            val archive = dest.parent.resolve("refact-8.1.0-x86_64-unknown-linux-gnu.tar.gz")
                            Files.writeString(dest, "${sha256(archive)}  archive\n")
                        } else {
                            Files.writeString(dest, "archive")
                        }
                    },
                    extractor = { _, dest, _ -> Files.writeString(dest.resolve("refact"), "new-binary") },
                    chmod = {},
                )
            )

            assertEquals(shared.toAbsolutePath().normalize().toString(), resolved)
            assertEquals("new-binary", Files.readString(shared))
            assertEquals(2, downloads.size)
            assertTrue(downloads.first().endsWith("refact-8.1.0-x86_64-unknown-linux-gnu.tar.gz"))
            assertTrue(downloads.last().endsWith("refact-8.1.0-x86_64-unknown-linux-gnu.tar.gz.sha256"))
            assertEquals("download-start", events.first())
            assertEquals(3, events.size)
        } finally {
            root.deleteRecursively()
        }
    }

    @Test
    fun binaryPromotionFallsBackWhenAtomicMoveIsUnsupported() {
        val root = Files.createTempDirectory("refact-binary-resolver-atomic-fallback")
        val source = root.resolve("source")
        val target = root.resolve("target")
        val attempts = mutableListOf<Boolean>()
        try {
            Files.writeString(source, "new-binary")
            Files.writeString(target, "old-binary")

            moveReplacingWithAtomicFallback(source, target) { from, to, atomic ->
                attempts.add(atomic)
                if (atomic) {
                    throw AtomicMoveNotSupportedException(from.toString(), to.toString(), "unsupported")
                }
                Files.move(from, to, StandardCopyOption.REPLACE_EXISTING)
            }

            assertEquals(listOf(true, false), attempts)
            assertEquals("new-binary", Files.readString(target))
            assertFalse(Files.exists(source))
        } finally {
            root.deleteRecursively()
        }
    }

    @Test
    fun resolveLocalOrNullReturnsCompatibleSharedBinaryWithoutDownloading() {
        val root = Files.createTempDirectory("refact-binary-resolver-local-shared")
        val shared = sharedRefactBinaryPath(root.resolve("home"), "Linux")
        try {
            writeBinary(shared)

            val resolved = RefactBinaryResolver.resolveLocalOrNull(options(root = root))

            assertEquals(shared.toString(), resolved)
        } finally {
            root.deleteRecursively()
        }
    }

    @Test
    fun resolveLocalOrNullReturnsNullInsteadOfDownloading() {
        val root = Files.createTempDirectory("refact-binary-resolver-local-none")
        try {
            val resolved = RefactBinaryResolver.resolveLocalOrNull(
                options(
                    root = root,
                    downloader = { _, _ -> throw AssertionError("resolveLocalOrNull must never download") },
                )
            )

            assertEquals(null, resolved)
        } finally {
            root.deleteRecursively()
        }
    }

    @Test
    fun pinnedReleaseMissingFallsBackToLatestPublishedEngine() {
        val root = Files.createTempDirectory("refact-binary-resolver-fallback-latest")
        val shared = sharedRefactBinaryPath(root.resolve("home"), "Linux")
        val downloads = mutableListOf<String>()
        val fallbacks = mutableListOf<Pair<String, String>>()
        try {
            writeBinary(shared, "old-binary")

            val resolved = RefactBinaryResolver.resolveDetailed(
                options(
                    root = root,
                    versionReader = { path ->
                        if (Files.isRegularFile(path) && Files.readString(path) == "new-binary") {
                            "refact 8.0.5"
                        } else {
                            "refact 7.9.0"
                        }
                    },
                    downloader = { uri, dest ->
                        downloads.add(uri.toString())
                        if (uri.toString().contains("8.1.0")) {
                            throw DownloadFailedException(404, "download failed 404 $uri")
                        }
                        Files.createDirectories(dest.parent)
                        if (uri.toString().endsWith(".sha256")) {
                            val archive = dest.parent.resolve("refact-8.0.5-x86_64-unknown-linux-gnu.tar.gz")
                            Files.writeString(dest, "${sha256(archive)}  archive\n")
                        } else {
                            Files.writeString(dest, "archive")
                        }
                    },
                    extractor = { _, dest, _ -> Files.writeString(dest.resolve("refact"), "new-binary") },
                    chmod = {},
                    releaseVersionsProvider = { listOf("8.0.1", "8.0.5", "7.5.0") },
                    onFallbackVersion = { pinned, chosen -> fallbacks.add(pinned to chosen) },
                )
            )

            assertEquals(shared.toString(), resolved.path)
            assertEquals("8.0.5", resolved.version)
            assertEquals("8.1.0", resolved.fallbackFromVersion)
            assertEquals(listOf("8.1.0" to "8.0.5"), fallbacks)
            assertEquals("new-binary", Files.readString(shared))
            assertTrue(downloads.first().contains("engine/v8.1.0"))
            assertTrue(downloads.last().contains("engine/v8.0.5"))
        } finally {
            root.deleteRecursively()
        }
    }

    @Test
    fun pinnedReleaseMissingReusesSharedBinaryMatchingLatestPublished() {
        val root = Files.createTempDirectory("refact-binary-resolver-fallback-reuse")
        val shared = sharedRefactBinaryPath(root.resolve("home"), "Linux")
        val downloads = mutableListOf<String>()
        val fallbacks = mutableListOf<Pair<String, String>>()
        try {
            writeBinary(shared, "fallback-binary")

            val resolved = RefactBinaryResolver.resolveDetailed(
                options(
                    root = root,
                    versionReader = { "refact 8.0.5" },
                    downloader = { uri, _ ->
                        downloads.add(uri.toString())
                        throw DownloadFailedException(404, "download failed 404 $uri")
                    },
                    releaseVersionsProvider = { listOf("8.0.5") },
                    onFallbackVersion = { pinned, chosen -> fallbacks.add(pinned to chosen) },
                )
            )

            assertEquals(shared.toString(), resolved.path)
            assertEquals("8.0.5", resolved.version)
            assertEquals("8.1.0", resolved.fallbackFromVersion)
            assertEquals(listOf("8.1.0" to "8.0.5"), fallbacks)
            assertEquals("fallback-binary", Files.readString(shared))
            assertEquals(1, downloads.size)
        } finally {
            root.deleteRecursively()
        }
    }

    @Test
    fun pinnedReleaseMissingWithNoOtherPublishedVersionRethrowsOriginalError() {
        val root = Files.createTempDirectory("refact-binary-resolver-fallback-none")
        try {
            val error = runCatching {
                RefactBinaryResolver.resolveDetailed(
                    options(
                        root = root,
                        versionReader = { "refact 7.9.0" },
                        downloader = { uri, _ -> throw DownloadFailedException(404, "download failed 404 $uri") },
                        releaseVersionsProvider = { listOf("8.1.0") },
                    )
                )
            }.exceptionOrNull()

            assertTrue(error is DownloadFailedException)
            assertEquals(404, (error as DownloadFailedException).statusCode)
        } finally {
            root.deleteRecursively()
        }
    }

    @Test
    fun nonNotFoundDownloadErrorDoesNotBrowsePublishedReleases() {
        val root = Files.createTempDirectory("refact-binary-resolver-fallback-503")
        var browsed = false
        try {
            val error = runCatching {
                RefactBinaryResolver.resolveDetailed(
                    options(
                        root = root,
                        versionReader = { "refact 7.9.0" },
                        downloader = { uri, _ -> throw DownloadFailedException(503, "download failed 503 $uri") },
                        releaseVersionsProvider = {
                            browsed = true
                            listOf("8.0.5")
                        },
                    )
                )
            }.exceptionOrNull()

            assertTrue(error is DownloadFailedException)
            assertEquals(503, (error as DownloadFailedException).statusCode)
            assertFalse(browsed)
        } finally {
            root.deleteRecursively()
        }
    }

    @Test
    fun prereleaseTagFallbackAcceptsBaseVersionBinary() {
        val root = Files.createTempDirectory("refact-binary-resolver-fallback-prerelease")
        val shared = sharedRefactBinaryPath(root.resolve("home"), "Linux")
        val downloads = mutableListOf<String>()
        try {
            val resolved = RefactBinaryResolver.resolveDetailed(
                options(
                    root = root,
                    versionReader = { path ->
                        if (Files.isRegularFile(path) && Files.readString(path) == "new-binary") {
                            "refact 8.0.5"
                        } else {
                            null
                        }
                    },
                    downloader = { uri, dest ->
                        downloads.add(uri.toString())
                        if (uri.toString().contains("8.1.0")) {
                            throw DownloadFailedException(404, "download failed 404 $uri")
                        }
                        Files.createDirectories(dest.parent)
                        if (uri.toString().endsWith(".sha256")) {
                            val archive = dest.parent.resolve("refact-8.0.5-rc-abc-x86_64-unknown-linux-gnu.tar.gz")
                            Files.writeString(dest, "${sha256(archive)}  archive\n")
                        } else {
                            Files.writeString(dest, "archive")
                        }
                    },
                    extractor = { _, dest, _ -> Files.writeString(dest.resolve("refact"), "new-binary") },
                    chmod = {},
                    releaseVersionsProvider = { listOf("8.0.5-rc-abc", "8.0.1") },
                )
            )

            assertEquals(shared.toString(), resolved.path)
            assertEquals("8.0.5", resolved.version)
            assertEquals("8.1.0", resolved.fallbackFromVersion)
            assertTrue(downloads.any { it.contains("engine/v8.0.5-rc-abc") })
        } finally {
            root.deleteRecursively()
        }
    }

    @Test
    fun parsePublishedEngineVersionsKeepsOnlyEngineTags() {
        val json = """
            [
                {"tag_name": "release/v8.2.3", "prerelease": false},
                {"tag_name": "engine/v8.2.3", "prerelease": false},
                {"tag_name": "engine/v8.2.4-foo-abc", "prerelease": true},
                {"tag_name": "v8.2.3-build-tag", "prerelease": false},
                {"name": "no tag"},
                {"tag_name": "engine/v"}
            ]
        """.trimIndent()

        assertEquals(listOf("8.2.3", "8.2.4-foo-abc"), parsePublishedEngineVersions(json))
        assertEquals(emptyList<String>(), parsePublishedEngineVersions("not json"))
        assertEquals(emptyList<String>(), parsePublishedEngineVersions("{\"tag_name\": \"engine/v1\"}"))
    }
}

private fun options(
    root: Path,
    explicitPath: String? = null,
    bundledDir: Path? = null,
    pathEnv: String = "",
    versionReader: (Path) -> String? = { "refact 8.1.0" },
    onDownloadStart: () -> Unit = {},
    downloader: (URI, Path) -> Unit = { _, _ -> throw AssertionError("download should not run") },
    extractor: (Path, Path, Boolean) -> Unit = { _, _, _ -> },
    chmod: (Path) -> Unit = {},
    releaseVersionsProvider: () -> List<String> = { throw AssertionError("release browsing should not run") },
    onFallbackVersion: (String, String) -> Unit = { _, _ -> },
): RefactBinaryResolverOptions {
    return RefactBinaryResolverOptions(
        explicitPath = explicitPath,
        bundledDir = bundledDir,
        minVersion = "8.1.0",
        pinnedVersion = "8.1.0",
        cacheDir = root.resolve("cache"),
        pathEnv = pathEnv,
        homeDir = root.resolve("home"),
        osName = "Linux",
        arch = "amd64",
        versionReader = versionReader,
        onDownloadStart = onDownloadStart,
        downloader = downloader,
        extractor = extractor,
        chmod = chmod,
        releaseVersionsProvider = releaseVersionsProvider,
        onFallbackVersion = onFallbackVersion,
    )
}

private fun writeBinary(path: Path, content: String = "binary") {
    Files.createDirectories(path.parent)
    Files.writeString(path, content)
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

private fun Path.deleteRecursively() {
    if (!Files.exists(this)) return
    Files.walk(this).use { paths ->
        paths.sorted(Comparator.reverseOrder()).forEach { Files.deleteIfExists(it) }
    }
}

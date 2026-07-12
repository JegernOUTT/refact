@file:OptIn(okhttp3.ExperimentalOkHttpApi::class)

package com.smallcloud.refactai.lsp

import mockwebserver3.MockResponse
import mockwebserver3.MockWebServer
import mockwebserver3.Dispatcher
import mockwebserver3.RecordedRequest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.ByteArrayOutputStream
import java.net.URI
import java.nio.file.Files
import java.nio.file.Path
import java.io.IOException
import java.security.MessageDigest
import java.util.Comparator
import java.util.concurrent.Callable
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
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
    fun binaryResolverPrefersSharedHomeBinaryBeforePath() {
        val root = Files.createTempDirectory("refact-binary-resolver-home-first")
        val pathDir = root.resolve("path-bin")
        val homeDir = root.resolve("home")
        val pathRefact = pathDir.resolve("refact")
        val homeRefact = sharedRefactBinaryPath(homeDir, "Linux")

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
                    versionReader = { "refact 8.1.0" },
                )
            )

            assertEquals(homeRefact.toAbsolutePath().normalize().toString(), resolved)
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
        val homeRefact = sharedRefactBinaryPath(homeDir, "Linux")

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
        val homeRefact = sharedRefactBinaryPath(homeDir, "Linux")
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
                    versionReader = { path ->
                        if (Files.isRegularFile(path) && Files.readString(path) == "new-binary") "refact 8.1.0" else "refact 7.9.0"
                    },
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
                    extractor = { _, dest, _ -> Files.writeString(dest.resolve("refact"), "new-binary") },
                    chmod = {},
                )
            )

            assertEquals(homeRefact.toString(), resolved)
            assertEquals("new-binary", Files.readString(homeRefact))
            assertFalse(Files.exists(root.resolve("cache").resolve("8.1.0").resolve("x86_64-unknown-linux-gnu").resolve("refact")))
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
    fun binaryResolverWaitsOnSharedInstallLockAndRechecksCompatibleBinary() {
        val root = Files.createTempDirectory("refact-binary-resolver-lock")
        val homeDir = root.resolve("home")
        val homeRefact = sharedRefactBinaryPath(homeDir, "Linux")
        val sharedBinDir = homeRefact.parent
        val sharedLock = sharedBinDir.resolve(".install.lock")
        val privateLock = sharedBinDir.resolve(".refact-install.lock")
        val initialVersionRead = java.util.concurrent.CountDownLatch(1)
        val downloads = AtomicInteger(0)
        val executor = Executors.newSingleThreadExecutor()

        try {
            Files.createDirectories(sharedBinDir)
            Files.writeString(homeRefact, "old-binary")
            Files.writeString(sharedLock, "pid=${ProcessHandle.current().pid()}\ntimestamp_ms=${System.currentTimeMillis()}\n")

            val resolved = executor.submit(Callable {
                RefactBinaryResolver.resolve(
                    RefactBinaryResolverOptions(
                        minVersion = "8.1.0",
                        pinnedVersion = "8.1.0",
                        cacheDir = root.resolve("cache"),
                        pathEnv = "",
                        homeDir = homeDir,
                        osName = "Linux",
                        arch = "amd64",
                        versionReader = { path ->
                            if (path == homeRefact) {
                                initialVersionRead.countDown()
                            }
                            if (Files.readString(path) == "new-binary") "refact 8.1.0" else "refact 7.9.0"
                        },
                        downloader = { _, _ -> downloads.incrementAndGet() },
                        installLockRetryMs = 10,
                        installLockTimeoutMs = 2_000,
                        chmod = {},
                    )
                )
            })

            assertTrue(initialVersionRead.await(1, TimeUnit.SECONDS))
            Thread.sleep(50)
            assertTrue(Files.exists(sharedLock))
            Files.writeString(homeRefact, "new-binary")
            Files.delete(sharedLock)

            assertEquals(homeRefact.toString(), resolved.get(1, TimeUnit.SECONDS))
            assertEquals(0, downloads.get())
            assertFalse(Files.exists(sharedLock))
            assertFalse(Files.exists(privateLock))
            assertEquals("new-binary", Files.readString(homeRefact))
        } finally {
            executor.shutdownNow()
            root.deleteRecursively()
        }
    }

    @Test
    fun binaryResolverBreaksDeadOldSharedInstallLockAndRemovesIt() {
        val root = Files.createTempDirectory("refact-binary-resolver-stale-lock")
        val homeDir = root.resolve("home")
        val homeRefact = sharedRefactBinaryPath(homeDir, "Linux")
        val sharedLock = homeRefact.parent.resolve(".install.lock")
        val downloads = mutableListOf<String>()
        val lockSnapshots = mutableListOf<String>()

        try {
            Files.createDirectories(homeRefact.parent)
            Files.writeString(sharedLock, "pid=9223372036854775807\ntimestamp_ms=1000\n")

            val resolved = RefactBinaryResolver.resolve(
                RefactBinaryResolverOptions(
                    minVersion = "8.1.0",
                    pinnedVersion = "8.1.0",
                    cacheDir = root.resolve("cache"),
                    pathEnv = "",
                    homeDir = homeDir,
                    osName = "Linux",
                    arch = "amd64",
                    versionReader = { path ->
                        if (Files.isRegularFile(path) && Files.readString(path) == "new-binary") "refact 8.1.0" else "refact 7.9.0"
                    },
                    downloader = { uri, dest ->
                        downloads.add(uri.toString())
                        lockSnapshots.add(Files.readString(sharedLock))
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
                    installLockRetryMs = 10,
                    installLockTimeoutMs = 500,
                    installLockStaleMs = 100,
                    installLockNowMs = { 10_000 },
                )
            )

            assertEquals(homeRefact.toString(), resolved)
            assertEquals("new-binary", Files.readString(homeRefact))
            assertEquals(2, downloads.size)
            assertTrue(lockSnapshots.any { it.contains("pid=") && it.contains("timestamp_ms=10000") })
            assertFalse(Files.exists(sharedLock))
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
    fun linuxDaemonCommandKeepsDetachedFallbacks() {
        val bin = "/home/user/.refact/bin/refact"

        val commands = daemonCommandCandidates(bin, DaemonSpawnOs.Linux)

        assertEquals(
            listOf(
                DaemonSpawnCommand(listOf("setsid", bin, "daemon")),
                DaemonSpawnCommand(listOf("nohup", bin, "daemon")),
                DaemonSpawnCommand(listOf(bin, "daemon")),
            ),
            commands,
        )
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
            expectedVersion = "8.0.0",
            expectedExecutableSha256 = null,
            commands = listOf(DaemonSpawnCommand(listOf("refact-lsp", "daemon"))),
            spawnCandidate = { spawns += 1 },
            pollCandidate = { _, _, _ ->
                polls += 1
                DaemonStatus(pid = 44, version = "9.0.0")
            },
            shutdown = { _, _ -> shutdowns += 1 },
            waitUntilDown = { _, _, _ ->
                waitUntilDowns += 1
                null
            },
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
            expectedVersion = "8.2.0",
            expectedExecutableSha256 = null,
            commands = listOf(DaemonSpawnCommand(listOf("refact", "daemon"))),
            spawnCandidate = { spawns += 1 },
            pollCandidate = { expectedVersion, rejectedPid, expectedHash ->
                assertEquals("8.2.0", expectedVersion)
                assertEquals(33, rejectedPid)
                assertEquals(null, expectedHash)
                DaemonStatus(pid = 44, version = "8.2.0")
            },
            shutdown = { _, _ -> shutdowns += 1 },
            waitUntilDown = { current, expectedVersion, _ ->
                assertEquals(33, current.pid)
                assertEquals("8.2.0", expectedVersion)
                waits += 1
                null
            },
        )

        assertEquals(DaemonStatus(pid = 44, version = "8.2.0"), status)
        assertEquals(1, spawns)
        assertEquals(1, shutdowns)
        assertEquals(1, waits)
        assertFalse(spawnedDaemonStatusAccepted(DaemonStatus(pid = 33, version = "8.2.0"), "8.2.0", 33, null))
        assertFalse(spawnedDaemonStatusAccepted(DaemonStatus(pid = 44, version = "8.1.0"), "8.2.0", 33, null))
        assertTrue(spawnedDaemonStatusAccepted(DaemonStatus(pid = 44, version = "8.2.0"), "8.2.0", 33, null))
    }

    @Test
    fun upgradeWaitReturnsDifferentCompatibleDaemonWithoutSpawning() {
        val current = DaemonStatus(pid = 33, version = "8.1.0", port = 8488)
        val compatible = DaemonStatus(pid = 44, version = "8.2.0", port = 9499)
        var spawns = 0
        var polls = 0
        var shutdowns = 0
        var waits = 0

        val status = ensureDaemonWithHealthGate(
            status = { current },
            expectedVersion = "8.2.0",
            expectedExecutableSha256 = null,
            commands = listOf(DaemonSpawnCommand(listOf("refact", "daemon"))),
            spawnCandidate = { spawns += 1 },
            pollCandidate = { _, _, _ ->
                polls += 1
                compatible
            },
            shutdown = { _, _ -> shutdowns += 1 },
            waitUntilDown = { oldDaemon, expectedVersion, _ ->
                assertEquals(current, oldDaemon)
                assertEquals("8.2.0", expectedVersion)
                waits += 1
                compatible
            },
        )

        assertEquals(compatible, status)
        assertEquals(0, spawns)
        assertEquals(0, polls)
        assertEquals(1, shutdowns)
        assertEquals(1, waits)
    }

    @Test
    fun daemonStatusMatchingHonorsExecutableHashOnlyForSameVersion() {
        val newer = DaemonStatus(pid = 1, version = "9.0.0", executableSha256 = "bbb")
        assertTrue(daemonStatusMatchesExpected(newer, "8.2.3", "aaa"))

        val older = DaemonStatus(pid = 1, version = "8.2.2", executableSha256 = "aaa")
        assertFalse(daemonStatusMatchesExpected(older, "8.2.3", "aaa"))

        val sameVersionSameHash = DaemonStatus(pid = 1, version = "8.2.3", executableSha256 = "aaa")
        assertTrue(daemonStatusMatchesExpected(sameVersionSameHash, "8.2.3", "aaa"))

        val sameVersionDifferentHash = DaemonStatus(pid = 1, version = "8.2.3", executableSha256 = "bbb")
        assertFalse(daemonStatusMatchesExpected(sameVersionDifferentHash, "8.2.3", "aaa"))

        val sameVersionNoReportedHash = DaemonStatus(pid = 1, version = "8.2.3")
        assertTrue(daemonStatusMatchesExpected(sameVersionNoReportedHash, "8.2.3", "aaa"))

        val sameVersionNoExpectedHash = DaemonStatus(pid = 1, version = "8.2.3", executableSha256 = "bbb")
        assertTrue(daemonStatusMatchesExpected(sameVersionNoExpectedHash, "8.2.3", null))
    }

    @Test
    fun executableHashVerificationOnlyAppliesToSameVersionDaemonsReportingHashes() {
        assertTrue(shouldVerifyDaemonExecutableHash(DaemonStatus(pid = 1, version = "8.2.3", executableSha256 = "aaa"), "8.2.3"))
        assertFalse(shouldVerifyDaemonExecutableHash(DaemonStatus(pid = 1, version = "8.2.3"), "8.2.3"))
        assertFalse(shouldVerifyDaemonExecutableHash(DaemonStatus(pid = 1, version = "9.0.0", executableSha256 = "aaa"), "8.2.3"))
        assertFalse(shouldVerifyDaemonExecutableHash(DaemonStatus(pid = 1, version = "8.2.3", executableSha256 = ""), "8.2.3"))
    }

    @Test
    fun binaryHashCacheRecomputesWhenFileChanges() {
        val root = Files.createTempDirectory("refact-binary-hash-cache")
        try {
            val binary = root.resolve("refact")
            Files.writeString(binary, "engine-build-one")
            Files.setLastModifiedTime(binary, java.nio.file.attribute.FileTime.fromMillis(1_000_000))

            val first = RefactBinaryHashCache.sha256OrNull(binary)
            val cached = RefactBinaryHashCache.sha256OrNull(binary)
            assertEquals(first, cached)
            assertEquals(64, first?.length)

            Files.writeString(binary, "engine-build-two!")
            Files.setLastModifiedTime(binary, java.nio.file.attribute.FileTime.fromMillis(2_000_000))
            val changed = RefactBinaryHashCache.sha256OrNull(binary)
            assertTrue(first != changed)

            assertEquals(null, RefactBinaryHashCache.sha256OrNull(root.resolve("missing")))
        } finally {
            root.toFile().deleteRecursively()
        }
    }

    @Test
    fun upgradeWaitAndSpawnPollRejectSameVersionDifferentHashReplacements() {
        val old = DaemonStatus(pid = 33, version = "8.2.3", executableSha256 = "old", port = 8488)
        val wrongHash = DaemonStatus(pid = 44, version = "8.2.3", executableSha256 = "bbb", port = 8488)
        val rightHash = DaemonStatus(pid = 44, version = "8.2.3", executableSha256 = "aaa", port = 8488)
        val noHash = DaemonStatus(pid = 44, version = "8.2.3", port = 8488)

        assertFalse(daemonUpgradeWaitSatisfied(old, wrongHash, "8.2.3", "aaa"))
        assertTrue(daemonUpgradeWaitSatisfied(old, rightHash, "8.2.3", "aaa"))
        assertTrue(daemonUpgradeWaitSatisfied(old, noHash, "8.2.3", "aaa"))

        assertFalse(daemonUpgradeWaitFinished(old, wrongHash, old, "8.2.3", "aaa"))
        assertTrue(daemonUpgradeWaitFinished(old, wrongHash, null, "8.2.3", "aaa"))

        assertFalse(spawnedDaemonStatusAccepted(wrongHash, "8.2.3", null, "aaa"))
        assertTrue(spawnedDaemonStatusAccepted(rightHash, "8.2.3", null, "aaa"))
        assertTrue(spawnedDaemonStatusAccepted(noHash, "8.2.3", null, "aaa"))
    }

    @Test
    fun sameVersionDifferentExecutableHashTriggersDaemonUpgrade() {
        val current = DaemonStatus(pid = 33, version = "8.2.3", executableSha256 = "bbb", port = 8488)
        val replacement = DaemonStatus(pid = 44, version = "8.2.3", executableSha256 = "aaa", port = 8488)
        var spawns = 0
        var shutdowns = 0
        var waits = 0

        val status = ensureDaemonWithHealthGate(
            status = { current },
            expectedVersion = "8.2.3",
            expectedExecutableSha256 = "aaa",
            commands = listOf(DaemonSpawnCommand(listOf("refact", "daemon"))),
            spawnCandidate = { spawns += 1 },
            pollCandidate = { _, rejectedPid, expectedHash ->
                assertEquals(33, rejectedPid)
                assertEquals("aaa", expectedHash)
                replacement
            },
            shutdown = { _, reason ->
                assertEquals("upgrade", reason)
                shutdowns += 1
            },
            waitUntilDown = { _, _, _ ->
                waits += 1
                null
            },
        )

        assertEquals(replacement, status)
        assertEquals(1, shutdowns)
        assertEquals(1, waits)
        assertEquals(1, spawns)
    }

    @Test
    fun sameVersionSameExecutableHashReusesRunningDaemon() {
        val current = DaemonStatus(pid = 33, version = "8.2.3", executableSha256 = "aaa", port = 8488)
        var spawns = 0
        var shutdowns = 0

        val status = ensureDaemonWithHealthGate(
            status = { current },
            expectedVersion = "8.2.3",
            expectedExecutableSha256 = "aaa",
            commands = listOf(DaemonSpawnCommand(listOf("refact", "daemon"))),
            spawnCandidate = { spawns += 1 },
            pollCandidate = { _, _, _ -> current },
            shutdown = { _, _ -> shutdowns += 1 },
            waitUntilDown = { _, _, _ -> null },
        )

        assertEquals(current, status)
        assertEquals(0, spawns)
        assertEquals(0, shutdowns)
    }

    @Test
    fun ensureDaemonAcceptsDaemonMatchingResolvedEngineVersionOlderThanPlugin() {
        val root = Files.createTempDirectory("refact-daemon-required-version")
        val originalHome = System.getProperty("user.home")
        val server = MockWebServer()
        try {
            server.start()
            System.setProperty("user.home", root.toString())
            val binPath = root.resolve("refact")
            Files.writeString(binPath, "fallback-engine")
            server.enqueue(
                MockResponse.Builder()
                    .code(200)
                    .addHeader("Content-Type", "application/json")
                    .body("{\"pid\":55,\"version\":\"8.1.0\",\"port\":${server.port},\"workers\":1}")
                    .build()
            )

            val client = HttpRefactDaemonClient(portProvider = { server.port }, pluginVersionProvider = { "9.9.9" })
            val status = client.ensureDaemon(binPath.toString(), "8.1.0")

            assertEquals(55, status.pid)
            assertEquals("8.1.0", status.version)
            assertEquals("/daemon/v1/status", server.takeRequest().path)
            assertEquals(null, server.takeRequest(100, TimeUnit.MILLISECONDS))
        } finally {
            server.shutdown()
            System.setProperty("user.home", originalHome)
            root.deleteRecursively()
        }
    }

    @Test
    fun daemonStatusDiscoversTokenFromDaemonJson() {
        val root = Files.createTempDirectory("refact-daemon-info")
        val originalHome = System.getProperty("user.home")
        val server = MockWebServer()
        try {
            server.start()
            System.setProperty("user.home", root.toString())
            val daemonDir = root.resolve(".cache").resolve("refact").resolve("daemon")
            Files.createDirectories(daemonDir)
            Files.writeString(daemonDir.resolve("daemon.json"), "{\"port\":${server.port},\"auth_token\":\"secret-token\"}")
            server.enqueue(
                MockResponse.Builder()
                    .code(200)
                    .addHeader("Content-Type", "application/json")
                    .body("{\"pid\":77,\"version\":\"8.1.0\",\"port\":${server.port},\"workers\":2}")
                    .build()
            )
            server.enqueue(
                MockResponse.Builder()
                    .code(200)
                    .addHeader("Content-Type", "application/json")
                    .body("{\"project_id\":\"project-token\"}")
                    .build()
            )

            val client = HttpRefactDaemonClient(portProvider = { server.port }, pluginVersionProvider = { "8.1.0" })
            val status = client.status()
            val project = client.openProject(root.toString(), LSPConfig(), status)

            assertEquals("secret-token", status.authToken)
            assertEquals("project-token", project.projectId)
            val statusRequest = server.takeRequest()
            assertEquals("/daemon/v1/status", statusRequest.path)
            assertEquals("Bearer secret-token", statusRequest.headers["Authorization"])
            val openRequest = server.takeRequest()
            assertEquals("/daemon/v1/projects/open", openRequest.path)
            assertEquals("Bearer secret-token", openRequest.headers["Authorization"])
            assertNotNull(openRequest.body.readUtf8())
        } finally {
            server.shutdown()
            System.setProperty("user.home", originalHome)
            root.deleteRecursively()
        }
    }

    @Test
    fun daemonStatusFallsBackToEndpointPortWhenReportedPortIsZero() {
        val root = Files.createTempDirectory("refact-daemon-zero-port")
        val originalHome = System.getProperty("user.home")
        val server = MockWebServer()
        try {
            server.start()
            System.setProperty("user.home", root.toString())
            writeDaemonJson(root, server.port, "zero-token")
            server.enqueue(
                MockResponse.Builder()
                    .code(200)
                    .addHeader("Content-Type", "application/json")
                    .body("{\"pid\":88,\"version\":\"8.1.0\",\"port\":0,\"workers\":3}")
                    .build()
            )
            server.enqueue(
                MockResponse.Builder()
                    .code(200)
                    .addHeader("Content-Type", "application/json")
                    .body("{\"project_id\":\"project-zero\"}")
                    .build()
            )

            val client = HttpRefactDaemonClient(portProvider = { server.port }, pluginVersionProvider = { "8.1.0" })
            val status = client.status()
            val project = client.openProject(root.toString(), LSPConfig(), status)

            assertEquals(88, status.pid)
            assertEquals(server.port, status.port)
            assertEquals("zero-token", status.authToken)
            assertEquals("project-zero", project.projectId)
            assertEquals(server.port, project.daemon.port)
            assertEquals("http://127.0.0.1:${server.port}/p/project-zero/", project.baseUrl.toString())
            val statusRequest = server.takeRequest()
            assertEquals("/daemon/v1/status", statusRequest.path)
            assertEquals("Bearer zero-token", statusRequest.headers["Authorization"])
            val openRequest = server.takeRequest()
            assertEquals("/daemon/v1/projects/open", openRequest.path)
            assertEquals("Bearer zero-token", openRequest.headers["Authorization"])
        } finally {
            server.shutdown()
            System.setProperty("user.home", originalHome)
            root.deleteRecursively()
        }
    }

    @Test
    fun daemonStatusPrefersCompatibleDiskDaemonOverOldPreferredDaemon() {
        val root = Files.createTempDirectory("refact-daemon-compatible-disk")
        val originalHome = System.getProperty("user.home")
        val preferredServer = MockWebServer()
        val diskServer = MockWebServer()
        try {
            preferredServer.start()
            diskServer.start()
            System.setProperty("user.home", root.toString())
            writeDaemonJson(root, diskServer.port, "disk-token")
            preferredServer.enqueue(
                MockResponse.Builder()
                    .code(200)
                    .addHeader("Content-Type", "application/json")
                    .body("{\"pid\":11,\"version\":\"8.1.0\",\"port\":${preferredServer.port},\"workers\":1}")
                    .build()
            )
            diskServer.enqueue(
                MockResponse.Builder()
                    .code(200)
                    .addHeader("Content-Type", "application/json")
                    .body("{\"pid\":22,\"version\":\"8.2.0\",\"port\":${diskServer.port},\"workers\":2}")
                    .build()
            )

            val client = HttpRefactDaemonClient(portProvider = { preferredServer.port }, pluginVersionProvider = { "8.2.0" })
            val status = client.status()

            assertEquals(22, status.pid)
            assertEquals("8.2.0", status.version)
            assertEquals(diskServer.port, status.port)
            assertEquals("disk-token", status.authToken)
            val preferredRequest = preferredServer.takeRequest()
            assertEquals("/daemon/v1/status", preferredRequest.path)
            assertEquals(null, preferredRequest.headers["Authorization"])
            val diskRequest = diskServer.takeRequest()
            assertEquals("/daemon/v1/status", diskRequest.path)
            assertEquals("Bearer disk-token", diskRequest.headers["Authorization"])
        } finally {
            preferredServer.shutdown()
            diskServer.shutdown()
            System.setProperty("user.home", originalHome)
            root.deleteRecursively()
        }
    }

    @Test
    fun openProjectUsesSuppliedDaemonEndpointWithoutRediscovery() {
        val root = Files.createTempDirectory("refact-daemon-open-validated")
        val originalHome = System.getProperty("user.home")
        val selectedServer = MockWebServer()
        val staleServer = MockWebServer()
        try {
            selectedServer.start()
            staleServer.start()
            System.setProperty("user.home", root.toString())
            writeDaemonJson(root, staleServer.port, "stale-token")
            staleServer.enqueue(
                MockResponse.Builder()
                    .code(200)
                    .addHeader("Content-Type", "application/json")
                    .body("{\"pid\":11,\"version\":\"8.2.0\",\"port\":${staleServer.port},\"workers\":1}")
                    .build()
            )
            staleServer.enqueue(
                MockResponse.Builder()
                    .code(200)
                    .addHeader("Content-Type", "application/json")
                    .body("{\"project_id\":\"stale-project\"}")
                    .build()
            )
            selectedServer.enqueue(
                MockResponse.Builder()
                    .code(200)
                    .addHeader("Content-Type", "application/json")
                    .body("{\"project_id\":\"selected-project\"}")
                    .build()
            )

            val client = HttpRefactDaemonClient(portProvider = { staleServer.port }, pluginVersionProvider = { "8.2.0" })
            val rediscovered = client.status()
            val selected = DaemonStatus(pid = 22, version = "8.2.0", port = selectedServer.port, authToken = "selected-token")
            val project = client.openProject(root.toString(), LSPConfig(), selected)

            assertEquals(staleServer.port, rediscovered.port)
            assertEquals("stale-token", rediscovered.authToken)
            assertEquals("selected-project", project.projectId)
            assertEquals(selectedServer.port, project.daemon.port)
            assertEquals("selected-token", project.daemon.authToken)
            val openRequest = selectedServer.takeRequest()
            assertEquals("/daemon/v1/projects/open", openRequest.path)
            assertEquals("Bearer selected-token", openRequest.headers["Authorization"])
            val rediscoveredRequest = staleServer.takeRequest()
            assertEquals("/daemon/v1/status", rediscoveredRequest.path)
            assertEquals("Bearer stale-token", rediscoveredRequest.headers["Authorization"])
            assertEquals(null, staleServer.takeRequest(100, TimeUnit.MILLISECONDS))
        } finally {
            selectedServer.shutdown()
            staleServer.shutdown()
            System.setProperty("user.home", originalHome)
            root.deleteRecursively()
        }
    }

    @Test
    fun slowOpenProjectSucceedsWithinLongTimeoutAndEncodesProjectId() {
        val root = Files.createTempDirectory("refact-daemon-slow-open")
        val originalHome = System.getProperty("user.home")
        val server = MockWebServer()
        try {
            server.start()
            System.setProperty("user.home", root.toString())
            writeDaemonJson(root, server.port, "slow-token")
            server.enqueue(
                MockResponse.Builder()
                    .code(200)
                    .addHeader("Content-Type", "application/json")
                    .body("{\"project_id\":\"project with/slash\"}")
                    .bodyDelay(5500, TimeUnit.MILLISECONDS)
                    .build()
            )

            val client = HttpRefactDaemonClient(portProvider = { server.port }, pluginVersionProvider = { "8.1.0" })
            val status = DaemonStatus(pid = 99, version = "8.1.0", port = server.port, authToken = "slow-token")
            val project = client.openProject(root.toString(), LSPConfig(), status)

            assertEquals("project with/slash", project.projectId)
            assertEquals("http://127.0.0.1:${server.port}/p/project%20with%2Fslash/", project.baseUrl.toString())
            val openRequest = server.takeRequest()
            assertEquals("/daemon/v1/projects/open", openRequest.path)
            assertEquals("Bearer slow-token", openRequest.headers["Authorization"])
        } finally {
            server.shutdown()
            System.setProperty("user.home", originalHome)
            root.deleteRecursively()
        }
    }

    @Test
    fun staleTokenAfterPublicStatusRereadsDaemonJsonAndRetriesOnce() {
        val root = Files.createTempDirectory("refact-daemon-stale-token")
        val originalHome = System.getProperty("user.home")
        val server = MockWebServer()
        val authHeaders = mutableListOf<String?>()
        try {
            server.start()
            System.setProperty("user.home", root.toString())
            writeDaemonJson(root, server.port, "stale-token")
            var openCalls = 0
            server.dispatcher = object : Dispatcher() {
                override fun dispatch(request: RecordedRequest): MockResponse {
                    authHeaders.add(request.headers["Authorization"])
                    return when (request.path) {
                        "/daemon/v1/status" -> {
                            writeDaemonJson(root, server.port, "fresh-token")
                            MockResponse.Builder()
                                .code(200)
                                .addHeader("Content-Type", "application/json")
                                .body("{\"pid\":100,\"version\":\"8.1.0\",\"port\":${server.port},\"workers\":1}")
                                .build()
                        }
                        "/daemon/v1/projects/open" -> {
                            openCalls += 1
                            if (request.headers["Authorization"] == "Bearer fresh-token") {
                                MockResponse.Builder()
                                    .code(200)
                                    .addHeader("Content-Type", "application/json")
                                    .body("{\"project_id\":\"project-fresh\"}")
                                    .build()
                            } else {
                                writeDaemonJson(root, server.port, "fresh-token")
                                MockResponse.Builder()
                                    .code(401)
                                    .addHeader("Content-Type", "application/json")
                                    .body("stale token")
                                    .build()
                            }
                        }
                        else -> MockResponse.Builder().code(404).build()
                    }
                }
            }

            val client = HttpRefactDaemonClient(portProvider = { server.port }, pluginVersionProvider = { "8.1.0" })
            val status = DaemonStatus(pid = 100, version = "8.1.0", port = server.port, authToken = "stale-token")
            val project = client.openProject(root.toString(), LSPConfig(), status)

            assertEquals("project-fresh", project.projectId)
            assertEquals("fresh-token", project.daemon.authToken)
            assertEquals(2, openCalls)
            assertEquals(listOf("Bearer stale-token", "Bearer fresh-token"), authHeaders)
        } finally {
            server.shutdown()
            System.setProperty("user.home", originalHome)
            root.deleteRecursively()
        }
    }

    @Test
    fun daemonStatusMissingVersionIsRejected() {
        val root = Files.createTempDirectory("refact-daemon-missing-version")
        val originalHome = System.getProperty("user.home")
        val server = MockWebServer()
        try {
            server.start()
            System.setProperty("user.home", root.toString())
            server.enqueue(
                MockResponse.Builder()
                    .code(200)
                    .addHeader("Content-Type", "application/json")
                    .body("{\"pid\":77,\"port\":${server.port},\"workers\":1}")
                    .build()
            )

            val client = HttpRefactDaemonClient(portProvider = { server.port }, pluginVersionProvider = { "8.1.0" })
            val error = runCatching { client.status() }.exceptionOrNull()

            assertTrue(error is IOException)
            assertEquals("daemon status response missing version", error?.cause?.message)
        } finally {
            server.shutdown()
            System.setProperty("user.home", originalHome)
            root.deleteRecursively()
        }
    }

    @Test
    fun transientDaemonHttpErrorsPreserveStatusAndAreRecoverable() {
        val root = Files.createTempDirectory("refact-daemon-http-error")
        val originalHome = System.getProperty("user.home")
        val server = MockWebServer()
        try {
            server.start()
            System.setProperty("user.home", root.toString())
            server.enqueue(
                MockResponse.Builder()
                    .code(503)
                    .addHeader("Content-Type", "application/json")
                    .body("warming")
                    .build()
            )

            val client = HttpRefactDaemonClient(portProvider = { server.port }, pluginVersionProvider = { "8.1.0" })
            val error = runCatching { client.status() }.exceptionOrNull()

            assertTrue(error is DaemonHttpStatusException)
            val httpError = error as DaemonHttpStatusException
            assertEquals(503, httpError.statusCode)
            assertEquals("warming", httpError.responseBody)
            assertTrue(isRecoverableHttpStatus(httpError))
            assertTrue(isRecoverableHttpStatus(IOException("wrapped", httpError)))
            assertFalse(isRecoverableHttpStatus(DaemonHttpStatusException(401, "auth", URI("http://127.0.0.1/"), "GET")))
        } finally {
            server.shutdown()
            System.setProperty("user.home", originalHome)
            root.deleteRecursively()
        }
    }

    @Test
    fun upgradeWaitAcceptsOldPidGoneOrDifferentCompatibleDaemon() {
        val oldDaemon = DaemonStatus(pid = 33, version = "8.1.0", port = 8488)

        assertTrue(
            daemonUpgradeWaitFinished(
                oldDaemon,
                DaemonStatus(pid = 44, version = "8.2.0", port = 9499),
                DaemonStatus(pid = 33, version = "8.1.0", port = 8488),
                "8.2.0",
                null,
            )
        )
        assertTrue(
            daemonUpgradeWaitFinished(
                oldDaemon,
                DaemonStatus(pid = 33, version = "8.1.0", port = 8488),
                null,
                "8.2.0",
                null,
            )
        )
        assertFalse(
            daemonUpgradeWaitFinished(
                oldDaemon,
                DaemonStatus(pid = 33, version = "8.2.0", port = 8488),
                DaemonStatus(pid = 33, version = "8.2.0", port = 8488),
                "8.2.0",
                null,
            )
        )
        assertFalse(
            daemonUpgradeWaitFinished(
                oldDaemon,
                DaemonStatus(pid = 44, version = "8.1.0", port = 9499),
                DaemonStatus(pid = 33, version = "8.1.0", port = 8488),
                "8.2.0",
                null,
            )
        )
    }
}

private fun writeDaemonJson(root: Path, port: Int, authToken: String) {
    val daemonDir = root.resolve(".cache").resolve("refact").resolve("daemon")
    Files.createDirectories(daemonDir)
    Files.writeString(daemonDir.resolve("daemon.json"), "{\"port\":$port,\"auth_token\":\"$authToken\"}")
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

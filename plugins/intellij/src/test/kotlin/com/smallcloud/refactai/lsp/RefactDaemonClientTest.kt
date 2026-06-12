package com.smallcloud.refactai.lsp

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.nio.file.Files
import java.nio.file.Path
import java.io.IOException

class RefactDaemonClientTest {
    @Test
    fun versionComparisonDetectsOlderDaemon() {
        assertTrue(versionIsOlder("8.1.0", "8.2.0"))
        assertTrue(versionIsOlder("8.1.0-alpha", "8.1.1"))
        assertFalse(versionIsOlder("8.2.0", "8.1.0"))
        assertFalse(versionIsOlder("", "8.1.0"))
    }

    @Test
    fun windowsDaemonCommandUsesPlainBinaryWithPathSpaces() {
        val bin = "C:\\Program Files\\Refact\\refact-lsp.exe"

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
            pollCandidate = {
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
}

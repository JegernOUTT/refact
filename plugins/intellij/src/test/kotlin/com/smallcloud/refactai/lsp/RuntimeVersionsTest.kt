package com.smallcloud.refactai.lsp

import org.junit.Assert.*
import org.junit.Test

class RuntimeVersionsTest {
    @Test
    fun missingPluginMetadataDoesNotBecomeObservedFallback() {
        val missing = com.smallcloud.refactai.resolveObservedPluginVersion(null, null)
        assertNull(missing)
        assertNull(com.smallcloud.refactai.resolveObservedPluginVersion(" ", ""))
        val snapshot = initialRuntimeVersions(missing, "0.0.0")
        assertNull(snapshot.plugin)
        assertEquals("0.0.0", snapshot.requestedVersion)
    }

    @Test
    fun realZeroVersionMetadataRemainsObservedAndXmlPrecedesManifest() {
        listOf("0.0.0" to null, null to "0.0.0").forEach { (xml, manifest) ->
            val metadata = com.smallcloud.refactai.resolveObservedPluginVersion(xml, manifest)
            assertEquals("0.0.0", initialRuntimeVersions(metadata, "0.0.0").plugin!!.version)
        }
        assertEquals("xml-version", com.smallcloud.refactai.resolveObservedPluginVersion("xml-version", "manifest-version"))
        assertEquals("manifest-version", com.smallcloud.refactai.resolveObservedPluginVersion(null, "manifest-version"))
    }

    @Test
    fun strictWorkerJsonStringsAndBounds() {
        val observation = workerVersionObservation("""{"version":"worker-3","commit":"abcdef"}""")!!
        assertEquals("worker-3", observation.version)
        assertEquals("abcdef", observation.commit)
        assertEquals("worker", observation.source)
        listOf("garbage", "[]", "null", "{}", """{"version":3,"commit":true}""",
            """{"version":"bad\nvalue"}""", """{"version":"${"v".repeat(161)}"}""").forEach {
            assertNull(it, workerVersionObservation(it))
        }
        assertEquals(160, runtimeVersionString("v".repeat(160))!!.length)
        assertNull(runtimeVersionString("a\u007fb"))
        assertNull(runtimeVersionString("a\u0085b"))
        assertEquals("commit-only", workerVersionObservation("""{"commit":"commit-only"}""")!!.commit)
    }

    @Test
    fun stalenessPreservesObservationTimeAndPlugin() {
        val plugin = RuntimeVersionObservation(version = "plugin", source = "plugin", observedAtMs = 1)
        val observation = RuntimeVersionObservation(version = "runtime", source = "local", observedAtMs = 2)
        val snapshot = RuntimeVersions(plugin = plugin, selected = observation, daemon = observation, worker = observation)
        val stale = snapshot.lastObserved()
        assertEquals(plugin, stale.plugin)
        listOf(stale.selected, stale.daemon, stale.worker).forEach {
            assertEquals("last_observed", it!!.state)
            assertEquals(2L, it.observedAtMs)
        }
        assertEquals("observed", snapshot.selected!!.state)
        assertNull(stale.installed)
        assertNull(stale.staged)
    }
}

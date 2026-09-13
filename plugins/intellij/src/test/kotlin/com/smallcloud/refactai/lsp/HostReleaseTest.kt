package com.smallcloud.refactai.lsp

import com.google.gson.JsonObject
import org.junit.Assert.*
import org.junit.Test
import java.nio.file.Files
import java.nio.file.Path
import java.util.Base64
import java.util.zip.ZipEntry
import java.util.zip.ZipOutputStream

class HostReleaseTest {
    private class Host : ReleaseHost {
        override var running = ReleaseRunning("e".repeat(64), "1", "session")
        var identity = ReleaseIdentity("e".repeat(64), "1")
        var accept = true; var signature = true; var version = "2"; var calls = 0; var fail = false
        override fun installed() = identity
        override fun inspectAndVerifySignature(path: Path): String { check(signature); return version }
        override fun confirm(request: ReleaseRequest) = accept
        override fun scheduleAfterRestart(path: Path) { calls++; if (fail) error("uncertain SDK failure") }
    }
    private class Fixture {
        val host = Host()
        val dir = Files.createTempDirectory("release-test")
        val bytes = java.io.ByteArrayOutputStream().also { output ->
            ZipOutputStream(output).use { it.putNextEntry(ZipEntry("refact/lib/refact.jar")); it.write(byteArrayOf(1, 2, 3)); it.closeEntry() }
        }.toByteArray()
        val request = ReleaseRequest(1, "a".repeat(64), "grant", "project", "session", "jetbrains", "install",
            "candidate", "b".repeat(64), "proof", releaseHash(bytes), manifestHash(zipManifest(bytes)), "2",
            "e".repeat(64), "e".repeat(64), zipManifest(bytes))
        var envelope = releaseJson.toJsonTree(mapOf("request" to request, "artifact_base64" to Base64.getEncoder().encodeToString(bytes))).asJsonObject
        val reports = mutableListOf<Pair<String, JsonObject>>()
        val transport = object : ReleaseTransport {
            override fun post(project: String, endpoint: String, body: Any): JsonObject {
                if (endpoint == "artifact") return envelope
                reports.add(endpoint to releaseJson.toJsonTree(body).asJsonObject); return JsonObject()
            }
        }
        fun owner() = HostReleaseOwner(transport, host, ReleaseJournal(dir))
        fun release() = owner().release("project", request.prepared_hash, "grant")
        fun rejected() { try { release(); fail("must reject") } catch (_: IllegalArgumentException) {} catch (_: IllegalStateException) {} }
    }
    @Test fun manifestMatchesRustSortedJson() {
        assertEquals(releaseHash("{\"a\":\"1\",\"z\":\"2\"}".toByteArray()), manifestHash(mapOf("z" to "2", "a" to "1")))
    }
    @Test fun confirmationCancellationHasNoInstall() {
        val f = Fixture(); f.host.accept = false; assertEquals("declined", f.release())
        assertEquals(0, f.host.calls); assertEquals("declined", f.reports.last().second["status"].asString)
    }
    @Test fun sdkReturnIsPendingRestartNotRunningSuccessAndDoubleDoesNotReplay() {
        val f = Fixture(); assertTrue(f.release().contains("pending_restart")); f.release()
        assertEquals(1, f.host.calls)
        val receipt = f.reports.first().second
        assertEquals("installer_returned", receipt["status"].asString)
        assertEquals("1", receipt.getAsJsonObject("running")["version"].asString)
        assertEquals("1", receipt.getAsJsonObject("installed")["version"].asString)
    }
    @Test fun unknownInstallerOutcomeNeverReplays() {
        val f = Fixture(); f.host.fail = true; f.rejected(); f.release()
        assertEquals(1, f.host.calls); assertEquals("outcome_unknown", f.reports.last().second["status"].asString)
    }
    @Test fun signatureVersionHashSessionTargetAndCurrentVersionReject() {
        Fixture().apply { host.signature = false; rejected(); assertEquals(0, host.calls) }
        Fixture().apply { host.version = "3"; rejected(); assertEquals(0, host.calls) }
        for (field in listOf("artifact_digest", "host_session_id", "target", "expected_installed_manifest_digest", "manifest_digest")) {
            Fixture().apply { envelope.getAsJsonObject("request").addProperty(field, "wrong"); rejected(); assertEquals(0, host.calls) }
        }
    }
    @Test fun unknownArtifactCommandsReject() {
        val f = Fixture(); f.envelope.getAsJsonObject("request").addProperty("command", "install malicious")
        f.rejected(); assertEquals(0, f.host.calls)
    }
    @Test fun freshHostObservationIsSeparateAndCannotReplayInstall() {
        val f = Fixture(); f.release()
        f.host.identity = ReleaseIdentity(f.request.manifest_digest, "2")
        f.host.running = ReleaseRunning(f.request.manifest_digest, "2", "fresh-session")
        f.release()
        assertEquals(1, f.host.calls); assertEquals("observe", f.reports.last().first)
        assertEquals("fresh-session", f.reports.last().second.getAsJsonObject("running")["host_session_id"].asString)
    }
    @Test fun interruptedDurableIntentRefusesInstallation() {
        val f = Fixture(); ReleaseJournal(f.dir).claim(f.request); f.release(); assertEquals(0, f.host.calls)
    }
}

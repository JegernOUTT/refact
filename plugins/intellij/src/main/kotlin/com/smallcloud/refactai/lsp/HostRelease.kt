package com.smallcloud.refactai.lsp

import com.google.gson.GsonBuilder
import com.google.gson.JsonObject
import com.google.gson.JsonParser
import java.nio.file.Files
import java.nio.file.LinkOption
import java.nio.file.Path
import java.nio.file.StandardOpenOption
import java.nio.channels.FileChannel
import java.security.MessageDigest
import java.util.Base64
import java.util.TreeMap
import java.util.zip.ZipInputStream

internal val releaseJson = GsonBuilder().disableHtmlEscaping().serializeNulls().create()
internal fun releaseHash(bytes: ByteArray): String = MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }
internal fun manifestHash(files: Map<String, String>): String {
    // serde_json emits Unicode literally (including U+2028/U+2029); Gson does not.
    fun quote(value: String): String = buildString {
        append('"')
        value.forEach { c -> when (c) {
            '"' -> append("\\\""); '\\' -> append("\\\\"); '\b' -> append("\\b")
            '\u000c' -> append("\\f"); '\n' -> append("\\n"); '\r' -> append("\\r"); '\t' -> append("\\t")
            else -> if (c.code < 32) append("\\u%04x".format(c.code)) else append(c)
        } }
        append('"')
    }
    val canonical = TreeMap(files).entries.joinToString(",", "{", "}") { quote(it.key) + ":" + quote(it.value) }
    return releaseHash(canonical.toByteArray(Charsets.UTF_8))
}
internal data class ReleaseIdentity(val manifest_digest: String, val version: String)
internal data class ReleaseRunning(val manifest_digest: String, val version: String, val host_session_id: String)
internal data class ReleaseReceipt(val schema_version: Int = 1, val prepared_hash: String, val grant_id: String,
    val host_session_id: String, val artifact_digest: String, val status: String,
    val installed: ReleaseIdentity?, val running: ReleaseRunning?)
internal data class ReleaseRequest(val schema_version: Int, val prepared_hash: String, val grant_id: String,
    val project_id: String, val host_session_id: String, val target: String, val operation: String,
    val candidate_id: String, val candidate_digest: String, val proof_id: String, val artifact_digest: String,
    val manifest_digest: String, val package_version: String, val expected_installed_manifest_digest: String,
    val expected_running_manifest_digest: String, val files: Map<String, String>)
internal interface ReleaseTransport { fun post(project: String, endpoint: String, body: Any): JsonObject }
internal interface ReleaseHost {
    val running: ReleaseRunning
    fun installed(): ReleaseIdentity
    fun inspectAndVerifySignature(path: Path): String
    fun confirm(request: ReleaseRequest): Boolean
    fun scheduleAfterRestart(path: Path)
}

/** A full journal refuses new operations. Entries are NEVER evicted to make replay possible. */
internal class ReleaseJournal(private val root: Path) {
    init {
        Files.createDirectories(root)
        require(!Files.isSymbolicLink(root))
        if (Files.getFileStore(root).supportsFileAttributeView("posix")) {
            Files.setPosixFilePermissions(root, java.nio.file.attribute.PosixFilePermissions.fromString("rwx------"))
        }
    }
    fun path(hash: String): Path { require(Regex("[a-f0-9]{64}").matches(hash)); return root.resolve(hash) }
    fun existing(hash: String): JsonObject? = path(hash).takeIf { Files.exists(it) }?.let {
        require(!Files.isSymbolicLink(it) && Files.size(it) < 1024 * 1024)
        JsonParser.parseString(Files.readString(it)).asJsonObject
    }
    fun claim(request: ReleaseRequest): Path {
        require(Files.list(root).use { it.count() } < 128) { "Release journal full; manual reconciliation required" }
        val p = path(request.prepared_hash)
        FileChannel.open(p, StandardOpenOption.CREATE_NEW, StandardOpenOption.WRITE).use {
            it.write(java.nio.ByteBuffer.wrap(releaseJson.toJson(mapOf("request" to request)).toByteArray()))
            it.force(true)
        }
        syncDirectory()
        return root.resolve(request.prepared_hash + ".zip")
    }
    fun receipt(request: ReleaseRequest, receipt: ReleaseReceipt) {
        val p = path(request.prepared_hash)
        val tmp = root.resolve(request.prepared_hash + ".tmp")
        FileChannel.open(tmp, StandardOpenOption.CREATE_NEW, StandardOpenOption.WRITE).use {
            it.write(java.nio.ByteBuffer.wrap(releaseJson.toJson(mapOf("request" to request, "receipt" to receipt)).toByteArray()))
            it.force(true)
        }
        Files.move(tmp, p, java.nio.file.StandardCopyOption.ATOMIC_MOVE, java.nio.file.StandardCopyOption.REPLACE_EXISTING)
        syncDirectory()
    }
    private fun syncDirectory() {
        // Fail closed before effect on filesystems/platforms without durable directory sync.
        FileChannel.open(root, StandardOpenOption.READ).use { it.force(true) }
    }
}

internal fun zipManifest(bytes: ByteArray): Map<String, String> {
    val result = TreeMap<String, String>(); var root: String? = null; var total = 0L; var entries = 0
    ZipInputStream(bytes.inputStream()).use { zip ->
        while (true) {
            val entry = zip.nextEntry ?: break
            require(++entries <= 20000)
            val parts = entry.name.trimEnd('/').split('/')
            require(parts.size >= 1 && parts.all { it.isNotEmpty() && it != "." && it != ".." && !it.contains('\\') && it.none { c -> c.code < 32 } })
            if (root == null) root = parts[0]
            require(root == parts[0]) { "Multiple ZIP roots" }
            if (entry.isDirectory) continue
            require(parts.size > 1)
            val digest = MessageDigest.getInstance("SHA-256"); val buffer = ByteArray(65536)
            while (true) { val n = zip.read(buffer); if (n < 0) break; total += n; require(total <= 1024L * 1024 * 1024); digest.update(buffer, 0, n) }
            val name = parts.drop(1).joinToString("/")
            require(result.put(name, digest.digest().joinToString("") { "%02x".format(it) }) == null)
        }
    }
    require(result.isNotEmpty()); return result
}
internal fun directoryManifest(path: Path): Map<String, String> {
    require(Files.isDirectory(path, LinkOption.NOFOLLOW_LINKS))
    val result = TreeMap<String, String>(); var total = 0L
    Files.walk(path).use { stream -> stream.forEach { p ->
        require(!Files.isSymbolicLink(p))
        if (Files.isRegularFile(p, LinkOption.NOFOLLOW_LINKS)) {
            require(result.size < 20000); total += Files.size(p); require(total <= 1024L * 1024 * 1024)
            val digest = MessageDigest.getInstance("SHA-256")
            Files.newInputStream(p).use { input -> val buffer = ByteArray(65536); while (true) { val n = input.read(buffer); if (n < 0) break; digest.update(buffer, 0, n) } }
            result[path.relativize(p).toString().replace('\\', '/')] = digest.digest().joinToString("") { "%02x".format(it) }
        }
    } }
    require(result.isNotEmpty()); return result
}

internal class HostReleaseOwner(private val transport: ReleaseTransport, private val host: ReleaseHost, private val journal: ReleaseJournal) {
    @Synchronized fun release(project: String, hash: String, grant: String): String {
        require(project.matches(Regex("[A-Za-z0-9_-]{1,128}")) && grant.length in 1..256)
        val prior = journal.existing(hash)
        if (prior != null) {
            val request = releaseJson.fromJson(prior["request"], ReleaseRequest::class.java)
            require(request.project_id == project && request.grant_id == grant)
            if (prior.has("receipt")) transport.post(project, "receipt", prior["receipt"])
            // Reconnect observation is separate and NEVER calls the installer again.
            if (host.running.host_session_id != request.host_session_id) {
                transport.post(project, "observe", mapOf("prepared_hash" to hash, "grant_id" to grant,
                    "artifact_digest" to request.artifact_digest, "installed" to host.installed(), "running" to host.running))
            }
            return "Existing intent reconciled; installer not replayed"
        }
        val envelope = transport.post(project, "artifact", mapOf("prepared_hash" to hash, "grant_id" to grant))
        val obj = envelope.getAsJsonObject("request")
        val allowed = setOf("schema_version", "prepared_hash", "grant_id", "project_id", "host_session_id", "target", "operation", "candidate_id", "candidate_digest", "proof_id", "artifact_digest", "manifest_digest", "package_version", "expected_installed_manifest_digest", "expected_running_manifest_digest", "files")
        require(obj.keySet() == allowed) { "Unexpected release request fields" }
        val r = releaseJson.fromJson(obj, ReleaseRequest::class.java)
        require(r.schema_version == 1 && r.prepared_hash == hash && r.grant_id == grant && r.project_id == project)
        require(r.target == "jetbrains" && r.operation in setOf("install", "recover") && r.host_session_id == host.running.host_session_id)
        require(r.expected_installed_manifest_digest == host.installed().manifest_digest && r.expected_running_manifest_digest == host.running.manifest_digest)
        val encoded = envelope["artifact_base64"].asString; require(encoded.length <= 384 * 1024 * 1024)
        val bytes = Base64.getDecoder().decode(encoded)
        require(releaseHash(bytes) == r.artifact_digest && zipManifest(bytes) == r.files && manifestHash(r.files) == r.manifest_digest)
        val path = journal.claim(r) // durable no-replay barrier before any SDK effect
        FileChannel.open(path, StandardOpenOption.CREATE_NEW, StandardOpenOption.WRITE).use { channel ->
            val buffer = java.nio.ByteBuffer.wrap(bytes)
            while (buffer.hasRemaining()) channel.write(buffer)
            channel.force(true)
        }
        var status = "outcome_unknown"
        try {
            require(host.inspectAndVerifySignature(path) == r.package_version) { "Package version/signature mismatch" }
            if (!host.confirm(r)) status = "declined"
            else {
                require(host.installed().manifest_digest == r.expected_installed_manifest_digest)
                require(releaseHash(Files.readAllBytes(path)) == r.artifact_digest)
                host.scheduleAfterRestart(path)
                status = "installer_returned"
            }
        } finally {
            val receipt = ReleaseReceipt(prepared_hash = hash, grant_id = grant, host_session_id = r.host_session_id,
                artifact_digest = r.artifact_digest, status = status, installed = runCatching { host.installed() }.getOrNull(), running = host.running)
            journal.receipt(r, receipt)
            transport.post(project, "receipt", receipt)
        }
        return if (status == "installer_returned") "Installer returned; pending_restart. Running plugin unchanged." else status
    }
}

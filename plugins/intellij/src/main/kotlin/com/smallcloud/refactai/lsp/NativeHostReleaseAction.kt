package com.smallcloud.refactai.lsp

import com.google.gson.JsonObject
import com.google.gson.JsonParser
import com.intellij.credentialStore.CredentialAttributes
import com.intellij.ide.passwordSafe.PasswordSafe
import com.intellij.ide.plugins.loadDescriptorFromArtifact
import com.intellij.ide.plugins.PluginInstaller
import com.intellij.ide.plugins.IdeaPluginDescriptor
import com.intellij.openapi.actionSystem.AnAction
import com.intellij.openapi.actionSystem.AnActionEvent
import com.intellij.openapi.application.ApplicationInfo
import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.application.PathManager
import com.intellij.openapi.ui.Messages
import com.smallcloud.refactai.getThisPlugin
import java.net.HttpURLConnection
import java.net.URI
import java.nio.file.Files
import java.nio.file.Path
import java.util.Base64
import java.util.UUID

/** Captured once at project activation, never reconstructed from post-install files. */
internal object HostReleaseActivation {
    @Volatile private var captured: ReleaseRunning? = null
    @Synchronized fun capture() {
        if (captured != null) return
        val descriptor = requireNotNull(getThisPlugin())
        captured = ReleaseRunning(manifestHash(directoryManifest(requireNotNull(descriptor.pluginPath))), descriptor.version, UUID.randomUUID().toString())
    }
    fun running(): ReleaseRunning = requireNotNull(captured) { "Activation identity unavailable; restart IDE before releasing" }
}

private class ReleaseCredentialsDialog : com.intellij.openapi.ui.DialogWrapper(true) {
    val username = javax.swing.JTextField()
    val password = javax.swing.JPasswordField()
    val remember = javax.swing.JCheckBox("Save in IDE Password Safe", false)
    init { title = "Refact Release Issuing-User Authentication"; init() }
    override fun createCenterPanel(): javax.swing.JComponent = javax.swing.JPanel(java.awt.GridLayout(0, 1)).apply {
        add(javax.swing.JLabel("Basic username of the user who issued this grant")); add(username)
        add(javax.swing.JLabel("Password (never sent to the worker or webview)")); add(password); add(remember)
    }
}

/** Only the trusted daemon endpoint discovery is reused, never a worker bearer or artifact URL. */
internal class NativeReleaseTransport : ReleaseTransport {
    private var sessionCredential: com.intellij.credentialStore.Credentials? = null
    override fun post(project: String, endpoint: String, body: Any): JsonObject {
        require(endpoint in setOf("artifact", "receipt", "observe"))
        require(project.matches(Regex("[A-Za-z0-9_-]{1,128}")))
        val info = daemonJsonFile()
        require(info.isFile && info.length() < 65536 && !Files.isSymbolicLink(info.toPath()))
        val port = JsonParser.parseString(info.readText()).asJsonObject["port"].asInt
        require(port in 1..65535)
        // Provisioned by the operator in the platform credential store, not settings/webview/worker state.
        val attributes = CredentialAttributes("Refact Host Release Owner")
        val credential = sessionCredential ?: PasswordSafe.instance.get(attributes) ?: run {
            var entered: com.intellij.credentialStore.Credentials? = null
            ApplicationManager.getApplication().invokeAndWait {
                val dialog = ReleaseCredentialsDialog()
                if (dialog.showAndGet()) {
                    entered = com.intellij.credentialStore.Credentials(dialog.username.text, String(dialog.password.password))
                    dialog.password.text = ""
                    if (dialog.remember.isSelected) PasswordSafe.instance.set(attributes, entered)
                }
            }
            requireNotNull(entered) { "Issuing-user authentication cancelled" }
        }
        sessionCredential = credential
        val user = requireNotNull(credential.userName); val password = requireNotNull(credential.getPasswordAsString())
        require(user.isNotBlank() && ':' !in user && password.isNotEmpty())
        val connection = URI("http://127.0.0.1:$port/daemon/v1/projects/$project/host-release/$endpoint").toURL().openConnection() as HttpURLConnection
        try {
            connection.instanceFollowRedirects = false
            connection.connectTimeout = 2000; connection.readTimeout = 5000
            connection.requestMethod = "POST"; connection.doOutput = true
            connection.setRequestProperty("Content-Type", "application/json")
            connection.setRequestProperty("Authorization", "Basic " + Base64.getEncoder().encodeToString("$user:$password".toByteArray(Charsets.UTF_8)))
            connection.outputStream.use { it.write(releaseJson.toJson(body).toByteArray(Charsets.UTF_8)) }
            require(connection.responseCode in 200..299) { "Release daemon denied request (HTTP ${connection.responseCode})" }
            val limit = if (endpoint == "artifact") 384 * 1024 * 1024 else 1024 * 1024
            val bytes = connection.inputStream.use { it.readNBytes(limit + 1) }
            require(bytes.size <= limit)
            return JsonParser.parseString(bytes.toString(Charsets.UTF_8)).asJsonObject
        } finally { connection.disconnect() }
    }
}

internal class JetBrainsReleaseHost(private val confirmOnEdt: (ReleaseRequest) -> Boolean) : ReleaseHost {
    override val running: ReleaseRunning get() = HostReleaseActivation.running()
    private var verified: IdeaPluginDescriptor? = null
    override fun installed(): ReleaseIdentity {
        val plugin = requireNotNull(getThisPlugin())
        return ReleaseIdentity(manifestHash(directoryManifest(requireNotNull(plugin.pluginPath))), plugin.version)
    }
    override fun inspectAndVerifySignature(path: Path): String {
        val descriptor = requireNotNull(loadDescriptorFromArtifact(path, ApplicationInfo.getInstance().build))
        require(descriptor.pluginId == requireNotNull(getThisPlugin()).pluginId) { "Foreign plugin ID" }
        require(PluginInstaller.rootEntryName(path) == requireNotNull(requireNotNull(getThisPlugin()).pluginPath).fileName.toString()) {
            "ZIP root must match installed plugin directory; refuse deleting another plugin's directory"
        }
        // SDK 251 (PluginInstaller.java) and later (PluginInstaller.kt) expose the same verifier
        // under different exact owners. No arbitrary class/method, command, or URL is accepted.
        val verifiedSignature = try {
            val checker = Class.forName("com.intellij.ide.plugins.marketplace.PluginSignatureChecker")
            checker.getMethod("verifyIfRequired", IdeaPluginDescriptor::class.java, Path::class.java,
                Boolean::class.javaPrimitiveType, Boolean::class.javaPrimitiveType).invoke(null, descriptor, path, false, false) as Boolean
        } catch (_: ClassNotFoundException) {
            PluginInstaller::class.java.getMethod("verifyPluginSignatureIfRequired", IdeaPluginDescriptor::class.java,
                Path::class.java, Boolean::class.javaPrimitiveType).invoke(null, descriptor, path, false) as Boolean
        }
        require(verifiedSignature) { "SDK signature policy rejected package" }
        verified = descriptor
        return requireNotNull(descriptor.version) { "SDK package version is missing" }
    }
    override fun confirm(request: ReleaseRequest): Boolean = confirmOnEdt(request)
    override fun scheduleAfterRestart(path: Path) {
        // SDK 251 PluginInstaller.installAfterRestart queues native StartupActionScriptManager
        // copy/unzip/delete operations. require-restart=true prohibits truthful dynamic self-update.
        // false retains the verified ZIP for the native startup installer; never force restart/unload.
        PluginInstaller.installAfterRestart(requireNotNull(verified), path, requireNotNull(getThisPlugin()).pluginPath, false)
    }
}

/** Find Action entry; no webview event can call this owner with artifact-controlled fields. */
class NativeHostReleaseAction : AnAction() {
    override fun actionPerformed(event: AnActionEvent) {
        val project = event.project ?: return
        val running = runCatching { HostReleaseActivation.running() }.getOrElse {
            Messages.showErrorDialog(project, "Startup identity unavailable. Restart IDE before preparing a native release.", "Refact Native Host Release")
            return
        }
        val id = Messages.showInputDialog(project,
            "Daemon project ID\nPrepare the grant for host session: ${running.host_session_id}\nRunning manifest: ${running.manifest_digest}",
            "Refact Native Host Release", null) ?: return
        val hash = Messages.showInputDialog(project, "Prepared action SHA-256", "Refact Native Host Release", null) ?: return
        val grant = Messages.showInputDialog(project, "Exact issuing-user grant ID", "Refact Native Host Release", null) ?: return
        ApplicationManager.getApplication().executeOnPooledThread {
            val result = runCatching {
                val host = JetBrainsReleaseHost { r ->
                    var accepted = false
                    ApplicationManager.getApplication().invokeAndWait {
                        accepted = Messages.showYesNoDialog(project,
                            "${r.operation} Refact ${r.package_version}\nProject: ${r.project_id}\nPackage SHA-256: ${r.artifact_digest}\n" +
                                "Prepared: ${r.prepared_hash}\nGrant: ${r.grant_id}\n" +
                                "The IDE will schedule this verified local package for your next manual restart. Running code does not change now.",
                            "Confirm Native Plugin Release", Messages.getWarningIcon()) == Messages.YES
                    }
                    accepted
                }
                HostReleaseOwner(NativeReleaseTransport(), host,
                    ReleaseJournal(Path.of(PathManager.getSystemPath(), "refact-host-release"))).release(id, hash, grant)
            }
            ApplicationManager.getApplication().invokeLater {
                Messages.showInfoMessage(project, result.getOrElse { "Release not completed; no automatic retry. ${it.javaClass.simpleName}" }, "Refact Native Host Release")
            }
        }
    }
}

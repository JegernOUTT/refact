package com.smallcloud.refactai.lsp

import com.google.gson.Gson
import com.google.gson.JsonObject
import com.intellij.openapi.Disposable
import com.intellij.openapi.application.ApplicationInfo
import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.components.service
import com.intellij.openapi.diagnostic.Logger
import com.intellij.openapi.project.Project
import com.intellij.openapi.util.SystemInfo
import com.intellij.openapi.util.io.FileUtil.getTempDirectory
import com.intellij.openapi.util.io.FileUtil.setExecutable
import com.intellij.util.concurrency.AppExecutorUtil
import com.intellij.util.messages.MessageBus
import com.intellij.util.messages.Topic
import com.smallcloud.refactai.Resources
import com.smallcloud.refactai.Resources.binPrefix
import com.smallcloud.refactai.io.ConnectionStatus
import com.smallcloud.refactai.io.HttpStatusException
import com.smallcloud.refactai.io.InferenceGlobalContextChangedNotifier
import com.smallcloud.refactai.notifications.emitError
import java.io.File
import java.io.FileOutputStream
import java.io.InputStream
import java.net.URI
import java.nio.file.Paths
import java.security.MessageDigest
import java.util.UUID
import java.util.concurrent.RejectedExecutionException
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference
import kotlin.io.path.Path
import com.smallcloud.refactai.io.InferenceGlobalContext.Companion.instance as InferenceGlobalContext

private fun getExeSuffix(): String {
    if (SystemInfo.isWindows) return ".exe"
    return ""
}

interface LSPProcessHolderChangedNotifier {
    fun capabilitiesChanged(newCaps: LSPCapabilities) {}
    fun lspIsActive(isActive: Boolean) {}
    fun ragStatusChanged(ragStatus: RagStatus) {}

    companion object {
        val TOPIC = Topic.create(
            "Refact.ai LSP Process Notifier", LSPProcessHolderChangedNotifier::class.java
        )
    }
}

open class LSPProcessHolder(val project: Project) : Disposable {
    @Volatile
    private var isDisposed = false
    private var lastConfig: LSPConfig? = null
    private val messageBus: MessageBus = ApplicationManager.getApplication().messageBus
    private var isWorking_ = false
    private val healthCheckerScheduler = AppExecutorUtil.createBoundedScheduledExecutorService(
        "SMCLSPHealthCheckerScheduler", 1
    )
    var ragStatusCache: RagStatus? = null
    private val ragStatusCheckerScheduler = AppExecutorUtil.createBoundedScheduledExecutorService(
        "SMCLSPRagStatusCheckerScheduler", 1
    )
    private val lifecycleScheduler = AppExecutorUtil.createBoundedScheduledExecutorService(
        "SMCLSPLifecycleScheduler", 1
    )
    private val lifecycleWorkerRunning = AtomicBoolean(false)
    private val lifecycleStartRequested = AtomicBoolean(false)
    private val lifecycleRestartRequested = AtomicBoolean(false)
    private val lifecycleReason = AtomicReference("initial")
    private val processStartLock = Any()
    @Volatile
    private var customizationCache: JsonObject? = null
    @Volatile
    private var startupInProgress = false
    @Volatile
    private var attachedProject: DaemonProject? = null
    protected open val daemonClient: RefactDaemonClient = HttpRefactDaemonClient()

    private val exitThread: Thread = Thread {
        closeAttachedProject()
        terminate()
    }

    open var isWorking: Boolean
        get() = isWorking_
        set(newValue) {
            if (isWorking_ == newValue) return
            isWorking_ = newValue
            if (!project.isDisposed) {
                project.messageBus.syncPublisher(LSPProcessHolderChangedNotifier.TOPIC).lspIsActive(newValue)
            }
        }

    private fun logIfBlockingOperationOnEdt(operation: String) {
        if (ApplicationManager.getApplication().isDispatchThread) {
            logger.error("LSP blocking operation '$operation' called on EDT")
        }
    }

    private fun defaultMdnsHost(): String {
        val label = runCatching { java.net.InetAddress.getLocalHost().hostName }
            .getOrNull()
            ?.lowercase()
            ?.replace(Regex("[^a-z0-9-]"), "-")
            ?.trim('-')
            ?.takeIf { it.isNotEmpty() }
            ?: "refact"
        return "$label.local"
    }

    private fun defaultBrowserHost(): String {
        return defaultMdnsHost()
    }

    open fun browserUrlOrNull(): URI? {
        val base = baseUrlOrNull() ?: return null
        val configuredHost = InferenceGlobalContext.browserHost.trim()
        val host = if (configuredHost.isNotEmpty() && configuredHost != "0.0.0.0") {
            configuredHost
        } else {
            defaultBrowserHost()
        }
        return URI("http://$host:${base.port}${base.rawPath}")
    }

    private fun shouldAbortLifecycleWork(): Boolean {
        return isDisposed || project.isDisposed
    }

    private fun requestLifecycleWork(reason: String, restart: Boolean) {
        try {
            if (isDisposed || project.isDisposed) {
                logger.info("Skipping lifecycle work for disposed LSPProcessHolder or project")
                return
            }

            lifecycleStartRequested.set(true)
            if (restart) {
                lifecycleRestartRequested.set(true)
            }
            lifecycleReason.set(reason)
            scheduleLifecycleWorkerIfNeeded()
        } catch (e: RejectedExecutionException) {
            if (e.message?.contains("Already shutdown") == true) {
                logger.info("Ignoring RejectedExecutionException during lifecycle scheduling: ${e.message}")
            } else {
                throw e
            }
        }
    }

    private fun scheduleLifecycleWorkerIfNeeded() {
        if (!lifecycleWorkerRunning.compareAndSet(false, true)) return

        try {
            lifecycleScheduler.submit {
                runLifecycleWorker()
            }
        } catch (e: RejectedExecutionException) {
            lifecycleWorkerRunning.set(false)
            if (e.message?.contains("Already shutdown") == true) {
                logger.info("Ignoring RejectedExecutionException during lifecycle startup: ${e.message}")
            } else {
                throw e
            }
        }
    }

    private fun runLifecycleWorker() {
        try {
            while (!isDisposed && !project.isDisposed) {
                val shouldRestart = lifecycleRestartRequested.getAndSet(false)
                val shouldStart = lifecycleStartRequested.getAndSet(false)
                if (!shouldRestart && !shouldStart) {
                    break
                }

                val reason = lifecycleReason.getAndSet("coalesced")
                logger.debug("Lifecycle worker run: restart=$shouldRestart start=$shouldStart reason=$reason")
                if (shouldRestart) {
                    applySettingsChangeBlocking(reason)
                } else {
                    ensureStartedBlocking(reason)
                }
            }
        } catch (e: Exception) {
            logger.warn("Exception during lifecycle worker: ${e.message}")
        } finally {
            lifecycleWorkerRunning.set(false)
            if (!isDisposed && !project.isDisposed && (lifecycleStartRequested.get() || lifecycleRestartRequested.get())) {
                scheduleLifecycleWorkerIfNeeded()
            }
        }
    }

    private fun applySettingsChangeBlocking(reason: String) {
        if (shouldAbortLifecycleWork()) {
            logger.info("Skipping settings change for disposed LSPProcessHolder or project")
            return
        }

        initialize()
        logger.info("Applying LSP settings change: $reason")
        customizationCache = null

        synchronized(processStartLock) {
            startProcess()
        }
    }

    protected open fun ensureStartedBlocking(reason: String) {
        if (shouldAbortLifecycleWork()) {
            logger.info("Skipping ensure-started for disposed LSPProcessHolder or project")
            return
        }

        initialize()
        logger.debug("Ensuring LSP is attached through daemon: $reason")

        synchronized(processStartLock) {
            if (!isWorking || attachedProject == null || lastConfig == null) {
                startProcess()
            }
        }
    }

    open fun ensureStartedAsync(reason: String = "external-request") {
        requestLifecycleWork(reason, restart = false)
    }

    fun hasPendingLifecycleWork(): Boolean {
        return lifecycleStartRequested.get() || lifecycleRestartRequested.get() || lifecycleWorkerRunning.get()
    }

    fun ensureStartedIfNeeded(reason: String = "external-request") {
        val app = ApplicationManager.getApplication()
        if (app.isDispatchThread) {
            ensureStartedAsync(reason)
        } else {
            ensureStartedBlocking(reason)
        }
    }

    init {
        messageBus.connect(this)
            .subscribe(InferenceGlobalContextChangedNotifier.TOPIC, object : InferenceGlobalContextChangedNotifier {
                override fun userInferenceUriChanged(newUrl: String?) {
                    settingsChanged("inference-uri-changed")
                }

                override fun astFlagChanged(newValue: Boolean) {
                    settingsChanged("ast-flag-changed")
                }

                override fun astFileLimitChanged(newValue: Int) {
                    settingsChanged("ast-file-limit-changed")
                }

                override fun vecdbFlagChanged(newValue: Boolean) {
                    settingsChanged("vecdb-flag-changed")
                }

                override fun vecdbFileLimitChanged(newValue: Int) {
                    settingsChanged("vecdb-file-limit-changed")
                }

                override fun xDebugLSPPortChanged(newPort: Int?) {
                    settingsChanged("daemon-port-changed")
                }

                override fun insecureSSLChanged(newValue: Boolean) {
                    settingsChanged("insecure-ssl-changed")
                }

                override fun experimentalLspFlagEnabledChanged(newValue: Boolean) {
                    settingsChanged("experimental-flag-changed")
                }

                override fun httpHostChanged(newValue: String) {
                    settingsChanged("http-host-changed")
                }
            })

        Runtime.getRuntime().addShutdownHook(exitThread)

        healthCheckerScheduler.scheduleWithFixedDelay({
            try {
                if (isDisposed || project.isDisposed) {
                    logger.info("Skipping health check for disposed LSPProcessHolder or project")
                    return@scheduleWithFixedDelay
                }

                if (lastConfig == null || startupInProgress) return@scheduleWithFixedDelay
                if (attachedProject == null || !isWorking) {
                    ensureStartedAsync("health-check-daemon-detached-or-unready")
                }
            } catch (e: RejectedExecutionException) {
                if (e.message?.contains("Already shutdown") == true) {
                    logger.info("Ignoring RejectedExecutionException during health check: ${e.message}")
                } else {
                    logger.warn("Unexpected RejectedExecutionException during health check: ${e.message}")
                }
            } catch (e: Exception) {
                logger.warn("Exception during health check: ${e.message}")
            }
        }, 1, 1, TimeUnit.SECONDS)
        ragStatusCheckerScheduler.schedule({ lspRagStatusSync() }, 1000, TimeUnit.MILLISECONDS)
    }

    open fun settingsChanged(reason: String = "settings-changed") {
        requestLifecycleWork(reason, restart = true)
    }

    open var capabilities: LSPCapabilities = LSPCapabilities()
        set(newValue) {
            if (newValue == field) return
            field = newValue
            if (!project.isDisposed) {
                project.messageBus.syncPublisher(LSPProcessHolderChangedNotifier.TOPIC).capabilitiesChanged(field)
            }
        }

    private fun currentConfig(): LSPConfig {
        return LSPConfig(
            ast = InferenceGlobalContext.astIsEnabled,
            astFileLimit = InferenceGlobalContext.astFileLimit,
            vecdb = InferenceGlobalContext.vecdbIsEnabled,
            vecdbFileLimit = InferenceGlobalContext.vecdbFileLimit,
            insecureSSL = InferenceGlobalContext.insecureSSL,
            experimental = InferenceGlobalContext.experimentalLspFlagEnabled,
            httpHost = InferenceGlobalContext.httpHost.trim().ifEmpty { "0.0.0.0" },
        )
    }

    private fun projectRootPath(): String? {
        return project.basePath?.let { path -> runCatching { File(path).canonicalPath }.getOrElse { path } }
    }

    open fun startProcess() {
        logIfBlockingOperationOnEdt("startProcess")
        val startedAt = System.currentTimeMillis()
        if (shouldAbortLifecycleWork()) return
        val newConfig = currentConfig()

        if (newConfig.sameRuntimeSettings(lastConfig) && attachedProject != null && isWorking) return

        startupInProgress = true
        try {
            capabilities = LSPCapabilities()
            closeAttachedProject()
            terminate()
            if (!newConfig.isValid) return
            val bin = BIN_PATH
            if (bin == null) {
                logger.warn("LSP daemon attach BIN_PATH is null")
                return
            }
            val root = projectRootPath()
            if (root == null) {
                logger.warn("LSP daemon attach project root is null")
                return
            }
            logger.debug("LSP daemon attach $bin ${newConfig.toSafeLogString()}")
            daemonClient.ensureDaemon(bin)
            val openedProject = daemonClient.openProject(root, newConfig)
            attachedProject = openedProject
            lastConfig = newConfig
            isWorking = true
            refreshAttachedWorkerState()
            if (shouldAbortLifecycleWork()) {
                closeAttachedProject()
                terminate()
                return
            }
            initializeAttachedProject()
            logger.info("LSP daemon attach finished in ${System.currentTimeMillis() - startedAt}ms (working=$isWorking)")
        } catch (e: Exception) {
            logger.warn("LSP daemon attach failed: ${e.message}", e)
            isWorking = false
            attachedProject = null
            lastConfig = null
        } finally {
            startupInProgress = false
        }
    }

    protected open fun refreshAttachedWorkerState() {
        buildInfo = getBuildInfo()
        logger.warn("LSP binary build info $buildInfo")
        capabilities = getCaps()
        fetchCustomizationFromServer()?.also { customizationCache = it }
    }

    protected open fun initializeAttachedProject() {
        lspProjectInitialize(this, project)
    }

    open fun fetchCustomization(): JsonObject? {
        logIfBlockingOperationOnEdt("fetchCustomization")
        customizationCache?.let { return it }
        if (baseUrlOrNull() == null) {
            ensureStartedIfNeeded("fetch-customization")
        }
        val server = fetchCustomizationFromServer()
        customizationCache = server
        return server
    }

    fun getCachedCustomization(): JsonObject? {
        return customizationCache
    }

    private fun shouldWakeAndRetry(error: Throwable?): Boolean {
        if (error is HttpStatusException) {
            return error.statusCode == 502 || error.statusCode == 503
        }
        return error?.cause?.let { shouldWakeAndRetry(it) } ?: false
    }

    protected open fun sleepBeforeWakeRetry(attempt: Int) {
        Thread.sleep((attempt * 100L).coerceAtMost(300L))
    }

    fun wakeWorkerForRetry(reason: String): Boolean {
        val bin = BIN_PATH ?: return false
        val root = projectRootPath() ?: return false
        val config = lastConfig ?: currentConfig()
        return try {
            logger.debug("LSP daemon wake retry: $reason")
            daemonClient.ensureDaemon(bin)
            attachedProject = daemonClient.openProject(root, config)
            lastConfig = config
            isWorking = true
            true
        } catch (e: Exception) {
            logger.warn("LSP wake retry failed: ${e.message}")
            false
        }
    }

    private fun <T> withWakeRetry(reason: String, block: () -> T?): T? {
        repeat(3) { attempt ->
            try {
                return block()
            } catch (e: Exception) {
                logger.warn("LSP $reason error ${e.message}")
                if (attempt < 2 && shouldWakeAndRetry(e)) {
                    if (!wakeWorkerForRetry("$reason-${attempt + 1}")) return null
                    sleepBeforeWakeRetry(attempt + 1)
                } else {
                    return null
                }
            }
        }
        return null
    }

    private fun fetchCustomizationFromServer(): JsonObject? {
        return withWakeRetry("customization-http") {
            val baseUrl = baseUrlOrNull() ?: return@withWakeRetry null
            val config = InferenceGlobalContext.connection.get(baseUrl.resolve("v1/customization"), dataReceiveEnded = {
                InferenceGlobalContext.status = ConnectionStatus.CONNECTED
                InferenceGlobalContext.lastErrorMsg = null
            }, errorDataReceived = {}, failedDataReceiveEnded = {
                InferenceGlobalContext.status = ConnectionStatus.ERROR
                if (it != null) {
                    InferenceGlobalContext.lastErrorMsg = it.message
                }
            }).join().get()
            Gson().fromJson(config as String, JsonObject::class.java)
        }
    }

    private fun lspRagStatusSync() {
        try {
            if (ragStatusCheckerScheduler.isShutdown || ragStatusCheckerScheduler.isTerminated || project.isDisposed || isDisposed) {
                return
            }
            if (!isWorking) {
                ragStatusCheckerScheduler.schedule({ lspRagStatusSync() }, 5000, TimeUnit.MILLISECONDS)
                return
            }
            val ragStatus = getRagStatus()
            if (ragStatus == null) {
                ragStatusCheckerScheduler.schedule({ lspRagStatusSync() }, 5000, TimeUnit.MILLISECONDS)
                return
            }
            if (ragStatus != ragStatusCache) {
                ragStatusCache = ragStatus
                project.messageBus.syncPublisher(LSPProcessHolderChangedNotifier.TOPIC).ragStatusChanged(ragStatusCache!!)
            }

            if (ragStatus.ast != null && ragStatus.ast.astMaxFilesHit) {
                ragStatusCheckerScheduler.schedule({ lspRagStatusSync() }, 5000, TimeUnit.MILLISECONDS)
                return
            }
            if (ragStatus.vecdb != null && ragStatus.vecdb.vecdbMaxFilesHit) {
                ragStatusCheckerScheduler.schedule({ lspRagStatusSync() }, 5000, TimeUnit.MILLISECONDS)
                return
            }

            if ((ragStatus.ast != null && listOf("starting", "parsing", "indexing").contains(ragStatus.ast.state))
                || (ragStatus.vecdb != null && listOf("starting", "parsing").contains(ragStatus.vecdb.state))
            ) {
                ragStatusCheckerScheduler.schedule({ lspRagStatusSync() }, 700, TimeUnit.MILLISECONDS)
            } else {
                ragStatusCheckerScheduler.schedule({ lspRagStatusSync() }, 5000, TimeUnit.MILLISECONDS)
            }
        } catch (_: Exception) {
            try {
                if (!ragStatusCheckerScheduler.isShutdown && !ragStatusCheckerScheduler.isTerminated) {
                    ragStatusCheckerScheduler.schedule({ lspRagStatusSync() }, 5000, TimeUnit.MILLISECONDS)
                }
            } catch (_: Exception) {
            }
        }
    }

    private fun closeAttachedProject() {
        val projectToClose = attachedProject ?: return
        try {
            daemonClient.closeProject(projectToClose)
        } catch (e: Exception) {
            logger.warn("LSP daemon project close failed: ${e.message}")
        }
    }

    private fun terminate() {
        if (!isDisposed) {
            logIfBlockingOperationOnEdt("terminate")
        }
        isWorking = false
        attachedProject = null
        lastConfig = null
    }

    override fun dispose() {
        isDisposed = true

        try {
            ragStatusCheckerScheduler.shutdown()
            closeAttachedProject()
            terminate()
            healthCheckerScheduler.shutdown()
            lifecycleScheduler.shutdown()
            Runtime.getRuntime().removeShutdownHook(exitThread)
        } catch (e: Exception) {
            logger.warn("Exception during LSPProcessHolder disposal: ${e.message}")
        }
    }

    private fun getBuildInfo(): String {
        logIfBlockingOperationOnEdt("getBuildInfo")
        return withWakeRetry("build-info") {
            InferenceGlobalContext.connection.get(url.resolve("v1/build_info"), dataReceiveEnded = {
                InferenceGlobalContext.status = ConnectionStatus.CONNECTED
                InferenceGlobalContext.lastErrorMsg = null
            }, errorDataReceived = {}, failedDataReceiveEnded = {
                InferenceGlobalContext.status = ConnectionStatus.ERROR
                if (it != null) {
                    InferenceGlobalContext.lastErrorMsg = it.message
                }
            }).get().get() as String
        } ?: ""
    }

    open val url: URI
        get() {
            val base = baseUrlOrNull() ?: return URI("")
            return base
        }

    open fun baseUrlOrNull(): URI? {
        if (!isWorking) return null
        return attachedProject?.baseUrl
    }

    open fun getCaps(): LSPCapabilities {
        logIfBlockingOperationOnEdt("getCaps")
        return withWakeRetry("caps") {
            val out = InferenceGlobalContext.connection.get(url.resolve("v1/caps"), dataReceiveEnded = {
                InferenceGlobalContext.status = ConnectionStatus.CONNECTED
                InferenceGlobalContext.lastErrorMsg = null
            }, errorDataReceived = {}, failedDataReceiveEnded = {
                if (it != null) {
                    InferenceGlobalContext.lastErrorMsg = it.message
                }
            }).get().get() as String
            Gson().fromJson(out, LSPCapabilities::class.java)
        } ?: LSPCapabilities()
    }

    fun getRagStatus(): RagStatus? {
        logIfBlockingOperationOnEdt("getRagStatus")
        return withWakeRetry("rag-status") {
            val out = InferenceGlobalContext.connection.get(url.resolve("v1/rag-status"),
                requestProperties = mapOf("redirect" to "follow", "cache" to "no-cache", "referrer" to "no-referrer"),
                dataReceiveEnded = {
                    InferenceGlobalContext.status = ConnectionStatus.CONNECTED
                    InferenceGlobalContext.lastErrorMsg = null
                },
                errorDataReceived = {},
                failedDataReceiveEnded = {
                    InferenceGlobalContext.status = ConnectionStatus.ERROR
                    if (it != null) {
                        InferenceGlobalContext.lastErrorMsg = it.message
                    }
                }).get().get() as String
            Gson().fromJson(out, RagStatus::class.java)
        }
    }

    fun attempingToReach(): String {
        val port = InferenceGlobalContext.xDebugLSPPort ?: DEFAULT_REFACT_DAEMON_PORT
        return "Refact daemon on port $port"
    }

    companion object {
        @Volatile
        var BIN_PATH: String? = null
        private var TMP_BIN_PATH: String? = null

        @JvmStatic
        fun getInstance(project: Project): LSPProcessHolder = project.service()

        var buildInfo: String = ""
        private val initialized = AtomicBoolean(false)
        private val logger = Logger.getInstance("LSPProcessHolder")

        private fun generateMD5HexAndWriteInTmpFile(input: InputStream, tmpFileName: File): String {
            val digest = MessageDigest.getInstance("MD5")
            val buffer = ByteArray(1024)
            var bytesRead: Int
            val fileOut = FileOutputStream(tmpFileName)
            while (input.read(buffer).also { bytesRead = it } != -1) {
                digest.update(buffer, 0, bytesRead)
                fileOut.write(buffer, 0, bytesRead)
            }
            fileOut.flush()
            fileOut.close()
            input.close()
            return digest.digest().joinToString("") { String.format("%02x", it) }
        }

        @Synchronized
        fun initialize() {
            logger.warn("LSP initialize start")
            if (initialized.get()) return

            val input: InputStream? = Companion::class.java.getResourceAsStream(
                "/bin/${binPrefix}/refact${getExeSuffix()}"
            )
            if (input == null) {
                emitError("Refact binary is not found for host operating system, please contact support")
                logger.warn("LSP initialize finished")
                return
            }
            input.use { stream ->
                val tmpFile = Path(getTempDirectory(), "${UUID.randomUUID()}${getExeSuffix()}").toFile()
                val hash = try {
                    generateMD5HexAndWriteInTmpFile(stream, tmpFile)
                } catch (e: Exception) {
                    logger.warn("LSP initialize: failed to write temp binary: ${e.message}")
                    tmpFile.delete()
                    return
                }

                val targetName = ApplicationInfo.getInstance().build.toString()
                    .replace(Regex("[^A-Za-z0-9 ]"), "_") + "_refact_${hash}${getExeSuffix()}"
                val targetPath = Paths.get(getTempDirectory(), targetName)
                val targetFile = targetPath.toFile()

                var resolvedPath: String? = null

                for (attempt in 1..5) {
                    try {
                        targetPath.parent.toFile().mkdirs()
                        if (targetFile.exists()) {
                            if (targetFile.canExecute()) {
                                resolvedPath = targetFile.canonicalPath
                                break
                            }
                            setExecutable(targetFile)
                            if (targetFile.canExecute()) {
                                resolvedPath = targetFile.canonicalPath
                                break
                            }
                        }
                        java.nio.file.Files.move(
                            tmpFile.toPath(), targetPath,
                            java.nio.file.StandardCopyOption.REPLACE_EXISTING
                        )
                        setExecutable(targetFile)
                        if (targetFile.exists() && targetFile.canExecute()) {
                            resolvedPath = targetFile.canonicalPath
                            break
                        }
                        logger.warn("LSP initialize: move succeeded but binary not ready (attempt $attempt)")
                    } catch (e: Exception) {
                        logger.warn("LSP initialize: attempt $attempt failed to install binary: ${e.message}")
                    }
                }

                if (resolvedPath == null) {
                    setExecutable(tmpFile)
                    if (tmpFile.exists() && tmpFile.canExecute()) {
                        logger.warn("LSP initialize: using temp path as fallback")
                        resolvedPath = tmpFile.canonicalPath
                        TMP_BIN_PATH = resolvedPath
                    } else {
                        logger.warn("LSP initialize: binary could not be installed or made executable — giving up")
                        tmpFile.delete()
                        return
                    }
                } else {
                    if (tmpFile.exists()) tmpFile.deleteOnExit()
                }

                BIN_PATH = resolvedPath
                initialized.set(true)
            }
            logger.warn("LSP initialize finished")
            logger.warn("LSP initialize BIN_PATH=$BIN_PATH")
        }

        fun cleanup() {
        }

    }
}

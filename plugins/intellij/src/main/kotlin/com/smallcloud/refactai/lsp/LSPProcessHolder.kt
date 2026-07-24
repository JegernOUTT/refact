package com.smallcloud.refactai.lsp

import com.google.gson.Gson
import com.google.gson.JsonObject
import com.intellij.openapi.Disposable
import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.application.PathManager
import com.intellij.openapi.components.service
import com.intellij.openapi.diagnostic.Logger
import com.intellij.openapi.project.Project
import com.intellij.util.concurrency.AppExecutorUtil
import com.intellij.util.messages.MessageBus
import com.intellij.util.messages.Topic
import com.smallcloud.refactai.Resources
import com.smallcloud.refactai.getThisPlugin
import com.smallcloud.refactai.io.ConnectionStatus
import com.smallcloud.refactai.io.InferenceGlobalContextChangedNotifier
import com.smallcloud.refactai.notifications.emitError
import com.smallcloud.refactai.notifications.emitWarning
import java.io.File
import java.net.URI
import java.net.URLEncoder
import java.nio.charset.StandardCharsets
import java.nio.file.Path
import java.util.concurrent.RejectedExecutionException
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference
import com.smallcloud.refactai.io.InferenceGlobalContext.Companion.instance as InferenceGlobalContext

interface LSPProcessHolderChangedNotifier {
    fun capabilitiesChanged(newCaps: LSPCapabilities) {}
    fun lspIsActive(isActive: Boolean) {}
    fun backendConnectionStatusChanged(newStatus: LSPBackendConnectionStatus) {}
    fun ragStatusChanged(ragStatus: RagStatus) {}

    companion object {
        val TOPIC = Topic.create(
            "Refact.ai LSP Process Notifier", LSPProcessHolderChangedNotifier::class.java
        )
    }
}

enum class LSPBackendConnectionStatus(val wireName: String) {
    CONNECTING("connecting"),
    STARTING("starting"),
    INSTALLING("installing"),
    READY("ready"),
    FAILED("failed")
}

private data class BinaryResolutionFailure(
    val message: String,
    val failedAtMs: Long,
    val retryAfterMs: Long,
)

private const val WORKER_HEALTH_FAILURE_THRESHOLD = 3

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
    private var nextHealthCheckAtMs = 0L
    @Volatile
    private var healthBackoffMs = 1_000L
    @Volatile
    private var binaryResolutionFailure: BinaryResolutionFailure? = null
    @Volatile
    private var consecutiveWorkerHealthFailures = 0
    @Volatile
    private var backendConnectionStatus: LSPBackendConnectionStatus = LSPBackendConnectionStatus.CONNECTING
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

    open fun backendConnectionStatus(): LSPBackendConnectionStatus {
        return backendConnectionStatus
    }

    fun backendReady(): Boolean {
        return backendConnectionStatus() == LSPBackendConnectionStatus.READY
    }

    private fun setBackendConnectionStatus(newStatus: LSPBackendConnectionStatus) {
        if (backendConnectionStatus == newStatus) return
        backendConnectionStatus = newStatus
        if (!project.isDisposed) {
            project.messageBus.syncPublisher(LSPProcessHolderChangedNotifier.TOPIC).backendConnectionStatusChanged(newStatus)
        }
    }

    private fun logIfBlockingOperationOnEdt(operation: String) {
        if (ApplicationManager.getApplication().isDispatchThread) {
            logger.error("LSP blocking operation '$operation' called on EDT")
        }
    }

    open fun browserUrlOrNull(): URI? {
        val base = baseUrlOrNull() ?: return null
        val configuredHost = InferenceGlobalContext.browserHost.trim()
        val host = if (configuredHost.isNotEmpty() && configuredHost != "0.0.0.0") {
            configuredHost
        } else {
            base.host ?: "127.0.0.1"
        }
        return browserUrl(base, host, attachedProject?.daemon?.authToken)
    }

    open fun embeddedBrowserUrlOrNull(): URI? {
        val base = baseUrlOrNull() ?: return null
        return browserUrl(base, base.host ?: "127.0.0.1", attachedProject?.daemon?.authToken)
    }

    private fun browserUrl(base: URI, host: String, authToken: String?): URI {
        val scheme = base.scheme?.takeIf { it.isNotBlank() } ?: "http"
        val path = base.rawPath?.takeIf { it.isNotEmpty() } ?: "/"
        val token = authToken?.trim()?.takeIf { it.isNotEmpty() }
        val query = token?.let {
            "?daemon_token=${URLEncoder.encode(it, StandardCharsets.UTF_8).replace("+", "%20")}"
        }.orEmpty()
        return URI("$scheme://$host:${base.port}$path$query")
    }

    private fun shouldAbortLifecycleWork(): Boolean {
        return isDisposed || project.isDisposed
    }

    private fun retrySuppressedByBinaryResolutionFailure(nowMs: Long = healthNowMs()): Boolean {
        val failure = binaryResolutionFailure ?: return false
        if (nowMs >= failure.retryAfterMs) return false
        logger.debug("Suppressing binary resolution retry until ${failure.retryAfterMs}; failedAt=${failure.failedAtMs}: ${failure.message}")
        setBackendConnectionStatus(LSPBackendConnectionStatus.FAILED)
        nextHealthCheckAtMs = maxOf(nextHealthCheckAtMs, failure.retryAfterMs)
        return true
    }

    private fun recordBinaryResolutionFailure(message: String) {
        val nowMs = healthNowMs()
        val retryAfterMs = nowMs + healthBackoffMs
        binaryResolutionFailure = BinaryResolutionFailure(message, nowMs, retryAfterMs)
        nextHealthCheckAtMs = retryAfterMs
        healthBackoffMs = (healthBackoffMs * 2).coerceAtMost(30_000L)
        setBackendConnectionStatus(LSPBackendConnectionStatus.FAILED)
    }

    private fun clearBinaryResolutionFailure() {
        binaryResolutionFailure = null
    }

    private fun resetWorkerHealthFailures() {
        consecutiveWorkerHealthFailures = 0
    }

    private fun workerHealthFailureThresholdReached(): Boolean {
        consecutiveWorkerHealthFailures += 1
        return consecutiveWorkerHealthFailures >= WORKER_HEALTH_FAILURE_THRESHOLD
    }

    private fun requestLifecycleWork(reason: String, restart: Boolean) {
        try {
            if (isDisposed || project.isDisposed) {
                logger.info("Skipping lifecycle work for disposed LSPProcessHolder or project")
                return
            }

            if (restart) {
                clearBinaryResolutionFailure()
                resetWorkerHealthFailures()
                setBackendConnectionStatus(LSPBackendConnectionStatus.CONNECTING)
            } else if (retrySuppressedByBinaryResolutionFailure()) {
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
        if (retrySuppressedByBinaryResolutionFailure()) return

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

                override fun refactBinaryPathChanged(newPath: String?) {
                    resetBinaryResolution()
                    settingsChanged("refact-binary-path-changed")
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

                override fun codegraphFlagChanged(newValue: Boolean) {
                    settingsChanged("codegraph-flag-changed")
                }

                override fun codegraphFileLimitChanged(newValue: Int) {
                    settingsChanged("codegraph-file-limit-changed")
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
                runHealthCheckOnce()
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

    protected open fun runHealthCheckOnce() {
        if (isDisposed || project.isDisposed) {
            logger.info("Skipping health check for disposed LSPProcessHolder or project")
            return
        }

        if (lastConfig == null || startupInProgress) return
        if (healthNowMs() < nextHealthCheckAtMs) return
        if (attachedProject == null || !isWorking) {
            val retryingBinaryResolutionFailure = binaryResolutionFailure != null
            ensureStartedAsync("health-check-daemon-detached-or-unready")
            if (!retryingBinaryResolutionFailure) {
                deferHealthRetry()
            }
            return
        }
        if (!probeAttachedWorker()) {
            if (!workerHealthFailureThresholdReached()) {
                logger.warn("LSP health probe failed; waiting for consecutive failure threshold")
                return
            }
            logger.warn("LSP health probe failed repeatedly; restarting attached worker")
            clearAttachedProjectState(preserveConfig = true, detach = true)
            setBackendConnectionStatus(LSPBackendConnectionStatus.FAILED)
            ensureStartedAsync("health-check-worker-unreachable")
            deferHealthRetry()
            return
        }
        resetWorkerHealthFailures()
        resetHealthBackoff()
    }

    open fun settingsChanged(reason: String = "settings-changed") {
        resetBinaryResolutionFailureForSettingsChange()
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
            codegraph = InferenceGlobalContext.codegraphIsEnabled,
            codegraphFileLimit = InferenceGlobalContext.codegraphFileLimit,
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
        if (retrySuppressedByBinaryResolutionFailure()) return

        if (newConfig.sameRuntimeSettings(lastConfig) && attachedProject != null && isWorking) {
            setBackendConnectionStatus(LSPBackendConnectionStatus.READY)
            return
        }

        startupInProgress = true
        try {
            capabilities = LSPCapabilities()
            closeAttachedProject()
            if (!newConfig.isValid) {
                terminate(LSPBackendConnectionStatus.FAILED)
                setBackendConnectionStatus(LSPBackendConnectionStatus.FAILED)
                return
            }
            lastConfig = newConfig
            terminate(LSPBackendConnectionStatus.STARTING, preserveConfig = true)
            val root = projectRootPath()
            if (root == null) {
                logger.warn("LSP daemon attach project root is null")
                setBackendConnectionStatus(LSPBackendConnectionStatus.FAILED)
                return
            }
            val compatibleDaemonStatus = compatibleDaemonStatusOrNull()
            val daemonStatus = if (compatibleDaemonStatus == null) {
                val bin = resolveBinaryPathForDaemon {
                    setBackendConnectionStatus(LSPBackendConnectionStatus.INSTALLING)
                } ?: run {
                    recordBinaryResolutionFailure(binaryResolutionFailureMessage())
                    return
                }
                clearBinaryResolutionFailure()
                logger.debug("LSP daemon spawn/upgrade $bin ${newConfig.toSafeLogString()}")
                daemonClient.ensureDaemon(bin, requiredDaemonVersion())
            } else {
                clearBinaryResolutionFailure()
                logger.debug("LSP daemon attach existing pid=${compatibleDaemonStatus.pid} version=${compatibleDaemonStatus.version} ${newConfig.toSafeLogString()}")
                compatibleDaemonStatus
            }
            val openedProject = daemonClient.openProject(root, newConfig, daemonStatus)
            attachedProject = openedProject
            isWorking = true
            refreshAttachedWorkerState()
            if (shouldAbortLifecycleWork()) {
                closeAttachedProject()
                terminate()
                return
            }
            initializeAttachedProject()
            setBackendConnectionStatus(LSPBackendConnectionStatus.READY)
            clearBinaryResolutionFailure()
            resetWorkerHealthFailures()
            resetHealthBackoff()
            logger.info("LSP daemon attach finished in ${System.currentTimeMillis() - startedAt}ms (working=$isWorking)")
        } catch (e: Exception) {
            logger.warn("LSP daemon attach failed: ${e.message}", e)
            clearAttachedProjectState(preserveConfig = true, detach = true)
            setBackendConnectionStatus(LSPBackendConnectionStatus.FAILED)
            deferHealthRetry()
        } finally {
            startupInProgress = false
        }
    }

    private fun compatibleDaemonStatusOrNull(): DaemonStatus? {
        val status = runCatching { daemonClient.status() }
            .onFailure { logger.debug("LSP daemon status probe failed: ${it.message}") }
            .getOrNull()
            ?: return null
        val requiredVersion = requiredDaemonVersion()
        if (versionIsOlder(status.version, requiredVersion)) {
            logger.info("LSP daemon version ${status.version} is older than plugin $requiredVersion")
            return null
        }
        val expectedExecutableSha256 = if (shouldVerifyDaemonExecutableHash(status, requiredVersion)) {
            localBinaryExecutableSha256OrNull(requiredVersion)
        } else {
            null
        }
        if (!daemonStatusMatchesExpected(status, requiredVersion, expectedExecutableSha256)) {
            logger.info(
                "LSP daemon version ${status.version} runs executable ${status.executableSha256} " +
                    "but the local binary is $expectedExecutableSha256; forcing daemon upgrade",
            )
            return null
        }
        return status
    }

    protected open fun localBinaryExecutableSha256OrNull(requiredVersion: String): String? {
        val localBinary = runCatching {
            RefactBinaryResolver.resolveLocalOrNull(
                RefactBinaryResolverOptions(
                    explicitPath = InferenceGlobalContext.refactBinaryPath,
                    bundledDir = getThisPlugin()?.pluginPath,
                    minVersion = requiredVersion,
                    pinnedVersion = requiredVersion,
                    cacheDir = BIN_CACHE_DIR,
                ),
            )
        }.getOrNull() ?: return null
        return RefactBinaryHashCache.sha256OrNull(java.nio.file.Path.of(localBinary))
    }

    protected open fun requiredDaemonVersion(): String {
        return RESOLVED_ENGINE_VERSION ?: Resources.version
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
        return isRecoverableHttpStatus(error)
    }

    protected open fun sleepBeforeWakeRetry(attempt: Int) {
        Thread.sleep((attempt * 100L).coerceAtMost(300L))
    }

    fun wakeWorkerForRetry(reason: String): Boolean {
        val root = projectRootPath() ?: run {
            clearAttachedProjectState(preserveConfig = true, detach = false)
            setBackendConnectionStatus(LSPBackendConnectionStatus.FAILED)
            return false
        }
        val config = lastConfig ?: currentConfig()
        if (!config.isValid) {
            clearAttachedProjectState(preserveConfig = false, detach = false)
            setBackendConnectionStatus(LSPBackendConnectionStatus.FAILED)
            return false
        }
        if (retrySuppressedByBinaryResolutionFailure()) return false
        return try {
            logger.debug("LSP daemon wake retry: $reason")
            setBackendConnectionStatus(LSPBackendConnectionStatus.STARTING)
            val compatibleDaemonStatus = compatibleDaemonStatusOrNull()
            val daemonStatus = if (compatibleDaemonStatus == null) {
                val bin = resolveBinaryPathForDaemon {
                    setBackendConnectionStatus(LSPBackendConnectionStatus.INSTALLING)
                } ?: run {
                    clearAttachedProjectState(preserveConfig = true, detach = false)
                    recordBinaryResolutionFailure(binaryResolutionFailureMessage())
                    return false
                }
                clearBinaryResolutionFailure()
                daemonClient.ensureDaemon(bin, requiredDaemonVersion())
            } else {
                compatibleDaemonStatus
            }
            attachedProject = daemonClient.openProject(root, config, daemonStatus)
            lastConfig = config
            isWorking = true
            refreshAttachedWorkerState()
            initializeAttachedProject()
            setBackendConnectionStatus(LSPBackendConnectionStatus.READY)
            clearBinaryResolutionFailure()
            resetWorkerHealthFailures()
            resetHealthBackoff()
            true
        } catch (e: Exception) {
            logger.warn("LSP wake retry failed: ${e.message}")
            clearAttachedProjectState(preserveConfig = true, detach = true)
            setBackendConnectionStatus(LSPBackendConnectionStatus.FAILED)
            deferHealthRetry()
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
                || (ragStatus.codegraph != null && ragStatus.codegraph.state == "indexing")
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
            daemonClient.detachProject(projectToClose)
        } catch (e: Exception) {
            logger.warn("LSP daemon project close failed: ${e.message}")
        }
    }

    private fun clearAttachedProjectState(preserveConfig: Boolean, detach: Boolean) {
        if (detach) {
            closeAttachedProject()
        }
        isWorking = false
        attachedProject = null
        if (!preserveConfig) {
            lastConfig = null
        }
    }

    protected open fun probeAttachedWorker(): Boolean {
        val base = baseUrlOrNull() ?: return false
        return runCatching {
            InferenceGlobalContext.connection.get(base.resolve("v1/build_info")).join().get()
        }.isSuccess
    }

    protected open fun healthNowMs(): Long {
        return System.currentTimeMillis()
    }

    protected open fun resolveBinaryPathForDaemon(onDownloadStart: () -> Unit): String? {
        return binaryPathForDaemon(getThisPlugin()?.pluginPath, onDownloadStart)
    }

    protected open fun binaryResolutionFailureMessage(): String {
        return lastBinaryResolutionErrorMessage()
    }

    private fun resetHealthBackoff() {
        healthBackoffMs = 1_000L
        nextHealthCheckAtMs = 0L
    }

    private fun deferHealthRetry() {
        nextHealthCheckAtMs = healthNowMs() + healthBackoffMs
        healthBackoffMs = (healthBackoffMs * 2).coerceAtMost(30_000L)
    }

    protected fun resetBinaryResolutionFailureForSettingsChange() {
        clearBinaryResolutionFailure()
        resetWorkerHealthFailures()
        nextHealthCheckAtMs = 0L
        healthBackoffMs = 1_000L
    }

    private fun terminate(
        newStatus: LSPBackendConnectionStatus = LSPBackendConnectionStatus.CONNECTING,
        preserveConfig: Boolean = false,
    ) {
        if (!isDisposed) {
            logIfBlockingOperationOnEdt("terminate")
        }
        setBackendConnectionStatus(newStatus)
        clearAttachedProjectState(preserveConfig = preserveConfig, detach = false)
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
        @Volatile
        private var LAST_BINARY_RESOLUTION_ERROR: String? = null
        @Volatile
        var RESOLVED_ENGINE_VERSION: String? = null
            private set
        private val WARNED_ENGINE_FALLBACKS: MutableSet<String> = java.util.concurrent.ConcurrentHashMap.newKeySet()
        private var BIN_CACHE_DIR: Path = Path.of(PathManager.getSystemPath(), "refactai", "bin")

        @JvmStatic
        fun getInstance(project: Project): LSPProcessHolder = project.service()

        var buildInfo: String = ""
        private val initialized = AtomicBoolean(false)
        private val logger = Logger.getInstance("LSPProcessHolder")

        fun setBinaryCacheDirForTest(path: Path) {
            BIN_CACHE_DIR = path
            initialized.set(false)
            BIN_PATH = null
            RESOLVED_ENGINE_VERSION = null
            LAST_BINARY_RESOLUTION_ERROR = null
        }

        fun resetBinaryResolution() {
            initialized.set(false)
            BIN_PATH = null
            RESOLVED_ENGINE_VERSION = null
            LAST_BINARY_RESOLUTION_ERROR = null
        }

        fun lastBinaryResolutionErrorMessage(): String {
            return LAST_BINARY_RESOLUTION_ERROR
                ?: "Failed to download the Refact engine (version ${Resources.version}) from GitHub releases. " +
                    "Check network/proxy settings or set refactai.binaryPath to a compatible refact executable."
        }

        @Synchronized
        fun binaryPathForDaemon(bundledDir: Path? = null, onDownloadStart: () -> Unit = {}): String? {
            if (ApplicationManager.getApplication().isUnitTestMode && BIN_PATH != null) {
                return BIN_PATH
            }
            BIN_PATH?.let { return it }
            val resolved = try {
                RefactBinaryResolver.resolveDetailed(
                    RefactBinaryResolverOptions(
                        explicitPath = InferenceGlobalContext.refactBinaryPath,
                        bundledDir = bundledDir,
                        minVersion = Resources.version,
                        pinnedVersion = Resources.version,
                        cacheDir = BIN_CACHE_DIR,
                        onDownloadStart = onDownloadStart,
                        onFallbackVersion = { pinned, chosen -> warnAboutEngineVersionFallback(pinned, chosen) },
                    )
                )
            } catch (e: Exception) {
                val message = "Failed to download the Refact engine (version ${Resources.version}) from GitHub releases, " +
                    "and no published engine release could be used instead. " +
                    "Check network/proxy settings or set refactai.binaryPath to a compatible refact executable. ${e.message}"
                LAST_BINARY_RESOLUTION_ERROR = message
                emitError(message)
                logger.warn("LSP binary resolution failed: ${e.message}", e)
                return null
            }
            BIN_PATH = resolved.path
            RESOLVED_ENGINE_VERSION = resolved.version
            LAST_BINARY_RESOLUTION_ERROR = null
            logger.warn(
                "LSP initialize BIN_PATH=$BIN_PATH engineVersion=${resolved.version}" +
                    (resolved.fallbackFromVersion?.let { " fallbackFrom=v$it" } ?: "")
            )
            return resolved.path
        }

        private fun warnAboutEngineVersionFallback(pinned: String, chosen: String) {
            if (!WARNED_ENGINE_FALLBACKS.add("$pinned->$chosen")) return
            val message = "Refact engine v$pinned is not published on GitHub releases; " +
                "using the latest published engine v$chosen instead."
            logger.warn(message)
            runCatching { emitWarning(message) }
        }

        @Synchronized
        fun initialize() {
            if (!initialized.compareAndSet(false, true)) return
            logger.info("LSP initialize")
        }

        fun cleanup() {
        }

    }
}

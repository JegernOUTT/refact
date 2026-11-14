@file:OptIn(okhttp3.ExperimentalOkHttpApi::class)

package com.smallcloud.refactai.lsp

import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.project.Project
import com.intellij.serviceContainer.AlreadyDisposedException
import com.intellij.testFramework.fixtures.BasePlatformTestCase
import com.intellij.util.concurrency.AppExecutorUtil
import com.intellij.util.messages.MessageBus
import com.smallcloud.refactai.io.AsyncConnection
import com.smallcloud.refactai.io.HttpStatusException
import com.smallcloud.refactai.panes.sharedchat.Events
import com.smallcloud.refactai.panes.sharedchat.configConnectionIssue
import com.smallcloud.refactai.settings.AppSettingsState
import com.smallcloud.refactai.io.InferenceGlobalContext.Companion.instance as InferenceGlobalContext
import mockwebserver3.MockResponse
import mockwebserver3.MockWebServer
import okhttp3.Protocol
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import org.mockito.Mockito.`when`
import org.mockito.Mockito.mock
import java.net.URI
import java.util.Collections
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicLong

class LSPProcessHolderTest : BasePlatformTestCase() {

    class FakeDaemonClient : RefactDaemonClient {
        val statusCalls = AtomicInteger(0)
        val ensureDaemonCalls = AtomicInteger(0)
        val openProjectCalls = AtomicInteger(0)
        val detachProjectCalls = AtomicInteger(0)
        val ensureDaemonPaths = Collections.synchronizedList(mutableListOf<String>())
        val ensureDaemonRequiredVersions = Collections.synchronizedList(mutableListOf<String?>())
        val detachedProjects = Collections.synchronizedList(mutableListOf<DaemonProject>())
        val openedSettings = Collections.synchronizedList(mutableListOf<LSPConfig>())
        val openedDaemons = Collections.synchronizedList(mutableListOf<DaemonStatus>())
        val openProjectEntered = CountDownLatch(1)
        val releaseOpenProject = CountDownLatch(1)
        var blockOpenProject = false
        var port = 8488
        var projectId = "project-123"
        var baseUrlOverride: URI? = null
        var statusVersion = "8.1.0"
        var ensuredVersion = "8.1.0"
        var authToken: String? = null
        var statusError: RuntimeException? = null
        var openProjectError: Exception? = null
        var ensureDaemonError: RuntimeException? = null
        @Volatile
        var openGateEntered: CountDownLatch? = null
        @Volatile
        var openGate: CountDownLatch? = null
        @Volatile
        var statusGate: CountDownLatch? = null
        @Volatile
        var statusGateEntered: CountDownLatch? = null
        @Volatile
        var statusGateReleaseError: RuntimeException? = null

        override fun status(): DaemonStatus {
            statusCalls.incrementAndGet()
            val gate = statusGate
            if (gate != null) {
                statusGate = null
                statusGateEntered?.countDown()
                gate.await(5, TimeUnit.SECONDS)
                statusGateReleaseError?.let { throw it }
            }
            statusError?.let { throw it }
            return DaemonStatus(version = statusVersion, port = port, authToken = authToken)
        }

        override fun ensureDaemon(binPath: String, requiredVersion: String?): DaemonStatus {
            ensureDaemonCalls.incrementAndGet()
            ensureDaemonPaths.add(binPath)
            ensureDaemonRequiredVersions.add(requiredVersion)
            ensureDaemonError?.let { throw it }
            statusError = null
            statusVersion = ensuredVersion
            return DaemonStatus(version = ensuredVersion, port = port, authToken = authToken)
        }

        override fun openProject(root: String, settings: LSPConfig, daemon: DaemonStatus): DaemonProject {
            openProjectCalls.incrementAndGet()
            openGateEntered?.countDown()
            openGate?.await(5, TimeUnit.SECONDS)
            openProjectError?.let { throw it }
            openedSettings.add(settings)
            openedDaemons.add(daemon)
            openProjectEntered.countDown()
            if (blockOpenProject) {
                releaseOpenProject.await(2, TimeUnit.SECONDS)
            }
            return DaemonProject(
                projectId,
                baseUrlOverride ?: URI("http://127.0.0.1:${daemon.port}/p/$projectId/"),
                daemon,
            )
        }

        override fun detachProject(project: DaemonProject) {
            detachProjectCalls.incrementAndGet()
            detachedProjects.add(project)
        }
    }

    class TestLspProcessHolder(
        project: Project,
        private val fakeDaemonClient: FakeDaemonClient,
        private val refreshWorkerState: Boolean = false,
        private val requiredVersion: String = "8.1.0",
    ) : LSPProcessHolder(project) {
        private val latch = CountDownLatch(1)
        val retryAttempts = mutableListOf<Int>()
        val initializeCalls = AtomicInteger(0)
        val initializeEntered = CountDownLatch(1)
        val releaseInitialize = CountDownLatch(1)
        val ensureStartedBlockingEntered = CountDownLatch(1)
        val releaseEnsureStartedBlocking = CountDownLatch(1)
        val binaryResolutionCalls = AtomicInteger(0)
        private val healthClockMs = AtomicLong(0)
        private val binaryResolutionResults = mutableListOf<String?>()
        private val binaryResolutionResultIndex = AtomicInteger(0)
        val binaryResolutionEntered = CountDownLatch(1)
        val releaseBinaryResolution = CountDownLatch(1)
        var blockEnsureStartedBlocking = false
        var blockInitialize = false
        var blockBinaryResolution = false
        var signalDownloadStartDuringResolution = false
        val signalDownloadStartAfterBlockOnce = java.util.concurrent.atomic.AtomicBoolean(false)
        val onRefreshOnce = java.util.concurrent.atomic.AtomicReference<(() -> Unit)?>(null)
        val initializedPorts = Collections.synchronizedList(mutableListOf<Int?>())
        @Volatile
        var sharedPathGate: CountDownLatch? = null
        @Volatile
        var sharedPathGateEntered: CountDownLatch? = null
        var initializeError: RuntimeException? = null
        var probeResult = true
        var compatibleSharedBinaryPathResult: String? = null
        val terminalRegistrationPaths = Collections.synchronizedList(mutableListOf<String>())
        var binaryFailureMessage = "Failed to download the Refact engine (version 8.1.0) from GitHub releases"
        var selectedVersion: String? = null
        var buildInfoResult: String? = null
        var skipWorkerRefresh = false
        val onBuildInfoOnce = java.util.concurrent.atomic.AtomicReference<(() -> Unit)?>(null)

        override fun selectedBinaryVersion(path: String): String? = selectedVersion

        override fun getBuildInfo(): String {
            val result = buildInfoResult
            onBuildInfoOnce.getAndSet(null)?.invoke()
            return result ?: super.getBuildInfo()
        }

        override val daemonClient: RefactDaemonClient
            get() = fakeDaemonClient

        override fun refreshAttachedWorkerState() {
            onRefreshOnce.getAndSet(null)?.invoke()
            if (refreshWorkerState && !skipWorkerRefresh) super.refreshAttachedWorkerState()
        }

        override fun initializeAttachedProject() {
            initializeCalls.incrementAndGet()
            initializedPorts.add(baseUrlOrNull()?.port)
            initializeEntered.countDown()
            if (blockInitialize) {
                releaseInitialize.await(2, TimeUnit.SECONDS)
            }
            initializeError?.let { throw it }
        }

        override fun requiredDaemonVersion(): String {
            return requiredVersion
        }

        override fun compatibleSharedBinaryPathOrNull(requiredVersion: String): String? {
            val gate = sharedPathGate
            if (gate != null) {
                sharedPathGate = null
                sharedPathGateEntered?.countDown()
                gate.await(5, TimeUnit.SECONDS)
            }
            return compatibleSharedBinaryPathResult ?: super.compatibleSharedBinaryPathOrNull(requiredVersion)
        }

        override fun registerSharedBinaryPathForTerminal(resolvedPath: String) {
            terminalRegistrationPaths.add(resolvedPath)
        }

        override fun sleepBeforeWakeRetry(attempt: Int) {
            retryAttempts.add(attempt)
        }

        override fun probeAttachedWorker(): Boolean {
            return probeResult
        }

        override fun healthNowMs(): Long {
            return healthClockMs.get()
        }

        override fun resolveBinaryPathForDaemon(onDownloadStart: () -> Unit): String? {
            binaryResolutionCalls.incrementAndGet()
            if (signalDownloadStartDuringResolution) {
                onDownloadStart()
            }
            if (blockBinaryResolution) {
                binaryResolutionEntered.countDown()
                releaseBinaryResolution.await(2, TimeUnit.SECONDS)
            }
            if (signalDownloadStartAfterBlockOnce.getAndSet(false)) {
                onDownloadStart()
            }
            val index = binaryResolutionResultIndex.getAndIncrement()
            return if (index < binaryResolutionResults.size) {
                binaryResolutionResults[index]
            } else {
                LSPProcessHolder.BIN_PATH
            }
        }

        override fun binaryResolutionFailureMessage(): String {
            return binaryFailureMessage
        }

        fun simulateRaceConditionWithScheduledTask(makeProjectDisposed: () -> Unit): AlreadyDisposedException? {
            var caughtException: AlreadyDisposedException? = null
            val future = AppExecutorUtil.getAppScheduledExecutorService().submit {
                try {
                    latch.await(1, TimeUnit.SECONDS)
                    capabilities = LSPCapabilities(cloudName = "test-cloud")
                } catch (e: Exception) {
                    if (e is AlreadyDisposedException) {
                        caughtException = e
                    }
                }
            }

            makeProjectDisposed()
            latch.countDown()
            future.get(2, TimeUnit.SECONDS)
            return caughtException
        }

        override fun ensureStartedBlocking(reason: String) {
            if (blockEnsureStartedBlocking) {
                ensureStartedBlockingEntered.countDown()
                releaseEnsureStartedBlocking.await(2, TimeUnit.SECONDS)
            }
            super.ensureStartedBlocking(reason)
        }

        fun ensureStartedBlockingForTest(reason: String) {
            ensureStartedBlocking(reason)
        }

        fun runHealthCheckOnceForTest() {
            runHealthCheckOnce()
        }

        fun advanceHealthClock(ms: Long) {
            healthClockMs.addAndGet(ms)
        }

        fun setBinaryResolutionResults(vararg results: String?) {
            binaryResolutionResults.clear()
            binaryResolutionResults.addAll(results.toList())
            binaryResolutionResultIndex.set(0)
            binaryResolutionCalls.set(0)
        }

        fun hasBinaryResolutionFailureForTest(): Boolean {
            val field = LSPProcessHolder::class.java.getDeclaredField("binaryResolutionFailure")
            field.isAccessible = true
            return field.get(this) != null
        }
    }

    private fun mockProject(root: String? = null): Project {
        return mockProject(root, mock(LSPProcessHolderChangedNotifier::class.java))
    }

    private fun mockProject(root: String? = null, publisher: LSPProcessHolderChangedNotifier): Project {
        val mockProject = mock(Project::class.java)
        val mockMessageBus = mock(MessageBus::class.java)
        `when`(mockProject.isDisposed).thenReturn(false)
        `when`(mockProject.messageBus).thenReturn(mockMessageBus)
        `when`(mockMessageBus.syncPublisher(LSPProcessHolderChangedNotifier.TOPIC)).thenReturn(publisher)
        if (root != null) {
            `when`(mockProject.basePath).thenReturn(root)
        }
        return mockProject
    }

    private class RecordingStatusPublisher : LSPProcessHolderChangedNotifier {
        val statuses = Collections.synchronizedList(mutableListOf<LSPBackendConnectionStatus>())

        override fun backendConnectionStatusChanged(newStatus: LSPBackendConnectionStatus) {
            statuses.add(newStatus)
        }
    }

    /** Records every status broadcast together with the config payload fields the editor would send. */
    private class RecordingConfigPublisher : LSPProcessHolderChangedNotifier {
        data class Broadcast(
            val status: LSPBackendConnectionStatus,
            val issue: Events.Config.ConnectionIssue?,
            val daemonPort: Int,
        )

        @Volatile
        var holder: LSPProcessHolder? = null
        val broadcasts = Collections.synchronizedList(mutableListOf<Broadcast>())

        override fun backendConnectionStatusChanged(newStatus: LSPBackendConnectionStatus) {
            val h = holder ?: return
            broadcasts.add(
                Broadcast(
                    newStatus,
                    configConnectionIssue(h.backendConnectionStatus(), h.connectionIssue()),
                    h.daemonPort(),
                )
            )
        }
    }

    private fun <T> runOffEdt(block: () -> T): T {
        return ApplicationManager.getApplication().executeOnPooledThread<T> { block() }.get(10, TimeUnit.SECONDS)
    }

    private fun withServer(block: (MockWebServer) -> Unit) {
        val server = MockWebServer()
        try {
            server.protocols = listOf(Protocol.H2_PRIOR_KNOWLEDGE)
            server.start()
            block(server)
        } finally {
            server.shutdown()
        }
    }

    private fun waitForLifecycle(holder: TestLspProcessHolder) {
        val deadline = System.currentTimeMillis() + 3000
        while (holder.hasPendingLifecycleWork() && System.currentTimeMillis() < deadline) {
            Thread.sleep(25)
        }
        if (holder.hasPendingLifecycleWork()) {
            Thread.sleep(25)
        }
        assertFalse(holder.hasPendingLifecycleWork())
    }

    private fun withBrowserHost(host: String, block: () -> Unit) {
        val previous = InferenceGlobalContext.browserHost
        try {
            InferenceGlobalContext.browserHost = host
            block()
        } finally {
            InferenceGlobalContext.browserHost = previous
        }
    }

    private fun withConfiguredDaemonPort(block: () -> Unit) {
        val previous = AppSettingsState.instance.xDebugLSPPort
        try {
            block()
        } finally {
            AppSettingsState.instance.xDebugLSPPort = previous
        }
    }

    @Test
    fun testFailedConfigVersionAccessorDoesNotFetchWorkerOrDaemon() = withServer { server ->
        val fake = FakeDaemonClient().apply {
            baseUrlOverride = URI(server.url("/p/failed/").toString())
            openProjectError = RuntimeException("failed")
        }
        val holder = TestLspProcessHolder(mockProject(createTempDir().canonicalPath), fake)
        try {
            runOffEdt { holder.ensureStartedBlockingForTest("failed") }
            assertEquals(LSPBackendConnectionStatus.FAILED, holder.backendConnectionStatus())
            val statusCalls = fake.statusCalls.get()
            val versions = holder.runtimeVersions()
            repeat(20) {
                assertSame(versions, holder.runtimeVersions())
                com.smallcloud.refactai.panes.sharedchat.ensureBackendStartedForConfig(
                    holder.backendReady(), holder.backendConnectionStatus(),
                ) { holder.ensureStartedAsync("editor-config-request") }
            }
            waitForLifecycle(holder)
            assertEquals(statusCalls, fake.statusCalls.get())
            assertEquals(0, server.requestCount)
            assertNull(holder.runtimeVersions().worker)
        } finally { holder.dispose() }
    }

    @Test
    fun testBuildInfoResultDiscardedWhenWakeReplacesAttachmentInSameGeneration() {
        val fake = FakeDaemonClient()
        val holder = TestLspProcessHolder(mockProject(createTempDir().canonicalPath), fake, refreshWorkerState = true)
        holder.buildInfoResult = """{"version":"stale-worker"}"""
        holder.onBuildInfoOnce.set {
            holder.skipWorkerRefresh = true
            assertTrue(holder.wakeWorkerForRetry("nested-build-info-wake"))
        }
        LSPProcessHolder.buildInfo = "previous-compatible-global"
        try {
            runOffEdt { holder.ensureStartedBlockingForTest("outer") }
            assertEquals(2, fake.openProjectCalls.get())
            assertEquals(1, holder.initializeCalls.get())
            assertEquals(LSPBackendConnectionStatus.READY, holder.backendConnectionStatus())
            assertNull(holder.runtimeVersions().worker)
            assertEquals("previous-compatible-global", LSPProcessHolder.buildInfo)
        } finally { holder.dispose() }
    }

    @Test
    fun testBuildInfoResultDiscardedAfterExplicitGenerationChange() {
        val fake = FakeDaemonClient()
        val holder = TestLspProcessHolder(mockProject(createTempDir().canonicalPath), fake, refreshWorkerState = true)
        holder.buildInfoResult = """{"version":"old-worker"}"""
        holder.onBuildInfoOnce.set {
            holder.skipWorkerRefresh = true
            fake.statusVersion = "8.4.0"
            holder.settingsChanged("new-owner")
        }
        try {
            runOffEdt { holder.ensureStartedBlockingForTest("old-owner") }
            waitForLifecycle(holder)
            assertEquals("8.4.0", holder.runtimeVersions().daemon!!.version)
            assertNull(holder.runtimeVersions().worker)
            assertEquals(1, holder.initializeCalls.get())
        } finally { holder.dispose() }
    }

    @Test
    fun testRuntimeVersionsAreHolderOwnedAndUnknownSelectedIsNotPolicyOrGlobal() {
        val fakeA = FakeDaemonClient().apply { statusError = RuntimeException("missing"); ensuredVersion = "8.2.0" }
        val fakeB = FakeDaemonClient().apply { statusVersion = "8.3.0" }
        val a = TestLspProcessHolder(mockProject(createTempDir().canonicalPath), fakeA).apply { selectedVersion = "8.1.7" }
        val b = TestLspProcessHolder(mockProject(createTempDir().canonicalPath), fakeB)
        LSPProcessHolder.BIN_PATH = "/tmp/poisoned-global"
        LSPProcessHolder.buildInfo = """{"version":"poisoned-worker"}"""
        try {
            runOffEdt { a.ensureStartedBlockingForTest("a") }
            val snapshotA = a.runtimeVersions()
            assertEquals(com.smallcloud.refactai.Resources.version, snapshotA.plugin!!.version)
            assertEquals("8.1.7", snapshotA.selected!!.version)
            assertEquals("8.2.0", snapshotA.daemon!!.version)
            assertNull(snapshotA.worker)
            runOffEdt { b.ensureStartedBlockingForTest("b") }
            assertNull(b.runtimeVersions().selected)
            assertNull(b.runtimeVersions().worker)
            assertEquals("8.3.0", b.runtimeVersions().daemon!!.version)
            assertEquals(snapshotA, a.runtimeVersions())
            assertNull(snapshotA.installed)
            assertNull(snapshotA.staged)
        } finally { a.dispose(); b.dispose() }
    }

    @Test
    fun testUnknownExplicitBinaryObservationHasNullVersion() {
        val fake = FakeDaemonClient().apply { statusError = RuntimeException("missing") }
        val holder = TestLspProcessHolder(mockProject(createTempDir().canonicalPath), fake)
        LSPProcessHolder.BIN_PATH = "/tmp/unknown-explicit"
        try {
            runOffEdt { holder.ensureStartedBlockingForTest("unknown") }
            assertNotNull(holder.runtimeVersions().selected)
            assertNull(holder.runtimeVersions().selected!!.version)
            assertEquals("observed", holder.runtimeVersions().selected!!.state)
        } finally { holder.dispose() }
    }

    @Test
    fun testAutomaticFailureRetainsTimestampButExplicitInvalidationClearsBeforeBroadcast() {
        val fake = FakeDaemonClient()
        val publisher = object : LSPProcessHolderChangedNotifier {
            var holder: LSPProcessHolder? = null
            val connectingSnapshots = mutableListOf<RuntimeVersions>()
            override fun backendConnectionStatusChanged(newStatus: LSPBackendConnectionStatus) {
                if (newStatus == LSPBackendConnectionStatus.CONNECTING) connectingSnapshots.add(holder!!.runtimeVersions())
            }
        }
        val holder = TestLspProcessHolder(mockProject(createTempDir().canonicalPath, publisher), fake)
        publisher.holder = holder
        try {
            runOffEdt { holder.ensureStartedBlockingForTest("initial") }
            val observation = holder.runtimeVersions().daemon!!
            fake.statusError = RuntimeException("gone")
            assertFalse(runOffEdt { holder.wakeWorkerForRetry("automatic") })
            assertEquals(observation.copy(state = "last_observed"), holder.runtimeVersions().daemon)
            holder.blockEnsureStartedBlocking = true
            fake.ensureDaemonError = RuntimeException("still gone")
            holder.settingsChanged("explicit")
            waitForLifecycle(holder)
            val cleared = publisher.connectingSnapshots.single()
            assertNull(cleared.selected)
            assertNull(cleared.daemon)
            assertNull(cleared.worker)
        } finally { holder.dispose() }
    }

    @Test
    fun testAutomaticBusyRetryKeepsBusyIssueOnEveryBroadcastUntilReady() {
        val root = createTempDir().canonicalPath
        val publisher = RecordingConfigPublisher()
        val busy = DaemonWorkspaceBusyException(WorkspaceBusyConflict(holderPid = 4242, holderHttpPort = 8005))
        val fake = FakeDaemonClient().apply { openProjectError = busy }
        val holder = TestLspProcessHolder(mockProject(root, publisher), fake)
        publisher.holder = holder
        LSPProcessHolder.BIN_PATH = "/tmp/refact"
        val busyIssue = Events.Config.ConnectionIssue("workspace_busy", holderPid = 4242, holderHttpPort = 8005)
        val gate = CountDownLatch(1)
        val gateEntered = CountDownLatch(1)
        try {
            runOffEdt { holder.ensureStartedBlockingForTest("busy-start") }
            assertEquals(LSPBackendConnectionStatus.FAILED, holder.backendConnectionStatus())
            assertEquals(busyIssue, publisher.broadcasts.last().issue)
            publisher.broadcasts.clear()

            fake.openGateEntered = gateEntered
            fake.openGate = gate
            holder.advanceHealthClock(5_000)
            holder.ensureStartedAsync("editor-config-request")
            assertTrue(gateEntered.await(3, TimeUnit.SECONDS))
            assertEquals(LSPBackendConnectionStatus.STARTING, holder.backendConnectionStatus())
            assertEquals(busyIssue, configConnectionIssue(holder.backendConnectionStatus(), holder.connectionIssue()))
            assertEquals(
                listOf(RecordingConfigPublisher.Broadcast(LSPBackendConnectionStatus.STARTING, busyIssue, 8488)),
                publisher.broadcasts.toList(),
            )
            gate.countDown()
            waitForLifecycle(holder)
            fake.openGate = null
            fake.openGateEntered = null

            assertEquals(LSPBackendConnectionStatus.FAILED, holder.backendConnectionStatus())
            assertTrue(publisher.broadcasts.all { it.issue == busyIssue })
            assertEquals(2, fake.openProjectCalls.get())

            publisher.broadcasts.clear()
            fake.openProjectError = null
            holder.advanceHealthClock(10_000)
            holder.ensureStartedAsync("editor-config-request")
            waitForLifecycle(holder)

            assertEquals(LSPBackendConnectionStatus.READY, holder.backendConnectionStatus())
            assertNull(holder.connectionIssue())
            assertEquals(
                listOf(
                    RecordingConfigPublisher.Broadcast(LSPBackendConnectionStatus.STARTING, busyIssue, 8488),
                    RecordingConfigPublisher.Broadcast(LSPBackendConnectionStatus.READY, null, 8488),
                ),
                publisher.broadcasts.toList(),
            )
            assertEquals(0, fake.ensureDaemonCalls.get())
        } finally {
            gate.countDown()
            holder.dispose()
        }
    }

    @Test
    fun testExplicitSettingsChangeClearsBusyAndDifferentFailureReplacesIt() {
        val root = createTempDir().canonicalPath
        val publisher = RecordingConfigPublisher()
        val fake = FakeDaemonClient().apply {
            openProjectError = DaemonWorkspaceBusyException(WorkspaceBusyConflict(holderPid = 7))
        }
        val holder = TestLspProcessHolder(mockProject(root, publisher), fake)
        publisher.holder = holder
        LSPProcessHolder.BIN_PATH = "/tmp/refact"
        try {
            runOffEdt { holder.ensureStartedBlockingForTest("busy-start") }
            publisher.broadcasts.clear()

            fake.openProjectError = RuntimeException("other failure")
            holder.settingsChanged("explicit-change")
            waitForLifecycle(holder)

            assertEquals(LSPBackendConnectionStatus.CONNECTING, publisher.broadcasts.first().status)
            assertNull(publisher.broadcasts.first().issue)
            assertTrue(publisher.broadcasts.none { it.issue?.code == "workspace_busy" })
            assertEquals(
                Events.Config.ConnectionIssue("engine_start_failed", stage = "project_open"),
                publisher.broadcasts.last().issue,
            )
        } finally {
            holder.dispose()
        }
    }

    @Test
    fun testExplicitDaemonPortChangeToUnavailablePortNeverReportsPreviousDaemon() = withConfiguredDaemonPort {
        val root = createTempDir().canonicalPath
        val publisher = RecordingConfigPublisher()
        val fake = FakeDaemonClient().apply { port = 9101 }
        val holder = TestLspProcessHolder(mockProject(root, publisher), fake)
        publisher.holder = holder
        LSPProcessHolder.BIN_PATH = "/tmp/refact"
        try {
            AppSettingsState.instance.xDebugLSPPort = 9101
            runOffEdt { holder.ensureStartedBlockingForTest("port-a") }
            assertEquals(LSPBackendConnectionStatus.READY, holder.backendConnectionStatus())
            assertEquals(9101, holder.daemonPort())
            publisher.broadcasts.clear()

            AppSettingsState.instance.xDebugLSPPort = 9202
            fake.statusError = RuntimeException("port B unavailable")
            fake.ensureDaemonError = RuntimeException("port B unavailable")
            holder.settingsChanged("daemon-port-changed")
            waitForLifecycle(holder)

            assertEquals(LSPBackendConnectionStatus.FAILED, holder.backendConnectionStatus())
            assertEquals(9202, holder.daemonPort())
            assertNull(holder.baseUrlOrNull())
            assertTrue(publisher.broadcasts.isNotEmpty())
            assertEquals(LSPBackendConnectionStatus.CONNECTING, publisher.broadcasts.first().status)
            assertTrue(publisher.broadcasts.all { it.daemonPort == 9202 })
            assertEquals(1, fake.ensureDaemonCalls.get())
            assertEquals(1, fake.openProjectCalls.get())
        } finally {
            holder.dispose()
        }
    }

    @Test
    fun testStaleInFlightOpenForPreviousDaemonIsDiscardedAfterExplicitPortChange() = withConfiguredDaemonPort {
        val root = createTempDir().canonicalPath
        val publisher = RecordingConfigPublisher()
        val gate = CountDownLatch(1)
        val gateEntered = CountDownLatch(1)
        val fake = FakeDaemonClient().apply {
            port = 9101
            openGate = gate
            openGateEntered = gateEntered
        }
        val holder = TestLspProcessHolder(mockProject(root, publisher), fake)
        publisher.holder = holder
        LSPProcessHolder.BIN_PATH = "/tmp/refact"
        try {
            AppSettingsState.instance.xDebugLSPPort = 9101
            val first = ApplicationManager.getApplication().executeOnPooledThread {
                holder.ensureStartedBlockingForTest("port-a-in-flight")
            }
            assertTrue(gateEntered.await(3, TimeUnit.SECONDS))

            AppSettingsState.instance.xDebugLSPPort = 9202
            fake.port = 9202
            holder.settingsChanged("daemon-port-changed")
            assertEquals(9202, holder.daemonPort())

            gate.countDown()
            first.get(5, TimeUnit.SECONDS)
            waitForLifecycle(holder)

            assertEquals(LSPBackendConnectionStatus.READY, holder.backendConnectionStatus())
            assertEquals(9202, holder.daemonPort())
            assertEquals(9202, holder.baseUrlOrNull()?.port)
            assertEquals(listOf(9101, 9202), fake.openedDaemons.map { it.port })
            assertEquals(listOf(9101), fake.detachedProjects.map { it.daemon.port })
            assertTrue(publisher.broadcasts.none { it.daemonPort == 9101 && it.status == LSPBackendConnectionStatus.READY })
            assertEquals(1, publisher.broadcasts.count { it.status == LSPBackendConnectionStatus.READY })
        } finally {
            gate.countDown()
            holder.dispose()
        }
    }

    @Test
    fun testAutomaticRecoveryWithIncompatibleOrMissingDaemonNeverResolvesOrEnsures() {
        val root = createTempDir().canonicalPath
        val fake = FakeDaemonClient()
        val holder = TestLspProcessHolder(mockProject(root), fake)
        LSPProcessHolder.BIN_PATH = "/tmp/refact"
        try {
            runOffEdt { holder.ensureStartedBlockingForTest("initial") }
            assertEquals(LSPBackendConnectionStatus.READY, holder.backendConnectionStatus())
            holder.setBinaryResolutionResults()

            fake.statusVersion = "8.0.0"
            holder.probeResult = false
            repeat(3) { holder.runHealthCheckOnceForTest() }
            waitForLifecycle(holder)
            assertEquals(LSPBackendConnectionStatus.FAILED, holder.backendConnectionStatus())
            assertEquals(
                LSPConnectionIssue(LSPConnectionIssueCode.ENGINE_START_FAILED, stage = LSPStartupStage.DAEMON_START),
                holder.connectionIssue(),
            )

            fake.statusError = RuntimeException("daemon missing")
            repeat(5) {
                holder.advanceHealthClock(60_000)
                holder.runHealthCheckOnceForTest()
                waitForLifecycle(holder)
                holder.ensureStartedAsync("editor-config-request")
                waitForLifecycle(holder)
                runOffEdt { holder.ensureStartedBlockingForTest("automatic-$it") }
                assertFalse(runOffEdt { holder.wakeWorkerForRetry("wake-$it") })
            }

            assertEquals(0, holder.binaryResolutionCalls.get())
            assertEquals(0, fake.ensureDaemonCalls.get())
            assertEquals(1, fake.openProjectCalls.get())
            assertEquals(LSPBackendConnectionStatus.FAILED, holder.backendConnectionStatus())

            holder.probeResult = true
            holder.settingsChanged("explicit-retry")
            waitForLifecycle(holder)

            assertEquals(LSPBackendConnectionStatus.READY, holder.backendConnectionStatus())
            assertEquals(1, holder.binaryResolutionCalls.get())
            assertEquals(listOf("/tmp/refact"), fake.ensureDaemonPaths)
            assertEquals(2, fake.openProjectCalls.get())
        } finally {
            holder.dispose()
        }
    }

    @Test
    fun testAlreadyDisposedException() {
        val mockProject = mockProject()
        val holder = TestLspProcessHolder(mockProject, FakeDaemonClient())

        val exception = holder.simulateRaceConditionWithScheduledTask {
            `when`(mockProject.isDisposed).thenReturn(true)
            `when`(mockProject.messageBus).thenThrow(
                AlreadyDisposedException("Already disposed")
            )
        }

        assertNull("With the fix, no AlreadyDisposedException should be thrown", exception)
        assertEquals("test-cloud", holder.capabilities.cloudName)
        holder.dispose()
    }

    @Test
    fun testExplicitFirstStartupConsumesInitialProvisioningSoLaterAutomaticRecoveryDoesNotProvision() {
        val root = createTempDir().canonicalPath
        val fake = FakeDaemonClient()
        val holder = TestLspProcessHolder(mockProject(root), fake)
        LSPProcessHolder.BIN_PATH = "/tmp/refact"
        try {
            holder.settingsChanged("explicit-first-startup")
            waitForLifecycle(holder)
            assertEquals(LSPBackendConnectionStatus.READY, holder.backendConnectionStatus())
            assertEquals(0, fake.ensureDaemonCalls.get())
            holder.setBinaryResolutionResults()

            fake.statusError = RuntimeException("daemon missing")
            holder.probeResult = false
            repeat(3) { holder.runHealthCheckOnceForTest() }
            waitForLifecycle(holder)
            holder.advanceHealthClock(60_000)
            holder.ensureStartedAsync("editor-config-request")
            waitForLifecycle(holder)
            runOffEdt { holder.ensureStartedBlockingForTest("automatic-after-explicit") }

            assertEquals(LSPBackendConnectionStatus.FAILED, holder.backendConnectionStatus())
            assertEquals(0, holder.binaryResolutionCalls.get())
            assertEquals(0, fake.ensureDaemonCalls.get())
            assertEquals(1, fake.openProjectCalls.get())
        } finally {
            holder.dispose()
        }
    }

    @Test
    fun testStaleWakeStatusFailureDoesNotOverwriteNewerExplicitAttachment() = withConfiguredDaemonPort {
        val root = createTempDir().canonicalPath
        val fake = FakeDaemonClient().apply { port = 9101 }
        val holder = TestLspProcessHolder(mockProject(root), fake)
        LSPProcessHolder.BIN_PATH = "/tmp/refact"
        val statusGate = CountDownLatch(1)
        val statusEntered = CountDownLatch(1)
        try {
            AppSettingsState.instance.xDebugLSPPort = 9101
            runOffEdt { holder.ensureStartedBlockingForTest("port-a") }
            assertEquals(LSPBackendConnectionStatus.READY, holder.backendConnectionStatus())

            fake.statusGateEntered = statusEntered
            fake.statusGateReleaseError = RuntimeException("daemon A gone")
            fake.statusGate = statusGate
            val wake = ApplicationManager.getApplication().executeOnPooledThread<Boolean> {
                holder.wakeWorkerForRetry("stale-wake-a")
            }
            assertTrue(statusEntered.await(3, TimeUnit.SECONDS))

            AppSettingsState.instance.xDebugLSPPort = 9202
            fake.port = 9202
            holder.settingsChanged("daemon-port-changed")
            waitForLifecycle(holder)
            assertEquals(LSPBackendConnectionStatus.READY, holder.backendConnectionStatus())
            val detachesBeforeStaleRelease = fake.detachProjectCalls.get()
            val opensBeforeStaleRelease = fake.openProjectCalls.get()

            statusGate.countDown()
            assertFalse(wake.get(5, TimeUnit.SECONDS))

            assertEquals(LSPBackendConnectionStatus.READY, holder.backendConnectionStatus())
            assertNull(holder.connectionIssue())
            assertEquals(9202, holder.baseUrlOrNull()?.port)
            assertEquals(9202, holder.daemonPort())
            assertEquals("observed", holder.runtimeVersions().daemon!!.state)
            assertNull(holder.runtimeVersions().worker)
            assertEquals(detachesBeforeStaleRelease, fake.detachProjectCalls.get())
            assertEquals(opensBeforeStaleRelease, fake.openProjectCalls.get())
            assertEquals(0, fake.ensureDaemonCalls.get())
        } finally {
            statusGate.countDown()
            holder.dispose()
        }
    }

    @Test
    fun testStaleWakeStatusSuccessDoesNotOpenPreviousDaemonAfterExplicitChange() = withConfiguredDaemonPort {
        val root = createTempDir().canonicalPath
        val fake = FakeDaemonClient().apply { port = 9101 }
        val holder = TestLspProcessHolder(mockProject(root), fake)
        LSPProcessHolder.BIN_PATH = "/tmp/refact"
        val statusGate = CountDownLatch(1)
        val statusEntered = CountDownLatch(1)
        try {
            AppSettingsState.instance.xDebugLSPPort = 9101
            runOffEdt { holder.ensureStartedBlockingForTest("port-a") }

            fake.statusGateEntered = statusEntered
            fake.statusGate = statusGate
            val wake = ApplicationManager.getApplication().executeOnPooledThread<Boolean> {
                holder.wakeWorkerForRetry("stale-wake-a-success")
            }
            assertTrue(statusEntered.await(3, TimeUnit.SECONDS))

            AppSettingsState.instance.xDebugLSPPort = 9202
            fake.port = 9202
            holder.settingsChanged("daemon-port-changed")
            waitForLifecycle(holder)
            val opensBeforeStaleRelease = fake.openProjectCalls.get()
            val detachesBeforeStaleRelease = fake.detachProjectCalls.get()

            statusGate.countDown()
            assertFalse(wake.get(5, TimeUnit.SECONDS))

            assertEquals(opensBeforeStaleRelease, fake.openProjectCalls.get())
            assertEquals(detachesBeforeStaleRelease, fake.detachProjectCalls.get())
            assertEquals(LSPBackendConnectionStatus.READY, holder.backendConnectionStatus())
            assertEquals(9202, holder.baseUrlOrNull()?.port)
        } finally {
            statusGate.countDown()
            holder.dispose()
        }
    }

    @Test
    fun testStaleInitialResolverCallbackAndFailureDoNotAffectQueuedExplicitStartup() {
        val root = createTempDir().canonicalPath
        val publisher = RecordingStatusPublisher()
        val fake = FakeDaemonClient().apply { statusError = RuntimeException("daemon missing") }
        val holder = TestLspProcessHolder(mockProject(root, publisher), fake).apply {
            blockBinaryResolution = true
            signalDownloadStartAfterBlockOnce.set(true)
            setBinaryResolutionResults(null, "/tmp/refact-b")
        }
        LSPProcessHolder.BIN_PATH = null
        val initial = ApplicationManager.getApplication().executeOnPooledThread {
            holder.ensureStartedBlockingForTest("initial-a")
        }
        try {
            assertTrue(holder.binaryResolutionEntered.await(3, TimeUnit.SECONDS))
            holder.settingsChanged("explicit-b")
            assertEquals(LSPBackendConnectionStatus.CONNECTING, holder.backendConnectionStatus())
            publisher.statuses.clear()

            holder.releaseBinaryResolution.countDown()
            initial.get(5, TimeUnit.SECONDS)
            val afterStale = publisher.statuses.toList()
            val failureAfterStale = holder.hasBinaryResolutionFailureForTest()
            waitForLifecycle(holder)

            assertFalse(afterStale.contains(LSPBackendConnectionStatus.INSTALLING))
            assertFalse(afterStale.contains(LSPBackendConnectionStatus.FAILED))
            assertFalse(failureAfterStale)
            assertFalse(publisher.statuses.contains(LSPBackendConnectionStatus.FAILED))
            assertEquals(LSPBackendConnectionStatus.READY, holder.backendConnectionStatus())
            assertEquals(2, holder.binaryResolutionCalls.get())
            assertEquals(listOf("/tmp/refact-b"), fake.ensureDaemonPaths)
            assertNull(holder.runtimeVersions().selected!!.version)
            assertEquals("observed", holder.runtimeVersions().selected!!.state)
        } finally {
            holder.releaseBinaryResolution.countDown()
            holder.dispose()
        }
    }

    @Test
    fun testExplicitSwitchDuringRefreshPreventsInitializingPreviousDaemon() = withConfiguredDaemonPort {
        val root = createTempDir().canonicalPath
        val fake = FakeDaemonClient().apply { port = 9101 }
        val holder = TestLspProcessHolder(mockProject(root), fake)
        LSPProcessHolder.BIN_PATH = "/tmp/refact"
        try {
            AppSettingsState.instance.xDebugLSPPort = 9101
            holder.onRefreshOnce.set {
                AppSettingsState.instance.xDebugLSPPort = 9202
                fake.port = 9202
                holder.settingsChanged("daemon-port-changed")
            }
            runOffEdt { holder.ensureStartedBlockingForTest("port-a") }
            val initializedAfterStale = holder.initializedPorts.toList()
            waitForLifecycle(holder)

            assertFalse(initializedAfterStale.contains(9101))
            assertEquals(listOf<Int?>(9202), holder.initializedPorts.toList())
            assertEquals(listOf(9101), fake.detachedProjects.map { it.daemon.port })
            assertEquals(LSPBackendConnectionStatus.READY, holder.backendConnectionStatus())
            assertEquals(9202, holder.baseUrlOrNull()?.port)
        } finally {
            holder.dispose()
        }
    }

    @Test
    fun testStaleWakeBlockedInSharedBinaryLookupDoesNotOpenPreviousDaemon() = withConfiguredDaemonPort {
        val root = createTempDir().canonicalPath
        val fake = FakeDaemonClient().apply { port = 9101 }
        val holder = TestLspProcessHolder(mockProject(root), fake)
        LSPProcessHolder.BIN_PATH = "/tmp/refact"
        val gate = CountDownLatch(1)
        val entered = CountDownLatch(1)
        try {
            AppSettingsState.instance.xDebugLSPPort = 9101
            runOffEdt { holder.ensureStartedBlockingForTest("port-a") }

            holder.sharedPathGateEntered = entered
            holder.sharedPathGate = gate
            val wake = ApplicationManager.getApplication().executeOnPooledThread<Boolean> {
                holder.wakeWorkerForRetry("stale-wake-shared-lookup")
            }
            assertTrue(entered.await(3, TimeUnit.SECONDS))

            AppSettingsState.instance.xDebugLSPPort = 9202
            fake.port = 9202
            holder.settingsChanged("daemon-port-changed")
            waitForLifecycle(holder)
            assertEquals(LSPBackendConnectionStatus.READY, holder.backendConnectionStatus())
            val opensBeforeRelease = fake.openProjectCalls.get()
            val detachesBeforeRelease = fake.detachProjectCalls.get()
            val initializedBeforeRelease = holder.initializedPorts.toList()

            gate.countDown()
            assertFalse(wake.get(5, TimeUnit.SECONDS))

            assertEquals(opensBeforeRelease, fake.openProjectCalls.get())
            assertEquals(detachesBeforeRelease, fake.detachProjectCalls.get())
            assertEquals(initializedBeforeRelease, holder.initializedPorts.toList())
            assertEquals(LSPBackendConnectionStatus.READY, holder.backendConnectionStatus())
            assertNull(holder.connectionIssue())
            assertEquals(9202, holder.baseUrlOrNull()?.port)
        } finally {
            gate.countDown()
            holder.dispose()
        }
    }

    @Test
    fun testNestedFailedWakeDuringRefreshPreventsOuterReady() {
        val root = createTempDir().canonicalPath
        val fake = FakeDaemonClient()
        val holder = TestLspProcessHolder(mockProject(root), fake)
        LSPProcessHolder.BIN_PATH = "/tmp/refact"
        val nestedResult = java.util.concurrent.atomic.AtomicReference<Boolean?>(null)
        try {
            holder.onRefreshOnce.set {
                fake.openProjectError = RuntimeException("nested wake open failed")
                nestedResult.set(holder.wakeWorkerForRetry("nested-refresh-wake"))
            }
            runOffEdt { holder.ensureStartedBlockingForTest("outer-start") }

            assertEquals(false, nestedResult.get())
            assertEquals(LSPBackendConnectionStatus.FAILED, holder.backendConnectionStatus())
            assertFalse(holder.backendReady())
            assertNull(holder.baseUrlOrNull())
            assertEquals(0, holder.initializeCalls.get())
        } finally {
            holder.dispose()
        }
    }

    @Test
    fun testConcurrentBlockingEnsureStartedOnlyOpensProjectOnce() {
        val root = createTempDir().canonicalPath
        val fake = FakeDaemonClient().apply { blockOpenProject = true }
        val holder = TestLspProcessHolder(mockProject(root), fake)
        LSPProcessHolder.BIN_PATH = "/tmp/refact"
        val executor = Executors.newFixedThreadPool(3)
        try {
            val futures = (1..3).map {
                executor.submit {
                    holder.ensureStartedBlockingForTest("concurrent-$it")
                }
            }

            assertTrue(fake.openProjectEntered.await(2, TimeUnit.SECONDS))
            fake.releaseOpenProject.countDown()
            futures.forEach { it.get(3, TimeUnit.SECONDS) }

            assertEquals(1, fake.openProjectCalls.get())
            assertEquals(0, fake.ensureDaemonCalls.get())
        } finally {
            executor.shutdownNow()
            holder.dispose()
        }
    }

    @Test
    fun testConnectsToExistingDaemonWithNullBinPath() {
        val root = createTempDir().canonicalPath
        val fake = FakeDaemonClient().apply { statusVersion = "8.1.0" }
        val holder = TestLspProcessHolder(mockProject(root), fake)
        LSPProcessHolder.BIN_PATH = null
        try {
            runOffEdt { holder.ensureStartedBlockingForTest("existing-daemon") }

            assertEquals(1, fake.openProjectCalls.get())
            assertEquals(0, fake.ensureDaemonCalls.get())
            assertTrue(fake.statusCalls.get() >= 1)
            assertEquals(DaemonStatus(version = "8.1.0", port = 8488), fake.openedDaemons.single())
            assertEquals(URI("http://127.0.0.1:8488/p/project-123/"), holder.baseUrlOrNull())
        } finally {
            holder.dispose()
        }
    }

    @Test
    fun testExistingDaemonRegistersResolvedSharedBinaryForTerminal() {
        val root = createTempDir().canonicalPath
        val fake = FakeDaemonClient().apply { statusVersion = "8.1.0" }
        val holder = TestLspProcessHolder(mockProject(root), fake).apply {
            compatibleSharedBinaryPathResult = "/home/tester/.refact/bin/refact"
        }
        LSPProcessHolder.BIN_PATH = null
        try {
            runOffEdt { holder.ensureStartedBlockingForTest("existing-daemon-terminal-registration") }

            assertEquals(0, fake.ensureDaemonCalls.get())
            assertEquals(listOf("/home/tester/.refact/bin/refact"), holder.terminalRegistrationPaths)
        } finally {
            holder.dispose()
        }
    }

    @Test
    fun testStatusFailureFallsThroughToEnsureDaemon() {
        val root = createTempDir().canonicalPath
        val fake = FakeDaemonClient().apply {
            statusError = RuntimeException("daemon missing")
            port = 9595
            authToken = "ensured-token"
        }
        val holder = TestLspProcessHolder(mockProject(root), fake)
        LSPProcessHolder.BIN_PATH = "/tmp/refact-status-failure"
        try {
            runOffEdt { holder.ensureStartedBlockingForTest("missing-daemon") }

            assertEquals(1, fake.openProjectCalls.get())
            assertEquals(1, fake.ensureDaemonCalls.get())
            assertEquals(listOf("/tmp/refact-status-failure"), fake.ensureDaemonPaths)
            assertEquals(DaemonStatus(version = "8.1.0", port = 9595, authToken = "ensured-token"), fake.openedDaemons.single())
            assertEquals(URI("http://127.0.0.1:9595/p/project-123/"), holder.baseUrlOrNull())
        } finally {
            holder.dispose()
        }
    }

    @Test
    fun testOldDaemonFallsThroughToEnsureDaemon() {
        val root = createTempDir().canonicalPath
        val fake = FakeDaemonClient().apply {
            statusVersion = "8.0.0"
            ensuredVersion = "8.1.0"
        }
        val holder = TestLspProcessHolder(mockProject(root), fake, requiredVersion = "8.1.0")
        LSPProcessHolder.BIN_PATH = "/tmp/refact-upgrade"
        try {
            runOffEdt { holder.ensureStartedBlockingForTest("old-daemon") }

            assertEquals(1, fake.openProjectCalls.get())
            assertEquals(1, fake.ensureDaemonCalls.get())
            assertEquals(listOf("/tmp/refact-upgrade"), fake.ensureDaemonPaths)
        } finally {
            holder.dispose()
        }
    }

    @Test
    fun testBrowserUrlDefaultsToLoopbackBaseUrlAndPreservesToken() = withBrowserHost("") {
        val root = createTempDir().canonicalPath
        val fake = FakeDaemonClient().apply {
            port = 9999
            projectId = "abc123"
            authToken = "token with/slash"
        }
        val holder = TestLspProcessHolder(mockProject(root), fake)
        LSPProcessHolder.BIN_PATH = "/tmp/refact"
        try {
            runOffEdt { holder.ensureStartedBlockingForTest("browser-url") }

            assertEquals(URI("http://127.0.0.1:9999/p/abc123/?daemon_token=token%20with%2Fslash"), holder.browserUrlOrNull())
        } finally {
            holder.dispose()
        }
    }

    @Test
    fun testBrowserUrlUsesExplicitBrowserHost() = withBrowserHost("machine.local") {
        val root = createTempDir().canonicalPath
        val fake = FakeDaemonClient().apply {
            port = 9999
            projectId = "abc123"
            authToken = "embedded token"
        }
        val holder = TestLspProcessHolder(mockProject(root), fake)
        LSPProcessHolder.BIN_PATH = "/tmp/refact"
        try {
            runOffEdt { holder.ensureStartedBlockingForTest("browser-url-override") }

            assertEquals(
                URI("http://machine.local:9999/p/abc123/?daemon_token=embedded%20token"),
                holder.browserUrlOrNull()
            )
            assertEquals(
                URI("http://127.0.0.1:9999/p/abc123/?daemon_token=embedded%20token"),
                holder.embeddedBrowserUrlOrNull()
            )
        } finally {
            holder.dispose()
        }
    }

    @Test
    fun testBuildInfoUsesDaemonProjectV1Namespace() {
        val root = createTempDir().canonicalPath
        val fake = FakeDaemonClient()
        val holder = TestLspProcessHolder(mockProject(root), fake, refreshWorkerState = true)
        LSPProcessHolder.BIN_PATH = "/tmp/refact"
        val previousConnection = InferenceGlobalContext.connection
        val testConnection = AsyncConnection()
        InferenceGlobalContext.connection = testConnection
        try {
            withServer { server ->
                fake.port = server.port
                fake.baseUrlOverride = URI(server.url("/p/project-123/").toString())
                server.enqueue(
                    MockResponse.Builder()
                        .code(200)
                        .body("""{"version":"worker-9","commit":"worker-commit"}""")
                        .build()
                )
                server.enqueue(
                    MockResponse.Builder()
                        .code(200)
                        .addHeader("Content-Type", "application/json")
                        .body("{}")
                        .build()
                )
                server.enqueue(
                    MockResponse.Builder()
                        .code(200)
                        .addHeader("Content-Type", "application/json")
                        .body("{}")
                        .build()
                )

                runOffEdt { holder.ensureStartedBlockingForTest("build-info") }

                assertEquals("""{"version":"worker-9","commit":"worker-commit"}""", LSPProcessHolder.buildInfo)
                assertEquals("worker-9", holder.runtimeVersions().worker!!.version)
                assertEquals("worker-commit", holder.runtimeVersions().worker!!.commit)
                assertEquals("8.1.0", holder.runtimeVersions().daemon!!.version)
                assertEquals("/p/project-123/v1/build_info", server.takeRequest().path)
                assertEquals("/p/project-123/v1/caps", server.takeRequest().path)
                assertEquals("/p/project-123/v1/customization", server.takeRequest().path)
            }
        } finally {
            InferenceGlobalContext.connection = previousConnection
            testConnection.dispose()
            holder.dispose()
        }
    }

    @Test
    fun testDisposeDetachesDaemonProjectAndForgetsAttachState() {
        val root = createTempDir().canonicalPath
        val fake = FakeDaemonClient()
        val holder = TestLspProcessHolder(mockProject(root), fake)
        LSPProcessHolder.BIN_PATH = "/tmp/refact"

        runOffEdt { holder.ensureStartedBlockingForTest("dispose") }
        assertNotNull(holder.baseUrlOrNull())
        holder.dispose()

        assertNull(holder.baseUrlOrNull())
        assertEquals(1, fake.openProjectCalls.get())
        assertEquals(1, fake.detachProjectCalls.get())
        assertEquals("project-123", fake.detachedProjects.single().projectId)
    }

    @Test
    fun testFetchCustomizationRetriesAfterTransientWakeFailure() {
        val root = createTempDir().canonicalPath
        val fake = FakeDaemonClient()
        val holder = TestLspProcessHolder(mockProject(root), fake)
        LSPProcessHolder.BIN_PATH = "/tmp/refact"
        val previousConnection = InferenceGlobalContext.connection
        val testConnection = AsyncConnection()
        InferenceGlobalContext.connection = testConnection
        try {
            withServer { server ->
                fake.port = server.port
                fake.baseUrlOverride = URI(server.url("/p/project-123/").toString())
                server.enqueue(
                    MockResponse.Builder()
                        .code(503)
                        .addHeader("Content-Type", "application/json")
                        .body("worker warming")
                        .build()
                )
                server.enqueue(
                    MockResponse.Builder()
                        .code(200)
                        .addHeader("Content-Type", "application/json")
                        .body("{\"code_lens\":{}}")
                        .build()
                )

                runOffEdt { holder.ensureStartedBlockingForTest("customization") }
                val customization = runOffEdt { holder.fetchCustomization() }

                assertNotNull(customization)
                assertTrue(customization!!.has("code_lens"))
                assertEquals(2, fake.openProjectCalls.get())
                assertEquals(listOf(1), holder.retryAttempts)
                assertEquals("/p/project-123/v1/customization", server.takeRequest().path)
                assertEquals("/p/project-123/v1/customization", server.takeRequest().path)
            }
        } finally {
            InferenceGlobalContext.connection = previousConnection
            testConnection.dispose()
            holder.dispose()
        }
    }

    @Test
    fun testAsyncConnectionNon2xxFailsWithBody() {
        val previousConnection = InferenceGlobalContext.connection
        val connection = AsyncConnection()
        InferenceGlobalContext.connection = connection
        try {
            withServer { server ->
                server.enqueue(
                    MockResponse.Builder()
                        .code(503)
                        .addHeader("Content-Type", "application/json")
                        .body("wake me")
                        .build()
                )

                val future = connection.get(URI(server.url("/v1/caps").toString())).get()

                try {
                    future.get()
                    fail("non-2xx response should fail")
                } catch (error: Exception) {
                    val cause = error.cause
                    assertTrue("expected HttpStatusException, got $cause", cause is HttpStatusException)
                    val status = cause as HttpStatusException
                    assertEquals(503, status.statusCode)
                    assertEquals("wake me", status.responseBody)
                }
            }
        } finally {
            InferenceGlobalContext.connection = previousConnection
            connection.dispose()
        }
    }

    @Test
    fun testPendingLifecycleFlagDefaultsFalse() {
        val holder = TestLspProcessHolder(mockProject(), FakeDaemonClient())
        assertFalse(holder.hasPendingLifecycleWork())
        holder.dispose()
    }

    @Test
    fun testBackendConnectionStatusDefaultsConnecting() {
        val holder = TestLspProcessHolder(mockProject(), FakeDaemonClient())
        assertEquals(LSPBackendConnectionStatus.CONNECTING, holder.backendConnectionStatus())
        assertFalse(holder.backendReady())
        holder.dispose()
    }

    @Test
    fun testBinaryResolutionPublishesInstallingThenFailureAndSettingsResetRecovers() {
        val root = createTempDir().canonicalPath
        val publisher = RecordingStatusPublisher()
        val fake = FakeDaemonClient().apply { statusError = RuntimeException("daemon missing") }
        val holder = TestLspProcessHolder(mockProject(root, publisher), fake).apply {
            blockBinaryResolution = true
            signalDownloadStartDuringResolution = true
            setBinaryResolutionResults(null, "/tmp/refact-after-reset")
        }
        LSPProcessHolder.BIN_PATH = null
        val future = ApplicationManager.getApplication().executeOnPooledThread {
            holder.ensureStartedBlockingForTest("binary-slow-failure")
        }
        try {
            assertTrue(holder.binaryResolutionEntered.await(2, TimeUnit.SECONDS))
            assertEquals(LSPBackendConnectionStatus.INSTALLING, holder.backendConnectionStatus())
            holder.releaseBinaryResolution.countDown()
            future.get(3, TimeUnit.SECONDS)

            assertEquals(LSPBackendConnectionStatus.FAILED, holder.backendConnectionStatus())
            assertTrue(holder.hasBinaryResolutionFailureForTest())
            assertEquals(1, holder.binaryResolutionCalls.get())
            assertTrue(publisher.statuses.contains(LSPBackendConnectionStatus.INSTALLING))
            assertEquals(LSPBackendConnectionStatus.FAILED, publisher.statuses.last())

            holder.settingsChanged("binary-path-changed")
            waitForLifecycle(holder)

            assertEquals(LSPBackendConnectionStatus.READY, holder.backendConnectionStatus())
            assertFalse(holder.hasBinaryResolutionFailureForTest())
            assertEquals(2, holder.binaryResolutionCalls.get())
            assertEquals(listOf("/tmp/refact-after-reset"), fake.ensureDaemonPaths)
        } finally {
            holder.releaseBinaryResolution.countDown()
            holder.dispose()
        }
    }

    @Test
    fun testBackendReadyWaitsForInitialize() = withBrowserHost("") {
        val root = createTempDir().canonicalPath
        val fake = FakeDaemonClient().apply {
            authToken = "startup token/slash"
        }
        val holder = TestLspProcessHolder(mockProject(root), fake).apply {
            blockInitialize = true
        }
        LSPProcessHolder.BIN_PATH = "/tmp/refact"
        val future = ApplicationManager.getApplication().executeOnPooledThread {
            holder.ensureStartedBlockingForTest("backend-ready-after-init")
        }
        val expectedUrl = URI(
            "http://127.0.0.1:8488/p/project-123/?daemon_token=startup%20token%2Fslash"
        )
        try {
            assertTrue(holder.initializeEntered.await(2, TimeUnit.SECONDS))
            assertEquals(LSPBackendConnectionStatus.STARTING, holder.backendConnectionStatus())
            assertFalse(holder.backendReady())
            assertTrue(holder.isWorking)
            assertNotNull(holder.baseUrlOrNull())
            assertEquals(expectedUrl, holder.browserUrlOrNull())
            assertNull(holder.embeddedBrowserUrlOrNull())

            holder.releaseInitialize.countDown()
            future.get(3, TimeUnit.SECONDS)

            assertEquals(LSPBackendConnectionStatus.READY, holder.backendConnectionStatus())
            assertTrue(holder.backendReady())
            assertEquals(expectedUrl, holder.browserUrlOrNull())
            assertEquals(expectedUrl, holder.embeddedBrowserUrlOrNull())
        } finally {
            holder.releaseInitialize.countDown()
            holder.dispose()
        }
    }

    @Test
    fun testInitializeFailureDoesNotPublishReady() {
        val root = createTempDir().canonicalPath
        val holder = TestLspProcessHolder(mockProject(root), FakeDaemonClient()).apply {
            initializeError = RuntimeException("init failed")
        }
        LSPProcessHolder.BIN_PATH = "/tmp/refact"
        try {
            runOffEdt { holder.ensureStartedBlockingForTest("init-failure") }

            assertEquals(LSPBackendConnectionStatus.FAILED, holder.backendConnectionStatus())
            assertFalse(holder.backendReady())
            assertFalse(holder.isWorking)
            assertNull(holder.baseUrlOrNull())
        } finally {
            holder.dispose()
        }
    }

    @Test
    fun testFailedStateHealthCheckRetriesAndRecovers() {
        val root = createTempDir().canonicalPath
        val fake = FakeDaemonClient().apply {
            openProjectError = RuntimeException("first open failed")
        }
        val holder = TestLspProcessHolder(mockProject(root), fake)
        LSPProcessHolder.BIN_PATH = "/tmp/refact"
        try {
            runOffEdt { holder.ensureStartedBlockingForTest("transient-failure") }
            assertEquals(LSPBackendConnectionStatus.FAILED, holder.backendConnectionStatus())
            assertNull(holder.baseUrlOrNull())

            fake.openProjectError = null
            holder.advanceHealthClock(1_000)
            holder.runHealthCheckOnceForTest()
            waitForLifecycle(holder)

            assertEquals(LSPBackendConnectionStatus.READY, holder.backendConnectionStatus())
            assertTrue(holder.backendReady())
            assertEquals(2, fake.openProjectCalls.get())
        } finally {
            holder.dispose()
        }
    }

    @Test
    fun testBinaryResolutionFailureBacksOffAndSettingsResetAllowsRetry() {
        val root = createTempDir().canonicalPath
        val fake = FakeDaemonClient().apply { statusError = RuntimeException("daemon missing") }
        val holder = TestLspProcessHolder(mockProject(root), fake).apply {
            setBinaryResolutionResults(null, "/tmp/refact-after-reset")
        }
        LSPProcessHolder.BIN_PATH = null
        try {
            runOffEdt { holder.ensureStartedBlockingForTest("binary-missing") }

            assertEquals(LSPBackendConnectionStatus.FAILED, holder.backendConnectionStatus())
            assertTrue(holder.hasBinaryResolutionFailureForTest())
            assertEquals(1, holder.binaryResolutionCalls.get())
            assertEquals(0, fake.ensureDaemonCalls.get())

            holder.runHealthCheckOnceForTest()
            waitForLifecycle(holder)
            runOffEdt { holder.ensureStartedBlockingForTest("suppressed-manual-retry") }

            assertEquals(1, holder.binaryResolutionCalls.get())
            assertEquals(LSPBackendConnectionStatus.FAILED, holder.backendConnectionStatus())

            holder.settingsChanged("binary-path-changed")
            waitForLifecycle(holder)

            assertEquals(LSPBackendConnectionStatus.READY, holder.backendConnectionStatus())
            assertFalse(holder.hasBinaryResolutionFailureForTest())
            assertEquals(2, holder.binaryResolutionCalls.get())
            assertEquals(listOf("/tmp/refact-after-reset"), fake.ensureDaemonPaths)
        } finally {
            holder.dispose()
        }
    }

    @Test
    fun testBinaryResolutionFailureAutomaticRetryNeverResolvesOrEnsuresDaemon() {
        val root = createTempDir().canonicalPath
        val fake = FakeDaemonClient().apply { statusError = RuntimeException("daemon missing") }
        val holder = TestLspProcessHolder(mockProject(root), fake).apply {
            setBinaryResolutionResults(null, "/tmp/refact-after-explicit")
        }
        LSPProcessHolder.BIN_PATH = null
        try {
            runOffEdt { holder.ensureStartedBlockingForTest("binary-missing") }

            assertEquals(LSPBackendConnectionStatus.FAILED, holder.backendConnectionStatus())
            assertTrue(holder.hasBinaryResolutionFailureForTest())
            assertEquals(1, holder.binaryResolutionCalls.get())

            holder.advanceHealthClock(999)
            holder.runHealthCheckOnceForTest()
            waitForLifecycle(holder)
            assertEquals(1, holder.binaryResolutionCalls.get())

            holder.advanceHealthClock(1)
            holder.runHealthCheckOnceForTest()
            waitForLifecycle(holder)
            holder.advanceHealthClock(60_000)
            holder.ensureStartedAsync("editor-config-request")
            waitForLifecycle(holder)
            assertFalse(runOffEdt { holder.wakeWorkerForRetry("automatic-wake") })

            assertEquals(LSPBackendConnectionStatus.FAILED, holder.backendConnectionStatus())
            assertEquals(1, holder.binaryResolutionCalls.get())
            assertEquals(0, fake.ensureDaemonCalls.get())
            assertEquals(0, fake.openProjectCalls.get())
            assertEquals(LSPConnectionIssueCode.ENGINE_START_FAILED, holder.connectionIssue()?.code)

            holder.settingsChanged("explicit-retry")
            waitForLifecycle(holder)

            assertEquals(LSPBackendConnectionStatus.READY, holder.backendConnectionStatus())
            assertFalse(holder.hasBinaryResolutionFailureForTest())
            assertEquals(2, holder.binaryResolutionCalls.get())
            assertEquals(listOf("/tmp/refact-after-explicit"), fake.ensureDaemonPaths)
        } finally {
            holder.dispose()
        }
    }

    @Test
    fun testSingleFailedHealthProbeKeepsBackendReady() {
        val root = createTempDir().canonicalPath
        val publisher = RecordingStatusPublisher()
        val fake = FakeDaemonClient()
        val holder = TestLspProcessHolder(mockProject(root, publisher), fake)
        LSPProcessHolder.BIN_PATH = "/tmp/refact"
        try {
            runOffEdt { holder.ensureStartedBlockingForTest("health-probe-start") }
            publisher.statuses.clear()

            holder.probeResult = false
            holder.runHealthCheckOnceForTest()

            assertEquals(LSPBackendConnectionStatus.READY, holder.backendConnectionStatus())
            assertTrue(holder.backendReady())
            assertTrue(holder.isWorking)
            assertNotNull(holder.baseUrlOrNull())
            assertFalse(holder.hasPendingLifecycleWork())
            assertEquals(0, fake.detachProjectCalls.get())
            assertFalse(publisher.statuses.contains(LSPBackendConnectionStatus.FAILED))

            holder.probeResult = true
            holder.runHealthCheckOnceForTest()

            assertEquals(LSPBackendConnectionStatus.READY, holder.backendConnectionStatus())
            assertEquals(1, fake.openProjectCalls.get())
        } finally {
            holder.dispose()
        }
    }

    @Test
    fun testSuccessfulHealthProbeResetsFailedProbeCounter() {
        val root = createTempDir().canonicalPath
        val fake = FakeDaemonClient()
        val holder = TestLspProcessHolder(mockProject(root), fake)
        LSPProcessHolder.BIN_PATH = "/tmp/refact"
        try {
            runOffEdt { holder.ensureStartedBlockingForTest("health-probe-start") }

            holder.probeResult = false
            holder.runHealthCheckOnceForTest()
            holder.runHealthCheckOnceForTest()
            assertEquals(LSPBackendConnectionStatus.READY, holder.backendConnectionStatus())

            holder.probeResult = true
            holder.runHealthCheckOnceForTest()

            holder.probeResult = false
            holder.runHealthCheckOnceForTest()
            holder.runHealthCheckOnceForTest()

            assertEquals(LSPBackendConnectionStatus.READY, holder.backendConnectionStatus())
            assertTrue(holder.isWorking)
            assertNotNull(holder.baseUrlOrNull())
            assertFalse(holder.hasPendingLifecycleWork())
            assertEquals(1, fake.openProjectCalls.get())
            assertEquals(0, fake.detachProjectCalls.get())
        } finally {
            holder.dispose()
        }
    }

    @Test
    fun testSettingsChangeResetsFailedProbeCounter() {
        val root = createTempDir().canonicalPath
        val fake = FakeDaemonClient()
        val holder = TestLspProcessHolder(mockProject(root), fake)
        LSPProcessHolder.BIN_PATH = "/tmp/refact"
        try {
            runOffEdt { holder.ensureStartedBlockingForTest("health-probe-start") }

            holder.probeResult = false
            holder.runHealthCheckOnceForTest()
            holder.runHealthCheckOnceForTest()
            assertEquals(LSPBackendConnectionStatus.READY, holder.backendConnectionStatus())

            holder.settingsChanged("health-counter-reset")
            waitForLifecycle(holder)
            assertEquals(LSPBackendConnectionStatus.READY, holder.backendConnectionStatus())

            holder.runHealthCheckOnceForTest()
            holder.runHealthCheckOnceForTest()

            assertEquals(LSPBackendConnectionStatus.READY, holder.backendConnectionStatus())
            assertTrue(holder.isWorking)
            assertNotNull(holder.baseUrlOrNull())
            assertFalse(holder.hasPendingLifecycleWork())
            assertEquals(1, fake.openProjectCalls.get())
            assertEquals(0, fake.detachProjectCalls.get())
        } finally {
            holder.dispose()
        }
    }

    @Test
    fun testFailedHealthProbeClearsStateAndBacksOff() {
        val root = createTempDir().canonicalPath
        val fake = FakeDaemonClient()
        val holder = TestLspProcessHolder(mockProject(root), fake)
        LSPProcessHolder.BIN_PATH = "/tmp/refact"
        try {
            runOffEdt { holder.ensureStartedBlockingForTest("health-probe-start") }
            assertNotNull(holder.baseUrlOrNull())

            holder.blockEnsureStartedBlocking = true
            holder.probeResult = false
            holder.runHealthCheckOnceForTest()
            holder.runHealthCheckOnceForTest()
            assertEquals(LSPBackendConnectionStatus.READY, holder.backendConnectionStatus())
            assertTrue(holder.isWorking)
            assertNotNull(holder.baseUrlOrNull())
            assertFalse(holder.hasPendingLifecycleWork())

            holder.runHealthCheckOnceForTest()
            assertTrue(holder.ensureStartedBlockingEntered.await(2, TimeUnit.SECONDS))

            assertEquals(LSPBackendConnectionStatus.FAILED, holder.backendConnectionStatus())
            assertFalse(holder.isWorking)
            assertNull(holder.baseUrlOrNull())

            holder.probeResult = true
            holder.runHealthCheckOnceForTest()
            assertEquals(1, fake.openProjectCalls.get())

            holder.advanceHealthClock(1_000)
            holder.releaseEnsureStartedBlocking.countDown()
            waitForLifecycle(holder)
            assertEquals(LSPBackendConnectionStatus.READY, holder.backendConnectionStatus())
            assertEquals(2, fake.openProjectCalls.get())
        } finally {
            holder.releaseEnsureStartedBlocking.countDown()
            holder.dispose()
        }
    }

    @Test
    fun testWakeRetryRunsInitialize() {
        val root = createTempDir().canonicalPath
        val fake = FakeDaemonClient()
        val holder = TestLspProcessHolder(mockProject(root), fake)
        LSPProcessHolder.BIN_PATH = "/tmp/refact"
        try {
            runOffEdt { holder.ensureStartedBlockingForTest("wake-init-start") }
            assertEquals(1, holder.initializeCalls.get())

            val recovered = runOffEdt { holder.wakeWorkerForRetry("wake-init") }

            assertTrue(recovered)
            assertEquals(LSPBackendConnectionStatus.READY, holder.backendConnectionStatus())
            assertEquals(2, holder.initializeCalls.get())
            assertEquals(2, fake.openProjectCalls.get())
        } finally {
            holder.dispose()
        }
    }

    @Test
    fun testWakeRetryRegistersCompatibleSharedBinaryWhenReusingDaemon() {
        val root = createTempDir().canonicalPath
        val fake = FakeDaemonClient()
        val holder = TestLspProcessHolder(mockProject(root), fake).apply {
            compatibleSharedBinaryPathResult = "/home/tester/.refact/bin/refact"
        }
        LSPProcessHolder.BIN_PATH = "/tmp/refact"
        try {
            runOffEdt { holder.ensureStartedBlockingForTest("wake-terminal-registration-start") }
            holder.terminalRegistrationPaths.clear()

            val recovered = runOffEdt { holder.wakeWorkerForRetry("wake-terminal-registration") }

            assertTrue(recovered)
            assertEquals(listOf("/home/tester/.refact/bin/refact"), holder.terminalRegistrationPaths)
        } finally {
            holder.dispose()
        }
    }

    @Test
    fun testWakeRetryWithMissingDaemonDoesNotProvisionOrPublishInstalling() {
        val root = createTempDir().canonicalPath
        val publisher = RecordingStatusPublisher()
        val fake = FakeDaemonClient()
        val holder = TestLspProcessHolder(mockProject(root, publisher), fake)
        LSPProcessHolder.BIN_PATH = "/tmp/refact-wake"
        try {
            runOffEdt { holder.ensureStartedBlockingForTest("wake-installing-start") }
            assertEquals(LSPBackendConnectionStatus.READY, holder.backendConnectionStatus())

            publisher.statuses.clear()
            holder.setBinaryResolutionResults()
            fake.statusError = RuntimeException("daemon missing")
            val recovered = runOffEdt { holder.wakeWorkerForRetry("wake-installing") }

            assertFalse(recovered)
            assertEquals(LSPBackendConnectionStatus.FAILED, holder.backendConnectionStatus())
            assertFalse(publisher.statuses.contains(LSPBackendConnectionStatus.INSTALLING))
            assertEquals(0, holder.binaryResolutionCalls.get())
            assertEquals(0, fake.ensureDaemonCalls.get())
            assertEquals(
                LSPConnectionIssue(LSPConnectionIssueCode.ENGINE_START_FAILED, stage = LSPStartupStage.DAEMON_START),
                holder.connectionIssue(),
            )
        } finally {
            holder.dispose()
        }
    }

    @Test
    fun testFailedWakeClearsAttachedWorkerState() {
        val root = createTempDir().canonicalPath
        val fake = FakeDaemonClient()
        val holder = TestLspProcessHolder(mockProject(root), fake)
        LSPProcessHolder.BIN_PATH = "/tmp/refact"
        try {
            runOffEdt { holder.ensureStartedBlockingForTest("wake-failure-start") }
            assertNotNull(holder.baseUrlOrNull())

            fake.openProjectError = RuntimeException("wake failed")
            val recovered = runOffEdt { holder.wakeWorkerForRetry("wake-failure") }

            assertFalse(recovered)
            assertEquals(LSPBackendConnectionStatus.FAILED, holder.backendConnectionStatus())
            assertFalse(holder.isWorking)
            assertNull(holder.baseUrlOrNull())
        } finally {
            holder.dispose()
        }
    }

    @Test
    fun testHttp504IsRecoverable() {
        assertTrue(isRecoverableHttpStatus(HttpStatusException(504, "timeout", URI("http://127.0.0.1"))))
        assertTrue(
            isRecoverableHttpStatus(
                DaemonHttpStatusException(504, "timeout", URI("http://127.0.0.1"), "POST")
            )
        )
        assertFalse(isRecoverableHttpStatus(HttpStatusException(499, "client closed", URI("http://127.0.0.1"))))
    }

    @Test
    fun testEnsureStartedAsyncRetriesAfterFailedStatus() {
        val root = createTempDir().canonicalPath
        val fake = FakeDaemonClient().apply {
            openProjectError = RuntimeException("first open failed")
        }
        val holder = TestLspProcessHolder(mockProject(root), fake)
        LSPProcessHolder.BIN_PATH = "/tmp/refact"
        try {
            runOffEdt { holder.ensureStartedBlockingForTest("backend-failed") }
            assertEquals(LSPBackendConnectionStatus.FAILED, holder.backendConnectionStatus())

            fake.openProjectError = null
            holder.advanceHealthClock(1_000)
            holder.blockEnsureStartedBlocking = true
            holder.ensureStartedAsync("failed-config-request")

            assertEquals(LSPBackendConnectionStatus.FAILED, holder.backendConnectionStatus())
            assertTrue(holder.ensureStartedBlockingEntered.await(2, TimeUnit.SECONDS))
            assertEquals(LSPBackendConnectionStatus.FAILED, holder.backendConnectionStatus())

            holder.releaseEnsureStartedBlocking.countDown()
            waitForLifecycle(holder)

            assertEquals(LSPBackendConnectionStatus.READY, holder.backendConnectionStatus())
            assertTrue(holder.backendReady())
            assertEquals(2, fake.openProjectCalls.get())
        } finally {
            holder.releaseEnsureStartedBlocking.countDown()
            holder.dispose()
        }
    }

    @Test
    fun testWorkspaceBusyHoldsRetriesAcrossEntrypointsAndClearsWhenReady() {
        val root = createTempDir().canonicalPath
        val fake = FakeDaemonClient().apply {
            openProjectError = DaemonWorkspaceBusyException(WorkspaceBusyConflict(holderPid = 4082485, holderHttpPort = 8005))
        }
        val holder = TestLspProcessHolder(mockProject(root), fake)
        LSPProcessHolder.BIN_PATH = "/tmp/refact"
        try {
            runOffEdt { holder.ensureStartedBlockingForTest("busy-start") }
            assertEquals(LSPBackendConnectionStatus.FAILED, holder.backendConnectionStatus())
            assertEquals(
                LSPConnectionIssue(LSPConnectionIssueCode.WORKSPACE_BUSY, holderPid = 4082485, holderHttpPort = 8005),
                holder.connectionIssue(),
            )

            repeat(20) {
                holder.ensureStartedAsync("editor-config-request")
                waitForLifecycle(holder)
                runOffEdt { holder.ensureStartedBlockingForTest("blocking-$it") }
                assertFalse(runOffEdt { holder.wakeWorkerForRetry("wake-$it") })
                holder.runHealthCheckOnceForTest()
                waitForLifecycle(holder)
            }
            holder.advanceHealthClock(4_999)
            holder.ensureStartedAsync("editor-config-request")
            holder.runHealthCheckOnceForTest()
            waitForLifecycle(holder)

            assertEquals(1, fake.openProjectCalls.get())
            assertEquals(0, fake.ensureDaemonCalls.get())
            assertEquals(LSPConnectionIssueCode.WORKSPACE_BUSY, holder.connectionIssue()?.code)
            assertEquals(8488, holder.daemonPort())

            holder.advanceHealthClock(1)
            holder.ensureStartedAsync("editor-config-request")
            waitForLifecycle(holder)
            assertEquals(2, fake.openProjectCalls.get())
            assertEquals(LSPConnectionIssueCode.WORKSPACE_BUSY, holder.connectionIssue()?.code)

            holder.advanceHealthClock(9_999)
            holder.ensureStartedAsync("editor-config-request")
            waitForLifecycle(holder)
            assertEquals(2, fake.openProjectCalls.get())

            fake.openProjectError = null
            holder.advanceHealthClock(1)
            holder.ensureStartedAsync("editor-config-request")
            waitForLifecycle(holder)
            assertEquals(3, fake.openProjectCalls.get())
            assertEquals(LSPBackendConnectionStatus.READY, holder.backendConnectionStatus())
            assertNull(holder.connectionIssue())
            assertEquals(0, fake.ensureDaemonCalls.get())
        } finally {
            holder.dispose()
        }
    }

    @Test
    fun testWorkspaceBusyBackoffIsBoundedAtSixtySeconds() {
        val root = createTempDir().canonicalPath
        val fake = FakeDaemonClient().apply {
            openProjectError = DaemonWorkspaceBusyException(WorkspaceBusyConflict())
        }
        val holder = TestLspProcessHolder(mockProject(root), fake)
        LSPProcessHolder.BIN_PATH = "/tmp/refact"
        try {
            runOffEdt { holder.ensureStartedBlockingForTest("busy-start") }
            listOf(5_000L, 10_000L, 20_000L, 40_000L, 60_000L, 60_000L).forEachIndexed { index, delay ->
                holder.advanceHealthClock(delay - 1)
                runOffEdt { holder.ensureStartedBlockingForTest("held-$index") }
                assertEquals(index + 1, fake.openProjectCalls.get())
                holder.advanceHealthClock(1)
                runOffEdt { holder.ensureStartedBlockingForTest("retry-$index") }
                assertEquals(index + 2, fake.openProjectCalls.get())
            }
            assertEquals(0, fake.ensureDaemonCalls.get())
        } finally {
            holder.dispose()
        }
    }

    @Test
    fun testGenericFailureReportsEngineStartFailedWithStageAndBacksOff() {
        val root = createTempDir().canonicalPath
        val fake = FakeDaemonClient().apply {
            openProjectError = DaemonHttpStatusException(409, "{\"error\":\"secret /root token\"}", URI("http://127.0.0.1"), "POST")
        }
        val holder = TestLspProcessHolder(mockProject(root), fake)
        LSPProcessHolder.BIN_PATH = "/tmp/refact"
        try {
            runOffEdt { holder.ensureStartedBlockingForTest("generic-start") }
            assertEquals(LSPBackendConnectionStatus.FAILED, holder.backendConnectionStatus())
            assertEquals(
                LSPConnectionIssue(LSPConnectionIssueCode.ENGINE_START_FAILED, stage = LSPStartupStage.PROJECT_OPEN),
                holder.connectionIssue(),
            )
            repeat(20) { holder.ensureStartedAsync("editor-config-request") }
            waitForLifecycle(holder)
            assertEquals(1, fake.openProjectCalls.get())

            fake.openProjectError = DaemonWorkspaceBusyException(WorkspaceBusyConflict(holderPid = 7))
            holder.advanceHealthClock(1_000)
            holder.ensureStartedAsync("editor-config-request")
            waitForLifecycle(holder)
            assertEquals(2, fake.openProjectCalls.get())
            assertEquals(LSPConnectionIssue(LSPConnectionIssueCode.WORKSPACE_BUSY, holderPid = 7), holder.connectionIssue())

            fake.openProjectError = RuntimeException("other failure")
            holder.settingsChanged("manual-retry")
            waitForLifecycle(holder)
            assertEquals(3, fake.openProjectCalls.get())
            assertEquals(LSPConnectionIssueCode.ENGINE_START_FAILED, holder.connectionIssue()?.code)
        } finally {
            holder.dispose()
        }
    }

    @Test
    fun testNullProjectRootReportsWorkspaceRequiredWithoutDaemonCalls() {
        val fake = FakeDaemonClient()
        val holder = TestLspProcessHolder(mockProject(null), fake)
        LSPProcessHolder.BIN_PATH = "/tmp/refact"
        try {
            runOffEdt { holder.ensureStartedBlockingForTest("no-root") }
            repeat(20) { holder.ensureStartedAsync("editor-config-request") }
            waitForLifecycle(holder)

            assertEquals(LSPBackendConnectionStatus.FAILED, holder.backendConnectionStatus())
            assertEquals(LSPConnectionIssue(LSPConnectionIssueCode.WORKSPACE_REQUIRED), holder.connectionIssue())
            assertEquals(0, fake.openProjectCalls.get())
            assertEquals(0, fake.statusCalls.get())
            assertEquals(0, fake.ensureDaemonCalls.get())
        } finally {
            holder.dispose()
        }
    }

    @Test
    fun testDaemonPortReportedFromStatusWhenProjectOpenFails() {
        val root = createTempDir().canonicalPath
        val fake = FakeDaemonClient().apply {
            port = 9123
            openProjectError = RuntimeException("open failed")
        }
        val holder = TestLspProcessHolder(mockProject(root), fake)
        LSPProcessHolder.BIN_PATH = "/tmp/refact"
        try {
            runOffEdt { holder.ensureStartedBlockingForTest("daemon-port") }
            assertNull(holder.baseUrlOrNull())
            assertEquals(9123, holder.daemonPort())
        } finally {
            holder.dispose()
        }
    }

}

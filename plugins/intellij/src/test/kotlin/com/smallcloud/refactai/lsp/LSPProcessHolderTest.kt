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
import com.smallcloud.refactai.io.InferenceGlobalContext.Companion.instance as InferenceGlobalContext
import mockwebserver3.MockResponse
import mockwebserver3.MockWebServer
import okhttp3.Protocol
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
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

class LSPProcessHolderTest : BasePlatformTestCase() {

    class FakeDaemonClient : RefactDaemonClient {
        val statusCalls = AtomicInteger(0)
        val ensureDaemonCalls = AtomicInteger(0)
        val openProjectCalls = AtomicInteger(0)
        val detachProjectCalls = AtomicInteger(0)
        val ensureDaemonPaths = Collections.synchronizedList(mutableListOf<String>())
        val detachedProjects = Collections.synchronizedList(mutableListOf<DaemonProject>())
        val openedSettings = Collections.synchronizedList(mutableListOf<LSPConfig>())
        val openProjectEntered = CountDownLatch(1)
        val releaseOpenProject = CountDownLatch(1)
        var blockOpenProject = false
        var port = 8488
        var projectId = "project-123"
        var baseUrlOverride: URI? = null
        var statusVersion = "8.1.0"
        var ensuredVersion = "8.1.0"
        var statusError: RuntimeException? = null

        override fun status(): DaemonStatus {
            statusCalls.incrementAndGet()
            statusError?.let { throw it }
            return DaemonStatus(version = statusVersion, port = port)
        }

        override fun ensureDaemon(binPath: String): DaemonStatus {
            ensureDaemonCalls.incrementAndGet()
            ensureDaemonPaths.add(binPath)
            statusError = null
            statusVersion = ensuredVersion
            return DaemonStatus(version = ensuredVersion, port = port)
        }

        override fun openProject(root: String, settings: LSPConfig): DaemonProject {
            openProjectCalls.incrementAndGet()
            openedSettings.add(settings)
            openProjectEntered.countDown()
            if (blockOpenProject) {
                releaseOpenProject.await(2, TimeUnit.SECONDS)
            }
            return DaemonProject(
                projectId,
                baseUrlOverride ?: URI("http://127.0.0.1:$port/p/$projectId/"),
                DaemonStatus(version = statusVersion, port = port),
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

        override val daemonClient: RefactDaemonClient
            get() = fakeDaemonClient

        override fun refreshAttachedWorkerState() {
            if (refreshWorkerState) super.refreshAttachedWorkerState()
        }

        override fun initializeAttachedProject() {}

        override fun requiredDaemonVersion(): String {
            return requiredVersion
        }

        override fun sleepBeforeWakeRetry(attempt: Int) {
            retryAttempts.add(attempt)
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

        fun ensureStartedBlockingForTest(reason: String) {
            ensureStartedBlocking(reason)
        }
    }

    private fun mockProject(root: String? = null): Project {
        val mockProject = mock(Project::class.java)
        val mockMessageBus = mock(MessageBus::class.java)
        val mockPublisher = mock(LSPProcessHolderChangedNotifier::class.java)
        `when`(mockProject.isDisposed).thenReturn(false)
        `when`(mockProject.messageBus).thenReturn(mockMessageBus)
        `when`(mockMessageBus.syncPublisher(LSPProcessHolderChangedNotifier.TOPIC)).thenReturn(mockPublisher)
        if (root != null) {
            `when`(mockProject.basePath).thenReturn(root)
        }
        return mockProject
    }

    private fun <T> runOffEdt(block: () -> T): T {
        return ApplicationManager.getApplication().executeOnPooledThread<T> { block() }.get(3, TimeUnit.SECONDS)
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
            assertEquals(URI("http://127.0.0.1:8488/p/project-123/"), holder.baseUrlOrNull())
        } finally {
            holder.dispose()
        }
    }

    @Test
    fun testStatusFailureFallsThroughToEnsureDaemon() {
        val root = createTempDir().canonicalPath
        val fake = FakeDaemonClient().apply { statusError = RuntimeException("daemon missing") }
        val holder = TestLspProcessHolder(mockProject(root), fake)
        LSPProcessHolder.BIN_PATH = "/tmp/refact-status-failure"
        try {
            runOffEdt { holder.ensureStartedBlockingForTest("missing-daemon") }

            assertEquals(1, fake.openProjectCalls.get())
            assertEquals(1, fake.ensureDaemonCalls.get())
            assertEquals(listOf("/tmp/refact-status-failure"), fake.ensureDaemonPaths)
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
    fun testBaseUrlUsesDaemonProjectProxy() {
        val root = createTempDir().canonicalPath
        val fake = FakeDaemonClient().apply {
            port = 9999
            projectId = "abc123"
        }
        val holder = TestLspProcessHolder(mockProject(root), fake)
        LSPProcessHolder.BIN_PATH = "/tmp/refact"
        try {
            runOffEdt { holder.ensureStartedBlockingForTest("base-url") }

            assertEquals(URI("http://127.0.0.1:9999/p/abc123/"), holder.baseUrlOrNull())
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
                        .body("worker build")
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

                assertEquals("worker build", LSPProcessHolder.buildInfo)
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
}

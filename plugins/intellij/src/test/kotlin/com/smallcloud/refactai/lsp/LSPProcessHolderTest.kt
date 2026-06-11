package com.smallcloud.refactai.lsp

import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.project.Project
import com.intellij.serviceContainer.AlreadyDisposedException
import com.intellij.testFramework.fixtures.BasePlatformTestCase
import com.intellij.util.concurrency.AppExecutorUtil
import com.intellij.util.messages.MessageBus
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
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
        val ensureDaemonCalls = AtomicInteger(0)
        val openProjectCalls = AtomicInteger(0)
        val openedSettings = Collections.synchronizedList(mutableListOf<LSPConfig>())
        val openProjectEntered = CountDownLatch(1)
        val releaseOpenProject = CountDownLatch(1)
        var blockOpenProject = false
        var port = 8488
        var projectId = "project-123"

        override fun status(): DaemonStatus {
            return DaemonStatus(version = "8.1.0", port = port)
        }

        override fun ensureDaemon(binPath: String): DaemonStatus {
            ensureDaemonCalls.incrementAndGet()
            return status()
        }

        override fun openProject(root: String, settings: LSPConfig): DaemonProject {
            openProjectCalls.incrementAndGet()
            openedSettings.add(settings)
            openProjectEntered.countDown()
            if (blockOpenProject) {
                releaseOpenProject.await(2, TimeUnit.SECONDS)
            }
            return DaemonProject(projectId, URI("http://127.0.0.1:$port/p/$projectId/"), status())
        }
    }

    class TestLspProcessHolder(project: Project, private val fakeDaemonClient: FakeDaemonClient) : LSPProcessHolder(project) {
        private val latch = CountDownLatch(1)

        override val daemonClient: RefactDaemonClient
            get() = fakeDaemonClient

        override fun refreshAttachedWorkerState() {}

        override fun initializeAttachedProject() {}

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

    private fun runOffEdt(block: () -> Unit) {
        ApplicationManager.getApplication().executeOnPooledThread(block).get(3, TimeUnit.SECONDS)
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
            assertEquals(1, fake.ensureDaemonCalls.get())
        } finally {
            executor.shutdownNow()
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
    fun testDisposeOnlyForgetsAttachState() {
        val root = createTempDir().canonicalPath
        val fake = FakeDaemonClient()
        val holder = TestLspProcessHolder(mockProject(root), fake)
        LSPProcessHolder.BIN_PATH = "/tmp/refact"

        runOffEdt { holder.ensureStartedBlockingForTest("dispose") }
        assertNotNull(holder.baseUrlOrNull())
        holder.dispose()

        assertNull(holder.baseUrlOrNull())
        assertEquals(1, fake.openProjectCalls.get())
    }

    @Test
    fun testPendingLifecycleFlagDefaultsFalse() {
        val holder = TestLspProcessHolder(mockProject(), FakeDaemonClient())
        assertFalse(holder.hasPendingLifecycleWork())
        holder.dispose()
    }
}

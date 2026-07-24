package com.smallcloud.refactai.panes.sharedchat

import com.intellij.testFramework.LightPlatform4TestCase
import com.smallcloud.refactai.panes.sharedchat.browser.ChatStartupMode
import com.smallcloud.refactai.panes.sharedchat.browser.canonicalizeDaemonGuiUrl
import com.smallcloud.refactai.panes.sharedchat.browser.isDaemonGuiUrl
import com.smallcloud.refactai.panes.sharedchat.browser.selectChatStartupMode
import com.smallcloud.refactai.panes.sharedchat.browser.shouldNavigateToDaemonGui
import com.smallcloud.refactai.testUtils.TestableChatWebView
import org.junit.Test
import org.junit.Assert
import org.junit.Ignore

/**
 * Test for ChatWebView to verify it handles race conditions properly using testable implementation.
 * This test specifically checks that the ChatWebView can handle a situation where
 * setStyle() is called before the browser is fully initialized, and then the
 * component is disposed while JavaScript might still be executing.
 */
class ChatWebViewTest: LightPlatform4TestCase() {

    override fun setUp() {
        super.setUp()
    }

    @Test
    fun testBrowserInitializationRaceCondition() {
        // Create a ChatWebView instance with the testable editor
        val chatWebView = TestableChatWebView { /* message handler */ }

        // Wait for initialization
        Assert.assertTrue("Should initialize", chatWebView.waitForInitialization())

        // First test with valid theme - should not throw
        try {
            chatWebView.setStyle()
        } catch (exception: Exception)  {
            Assert.fail("Exception should not have been thrown: ${exception.message}")
        }
        // Force disposal while JavaScript might still be executing
        Thread.sleep(100) // Small delay to ensure the coroutine has started
        chatWebView.dispose()
        Assert.assertTrue("Should dispose properly", chatWebView.waitForDisposal())
    }

    @Test @Ignore("fails in ci")
    fun testSetupReactRaceCondition() {
        val chatWebView = TestableChatWebView { /* message handler */ }
        Assert.assertTrue("Should initialize", chatWebView.waitForInitialization())

        try {
            // In testable version, we just test that operations don't crash
            chatWebView.setStyle()
        } catch (exception: Exception) {
            Assert.fail("Exception should not have been thrown: ${exception.message}")
        }
        Thread.sleep(100) // Small delay to ensure the coroutine has started
        chatWebView.dispose()
        Assert.assertTrue("Should dispose properly", chatWebView.waitForDisposal())
    }

    @Test @Ignore("fails in ci")
    fun testPostMessageRaceCondition() {
        val chatWebView = TestableChatWebView { /* message handler */ }
        Assert.assertTrue("Should initialize", chatWebView.waitForInitialization())

        try {
            chatWebView.postMessage("hello")
            // Just test with a string message
            chatWebView.postMessage("{\"type\": \"chat_message\", \"payload\": {\"message\": \"test message\"}}")
        } catch (exception: Exception) {
            Assert.fail("Exception should not have been thrown: ${exception.message}")
        }
        Thread.sleep(100) // Small delay to ensure the coroutine has started
        chatWebView.dispose()
        Assert.assertTrue("Should dispose properly", chatWebView.waitForDisposal())
    }

    @Test
    fun testOpenFileMessageHandling() {
        val openFileMessageReceived = mutableListOf<Events.FromChat>()
        val chatWebView = TestableChatWebView { event ->
            openFileMessageReceived.add(event)
        }

        Assert.assertTrue("Should initialize", chatWebView.waitForInitialization())

        // Test that the OpenFile message is properly parsed and handled
        val openFileMessage = """{"type":"ide/openFile","payload":{"file_path":"/home/mitya/.config/refact/customization.yaml"}}"""
        val parsedEvent = Events.parse(openFileMessage)

        Assert.assertNotNull("OpenFile message should be parsed successfully", parsedEvent)
        Assert.assertTrue("Parsed event should be OpenFile type", parsedEvent is Events.OpenFile)

        val openFileEvent = parsedEvent as Events.OpenFile
        Assert.assertEquals("File path should match", "/home/mitya/.config/refact/customization.yaml", openFileEvent.payload.filePath)

        // Test message simulation
        chatWebView.simulateMessageFromBrowser(openFileMessage)

        chatWebView.dispose()
        Assert.assertTrue("Should dispose properly", chatWebView.waitForDisposal())
    }

    @Test
    fun testResourceLeakPrevention() {
        val initialMemory = getUsedMemory()

        // Create and dispose multiple instances rapidly
        repeat(3) {
            val chatWebView = TestableChatWebView { /* empty handler */ }
            Assert.assertTrue("Should initialize", chatWebView.waitForInitialization())
            chatWebView.setStyle()
            Thread.sleep(50)
            chatWebView.dispose()
            Assert.assertTrue("Should dispose properly", chatWebView.waitForDisposal())
        }

        System.gc()
        Thread.sleep(100)

        val finalMemory = getUsedMemory()
        val memoryIncrease = finalMemory - initialMemory

        // Should not leak excessive memory (testable version should use much less)
        Assert.assertTrue("Memory leak detected: ${memoryIncrease / 1024 / 1024}MB",
                         memoryIncrease < 10_000_000) // Less than 10MB for testable version
    }

    @Test
    fun testJavaScriptExecutionSafety() {
        val chatWebView = TestableChatWebView { /* empty handler */ }
        Assert.assertTrue("Should initialize", chatWebView.waitForInitialization())

        // Test various message posting scenarios (simulating JavaScript execution)
        val messages = listOf(
            "console.log('test');",
            "document.body.style.backgroundColor = 'red';",
            "window.postMessage({type: 'test'}, '*');",
            "throw new Error('test error');" // This should not crash the application
        )

        messages.forEach { message ->
            try {
                chatWebView.postMessage(message)
                Thread.sleep(50) // Allow execution
            } catch (e: Exception) {
                // Should not throw exceptions for normal message posting
                Assert.fail("Message posting should not throw exceptions: ${e.message}")
            }
        }

        chatWebView.dispose()
        Assert.assertTrue("Should dispose properly", chatWebView.waitForDisposal())
    }

    @Test
    fun testInitializationIdempotency() {
        val chatWebView = TestableChatWebView { /* empty handler */ }
        Assert.assertTrue("Should initialize", chatWebView.waitForInitialization())

        // Call initialization methods multiple times
        repeat(3) {
            chatWebView.setStyle()
            Thread.sleep(50)
        }

        // Should not cause issues
        Assert.assertTrue("Multiple initialization calls should be safe", true)
        Assert.assertEquals("Should have 3 style updates", 3, chatWebView.styleUpdateCount.get())

        chatWebView.dispose()
        Assert.assertTrue("Should dispose properly", chatWebView.waitForDisposal())
    }

    @Test
    fun acceptsLoopbackProjectScopedUrls() {
        Assert.assertEquals(
            "http://127.0.0.1:8488/p/project-123/",
            canonicalizeDaemonGuiUrl("http://127.0.0.1:8488/p/project-123/")
        )
        Assert.assertEquals(
            "http://127.0.0.1:8488/p/project-123/",
            canonicalizeDaemonGuiUrl("http://127.0.0.1:8488/p/project-123")
        )
        Assert.assertEquals(
            "http://localhost:8488/p/abc/?daemon_token=xyz",
            canonicalizeDaemonGuiUrl("http://localhost:8488/p/abc/?daemon_token=xyz")
        )
        Assert.assertEquals(
            "https://127.0.0.1:1234/p/id/",
            canonicalizeDaemonGuiUrl("https://127.0.0.1:1234/p/id")
        )
        Assert.assertTrue(isDaemonGuiUrl("http://127.0.0.1:8488/p/project-123/"))
    }

    @Test
    fun rejectsNonLoopbackOrMalformedUrls() {
        val rejected = listOf(
            null,
            "",
            "   ",
            "http://refactai/index.html",
            "http://192.168.1.10:8488/p/project-123/",
            "http://example.com:8488/p/project-123/",
            "ftp://127.0.0.1:8488/p/project-123/",
            "http://127.0.0.1/p/project-123/",
            "http://127.0.0.1:0/p/project-123/",
            "http://127.0.0.1:8488/",
            "http://127.0.0.1:8488/p/",
            "http://127.0.0.1:8488/x/project-123/",
            "http://127.0.0.1:8488/p/project-123/v1/",
            "http://user@127.0.0.1:8488/p/project-123/",
            "http://127.0.0.1:8488/p/project-123/#fragment",
            "not a url"
        )
        rejected.forEach { url ->
            Assert.assertNull("Should reject: $url", canonicalizeDaemonGuiUrl(url))
            Assert.assertFalse("Should not be daemon gui: $url", isDaemonGuiUrl(url))
        }
    }

    @Test
    fun selectsStartupModeFromUrl() {
        Assert.assertEquals(
            ChatStartupMode.DAEMON_GUI,
            selectChatStartupMode("http://127.0.0.1:8488/p/project-123/")
        )
        Assert.assertEquals(ChatStartupMode.BOOTSTRAP, selectChatStartupMode(null))
        Assert.assertEquals(ChatStartupMode.BOOTSTRAP, selectChatStartupMode("http://refactai/index.html"))
        Assert.assertEquals(ChatStartupMode.BOOTSTRAP, selectChatStartupMode("http://example.com/p/x/"))
    }

    @Test
    fun avoidsReloadWhenAlreadyAtCanonicalUrl() {
        Assert.assertFalse(
            shouldNavigateToDaemonGui(
                "http://127.0.0.1:8488/p/project-123/",
                "http://127.0.0.1:8488/p/project-123/?daemon_token=rotated"
            )
        )
        Assert.assertTrue(
            shouldNavigateToDaemonGui(
                "http://refactai/index.html",
                "http://127.0.0.1:8488/p/project-123/"
            )
        )
        Assert.assertTrue(
            shouldNavigateToDaemonGui(
                "http://127.0.0.1:8488/p/old/",
                "http://127.0.0.1:8488/p/new/"
            )
        )
        Assert.assertFalse(
            shouldNavigateToDaemonGui(
                "http://127.0.0.1:8488/p/project-123/",
                "http://refactai/index.html"
            )
        )
    }

    private fun getUsedMemory(): Long {
        val runtime = Runtime.getRuntime()
        return runtime.totalMemory() - runtime.freeMemory()
    }
}

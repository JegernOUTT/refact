package com.smallcloud.refactai.panes.sharedchat

import com.intellij.testFramework.LightPlatform4TestCase
import com.smallcloud.refactai.panes.sharedchat.browser.ChatStartupMode
import com.smallcloud.refactai.panes.sharedchat.browser.canonicalizeDaemonGuiUrl
import com.smallcloud.refactai.panes.sharedchat.browser.isDaemonGuiUrl
import com.smallcloud.refactai.panes.sharedchat.browser.selectChatStartupMode
import com.smallcloud.refactai.panes.sharedchat.browser.shouldNavigateToDaemonGui
import org.junit.Test
import org.junit.Assert

/** Tests daemon URL validation and navigation policy used by ChatWebView. */
class ChatWebViewTest: LightPlatform4TestCase() {

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
}

package com.smallcloud.refactai.panes.sharedchat

import org.junit.Assert.assertEquals
import org.junit.Test

class SharedChatPaneReadyTest {

    @Test
    fun onChatReadySyncSendsCurrentConfigBeforeQueueFlush() {
        val calls = mutableListOf<String>()
        val config = Events.Config.Update(
            Events.Config.UpdatePayload(
                features = Events.Config.Features(true, true),
                themeProps = Events.Config.ThemeProps("dark"),
                lspPort = 8488,
                keyBindings = Events.Config.KeyBindings(""),
                lspUrl = "http://127.0.0.1:8488/p/project-123/",
                backendReady = true,
                connectionStatus = "ready",
            )
        )
        var sentConfig: Events.Config.Update? = null

        onChatReadySync(
            config = config,
            sendConfig = {
                sentConfig = it
                calls.add("config")
            },
            flushSelection = { calls.add("selection") },
            sendProjectInfo = { calls.add("project") },
            flushQueue = { calls.add("queue") },
        )

        assertEquals(listOf("config", "selection", "project", "queue"), calls)
        assertEquals(config, sentConfig)
        assertEquals(
            true,
            Events.stringify(sentConfig!!).contains(
                "\"lspUrl\":\"http://127.0.0.1:8488/p/project-123/\""
            )
        )
    }
}

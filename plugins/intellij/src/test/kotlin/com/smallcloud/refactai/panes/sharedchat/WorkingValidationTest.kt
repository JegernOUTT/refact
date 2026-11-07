package com.smallcloud.refactai.panes.sharedchat

import com.intellij.testFramework.LightPlatform4TestCase
import com.smallcloud.refactai.utils.AsyncMessageHandler
import org.junit.Test

/** Tests the production message-handler lifecycle without JCEF infrastructure. */
class WorkingValidationTest : LightPlatform4TestCase() {
    @Test
    fun testResourceManagerComponents() {
        // Test AsyncMessageHandler
        val handler1 = AsyncMessageHandler<String>({ it }, { })
        assertFalse("Handler should not be disposed initially", handler1.isDisposed())
        assertEquals("Should have empty queue", 0, handler1.getQueueSize())

        handler1.dispose()
        assertTrue("Handler should be disposed", handler1.isDisposed())

        // Test that we can create multiple instances
        val handler2 = AsyncMessageHandler<String>({ it }, { })
        assertFalse("New handler should not be disposed", handler2.isDisposed())
        handler2.dispose()
    }
}

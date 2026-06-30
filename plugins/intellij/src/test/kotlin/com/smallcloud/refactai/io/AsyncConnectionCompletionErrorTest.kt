@file:OptIn(okhttp3.ExperimentalOkHttpApi::class)

package com.smallcloud.refactai.io

import com.intellij.testFramework.fixtures.BasePlatformTestCase
import mockwebserver3.MockResponse
import mockwebserver3.MockWebServer
import okhttp3.Protocol
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import java.net.URI
import com.smallcloud.refactai.io.InferenceGlobalContext.Companion.instance as InferenceGlobalContext

class AsyncConnectionCompletionErrorTest : BasePlatformTestCase() {
    @Test
    fun testFailedCompletionHttpRequestPublishesVisibleErrorState() {
        val previousConnection = InferenceGlobalContext.connection
        val previousStatus = InferenceGlobalContext.status
        val previousError = InferenceGlobalContext.lastErrorMsg
        val connection = AsyncConnection()
        val server = MockWebServer()
        InferenceGlobalContext.connection = connection
        InferenceGlobalContext.status = ConnectionStatus.PENDING
        InferenceGlobalContext.lastErrorMsg = null
        try {
            server.protocols = listOf(Protocol.H2_PRIOR_KNOWLEDGE)
            server.start()
            server.enqueue(
                MockResponse.Builder()
                    .code(503)
                    .addHeader("Content-Type", "application/json")
                    .body("completion unavailable")
                    .build()
            )

            val future = connection.post(URI(server.url("/v1/code-completion").toString()), "{}").get()

            try {
                future.get()
                fail("completion HTTP failure should fail the request")
            } catch (_: Exception) {
            }
            assertEquals(ConnectionStatus.ERROR, InferenceGlobalContext.status)
            val message = InferenceGlobalContext.lastErrorMsg.orEmpty()
            assertTrue(message.contains("HTTP 503"))
            assertTrue(message.contains("completion unavailable"))
        } finally {
            server.shutdown()
            InferenceGlobalContext.connection = previousConnection
            InferenceGlobalContext.status = previousStatus
            InferenceGlobalContext.lastErrorMsg = previousError
            connection.dispose()
        }
    }
}

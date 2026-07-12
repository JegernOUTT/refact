package com.smallcloud.refactai.panes.sharedchat.browser

import com.intellij.openapi.diagnostic.Logger
import com.intellij.openapi.project.DumbAware
import com.smallcloud.refactai.utils.ResourceCache
import org.cef.browser.CefBrowser
import org.cef.browser.CefFrame
import org.cef.callback.CefCallback
import org.cef.callback.CefSchemeHandlerFactory
import org.cef.handler.CefLoadHandler
import org.cef.handler.CefResourceHandler
import org.cef.misc.IntRef
import org.cef.misc.StringRef
import org.cef.network.CefRequest
import org.cef.network.CefResponse
import java.io.IOException
import java.io.InputStream
import java.lang.reflect.InvocationHandler
import java.lang.reflect.Method
import java.lang.reflect.Proxy
import java.net.URI
import java.net.URLConnection

class RequestHandlerFactory : CefSchemeHandlerFactory {
    override fun create(
        browser: CefBrowser?,
        frame: CefFrame?,
        schemeName: String,
        request: CefRequest
    ): CefResourceHandler {
        return createResourceHandlerProxy(RefactChatResourceHandler())
    }
}

private fun createResourceHandlerProxy(handler: RefactChatResourceHandler): CefResourceHandler {
    return Proxy.newProxyInstance(
        CefResourceHandler::class.java.classLoader,
        arrayOf(CefResourceHandler::class.java),
        CefResourceHandlerInvocation(handler)
    ) as CefResourceHandler
}

private class CefResourceHandlerInvocation(
    private val handler: RefactChatResourceHandler
) : InvocationHandler {
    override fun invoke(proxy: Any, method: Method, args: Array<out Any?>?): Any? {
        val actualArgs = args ?: emptyArray()
        return when (method.name) {
            "processRequest" -> handler.handleRequest(actualArgs[0] as CefRequest, actualArgs[1] as CefCallback)
            "open" -> {
                val handled = handler.handleRequest(actualArgs[0] as CefRequest, actualArgs[2] as CefCallback)
                setBooleanRef(actualArgs[1], handled)
                handled
            }
            "getResponseHeaders" -> {
                handler.getResponseHeaders(actualArgs[0] as CefResponse, actualArgs[1] as IntRef, actualArgs[2] as StringRef)
                null
            }
            "readResponse", "read" -> handler.read(actualArgs[0] as ByteArray, actualArgs[1] as Int, actualArgs[2] as IntRef)
            "skip" -> {
                val skipped = handler.skip(actualArgs[0] as Long)
                setLongRef(actualArgs[1], skipped)
                skipped >= 0
            }
            "cancel" -> {
                handler.cancel()
                null
            }
            "toString" -> "RefactChatResourceHandlerProxy($handler)"
            "hashCode" -> System.identityHashCode(proxy)
            "equals" -> proxy === actualArgs.getOrNull(0)
            else -> method.invoke(handler, *actualArgs)
        }
    }

    private fun setBooleanRef(ref: Any?, value: Boolean) {
        ref?.javaClass?.getMethod("set", Boolean::class.javaPrimitiveType)?.invoke(ref, value)
    }

    private fun setLongRef(ref: Any?, value: Long) {
        ref?.javaClass?.getMethod("set", java.lang.Long.TYPE)?.invoke(ref, value)
    }
}

data object ClosedConnection : ResourceHandlerState() {
    override fun getResponseHeaders(
        cefResponse: CefResponse,
        responseLength: IntRef,
        redirectUrl: StringRef
    ) {
        cefResponse.status = 404
    }
}

sealed class ResourceHandlerState {
    open fun getResponseHeaders(
        cefResponse: CefResponse,
        responseLength: IntRef,
        redirectUrl: StringRef
    ) {
    }

    open fun read(
        dataOut: ByteArray,
        bytesToRead: Int,
        bytesRead: IntRef
    ): Boolean = false

    open fun skip(bytesToSkip: Long): Long = -2L

    open fun close() {}
}

class OpenedConnection(private val connection: URLConnection?) :
    ResourceHandlerState() {

    private val inputStream: InputStream? by lazy {
        connection?.inputStream
    }

    override fun getResponseHeaders(
        cefResponse: CefResponse,
        responseLength: IntRef,
        redirectUrl: StringRef
    ) {
        try {
            if (connection != null) {
                val url = connection.url.toString()
                when {
                    url.contains(".css") -> cefResponse.mimeType = "text/css"
                    url.contains(".js") -> cefResponse.mimeType = "text/javascript"
                    url.contains(".html") -> cefResponse.mimeType = "text/html"
                    else -> cefResponse.mimeType = connection.contentType
                }
                val contentLength = connection.contentLength
                responseLength.set(if (contentLength >= 0) contentLength else -1)
                cefResponse.status = 200
            } else {
                cefResponse.error = CefLoadHandler.ErrorCode.ERR_FAILED
                cefResponse.statusText = "Connection is null"
                cefResponse.status = 500
            }
        } catch (e: IOException) {
            cefResponse.error = CefLoadHandler.ErrorCode.ERR_FILE_NOT_FOUND
            cefResponse.statusText = e.localizedMessage
            cefResponse.status = 404
        }
    }

    override fun read(
        dataOut: ByteArray,
        bytesToRead: Int,
        bytesRead: IntRef
    ): Boolean {
        return readFromStream(inputStream, dataOut, bytesToRead, bytesRead)
    }

    override fun skip(bytesToSkip: Long): Long {
        return skipStream(inputStream, bytesToSkip)
    }

    override fun close() {
        inputStream?.close()
    }
}

class CachedResourceState(private val cached: ResourceCache.CachedResource, private val url: String) :
    ResourceHandlerState() {
    private val logger = Logger.getInstance(CachedResourceState::class.java)
    private val inputStream = cached.createInputStream()

    override fun getResponseHeaders(
        cefResponse: CefResponse,
        responseLength: IntRef,
        redirectUrl: StringRef
    ) {
        cefResponse.mimeType = cached.mimeType
        responseLength.set(cached.data.size)
        cefResponse.status = 200
        logger.debug("Serving cached $url (${cached.data.size} bytes)")
    }

    override fun read(
        dataOut: ByteArray,
        bytesToRead: Int,
        bytesRead: IntRef
    ): Boolean {
        return try {
            readFromStream(inputStream, dataOut, bytesToRead, bytesRead)
        } catch (e: Exception) {
            logger.warn("Failed to read from cached stream", e)
            try { inputStream.close() } catch (_: Exception) {}
            false
        }
    }

    override fun skip(bytesToSkip: Long): Long {
        return skipStream(inputStream, bytesToSkip)
    }

    override fun close() {
        inputStream.close()
    }
}

class OpenedStream(private val inputStream: InputStream, private val url: String) :
    ResourceHandlerState() {
    private val logger = Logger.getInstance(OpenedStream::class.java)

    override fun getResponseHeaders(
        cefResponse: CefResponse,
        responseLength: IntRef,
        redirectUrl: StringRef
    ) {
        try {
            cefResponse.mimeType = when {
                url.endsWith(".css") -> "text/css"
                url.endsWith(".js") || url.endsWith(".cjs") -> "text/javascript"
                url.endsWith(".html") -> "text/html"
                url.endsWith(".json") -> "application/json"
                else -> URLConnection.guessContentTypeFromName(url) ?: "application/octet-stream"
            }
            responseLength.set(-1)
            cefResponse.status = 200
            logger.debug("Serving $url with MIME ${cefResponse.mimeType}")
        } catch (e: Exception) {
            logger.warn("Failed to set headers for $url", e)
            cefResponse.status = 500
        }
    }

    override fun read(
        dataOut: ByteArray,
        bytesToRead: Int,
        bytesRead: IntRef
    ): Boolean {
        return try {
            readFromStream(inputStream, dataOut, bytesToRead, bytesRead)
        } catch (e: Exception) {
            logger.warn("Failed to read from stream", e)
            try { inputStream.close() } catch (_: Exception) {}
            false
        }
    }

    override fun skip(bytesToSkip: Long): Long {
        return skipStream(inputStream, bytesToSkip)
    }

    override fun close() {
        try {
            inputStream.close()
        } catch (e: Exception) {
            logger.warn("Failed to close stream", e)
        }
    }
}

private fun readFromStream(
    inputStream: InputStream?,
    dataOut: ByteArray,
    bytesToRead: Int,
    bytesRead: IntRef
): Boolean {
    val stream = inputStream ?: return false
    val read = stream.read(dataOut, 0, bytesToRead)
    return if (read > 0) {
        bytesRead.set(read)
        true
    } else {
        bytesRead.set(0)
        stream.close()
        false
    }
}

private fun skipStream(inputStream: InputStream?, bytesToSkip: Long): Long {
    val stream = inputStream ?: return -2L
    return try {
        stream.skip(bytesToSkip)
    } catch (_: Exception) {
        -2L
    }
}

class RefactChatResourceHandler : DumbAware {
    private val logger = Logger.getInstance(RefactChatResourceHandler::class.java)
    private var state: ResourceHandlerState = ClosedConnection
    private var currentUrl: String? = null

    fun handleRequest(
        cefRequest: CefRequest,
        cefCallback: CefCallback
    ): Boolean {
        val url = cefRequest.url ?: run {
            logger.warn("Request URL is null")
            return false
        }

        if (!url.startsWith("http://refactai/")) {
            state = ClosedConnection
            currentUrl = url
            cefCallback.Continue()
            return true
        }

        val path = try {
            val uri = URI(url)
            uri.path?.removePrefix("/") ?: ""
        } catch (_: Exception) {
            state = ClosedConnection
            currentUrl = url
            cefCallback.Continue()
            return true
        }

        if (path.isBlank() || path.startsWith("/") || path.contains("..") || path.contains("\\")) {
            state = ClosedConnection
            currentUrl = url
            cefCallback.Continue()
            return true
        }

        val resourcePath = "webview/$path"

        val cached = ResourceCache.getOrLoad(resourcePath) {
            javaClass.classLoader.getResourceAsStream(resourcePath)
        }

        state = if (cached != null) {
            CachedResourceState(cached, url)
        } else {
            val fallbackUrl = javaClass.classLoader.getResource(resourcePath)
            if (fallbackUrl != null) {
                OpenedConnection(fallbackUrl.openConnection())
            } else {
                logger.debug("Resource not found: $resourcePath")
                ClosedConnection
            }
        }

        currentUrl = url
        cefCallback.Continue()
        return true
    }

    fun getResponseHeaders(
        cefResponse: CefResponse,
        responseLength: IntRef,
        redirectUrl: StringRef
    ) {
        if (currentUrl != null) {
            cefResponse.mimeType = when {
                currentUrl!!.endsWith(".css") -> "text/css"
                currentUrl!!.endsWith(".js") || currentUrl!!.endsWith(".cjs") -> "text/javascript"
                currentUrl!!.endsWith(".html") -> "text/html"
                currentUrl!!.endsWith(".json") -> "application/json"
                else -> "application/octet-stream"
            }
        }
        state.getResponseHeaders(cefResponse, responseLength, redirectUrl)
    }

    fun read(
        dataOut: ByteArray,
        bytesToRead: Int,
        bytesRead: IntRef
    ): Boolean {
        return state.read(dataOut, bytesToRead, bytesRead)
    }

    fun skip(bytesToSkip: Long): Long {
        return state.skip(bytesToSkip)
    }

    fun cancel() {
        state.close()
        state = ClosedConnection
    }
}

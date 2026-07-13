package com.smallcloud.refactai.utils

import com.intellij.ui.jcef.JBCefApp

/**
 * NoClassDefFoundError-safe JCEF availability probe.
 *
 * Since 2025.3.1 / 2026.x the JCEF API classes live behind the
 * `com.intellij.modules.jcef` plugin alias instead of the core classpath, and
 * IDEs without JCEF (some remote-dev/headless setups) do not expose the classes
 * at all. Referencing [JBCefApp] from startup code therefore must tolerate the
 * class being absent: the reference below is only resolved when [isAvailable]
 * is first invoked, and any linkage error is treated as "JCEF unavailable".
 */
object JcefSupport {
    @Volatile
    private var cached: Boolean? = null

    @JvmStatic
    fun isAvailable(): Boolean {
        cached?.let { return it }
        val value = try {
            JBCefApp.isSupported()
        } catch (_: Throwable) {
            false
        }
        cached = value
        return value
    }
}

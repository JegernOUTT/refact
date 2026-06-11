package com.smallcloud.refactai.lsp

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class RefactDaemonClientTest {
    @Test
    fun versionComparisonDetectsOlderDaemon() {
        assertTrue(versionIsOlder("8.1.0", "8.2.0"))
        assertTrue(versionIsOlder("8.1.0-alpha", "8.1.1"))
        assertFalse(versionIsOlder("8.2.0", "8.1.0"))
        assertFalse(versionIsOlder("", "8.1.0"))
    }
}

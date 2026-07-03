package com.smallcloud.refactai.status_bar

import com.smallcloud.refactai.lsp.AstStatus
import com.smallcloud.refactai.lsp.CodeGraphCounts
import com.smallcloud.refactai.lsp.CodeGraphStatus
import com.smallcloud.refactai.lsp.RagStatus
import com.smallcloud.refactai.lsp.VecDbStatus
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test

class StatusBarWidgetTest {
    @Test
    fun warningsAreDerivedFromLatestPayload() {
        val errorState = statusBarStateFromRagStatus(
            ragStatus(vecDbError = "vecdb failed", codegraphError = "codegraph failed"),
            codegraphFileLimit = 100
        )

        assertEquals("vecdb failed", errorState.vecdbWarning)
        assertEquals("codegraph failed", errorState.codegraphWarning)

        val healthy = ragStatus()
        val healthyState = statusBarStateFromRagStatus(healthy, codegraphFileLimit = 100)

        assertEquals("", healthyState.vecdbWarning)
        assertEquals("", healthyState.codegraphWarning)
        assertSame(healthy, healthyState.lastRagStatus)
    }

    @Test
    fun newWarningsReplacePreviousWarningText() {
        val state = statusBarStateFromRagStatus(
            ragStatus(vecDbError = "new vecdb warning", codegraphError = "new codegraph warning"),
            codegraphFileLimit = 100
        )

        assertEquals("new vecdb warning", state.vecdbWarning)
        assertEquals("new codegraph warning", state.codegraphWarning)
    }

    @Test
    fun limitFlagsAreDerivedFromLatestPayload() {
        val limitState = statusBarStateFromRagStatus(
            ragStatus(
                astMaxFilesHit = true,
                vecdbMaxFilesHit = true,
                codegraphFiles = 100,
            ),
            codegraphFileLimit = 100
        )

        assertTrue(limitState.astLimitHit)
        assertTrue(limitState.vecdbLimitHit)
        assertTrue(limitState.codegraphLimitHit)

        val healthyState = statusBarStateFromRagStatus(ragStatus(), codegraphFileLimit = 100)

        assertFalse(healthyState.astLimitHit)
        assertFalse(healthyState.vecdbLimitHit)
        assertFalse(healthyState.codegraphLimitHit)
    }

    private fun ragStatus(
        vecDbError: String = "",
        codegraphError: String? = null,
        astMaxFilesHit: Boolean = false,
        vecdbMaxFilesHit: Boolean = false,
        codegraphFiles: Long = 10,
    ): RagStatus {
        return RagStatus(
            ast = AstStatus(
                filesUnparsed = 0,
                filesTotal = 10,
                astIndexFilesTotal = 10,
                astIndexSymbolsTotal = 20,
                state = "idle",
                astMaxFilesHit = astMaxFilesHit
            ),
            vecdb = VecDbStatus(
                filesUnprocessed = 0,
                filesTotal = 10,
                requestsMadeSinceStart = 0,
                vectorsMadeSinceStart = 0,
                dbSize = 1,
                dbCacheSize = 1,
                state = "idle",
                vecdbMaxFilesHit = vecdbMaxFilesHit
            ),
            vecDbError = vecDbError,
            codegraph = CodeGraphStatus(
                counts = CodeGraphCounts(
                    nodes = 1,
                    edges = 2,
                    files = codegraphFiles,
                    ftsDocs = 3
                ),
                queued = 0,
                state = "working",
                error = ""
            ),
            codegraphError = codegraphError
        )
    }
}

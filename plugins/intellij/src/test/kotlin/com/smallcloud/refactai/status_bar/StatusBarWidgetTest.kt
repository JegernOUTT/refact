package com.smallcloud.refactai.status_bar

import com.google.gson.Gson
import com.smallcloud.refactai.lsp.AstStatus
import com.smallcloud.refactai.lsp.CodeGraphCounts
import com.smallcloud.refactai.lsp.CodeGraphStatus
import com.smallcloud.refactai.lsp.RagStatus
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test

class StatusBarWidgetTest {
    @Test
    fun warningsAreDerivedFromLatestPayload() {
        val errorState = statusBarStateFromRagStatus(
            ragStatus(codegraphError = "codegraph failed"),
            codegraphFileLimit = 100
        )

        assertEquals("codegraph failed", errorState.codegraphWarning)

        val healthy = ragStatus()
        val healthyState = statusBarStateFromRagStatus(healthy, codegraphFileLimit = 100)

        assertEquals("", healthyState.codegraphWarning)
        assertSame(healthy, healthyState.lastRagStatus)
    }

    @Test
    fun limitFlagsAreDerivedFromLatestPayload() {
        val limitState = statusBarStateFromRagStatus(
            ragStatus(
                astMaxFilesHit = true,
                codegraphFiles = 100,
            ),
            codegraphFileLimit = 100
        )

        assertTrue(limitState.astLimitHit)
        assertTrue(limitState.codegraphLimitHit)

        val healthyState = statusBarStateFromRagStatus(ragStatus(), codegraphFileLimit = 100)

        assertFalse(healthyState.astLimitHit)
        assertFalse(healthyState.codegraphLimitHit)
    }

    @Test
    fun legacyVecdbErrorNeverProducesAWarning() {
        val json = """{"ast":null,"ast_alive":"done","vecdb":{"files_unprocessed":3,"files_total":10,""" +
            """"requests_made_since_start":0,"vectors_made_since_start":0,"db_size":0,"db_cache_size":0,""" +
            """"state":"parsing","vecdb_max_files_hit":true},"vecdb_alive":false,""" +
            """"vec_db_error":"embedding model name or endpoint are empty",""" +
            """"codegraph":null,"codegraph_alive":"done","codegraph_error":""}"""
        val ragStatus = Gson().fromJson(json, RagStatus::class.java)

        val state = statusBarStateFromRagStatus(ragStatus, codegraphFileLimit = 100)

        assertEquals(StatusBarState(lastRagStatus = ragStatus), state)
    }

    @Test
    fun ragStatusParsesWithInertLegacyVecdbFields() {
        val json = """{"ast":{"files_unparsed":0,"files_total":2,"ast_index_files_total":2,""" +
            """"ast_index_symbols_total":5,"state":"done","ast_max_files_hit":false},"ast_alive":"done",""" +
            """"vecdb":null,"vecdb_alive":false,"vec_db_error":"",""" +
            """"codegraph":{"counts":{"nodes":1,"edges":2,"files":3,"fts_docs":4},"queued":0,"state":"idle","error":""},""" +
            """"codegraph_alive":"done","codegraph_error":""}"""

        val ragStatus = Gson().fromJson(json, RagStatus::class.java)

        assertEquals(5, ragStatus.ast!!.astIndexSymbolsTotal)
        assertEquals(3L, ragStatus.codegraph!!.counts.files)
        assertEquals("", ragStatus.codegraphError)
    }

    @Test
    fun ragStatusParsesWithoutLegacyVecdbFields() {
        val json = """{"ast":null,"ast_alive":"done","codegraph":null,"codegraph_alive":"done","codegraph_error":""}"""

        val ragStatus = Gson().fromJson(json, RagStatus::class.java)

        assertNotNull(ragStatus)
        val state = statusBarStateFromRagStatus(ragStatus, codegraphFileLimit = 100)
        assertEquals(StatusBarState(lastRagStatus = ragStatus), state)
    }

    private fun ragStatus(
        codegraphError: String? = null,
        astMaxFilesHit: Boolean = false,
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

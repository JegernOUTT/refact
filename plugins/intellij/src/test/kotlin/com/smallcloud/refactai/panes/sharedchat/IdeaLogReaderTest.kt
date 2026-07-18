package com.smallcloud.refactai.panes.sharedchat

import kotlin.test.Test
import org.junit.Assert.*

class IdeaLogReaderTest {

    @Test
    fun keepsRefactCategoryLineAndMapsFields() {
        val result = IdeaLogReader.parseAndFilter(
            listOf("2026-07-18 11:20:33,123 [1234567]   WARN - #c.s.r.l.LSPProcessHolder - engine restarted"),
            10
        )

        assertEquals(1, result.size)
        assertEquals("warn", result[0].level)
        assertEquals("#c.s.r.l.LSPProcessHolder - engine restarted", result[0].message)
        assertNotNull(result[0].at)
    }

    @Test
    fun dropsNonRefactCategories() {
        val result = IdeaLogReader.parseAndFilter(
            listOf("2026-07-18 11:20:34,000 [1234568]   INFO - #c.i.openapi.SomethingElse - noise"),
            10
        )

        assertEquals(emptyList<IdeLogLine>(), result)
    }

    @Test
    fun attachesContinuationsAfterKeptLineUntilNextHeader() {
        val result = IdeaLogReader.parseAndFilter(
            listOf(
                "2026-07-18 11:20:35,000 [1234569]   ERROR - #c.s.r.l.LSPProcessHolder - failed",
                "java.lang.RuntimeException: boom",
                "\tat com.smallcloud.refactai.Foo.bar(Foo.kt:10)",
                "2026-07-18 11:20:36,000 [1234570]   INFO - #c.i.openapi.SomethingElse - noise",
                "ignored continuation"
            ),
            10
        )

        assertEquals(3, result.size)
        assertEquals("error", result[0].level)
        assertNotNull(result[0].at)
        assertEquals("#c.s.r.l.LSPProcessHolder - failed", result[0].message)
        assertEquals("error", result[1].level)
        assertNull(result[1].at)
        assertEquals("java.lang.RuntimeException: boom", result[1].message)
        assertEquals("error", result[2].level)
        assertNull(result[2].at)
        assertEquals("\tat com.smallcloud.refactai.Foo.bar(Foo.kt:10)", result[2].message)
    }

    @Test
    fun mapsLevels() {
        val result = IdeaLogReader.parseAndFilter(
            listOf(
                "2026-07-18 11:20:37,000 [1234571]   SEVERE - #c.s.r.l.LSPProcessHolder - severe message",
                "2026-07-18 11:20:38,000 [1234572]   TRACE - #c.s.r.l.LSPProcessHolder - trace message",
                "2026-07-18 11:20:39,000 [1234573]   INFO - #c.s.r.l.LSPProcessHolder - info message"
            ),
            10
        )

        assertEquals(listOf("error", "debug", "info"), result.map { it.level })
    }

    @Test
    fun returnsLastLimitedLinesInOriginalOrder() {
        val lines = (1..10).map {
            "2026-07-18 11:20:${it.toString().padStart(2, '0')},000 [12345$it]   INFO - #c.s.r.l.LSPProcessHolder - message $it"
        }

        val result = IdeaLogReader.parseAndFilter(lines, 3)

        assertEquals(3, result.size)
        assertEquals(listOf("#c.s.r.l.LSPProcessHolder - message 8", "#c.s.r.l.LSPProcessHolder - message 9", "#c.s.r.l.LSPProcessHolder - message 10"), result.map { it.message })
    }

    @Test
    fun keepsFullNameRefactCategory() {
        val result = IdeaLogReader.parseAndFilter(
            listOf("2026-07-18 11:20:40,000 [1234574]   INFO - com.smallcloud.refactai.lsp.LSPProcessHolder - full category"),
            10
        )

        assertEquals(1, result.size)
        assertEquals("info", result[0].level)
        assertEquals("com.smallcloud.refactai.lsp.LSPProcessHolder - full category", result[0].message)
    }
}

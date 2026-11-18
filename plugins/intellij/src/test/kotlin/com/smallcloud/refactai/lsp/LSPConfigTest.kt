package com.smallcloud.refactai.lsp

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test

class LSPConfigTest {
    @Test
    fun openProjectSettingsUseDaemonShape() {
        val settings = LSPConfig(
            ast = true,
            astFileLimit = 123,
            codegraph = true,
            codegraphFileLimit = 789,
        ).toOpenProjectSettings()

        assertEquals(
            setOf("ast", "codegraph", "ast_max_files", "codegraph_max_files"),
            settings.keys
        )
        assertEquals(true, settings["ast"])
        assertEquals(true, settings["codegraph"])
        assertEquals(123, settings["ast_max_files"])
        assertEquals(789, settings["codegraph_max_files"])
        assertFalse(settings.containsKey("port"))
    }

    @Test
    fun openProjectSettingsNeverSendVecdbKeys() {
        val settings = LSPConfig().toOpenProjectSettings()

        assertFalse(settings.containsKey("vecdb"))
        assertFalse(settings.containsKey("vecdb_max_files"))
    }

    @Test
    fun openProjectSettingsFillDefaults() {
        val settings = LSPConfig(astFileLimit = null, codegraphFileLimit = null).toOpenProjectSettings()

        assertEquals(LSPConfig.DEFAULT_AST_MAX_FILES, settings["ast_max_files"])
        assertEquals(LSPConfig.DEFAULT_CODEGRAPH_MAX_FILES, settings["codegraph_max_files"])
    }
}

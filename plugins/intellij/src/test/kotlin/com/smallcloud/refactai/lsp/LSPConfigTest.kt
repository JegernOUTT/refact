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
            vecdb = false,
            vecdbFileLimit = 456,
            codegraph = true,
            codegraphFileLimit = 789,
        ).toOpenProjectSettings()

        assertEquals(true, settings["ast"])
        assertEquals(false, settings["vecdb"])
        assertEquals(true, settings["codegraph"])
        assertEquals(123, settings["ast_max_files"])
        assertEquals(456, settings["vecdb_max_files"])
        assertEquals(789, settings["codegraph_max_files"])
        assertFalse(settings.containsKey("port"))
    }

    @Test
    fun openProjectSettingsFillDefaults() {
        val settings = LSPConfig(astFileLimit = null, vecdbFileLimit = null, codegraphFileLimit = null).toOpenProjectSettings()

        assertEquals(LSPConfig.DEFAULT_AST_MAX_FILES, settings["ast_max_files"])
        assertEquals(LSPConfig.DEFAULT_VECDB_MAX_FILES, settings["vecdb_max_files"])
        assertEquals(LSPConfig.DEFAULT_CODEGRAPH_MAX_FILES, settings["codegraph_max_files"])
    }
}

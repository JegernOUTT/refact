package com.smallcloud.refactai.lsp

data class LSPConfig(
    var ast: Boolean = true,
    var astFileLimit: Int? = null,
    var vecdb: Boolean = true,
    var vecdbFileLimit: Int? = null,
    var codegraph: Boolean = true,
    var codegraphFileLimit: Int? = null,
    var insecureSSL: Boolean = false,
    val experimental: Boolean = false,
    val httpHost: String = "0.0.0.0"
) {
    fun toOpenProjectSettings(): Map<String, Any> {
        return mapOf(
            "ast" to ast,
            "vecdb" to vecdb,
            "codegraph" to codegraph,
            "ast_max_files" to (astFileLimit ?: DEFAULT_AST_MAX_FILES),
            "vecdb_max_files" to (vecdbFileLimit ?: DEFAULT_VECDB_MAX_FILES),
            "codegraph_max_files" to (codegraphFileLimit ?: DEFAULT_CODEGRAPH_MAX_FILES),
        )
    }

    fun toSafeLogString(): String {
        return toOpenProjectSettings()
            .entries
            .joinToString(" ") { "${it.key}=${it.value}" }
    }

    fun sameRuntimeSettings(other: LSPConfig?): Boolean {
        if (other == null) return false
        return ast == other.ast
            && vecdb == other.vecdb
            && codegraph == other.codegraph
            && astFileLimit == other.astFileLimit
            && vecdbFileLimit == other.vecdbFileLimit
            && codegraphFileLimit == other.codegraphFileLimit
    }

    val isValid: Boolean
        get() {
            return (!ast || astFileLimit == null || astFileLimit!! > 0)
                && (!vecdb || vecdbFileLimit == null || vecdbFileLimit!! > 0)
                && (!codegraph || codegraphFileLimit == null || codegraphFileLimit!! > 0)
        }

    companion object {
        const val DEFAULT_AST_MAX_FILES = 50000
        const val DEFAULT_VECDB_MAX_FILES = 15000
        const val DEFAULT_CODEGRAPH_MAX_FILES = 15000
    }
}

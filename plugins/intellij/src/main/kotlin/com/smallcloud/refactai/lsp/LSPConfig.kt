package com.smallcloud.refactai.lsp

data class LSPConfig(
    var ast: Boolean = true,
    var astFileLimit: Int? = null,
    var vecdb: Boolean = true,
    var vecdbFileLimit: Int? = null,
    var insecureSSL: Boolean = false,
    val experimental: Boolean = false,
    val httpHost: String = "0.0.0.0"
) {
    fun toOpenProjectSettings(): Map<String, Any> {
        return mapOf(
            "ast" to ast,
            "vecdb" to vecdb,
            "ast_max_files" to (astFileLimit ?: DEFAULT_AST_MAX_FILES),
            "vecdb_max_files" to (vecdbFileLimit ?: DEFAULT_VECDB_MAX_FILES),
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
            && astFileLimit == other.astFileLimit
            && vecdbFileLimit == other.vecdbFileLimit
    }

    val isValid: Boolean
        get() {
            return (!ast || astFileLimit == null || astFileLimit!! > 0)
                && (!vecdb || vecdbFileLimit == null || vecdbFileLimit!! > 0)
        }

    companion object {
        const val DEFAULT_AST_MAX_FILES = 50000
        const val DEFAULT_VECDB_MAX_FILES = 15000
    }
}

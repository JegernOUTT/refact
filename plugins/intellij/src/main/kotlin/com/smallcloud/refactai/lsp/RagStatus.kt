package com.smallcloud.refactai.lsp

import com.google.gson.annotations.SerializedName


data class RagStatus(
    @SerializedName("ast") val ast: AstStatus? = null,
    @SerializedName("ast_alive") val astAlive: String? = null,
    @SerializedName("codegraph") val codegraph: CodeGraphStatus? = null,
    @SerializedName("codegraph_alive") val codegraphAlive: String? = null,
    @SerializedName("codegraph_error") val codegraphError: String? = null
)

data class AstStatus(
    @SerializedName("files_unparsed") val filesUnparsed: Int,
    @SerializedName("files_total") val filesTotal: Int,
    @SerializedName("ast_index_files_total") val astIndexFilesTotal: Int,
    @SerializedName("ast_index_symbols_total") val astIndexSymbolsTotal: Int,
    @SerializedName("state") val state: String,
    @SerializedName("ast_max_files_hit") val astMaxFilesHit: Boolean
)

data class CodeGraphStatus(
    @SerializedName("counts") val counts: CodeGraphCounts,
    @SerializedName("queued") val queued: Int,
    @SerializedName("state") val state: String,
    @SerializedName("error") val error: String
)

data class CodeGraphCounts(
    @SerializedName("nodes") val nodes: Long,
    @SerializedName("edges") val edges: Long,
    @SerializedName("files") val files: Long,
    @SerializedName("fts_docs") val ftsDocs: Long
)

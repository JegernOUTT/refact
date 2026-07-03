pub const SCHEMA_VERSION: i64 = 7;

pub const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS nodes (
    id    INTEGER PRIMARY KEY,
    kind  TEXT NOT NULL,
    path  TEXT NOT NULL,
    name  TEXT NOT NULL,
    lang  TEXT NOT NULL DEFAULT '',
    line1 INTEGER NOT NULL DEFAULT 0,
    line2 INTEGER NOT NULL DEFAULT 0,
    data  TEXT
);
CREATE INDEX IF NOT EXISTS idx_nodes_path ON nodes(path);
CREATE INDEX IF NOT EXISTS idx_nodes_name ON nodes(name);

CREATE TABLE IF NOT EXISTS edges (
    id         INTEGER PRIMARY KEY,
    src        INTEGER NOT NULL,
    dst        INTEGER NOT NULL,
    kind       TEXT NOT NULL,
    confidence REAL NOT NULL DEFAULT 1.0,
    line       INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_edges_src ON edges(src);
CREATE INDEX IF NOT EXISTS idx_edges_dst ON edges(dst);

CREATE TABLE IF NOT EXISTS symbols (
    double_colon_path TEXT NOT NULL,
    symbol_path       TEXT NOT NULL DEFAULT '',
    reverse_symbol_path TEXT NOT NULL DEFAULT '',
    friendly_path      TEXT NOT NULL DEFAULT '',
    node_id           INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_symbols_path ON symbols(double_colon_path);
CREATE INDEX IF NOT EXISTS idx_symbols_symbol_path ON symbols(symbol_path, double_colon_path);
CREATE INDEX IF NOT EXISTS idx_symbols_reverse_symbol_path ON symbols(reverse_symbol_path COLLATE NOCASE, double_colon_path);
CREATE INDEX IF NOT EXISTS idx_symbols_friendly_path ON symbols(friendly_path, double_colon_path);
CREATE INDEX IF NOT EXISTS idx_symbols_node_id ON symbols(node_id);

CREATE VIRTUAL TABLE IF NOT EXISTS symbol_search USING fts5(
    double_colon_path,
    friendly_path,
    tokenize='trigram'
);

CREATE TABLE IF NOT EXISTS pending_refs (
    id           INTEGER PRIMARY KEY,
    from_node_id INTEGER NOT NULL,
    name         TEXT NOT NULL,
    kind         TEXT NOT NULL,
    line         INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_pending_from ON pending_refs(from_node_id);
CREATE INDEX IF NOT EXISTS idx_pending_name ON pending_refs(name);

CREATE TABLE IF NOT EXISTS dirty_paths (
    path TEXT PRIMARY KEY
);

CREATE TABLE IF NOT EXISTS file_hashes (
    path TEXT PRIMARY KEY,
    hash TEXT NOT NULL
);

CREATE VIRTUAL TABLE IF NOT EXISTS fts_code USING fts5(path UNINDEXED, text);
"#;

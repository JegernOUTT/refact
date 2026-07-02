use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use refact_codegraph_parsers::Resolver;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::extract::{edge_kind_str, extract_symbols};
use crate::schema;

fn normalized_file_namespace(path: &str) -> String {
    let raw = Path::new(path);
    let normalized_path = if raw.is_absolute() {
        let absolute = raw.canonicalize().unwrap_or_else(|_| raw.to_path_buf());
        std::env::current_dir()
            .ok()
            .and_then(|cwd| {
                let cwd = cwd.canonicalize().unwrap_or(cwd);
                cwd.ancestors().find_map(|ancestor| {
                    absolute
                        .strip_prefix(ancestor)
                        .ok()
                        .map(|p| p.to_path_buf())
                })
            })
            .unwrap_or(absolute)
    } else {
        raw.to_path_buf()
    };

    let mut parts = Vec::new();
    for component in normalized_path.components() {
        match component {
            std::path::Component::CurDir | std::path::Component::RootDir => {}
            std::path::Component::ParentDir => {
                parts.pop();
            }
            std::path::Component::Normal(part) => parts.push(part.to_string_lossy().to_string()),
            std::path::Component::Prefix(prefix) => {
                parts.push(prefix.as_os_str().to_string_lossy().to_string())
            }
        }
    }
    let normalized = parts.join("/");
    if normalized.is_empty() {
        "unknown".to_string()
    } else {
        normalized
    }
}

fn qualify(path: &str, in_file_path: &str) -> String {
    format!("{}::{}", normalized_file_namespace(path), in_file_path)
}

fn add_symbol_reference_keys(keys: &mut HashSet<String>, dcp: &str, name: &str) {
    if !name.is_empty() {
        keys.insert(name.to_string());
    }
    if !dcp.is_empty() {
        keys.insert(dcp.to_string());
    }
    let parts: Vec<&str> = dcp.split("::").collect();
    for i in 1..parts.len() {
        let suffix = parts[i..].join("::");
        if !suffix.is_empty() {
            keys.insert(suffix);
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    pub nodes: i64,
    pub edges: i64,
    pub files: i64,
    pub fts_docs: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolData {
    pub node_id: i64,
    pub path: String,
    pub data: String,
}

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = Connection::open(path).map_err(|e| format!("codegraph open {path:?}: {e}"))?;
        let conn = if Self::schema_mismatch(&conn)? {
            drop(conn);
            Self::remove_sqlite_files(path)?;
            Connection::open(path).map_err(|e| format!("codegraph reopen {path:?}: {e}"))?
        } else {
            conn
        };
        let store = Self { conn };
        store.apply_schema()?;
        Ok(store)
    }

    pub fn open_in_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory().map_err(|e| format!("codegraph open mem: {e}"))?;
        let store = Self { conn };
        store.apply_schema()?;
        Ok(store)
    }

    fn schema_mismatch(conn: &Connection) -> Result<bool, String> {
        let object_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type IN ('table', 'virtual table') \
                 AND name NOT LIKE 'sqlite_%'",
                [],
                |row| row.get(0),
            )
            .map_err(|e| format!("codegraph schema inspect: {e}"))?;
        if object_count == 0 {
            return Ok(false);
        }

        let meta_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'meta'",
                [],
                |row| row.get(0),
            )
            .map_err(|e| format!("codegraph meta inspect: {e}"))?;
        if meta_count == 0 {
            return Ok(true);
        }

        let raw: Option<String> = conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| format!("codegraph schema_version inspect: {e}"))?;
        Ok(raw.and_then(|v| v.parse::<i64>().ok()) != Some(schema::SCHEMA_VERSION))
    }

    fn remove_sqlite_files(path: &Path) -> Result<(), String> {
        for suffix in ["", "-wal", "-shm", "-journal"] {
            let candidate = if suffix.is_empty() {
                path.to_path_buf()
            } else {
                Path::new(&format!("{}{}", path.to_string_lossy(), suffix)).to_path_buf()
            };
            match fs::remove_file(&candidate) {
                Ok(()) => {}
                Err(err) if err.kind() == ErrorKind::NotFound => {}
                Err(err) => return Err(format!("codegraph remove stale db {candidate:?}: {err}")),
            }
        }
        Ok(())
    }

    fn apply_schema(&self) -> Result<(), String> {
        self.conn
            .execute_batch(schema::SCHEMA_SQL)
            .map_err(|e| format!("codegraph schema: {e}"))?;
        self.conn
            .execute(
                "INSERT INTO meta(key, value) VALUES('schema_version', ?1) \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![schema::SCHEMA_VERSION.to_string()],
            )
            .map_err(|e| format!("codegraph meta: {e}"))?;
        Ok(())
    }

    pub fn schema_version(&self) -> Result<i64, String> {
        let raw: String = self
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .map_err(|e| format!("codegraph schema_version: {e}"))?;
        raw.parse::<i64>()
            .map_err(|e| format!("codegraph schema_version parse: {e}"))
    }

    pub fn insert_node(
        &self,
        kind: &str,
        path: &str,
        name: &str,
        lang: &str,
        line1: i64,
        line2: i64,
    ) -> Result<i64, String> {
        self.conn
            .execute(
                "INSERT INTO nodes(kind, path, name, lang, line1, line2) \
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
                params![kind, path, name, lang, line1, line2],
            )
            .map_err(|e| format!("codegraph insert_node: {e}"))?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn insert_node_with_data(
        &self,
        kind: &str,
        path: &str,
        name: &str,
        lang: &str,
        line1: i64,
        line2: i64,
        data: &str,
    ) -> Result<i64, String> {
        self.conn
            .execute(
                "INSERT INTO nodes(kind, path, name, lang, line1, line2, data) \
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![kind, path, name, lang, line1, line2, data],
            )
            .map_err(|e| format!("codegraph insert_node_with_data: {e}"))?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn symbol_data_for_path(&self, path: &str) -> Result<Vec<SymbolData>, String> {
        self.query_symbol_data(
            "SELECT id, path, data FROM nodes WHERE path = ?1 AND data IS NOT NULL ORDER BY line1",
            path,
        )
    }

    pub fn symbol_data_by_dcp(&self, dcp: &str) -> Result<Vec<SymbolData>, String> {
        self.query_symbol_data(
            "SELECT n.id, n.path, n.data FROM symbols s JOIN nodes n ON n.id = s.node_id \
             WHERE (s.double_colon_path = ?1 OR s.double_colon_path LIKE '%::' || ?1) \
             AND n.data IS NOT NULL",
            dcp,
        )
    }

    fn query_symbol_data(&self, sql: &str, arg: &str) -> Result<Vec<SymbolData>, String> {
        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| format!("codegraph query_symbol_data prepare: {e}"))?;
        let rows = stmt
            .query_map(params![arg], |row| {
                Ok(SymbolData {
                    node_id: row.get(0)?,
                    path: row.get(1)?,
                    data: row.get(2)?,
                })
            })
            .map_err(|e| format!("codegraph query_symbol_data: {e}"))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| format!("codegraph query_symbol_data row: {e}"))?);
        }
        Ok(out)
    }

    pub fn usage_data_for_node(
        &self,
        node_id: i64,
    ) -> Result<Vec<(usize, String, String)>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT e.line, COALESCE(s.double_colon_path, dn.name), e.kind FROM edges e \
                 JOIN nodes dn ON dn.id = e.dst \
                 LEFT JOIN symbols s ON s.node_id = dn.id \
                 WHERE e.src = ?1 AND e.kind != 'defined_in' ORDER BY e.line, e.id",
            )
            .map_err(|e| format!("codegraph usage_data_for_node prepare: {e}"))?;
        let rows = stmt
            .query_map(params![node_id], |row| {
                Ok((
                    row.get::<_, i64>(0)? as usize,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| format!("codegraph usage_data_for_node: {e}"))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| format!("codegraph usage_data_for_node row: {e}"))?);
        }
        Ok(out)
    }

    pub fn all_symbol_dcps(&self) -> Result<Vec<String>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT double_colon_path FROM symbols")
            .map_err(|e| format!("codegraph all_symbol_dcps prepare: {e}"))?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| format!("codegraph all_symbol_dcps: {e}"))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| format!("codegraph all_symbol_dcps row: {e}"))?);
        }
        Ok(out)
    }

    pub fn add_edge(&self, src: i64, dst: i64, kind: &str, confidence: f64) -> Result<i64, String> {
        self.conn
            .execute(
                "INSERT INTO edges(src, dst, kind, confidence) VALUES(?1, ?2, ?3, ?4)",
                params![src, dst, kind, confidence],
            )
            .map_err(|e| format!("codegraph add_edge: {e}"))?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn add_edge_line(
        &self,
        src: i64,
        dst: i64,
        kind: &str,
        confidence: f64,
        line: i64,
    ) -> Result<i64, String> {
        self.conn
            .execute(
                "INSERT INTO edges(src, dst, kind, confidence, line) VALUES(?1, ?2, ?3, ?4, ?5)",
                params![src, dst, kind, confidence, line],
            )
            .map_err(|e| format!("codegraph add_edge_line: {e}"))?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn add_symbol(&self, double_colon_path: &str, node_id: i64) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO symbols(double_colon_path, node_id) VALUES(?1, ?2)",
                params![double_colon_path, node_id],
            )
            .map_err(|e| format!("codegraph add_symbol: {e}"))?;
        Ok(())
    }

    pub fn add_pending_ref(
        &self,
        from_node_id: i64,
        name: &str,
        kind: &str,
        line: i64,
    ) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO pending_refs(from_node_id, name, kind, line) VALUES(?1, ?2, ?3, ?4)",
                params![from_node_id, name, kind, line],
            )
            .map_err(|e| format!("codegraph add_pending_ref: {e}"))?;
        Ok(())
    }

    pub fn graph_edges(&self) -> Result<Vec<(i64, i64, String)>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT src, dst, kind FROM edges WHERE kind != 'defined_in'")
            .map_err(|e| format!("codegraph graph_edges prepare: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| format!("codegraph graph_edges: {e}"))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| format!("codegraph graph_edges row: {e}"))?);
        }
        Ok(out)
    }

    pub fn node_names(&self) -> Result<Vec<(i64, String, String)>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name, path FROM nodes WHERE kind != 'file'")
            .map_err(|e| format!("codegraph node_names prepare: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| format!("codegraph node_names: {e}"))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| format!("codegraph node_names row: {e}"))?);
        }
        Ok(out)
    }

    pub fn all_symbols(&self) -> Result<Vec<(String, i64)>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT double_colon_path, node_id FROM symbols")
            .map_err(|e| format!("codegraph all_symbols prepare: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|e| format!("codegraph all_symbols: {e}"))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| format!("codegraph all_symbols row: {e}"))?);
        }
        Ok(out)
    }

    fn all_pending_refs(&self) -> Result<Vec<(i64, String, String, String, i64)>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT p.from_node_id, n.path, p.name, p.kind, p.line \
                 FROM pending_refs p JOIN nodes n ON n.id = p.from_node_id",
            )
            .map_err(|e| format!("codegraph all_pending_refs prepare: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })
            .map_err(|e| format!("codegraph all_pending_refs: {e}"))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| format!("codegraph all_pending_refs row: {e}"))?);
        }
        Ok(out)
    }

    fn dirty_paths(&self) -> Result<Vec<String>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT path FROM dirty_paths ORDER BY path")
            .map_err(|e| format!("codegraph dirty_paths prepare: {e}"))?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| format!("codegraph dirty_paths: {e}"))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| format!("codegraph dirty_paths row: {e}"))?);
        }
        Ok(out)
    }

    pub fn has_dirty_paths(&self) -> Result<bool, String> {
        Ok(self.scalar_i64("SELECT COUNT(*) FROM dirty_paths")? > 0)
    }

    fn mark_path_dirty(&self, path: &str) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT OR IGNORE INTO dirty_paths(path) VALUES(?1)",
                params![path],
            )
            .map_err(|e| format!("codegraph mark dirty path: {e}"))?;
        Ok(())
    }

    fn symbol_reference_keys_for_path(&self, path: &str) -> Result<HashSet<String>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT s.double_colon_path, n.name FROM symbols s JOIN nodes n ON n.id = s.node_id \
                 WHERE n.path = ?1",
            )
            .map_err(|e| format!("codegraph symbol_reference_keys_for_path prepare: {e}"))?;
        let rows = stmt
            .query_map(params![path], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| format!("codegraph symbol_reference_keys_for_path: {e}"))?;
        let mut out = HashSet::new();
        for r in rows {
            let (dcp, name) =
                r.map_err(|e| format!("codegraph symbol_reference_keys_for_path row: {e}"))?;
            add_symbol_reference_keys(&mut out, &dcp, &name);
        }
        Ok(out)
    }

    fn mark_paths_referencing_keys(&self, keys: &HashSet<String>) -> Result<(), String> {
        for key in keys {
            self.conn
                .execute(
                    "INSERT OR IGNORE INTO dirty_paths(path) \
                     SELECT DISTINCT n.path FROM pending_refs p JOIN nodes n ON n.id = p.from_node_id \
                     WHERE p.name = ?1",
                    params![key],
                )
                .map_err(|e| format!("codegraph mark referencing paths dirty: {e}"))?;
        }
        Ok(())
    }

    pub fn connect_usages(&self) -> Result<(), String> {
        let dirty_paths = self.dirty_paths()?;
        if dirty_paths.is_empty() {
            return Ok(());
        }
        debug!(
            "codegraph: connect_usages for {} dirty files",
            dirty_paths.len()
        );

        let dirty_set: HashSet<String> = dirty_paths.iter().cloned().collect();
        let symbols = self.all_symbols()?;
        let mut resolver = Resolver::new();
        let mut dcp_to_node: HashMap<String, i64> = HashMap::new();
        for (dcp, node_id) in &symbols {
            resolver.add_symbol(dcp);
            dcp_to_node.entry(dcp.clone()).or_insert(*node_id);
        }

        self.conn
            .execute(
                "DELETE FROM edges WHERE kind != 'defined_in' \
                 AND src IN (SELECT id FROM nodes WHERE path IN (SELECT path FROM dirty_paths))",
                [],
            )
            .map_err(|e| format!("codegraph connect_usages clear: {e}"))?;

        for (from_node_id, from_path, name, kind, line) in self.all_pending_refs()? {
            if !dirty_set.contains(&from_path) {
                continue;
            }
            let local_name = qualify(&from_path, &name);
            if let Some(res) = resolver
                .resolve(&local_name)
                .or_else(|| resolver.resolve(&name))
            {
                if let Some(&dst_id) = dcp_to_node.get(&res.target) {
                    if dst_id != from_node_id {
                        self.add_edge_line(
                            from_node_id,
                            dst_id,
                            &kind,
                            res.confidence as f64,
                            line,
                        )?;
                    }
                }
            }
        }

        self.conn
            .execute("DELETE FROM dirty_paths", [])
            .map_err(|e| format!("codegraph connect_usages clear dirty paths: {e}"))?;
        debug!(
            "codegraph: connect_usages complete for {} dirty files",
            dirty_paths.len()
        );
        Ok(())
    }

    pub fn inherits_pairs(&self) -> Result<Vec<(String, String)>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT sn.name, dn.name FROM edges e \
                 JOIN nodes sn ON sn.id = e.src \
                 JOIN nodes dn ON dn.id = e.dst \
                 WHERE e.kind = 'inherits'",
            )
            .map_err(|e| format!("codegraph inherits_pairs prepare: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| format!("codegraph inherits_pairs: {e}"))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| format!("codegraph inherits_pairs row: {e}"))?);
        }
        Ok(out)
    }

    pub fn doc_usages(&self, path: &str) -> Result<Vec<(usize, String)>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT e.line, dn.name FROM edges e \
                 JOIN nodes sn ON sn.id = e.src \
                 JOIN nodes dn ON dn.id = e.dst \
                 WHERE sn.path = ?1 AND e.kind != 'defined_in' ORDER BY e.line",
            )
            .map_err(|e| format!("codegraph doc_usages prepare: {e}"))?;
        let rows = stmt
            .query_map(params![path], |row| {
                Ok((row.get::<_, i64>(0)? as usize, row.get::<_, String>(1)?))
            })
            .map_err(|e| format!("codegraph doc_usages: {e}"))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| format!("codegraph doc_usages row: {e}"))?);
        }
        Ok(out)
    }

    pub fn symbol_count(&self) -> Result<i64, String> {
        self.scalar_i64("SELECT COUNT(*) FROM symbols")
    }

    pub fn usage_edge_count(&self) -> Result<i64, String> {
        self.scalar_i64("SELECT COUNT(*) FROM edges WHERE kind != 'defined_in'")
    }

    pub fn remove_path(&self, path: &str) -> Result<(), String> {
        let affected_keys = self.symbol_reference_keys_for_path(path)?;
        self.mark_path_dirty(path)?;
        self.mark_paths_referencing_keys(&affected_keys)?;
        self.conn
            .execute(
                "DELETE FROM pending_refs WHERE from_node_id IN \
                 (SELECT id FROM nodes WHERE path = ?1)",
                params![path],
            )
            .map_err(|e| format!("codegraph remove pending_refs: {e}"))?;
        self.conn
            .execute(
                "DELETE FROM edges WHERE src IN (SELECT id FROM nodes WHERE path = ?1) \
                 OR dst IN (SELECT id FROM nodes WHERE path = ?1)",
                params![path],
            )
            .map_err(|e| format!("codegraph remove edges: {e}"))?;
        self.conn
            .execute(
                "DELETE FROM symbols WHERE node_id IN (SELECT id FROM nodes WHERE path = ?1)",
                params![path],
            )
            .map_err(|e| format!("codegraph remove symbols: {e}"))?;
        self.conn
            .execute("DELETE FROM nodes WHERE path = ?1", params![path])
            .map_err(|e| format!("codegraph remove nodes: {e}"))?;
        self.conn
            .execute("DELETE FROM fts_code WHERE path = ?1", params![path])
            .map_err(|e| format!("codegraph remove fts: {e}"))?;
        Ok(())
    }

    pub fn index_file(&self, path: &str, text: &str, lang: &str) -> Result<i64, String> {
        self.remove_path(path)?;
        let line_count = text.lines().count() as i64;
        let name = Path::new(path)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string());
        let node_id = self.insert_node("file", path, &name, lang, 1, line_count.max(1))?;
        self.conn
            .execute(
                "INSERT INTO fts_code(path, text) VALUES(?1, ?2)",
                params![path, text],
            )
            .map_err(|e| format!("codegraph fts insert: {e}"))?;
        Ok(node_id)
    }

    pub fn index_file_graph(&self, path: &str, text: &str, lang: &str) -> Result<i64, String> {
        let file_id = self.index_file(path, text, lang)?;

        let (symbols, refs) = extract_symbols(lang, text);
        let routes = refact_codegraph_parsers::frameworks::detect_routes(lang, text);
        debug!(
            "codegraph: index_file_graph {path}: {} symbols, {} refs, {} routes",
            symbols.len(),
            refs.len(),
            routes.len()
        );
        if symbols.is_empty() && refs.is_empty() && routes.is_empty() {
            return Ok(file_id);
        }

        let mut resolver = Resolver::new();
        for symbol in &symbols {
            resolver.add_symbol(&qualify(path, &symbol.double_colon_path()));
        }

        let mut affected_keys = HashSet::new();
        for symbol in &symbols {
            let dcp = qualify(path, &symbol.double_colon_path());
            add_symbol_reference_keys(&mut affected_keys, &dcp, &symbol.name());
        }
        self.mark_paths_referencing_keys(&affected_keys)?;
        self.mark_path_dirty(path)?;

        let mut path_to_node: HashMap<String, i64> = HashMap::new();
        for symbol in &symbols {
            let kind = format!("{:?}", symbol.kind).to_lowercase();
            let data = serde_json::to_string(symbol)
                .map_err(|e| format!("codegraph serialize symbol: {e}"))?;
            let node_id = self.insert_node_with_data(
                &kind,
                path,
                &symbol.name(),
                lang,
                symbol.decl_line1 as i64,
                symbol.body_line2 as i64,
                &data,
            )?;
            let dcp = qualify(path, &symbol.double_colon_path());
            self.add_symbol(&dcp, node_id)?;
            self.add_edge(node_id, file_id, "defined_in", 1.0)?;
            path_to_node.insert(dcp, node_id);
        }

        for symbol in &symbols {
            if symbol.this_class_derived_from.is_empty() {
                continue;
            }
            let from_dcp = qualify(path, &symbol.double_colon_path());
            if let Some(&from_id) = path_to_node.get(&from_dcp) {
                for base in &symbol.this_class_derived_from {
                    self.add_pending_ref(from_id, base, "inherits", symbol.decl_line1 as i64)?;
                }
            }
        }

        for r in &refs {
            let from_dcp = qualify(path, &r.from);
            let Some(&src_id) = path_to_node.get(&from_dcp) else {
                continue;
            };
            self.add_pending_ref(src_id, &r.name, edge_kind_str(r.kind), r.line as i64)?;
            let local_name = qualify(path, &r.name);
            if let Some(res) = resolver
                .resolve(&local_name)
                .or_else(|| resolver.resolve(&r.name))
            {
                if let Some(&dst_id) = path_to_node.get(&res.target) {
                    if src_id != dst_id {
                        self.add_edge_line(
                            src_id,
                            dst_id,
                            edge_kind_str(r.kind),
                            res.confidence as f64,
                            r.line as i64,
                        )?;
                    }
                }
            }
        }

        for route in routes {
            let route_id = self.insert_node("route", path, &route.label(), lang, 0, 0)?;
            self.add_edge(route_id, file_id, "defined_in", 1.0)?;
            self.add_pending_ref(route_id, &route.handler, "route_handler", 0)?;
        }

        Ok(file_id)
    }

    pub fn counts(&self) -> Result<Counts, String> {
        Ok(Counts {
            nodes: self.scalar_i64("SELECT COUNT(*) FROM nodes")?,
            edges: self.scalar_i64("SELECT COUNT(*) FROM edges")?,
            files: self.scalar_i64("SELECT COUNT(*) FROM nodes WHERE kind = 'file'")?,
            fts_docs: self.scalar_i64("SELECT COUNT(*) FROM fts_code")?,
        })
    }

    pub fn fts_ranked(&self, match_query: &str, limit: i64) -> Result<Vec<(String, f64)>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT path, bm25(fts_code) AS rank FROM fts_code \
                 WHERE fts_code MATCH ?1 ORDER BY rank LIMIT ?2",
            )
            .map_err(|e| format!("codegraph fts_ranked prepare: {e}"))?;
        let rows = stmt
            .query_map(params![match_query, limit], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
            })
            .map_err(|e| format!("codegraph fts_ranked: {e}"))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| format!("codegraph fts_ranked row: {e}"))?);
        }
        Ok(out)
    }

    pub fn all_files_with_text(&self) -> Result<Vec<(String, String)>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT path, text FROM fts_code")
            .map_err(|e| format!("codegraph all_files_with_text prepare: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| format!("codegraph all_files_with_text: {e}"))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| format!("codegraph all_files_with_text row: {e}"))?);
        }
        Ok(out)
    }

    pub fn all_paths(&self) -> Result<Vec<String>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT path FROM fts_code")
            .map_err(|e| format!("codegraph all_paths prepare: {e}"))?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| format!("codegraph all_paths: {e}"))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| format!("codegraph all_paths row: {e}"))?);
        }
        Ok(out)
    }

    pub fn symbol_name_ranked(
        &self,
        like_term: &str,
        limit: i64,
    ) -> Result<Vec<(String, String, i64, i64)>, String> {
        let pattern = format!("%{}%", like_term.to_lowercase());
        let mut stmt = self
            .conn
            .prepare(
                "SELECT path, name, line1, line2 FROM nodes \
                 WHERE data IS NOT NULL AND lower(name) LIKE ?1 ORDER BY id LIMIT ?2",
            )
            .map_err(|e| format!("codegraph symbol_name_ranked prepare: {e}"))?;
        let rows = stmt
            .query_map(params![pattern, limit], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })
            .map_err(|e| format!("codegraph symbol_name_ranked: {e}"))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| format!("codegraph symbol_name_ranked row: {e}"))?);
        }
        Ok(out)
    }

    pub fn file_span(&self, path: &str) -> Result<Option<(usize, usize)>, String> {
        let span = self
            .conn
            .query_row(
                "SELECT line1, line2 FROM nodes WHERE kind = 'file' AND path = ?1",
                params![path],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()
            .map_err(|e| format!("codegraph file_span: {e}"))?;
        Ok(span.map(|(line1, line2)| (line1.max(1) as usize, line2.max(1) as usize)))
    }

    pub fn neighbor_paths(&self, path: &str) -> Result<Vec<String>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT DISTINCT dn.path FROM edges e \
                 JOIN nodes sn ON sn.id = e.src \
                 JOIN nodes dn ON dn.id = e.dst \
                 WHERE sn.path = ?1 AND e.kind != 'defined_in' AND dn.path != ?1",
            )
            .map_err(|e| format!("codegraph neighbor_paths prepare: {e}"))?;
        let rows = stmt
            .query_map(params![path], |row| row.get::<_, String>(0))
            .map_err(|e| format!("codegraph neighbor_paths: {e}"))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| format!("codegraph neighbor_paths row: {e}"))?);
        }
        Ok(out)
    }

    pub fn search_fts(&self, query: &str, limit: i64) -> Result<Vec<String>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT path FROM fts_code WHERE fts_code MATCH ?1 LIMIT ?2")
            .map_err(|e| format!("codegraph fts prepare: {e}"))?;
        let rows = stmt
            .query_map(params![query, limit], |row| row.get::<_, String>(0))
            .map_err(|e| format!("codegraph fts query: {e}"))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| format!("codegraph fts row: {e}"))?);
        }
        Ok(out)
    }

    fn scalar_i64(&self, sql: &str) -> Result<i64, String> {
        self.conn
            .query_row(sql, [], |row| row.get(0))
            .map_err(|e| format!("codegraph scalar {sql:?}: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_recreates_old_schema_db_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("codegraph.sqlite");
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(schema::SCHEMA_SQL).unwrap();
            conn.execute(
                "INSERT INTO meta(key, value) VALUES('schema_version', '1')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO nodes(kind, path, name, lang, line1, line2) \
                 VALUES('file', 'stale.rs', 'stale.rs', 'rust', 1, 1)",
                [],
            )
            .unwrap();
        }

        let store = Store::open(&db_path).unwrap();
        assert_eq!(store.schema_version().unwrap(), schema::SCHEMA_VERSION);
        assert_eq!(store.counts().unwrap(), Counts::default());
    }

    #[test]
    fn schema_roundtrip() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.schema_version().unwrap(), schema::SCHEMA_VERSION);

        let file_id = store
            .insert_node("file", "src/a.rs", "a.rs", "rust", 1, 10)
            .unwrap();
        let fn_id = store
            .insert_node("function", "src/a.rs", "foo", "rust", 3, 7)
            .unwrap();
        store.add_edge(fn_id, file_id, "defined_in", 1.0).unwrap();
        store.add_symbol("a::foo", fn_id).unwrap();

        let counts = store.counts().unwrap();
        assert_eq!(counts.nodes, 2);
        assert_eq!(counts.edges, 1);
        assert_eq!(counts.files, 1);
    }

    #[test]
    fn index_file_graph_extracts_rust_symbols_and_edges() {
        let store = Store::open_in_memory().unwrap();
        let src = "\
struct Widget;

impl Widget {
    fn render(&self) {
        helper();
    }
}

fn helper() {}
";
        store
            .index_file_graph("src/widget.rs", src, "rust")
            .unwrap();

        let counts = store.counts().unwrap();
        assert_eq!(counts.files, 1);
        assert!(
            counts.nodes >= 4,
            "expected file + Widget + render + helper nodes, got {}",
            counts.nodes
        );
        assert!(
            counts.edges >= 4,
            "expected defined_in edges + a calls edge, got {}",
            counts.edges
        );

        let calls: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM edges WHERE kind = 'calls'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(calls, 1, "render -> helper calls edge");
    }

    #[test]
    fn index_file_graph_is_idempotent() {
        let store = Store::open_in_memory().unwrap();
        let src = "fn a() { b(); }\nfn b() {}\n";
        store.index_file_graph("src/m.rs", src, "rust").unwrap();
        let first = store.counts().unwrap();
        store.index_file_graph("src/m.rs", src, "rust").unwrap();
        let second = store.counts().unwrap();
        assert_eq!(first, second, "re-indexing same file must not duplicate");
    }

    #[test]
    fn connect_usages_resolves_cross_file_calls() {
        let store = Store::open_in_memory().unwrap();
        store
            .index_file_graph("src/a.rs", "pub fn helper() {}\n", "rust")
            .unwrap();
        store
            .index_file_graph("src/b.rs", "fn run() { helper(); }\n", "rust")
            .unwrap();

        store.connect_usages().unwrap();

        let cross: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM edges e \
                 JOIN nodes sn ON sn.id = e.src \
                 JOIN nodes dn ON dn.id = e.dst \
                 WHERE e.kind = 'calls' AND sn.path = 'src/b.rs' AND dn.path = 'src/a.rs'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            cross, 1,
            "run (b.rs) -> helper (a.rs) cross-file calls edge"
        );
    }

    #[test]
    fn connect_usages_is_noop_when_nothing_is_dirty() {
        let store = Store::open_in_memory().unwrap();
        store
            .index_file_graph("src/a.rs", "pub fn helper() {}\n", "rust")
            .unwrap();
        store
            .index_file_graph("src/b.rs", "fn run() { helper(); }\n", "rust")
            .unwrap();
        store.connect_usages().unwrap();
        assert!(!store.has_dirty_paths().unwrap());

        let run_id: i64 = store
            .conn
            .query_row("SELECT id FROM nodes WHERE name = 'run'", [], |r| r.get(0))
            .unwrap();
        let helper_id: i64 = store
            .conn
            .query_row("SELECT id FROM nodes WHERE name = 'helper'", [], |r| {
                r.get(0)
            })
            .unwrap();
        store
            .add_edge_line(run_id, helper_id, "manual", 1.0, 99)
            .unwrap();

        store.connect_usages().unwrap();

        let manual_edges: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM edges WHERE kind = 'manual'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(manual_edges, 1);
    }

    #[test]
    fn new_symbol_marks_existing_referrers_dirty() {
        let store = Store::open_in_memory().unwrap();
        store
            .index_file_graph("src/b.rs", "fn run() { helper(); }\n", "rust")
            .unwrap();
        store.connect_usages().unwrap();

        let initial_calls: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM edges WHERE kind = 'calls'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(initial_calls, 0);

        store
            .index_file_graph("src/a.rs", "pub fn helper() {}\n", "rust")
            .unwrap();
        store.connect_usages().unwrap();

        let cross: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM edges e \
                 JOIN nodes sn ON sn.id = e.src \
                 JOIN nodes dn ON dn.id = e.dst \
                 WHERE e.kind = 'calls' AND sn.path = 'src/b.rs' AND dn.path = 'src/a.rs'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(cross, 1);
    }

    #[test]
    fn same_basename_files_do_not_collide() {
        let store = Store::open_in_memory().unwrap();
        store
            .index_file_graph("src/a/m.rs", "pub fn helper() {}\n", "rust")
            .unwrap();
        store
            .index_file_graph(
                "src/b/m.rs",
                "fn run() { helper(); }\nfn helper() {}\n",
                "rust",
            )
            .unwrap();

        store.connect_usages().unwrap();

        let local_calls: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM edges e \
                 JOIN nodes sn ON sn.id = e.src \
                 JOIN nodes dn ON dn.id = e.dst \
                 WHERE e.kind = 'calls' AND sn.path = 'src/b/m.rs' AND dn.path = 'src/b/m.rs'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let wrong_calls: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM edges e \
                 JOIN nodes sn ON sn.id = e.src \
                 JOIN nodes dn ON dn.id = e.dst \
                 WHERE e.kind = 'calls' AND sn.path = 'src/b/m.rs' AND dn.path = 'src/a/m.rs'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(local_calls, 1, "b/m.rs run should call local helper");
        assert_eq!(
            wrong_calls, 0,
            "same-basename helper must not steal the call"
        );
    }

    #[test]
    fn framework_routes_create_route_handler_edges() {
        let store = Store::open_in_memory().unwrap();
        let src = "\
@app.get(\"/users\")
def list_users():
    pass
";
        store.index_file_graph("api.py", src, "python").unwrap();
        store.connect_usages().unwrap();

        let route_edges: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM edges e \
                 JOIN nodes dn ON dn.id = e.dst \
                 WHERE e.kind = 'route_handler' AND dn.name = 'list_users'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            route_edges, 1,
            "GET /users -> list_users route_handler edge"
        );
    }

    #[test]
    fn non_js_python_framework_route_handler_edges_are_indexed() {
        let store = Store::open_in_memory().unwrap();
        let src = r#"
package main
func listUsers(c *gin.Context) {}
func main() {
    r.GET("/users", listUsers)
}
"#;
        store.index_file_graph("api.go", src, "go").unwrap();
        store.connect_usages().unwrap();

        let route_edges: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM edges e \
                 JOIN nodes dn ON dn.id = e.dst \
                 WHERE e.kind = 'route_handler' AND dn.name = 'listUsers'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(route_edges, 1, "GET /users -> listUsers route_handler edge");
    }

    #[test]
    fn connect_usages_resolves_cross_file_inheritance() {
        let store = Store::open_in_memory().unwrap();
        store
            .index_file_graph("src/base.py", "class Base:\n    pass\n", "python")
            .unwrap();
        store
            .index_file_graph(
                "src/derived.py",
                "class Derived(Base):\n    pass\n",
                "python",
            )
            .unwrap();

        store.connect_usages().unwrap();

        let inherits: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM edges WHERE kind = 'inherits'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(inherits, 1, "Derived -> Base inherits edge");
    }

    #[test]
    fn doc_usages_reports_lines_and_targets() {
        let store = Store::open_in_memory().unwrap();
        store
            .index_file_graph(
                "src/a.rs",
                "fn caller() {\n    callee();\n}\nfn callee() {}\n",
                "rust",
            )
            .unwrap();
        store.connect_usages().unwrap();

        let usages = store.doc_usages("src/a.rs").unwrap();
        assert!(
            usages
                .iter()
                .any(|(line, name)| *line == 2 && name == "callee"),
            "expected callee usage at line 2, got {:?}",
            usages
        );
    }

    #[test]
    fn index_file_is_idempotent_and_searchable() {
        let store = Store::open_in_memory().unwrap();
        store
            .index_file("src/main.rs", "fn main() { let codegraph = 1; }", "rust")
            .unwrap();
        store
            .index_file("src/main.rs", "fn main() { let codegraph = 2; }", "rust")
            .unwrap();

        let counts = store.counts().unwrap();
        assert_eq!(counts.files, 1);
        assert_eq!(counts.fts_docs, 1);

        let hits = store.search_fts("codegraph", 10).unwrap();
        assert_eq!(hits, vec!["src/main.rs".to_string()]);
    }

    #[test]
    fn all_files_with_text_and_paths_return_fts_rows() {
        let store = Store::open_in_memory().unwrap();
        store.index_file("src/a.rs", "fn a() {}", "rust").unwrap();
        store.index_file("src/b.rs", "fn b() {}", "rust").unwrap();

        let mut files = store.all_files_with_text().unwrap();
        files.sort();
        assert_eq!(
            files,
            vec![
                ("src/a.rs".to_string(), "fn a() {}".to_string()),
                ("src/b.rs".to_string(), "fn b() {}".to_string())
            ]
        );

        let mut paths = store.all_paths().unwrap();
        paths.sort();
        assert_eq!(paths, vec!["src/a.rs".to_string(), "src/b.rs".to_string()]);
    }
}

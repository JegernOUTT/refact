use rusqlite::{OpenFlags, Result};
use std::any::Any;
use std::collections::HashMap;
use std::fmt::{Debug, Formatter};
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio_rusqlite::Connection;
use tracing::info;
use zerocopy::IntoBytes;
use crate::vdb_structs::{SimpleTextHashVector, SplitResult, VecdbRecord};

impl Debug for VecDBSqlite {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "VecDBSqlite: {:?}", self.conn.type_id())
    }
}

pub struct VecDBSqlite {
    conn: Connection,
    emb_table_name: String,
}

#[derive(Debug, PartialEq)]
struct DataColumn {
    name: String,
    type_: String,
}

fn db_filename(model_name: &str, embedding_size: i32) -> String {
    format!(
        "vecdb_model_{}_esize_{}.sqlite",
        model_name.replace("/", "_"),
        embedding_size
    )
}

async fn move_file_with_fallback(src: &Path, dst: &Path) -> std::io::Result<()> {
    match fs::rename(src, dst).await {
        Ok(_) => Ok(()),
        Err(e)
            if e.kind() == std::io::ErrorKind::CrossesDevices || e.raw_os_error() == Some(18) =>
        {
            fs::copy(src, dst).await?;
            fs::remove_file(src).await?;
            Ok(())
        }
        Err(e) => Err(e),
    }
}

async fn migrate_sqlite_files(src: &Path, dst: &Path) -> std::io::Result<()> {
    move_file_with_fallback(src, dst).await?;
    for suffix in ["-wal", "-shm"] {
        let src_side = PathBuf::from(format!("{}{}", src.display(), suffix));
        let dst_side = PathBuf::from(format!("{}{}", dst.display(), suffix));
        if src_side.exists() {
            let _ = move_file_with_fallback(&src_side, &dst_side).await;
        }
    }
    Ok(())
}

pub async fn get_db_path(
    dest_dir: &PathBuf,
    legacy_cache_dir: &PathBuf,
    model_name: &str,
    embedding_size: i32,
) -> Result<PathBuf, String> {
    let filename = db_filename(model_name, embedding_size);
    let dest_path = dest_dir.join(&filename);

    if dest_path.exists() {
        return Ok(dest_path);
    }

    let legacy_locations = [
        legacy_cache_dir.join(&filename),
        legacy_cache_dir.join("refact_vecdb_cache").join(format!(
            "model_{}_esize_{}.sqlite",
            model_name.replace("/", "_"),
            embedding_size
        )),
    ];

    for legacy_path in &legacy_locations {
        if legacy_path.exists() {
            if let Some(parent) = dest_path.parent() {
                fs::create_dir_all(parent).await.map_err(|e| e.to_string())?;
            }
            match migrate_sqlite_files(legacy_path, &dest_path).await {
                Ok(_) => {
                    info!("migrated vecdb from {:?} to {:?}", legacy_path, dest_path);
                    return Ok(dest_path);
                }
                Err(e) => {
                    info!("failed to migrate vecdb from {:?}: {}", legacy_path, e);
                }
            }
        }
    }

    Ok(dest_path)
}

async fn migrate_202406(conn: &Connection) -> tokio_rusqlite::Result<()> {
    let expected_schema = vec![
        DataColumn { name: "vector".to_string(), type_: "BLOB".to_string() },
        DataColumn { name: "window_text".to_string(), type_: "TEXT".to_string() },
        DataColumn { name: "window_text_hash".to_string(), type_: "TEXT".to_string() },
    ];
    conn.call(move |conn| {
        match conn.execute("ALTER TABLE data RENAME TO embeddings;", []) {
            _ => {}
        };
        let mut stmt = conn.prepare("PRAGMA table_info(embeddings);")?;
        let schema_iter = stmt.query_map([], |row| {
            Ok(DataColumn { name: row.get(1)?, type_: row.get(2)? })
        })?;
        let mut schema = Vec::new();
        for column in schema_iter {
            schema.push(column?);
        }
        if schema != expected_schema {
            if schema.len() > 0 {
                info!("vector cache database has invalid schema, recreating the database");
            }
            conn.execute("DROP TABLE IF EXISTS embeddings", [])?;
            conn.execute(
                "CREATE TABLE embeddings (vector BLOB, window_text TEXT NOT NULL, window_text_hash TEXT NOT NULL)",
                [],
            )?;
            conn.execute(
                "CREATE INDEX IF NOT EXISTS idx_window_text_hash ON embeddings (window_text_hash)",
                [],
            )?;
        }
        Ok(())
    })
    .await
}

async fn migrate_202501(
    conn: &Connection,
    embedding_size: i32,
    emb_table_name: String,
) -> tokio_rusqlite::Result<()> {
    conn.call(move |conn| {
        match conn.execute("ALTER TABLE embeddings RENAME TO embeddings_cache;", []) {
            _ => {}
        };
        conn.execute(
            &format!(
                "CREATE VIRTUAL TABLE IF NOT EXISTS {emb_table_name} using vec0(
              embedding float[{embedding_size}] distance_metric=cosine,
              scope TEXT,
              +start_line INTEGER,
              +end_line INTEGER
            );"
            ),
            [],
        )?;
        Ok(())
    })
    .await
}

impl VecDBSqlite {
    pub async fn init(
        dest_dir: &PathBuf,
        legacy_cache_dir: &PathBuf,
        model_name: &str,
        embedding_size: i32,
        emb_table_name: &str,
    ) -> Result<VecDBSqlite, String> {
        let db_path = get_db_path(dest_dir, legacy_cache_dir, model_name, embedding_size).await?;
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent).await.map_err(|e| e.to_string())?;
        }
        let conn = match Connection::open_with_flags(
            &db_path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_URI,
        )
        .await
        {
            Ok(db) => db,
            Err(err) => return Err(format!("{:?}", err)),
        };
        conn.call(move |conn| {
            let _: String = conn.query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))?;
            Ok(())
        })
        .await
        .map_err(|e| e.to_string())?;
        migrate_202406(&conn).await.map_err(|e| e.to_string())?;
        migrate_202501(&conn, embedding_size, emb_table_name.to_string())
            .await
            .map_err(|e| e.to_string())?;
        crate::vdb_emb_aux::cleanup_old_emb_tables(&conn, 7, 10).await?;

        info!("vecdb initialized at {:?}", db_path);
        Ok(VecDBSqlite {
            conn,
            emb_table_name: emb_table_name.to_string(),
        })
    }

    pub async fn fetch_vectors_from_cache(
        &mut self,
        splits: &Vec<SplitResult>,
    ) -> Result<Vec<Option<Vec<f32>>>, String> {
        let placeholders: String = splits.iter().map(|_| "?").collect::<Vec<&str>>().join(",");
        let query = format!("SELECT * FROM embeddings_cache WHERE window_text_hash IN ({placeholders})");
        let splits_clone = splits.clone();
        let found_hashes = match self
            .conn
            .call(move |connection| {
                let mut statement = connection.prepare(&query)?;
                let params = rusqlite::params_from_iter(splits_clone.iter().map(|x| &x.window_text_hash));
                let x = match statement.query_map(params, |row| {
                    let vector_blob: Vec<u8> = row.get(0)?;
                    let vector: Vec<f32> = vector_blob
                        .chunks_exact(4)
                        .map(|b| f32::from_ne_bytes(b.try_into().unwrap()))
                        .collect();
                    let window_text: String = row.get(1)?;
                    let window_text_hash: String = row.get(2)?;
                    Ok((window_text_hash, (vector, window_text)))
                }) {
                    Ok(mapped_rows) => Ok(mapped_rows.filter_map(|r| r.ok()).collect::<HashMap<_, _>>()),
                    Err(e) => Err(tokio_rusqlite::Error::Rusqlite(e)),
                };
                x
            })
            .await
        {
            Ok(records) => records,
            Err(err) => return Err(format!("{:?}", err)),
        };
        let mut records: Vec<Option<Vec<f32>>> = vec![];
        for split in splits.iter() {
            if let Some(query_data) = found_hashes.get(&split.window_text_hash) {
                records.push(Some(query_data.0.clone()));
            } else {
                records.push(None);
            }
        }
        Ok(records)
    }

    pub async fn cache_add_new_records(&mut self, records: Vec<SimpleTextHashVector>) -> Result<(), String> {
        self.conn.call(|connection| {
            let transaction = connection.transaction()?;
            for record in records {
                let vector_as_bytes: Vec<u8> = match record.vector {
                    Some(vector) => vector.iter().flat_map(|&num| num.to_ne_bytes()).collect(),
                    None => {
                        tracing::error!("Skipping record with no vector: {:?}", record.window_text_hash);
                        continue;
                    }
                };
                match transaction.execute(
                    "INSERT INTO embeddings_cache (vector, window_text, window_text_hash) VALUES (?1, ?2, ?3)",
                    rusqlite::params![vector_as_bytes, record.window_text, record.window_text_hash],
                ) {
                    Ok(_) => {}
                    Err(err) => {
                        tracing::error!("Error while inserting record to cache: {:?}", err);
                        continue;
                    }
                }
            }
            match transaction.commit() {
                Ok(_) => Ok(()),
                Err(err) => Err(err.into()),
            }
        }).await.map_err(|e| e.to_string())
    }

    pub async fn cache_size(&self) -> Result<usize, String> {
        self.conn
            .call(move |connection| {
                let mut stmt = connection.prepare("SELECT COUNT(1) FROM embeddings_cache")?;
                let count: usize = stmt.query_row([], |row| row.get(0))?;
                Ok(count)
            })
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn size(&self) -> Result<usize, String> {
        let emb_table_name = self.emb_table_name.clone();
        self.conn
            .call(move |connection| {
                let mut stmt = connection.prepare(&format!("SELECT COUNT(1) FROM {}", emb_table_name))?;
                let count: usize = stmt.query_row([], |row| row.get(0))?;
                Ok(count)
            })
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn vecdb_records_add(&mut self, records: &Vec<VecdbRecord>) -> Result<(), String> {
        use crate::vdb_error::with_retry;
        use tokio::time::Duration;

        let records_owned = records.clone();
        let emb_table_name = self.emb_table_name.clone();

        with_retry(
            || {
                let records_owned = records_owned.clone();
                let emb_table_name = emb_table_name.clone();
                self.conn.call(move |connection| {
                    let tx = connection.transaction()?;
                    {
                        let mut stmt = tx.prepare(&format!(
                            "INSERT OR REPLACE INTO {} (embedding, scope, start_line, end_line) VALUES (?, ?, ?, ?)",
                            emb_table_name
                        ))?;
                        for record in &records_owned {
                            if let Some(vector) = &record.vector {
                                let embedding_bytes: Vec<u8> = vector.as_bytes().to_vec();
                                stmt.execute(rusqlite::params![
                                    embedding_bytes,
                                    record.file_path.to_string_lossy().to_string(),
                                    record.start_line,
                                    record.end_line
                                ])?;
                            }
                        }
                    }
                    tx.commit()?;
                    Ok(())
                })
            },
            3,
            Duration::from_millis(100),
            "add vector records",
        )
        .await
    }

    pub async fn vecdb_search(
        &mut self,
        embedding: &Vec<f32>,
        top_n: usize,
        vecdb_scope_filter_mb: Option<String>,
    ) -> Result<Vec<VecdbRecord>, String> {
        use crate::vdb_error::with_retry;
        use tokio::time::Duration;

        let embedding_bytes: Vec<u8> = embedding.as_bytes().to_vec();
        let emb_table_name = self.emb_table_name.clone();
        let scope_prefix = vecdb_scope_filter_mb.clone();

        let mut results = with_retry(
            || {
                let embedding_bytes = embedding_bytes.clone();
                let emb_table_name = emb_table_name.clone();
                let scope_prefix = scope_prefix.clone();

                self.conn.call(move |connection| {
                    let query = match &scope_prefix {
                        Some(_) => format!(
                            "SELECT scope, start_line, end_line, embedding, distance FROM {} WHERE embedding MATCH ? AND k=? ORDER BY distance",
                            emb_table_name
                        ),
                        None => format!(
                            "SELECT scope, start_line, end_line, embedding, distance FROM {} WHERE embedding MATCH ? AND k=? ORDER BY distance",
                            emb_table_name
                        ),
                    };

                    let mut stmt = connection.prepare(&query)?;
                    let params = match &scope_prefix {
                        Some(_) => rusqlite::params![&embedding_bytes, top_n],
                        None => rusqlite::params![&embedding_bytes, top_n],
                    };

                    let rows = stmt.query_map(params, |row| {
                        let vector_blob: Vec<u8> = row.get(3)?;
                        let vector: Vec<f32> = vector_blob
                            .chunks_exact(4)
                            .map(|b| f32::from_ne_bytes(b.try_into().unwrap()))
                            .collect();
                        Ok(VecdbRecord {
                            vector: Some(vector),
                            file_path: PathBuf::from(row.get::<_, String>(0)?),
                            start_line: row.get(1)?,
                            end_line: row.get(2)?,
                            distance: row.get(4)?,
                            usefulness: 0.0,
                        })
                    })?;

                    let mut results = Vec::new();
                    for row in rows {
                        results.push(row?);
                    }
                    Ok(results)
                })
            },
            3,
            Duration::from_millis(100),
            "vector search",
        )
        .await?;

        if let Some(prefix) = scope_prefix {
            results.retain(|r| r.file_path.to_string_lossy().starts_with(prefix.as_str()));
        }

        Ok(results)
    }

    pub async fn vecdb_records_remove(&mut self, scopes_to_remove: Vec<String>) -> Result<(), String> {
        use crate::vdb_error::with_retry;
        use tokio::time::Duration;

        if scopes_to_remove.is_empty() {
            return Ok(());
        }

        let placeholders: String = scopes_to_remove.iter().map(|_| "?").collect::<Vec<&str>>().join(",");
        let emb_table_name = self.emb_table_name.clone();

        with_retry(
            || {
                let scopes_to_remove = scopes_to_remove.clone();
                let emb_table_name = emb_table_name.clone();
                let placeholders = placeholders.clone();

                self.conn.call(move |connection| {
                    let tx = connection.transaction()?;
                    {
                        let mut stmt = tx.prepare(&format!(
                            "DELETE FROM {} WHERE scope IN ({})",
                            emb_table_name, placeholders
                        ))?;
                        stmt.execute(rusqlite::params_from_iter(scopes_to_remove.iter()))?;
                    }
                    tx.commit()?;
                    Ok(())
                })
            },
            3,
            Duration::from_millis(100),
            "remove vector records",
        )
        .await
    }
}

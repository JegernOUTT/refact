use std::path::{Path, PathBuf};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use super::types::*;

pub struct Store {
    pub conn: Connection,
    pub project: String,
}
fn safe(path: &Path, directory: bool) -> Result<(), Error> {
    match std::fs::symlink_metadata(path) {
        Ok(m) => {
            if m.file_type().is_symlink()
                || (directory && !m.is_dir())
                || (!directory && !m.is_file())
            {
                return Err(Error::denied());
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if !directory && m.nlink() != 1 {
                    return Err(Error::denied());
                }
            }
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(Error::storage(e)),
    }
}
impl Store {
    pub fn path(root: &Path) -> PathBuf {
        root.join(".refact/organization.sqlite")
    }
    pub fn open(root: &Path) -> Result<Self, Error> {
        let canonical = std::fs::canonicalize(root).map_err(Error::storage)?;
        let dir = canonical.join(".refact");
        safe(&dir, true)?;
        std::fs::create_dir_all(&dir).map_err(Error::storage)?;
        safe(&dir, true)?;
        let path = Self::path(&canonical);
        for suffix in ["", "-journal", "-wal", "-shm"] {
            safe(&PathBuf::from(format!("{}{suffix}", path.display())), false)?;
        }
        let conn = Connection::open_with_flags(
            &path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
                | rusqlite::OpenFlags::SQLITE_OPEN_CREATE
                | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX
                | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        safe(&path, false)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;
            CREATE TABLE IF NOT EXISTS meta (id INTEGER PRIMARY KEY CHECK(id=1), revision INTEGER NOT NULL, project TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS taxonomy (id TEXT PRIMARY KEY, kind TEXT NOT NULL, name TEXT NOT NULL, name_key TEXT NOT NULL, archived INTEGER NOT NULL DEFAULT 0, protected INTEGER NOT NULL DEFAULT 0);
            CREATE UNIQUE INDEX IF NOT EXISTS active_taxonomy_name ON taxonomy(kind,name_key) WHERE archived=0;
            CREATE TABLE IF NOT EXISTS assignments (item TEXT PRIMARY KEY, body TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS provenance (record TEXT PRIMARY KEY);
            CREATE TABLE IF NOT EXISTS requests (id TEXT PRIMARY KEY, payload TEXT NOT NULL, response TEXT NOT NULL);")?;
        let project = crate::coordination::roots::project_identity(&canonical);
        conn.execute("INSERT OR IGNORE INTO meta VALUES(1,0,?1)", [&project])?;
        let saved: String =
            conn.query_row("SELECT project FROM meta WHERE id=1", [], |r| r.get(0))?;
        if saved != project {
            return Err(Error::denied());
        }
        Ok(Self {
            conn,
            project,
        })
    }
    pub fn revision(&self) -> Result<u64, Error> {
        revision(&self.conn)
    }
    pub fn apply(
        &mut self,
        request: &ApplyRequest,
        authority: Authority,
    ) -> Result<ApplyResponse, Error> {
        self.apply_with_provenance(request, authority, &[])
    }
    pub fn apply_with_provenance(
        &mut self,
        request: &ApplyRequest,
        authority: Authority,
        records: &[refact_privacy::FileRecord],
    ) -> Result<ApplyResponse, Error> {
        self.apply_guarded(request, authority, records, || Ok(()))
    }
    pub fn apply_guarded(
        &mut self,
        request: &ApplyRequest,
        authority: Authority,
        records: &[refact_privacy::FileRecord],
        before_commit: impl FnOnce() -> Result<(), Error>,
    ) -> Result<ApplyResponse, Error> {
        if request.operations.len() > 100
            || request.operations.is_empty()
            || request.request_id.is_empty()
            || request.request_id.len() > 128
        {
            return Err(Error::invalid("invalid request bounds"));
        }
        let payload = serde_json::to_string(&(authority == Authority::User, request))?;
        if payload.len() > 128 * 1024 {
            return Err(Error::invalid("request exceeds 128 KiB"));
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some((old, response)) = tx
            .query_row(
                "SELECT payload,response FROM requests WHERE id=?1",
                [&request.request_id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()?
        {
            if old != payload {
                return Err(Error::conflict("request id reused with different payload"));
            }
            return Ok(serde_json::from_str(&response)?);
        }
        let current = revision(&tx)?;
        if current != request.expected_revision {
            return Err(Error::conflict("organization revision changed"));
        }
        let results = apply_ops(&tx, &request.operations, authority)?;
        merge_provenance(&tx, records)?;
        let revision = current
            .checked_add(1)
            .filter(|r| *r <= i64::MAX as u64)
            .ok_or_else(|| Error::conflict("revision exhausted"))?;
        let response = ApplyResponse {
            revision,
            results,
        };
        tx.execute("UPDATE meta SET revision=?1 WHERE id=1", [revision])?;
        tx.execute(
            "INSERT INTO requests VALUES(?1,?2,?3)",
            params![
                request.request_id,
                payload,
                serde_json::to_string(&response)?
            ],
        )?;
        before_commit()?;
        tx.commit()?;
        Ok(response)
    }
    pub fn taxonomy(
        &self,
        kind: TaxonomyKind,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<TaxonomyResponse, Error> {
        if !(1..=200).contains(&limit) {
            return Err(Error::invalid("limit must be 1..200"));
        }
        let _tx = self.conn.unchecked_transaction()?;
        let revision = self.revision()?;
        let after = if let Some(cursor) = cursor {
            if cursor.len() > 1024 {
                return Err(Error::invalid("invalid cursor"));
            }
            let (project, rev, k, after): (String, u64, String, String) =
                serde_json::from_str(cursor).map_err(|_| Error::invalid("invalid cursor"))?;
            if project != self.project || rev != revision || k != kind.key() {
                return Err(Error::conflict("stale or foreign taxonomy cursor"));
            }
            after
        } else {
            String::new()
        };
        let mut stmt = self.conn.prepare(
            "SELECT id,name,archived FROM taxonomy WHERE kind=?1 AND id>?2 ORDER BY id LIMIT ?3",
        )?;
        let mut items = stmt
            .query_map(params![kind.key(), after, limit + 1], |r| {
                Ok(Taxon {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    archived: r.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let more = items.len() > limit;
        items.truncate(limit);
        let next_cursor = if more {
            Some(serde_json::to_string(&(
                &self.project,
                revision,
                kind.key(),
                &items.last().ok_or_else(|| Error::storage("empty page"))?.id,
            ))?)
        } else {
            None
        };
        Ok(TaxonomyResponse {
            revision,
            items,
            next_cursor,
        })
    }
}
pub fn merge_provenance(
    conn: &Connection,
    records: &[refact_privacy::FileRecord],
) -> Result<(), Error> {
    for record in records {
        conn.execute(
            "INSERT OR IGNORE INTO provenance VALUES(?1)",
            [serde_json::to_string(record)?],
        )?;
    }
    Ok(())
}
pub fn provenance(conn: &Connection) -> Result<Vec<refact_privacy::FileRecord>, Error> {
    let mut stmt = conn.prepare("SELECT record FROM provenance ORDER BY record")?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    rows.iter()
        .map(|r| serde_json::from_str(r).map_err(Into::into))
        .collect()
}
pub fn revision(conn: &Connection) -> Result<u64, Error> {
    Ok(conn.query_row("SELECT revision FROM meta WHERE id=1", [], |r| r.get(0))?)
}
pub fn assignment(conn: &Connection, item: &Item) -> Result<Assignment, Error> {
    let body: Option<String> = conn
        .query_row(
            "SELECT body FROM assignments WHERE item=?1",
            [item.key()],
            |r| r.get(0),
        )
        .optional()?;
    Ok(match body {
        Some(body) => serde_json::from_str(&body)?,
        None => Assignment::empty(item.clone()),
    })
}
fn save(conn: &Connection, value: &Assignment) -> Result<(), Error> {
    conn.execute(
        "INSERT INTO assignments VALUES(?1,?2) ON CONFLICT(item) DO UPDATE SET body=excluded.body",
        params![value.item.key(), serde_json::to_string(value)?],
    )?;
    Ok(())
}
fn name(raw: &str) -> Result<String, Error> {
    let normalized = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty()
        || normalized.chars().count() > 120
        || normalized.chars().any(char::is_control)
    {
        return Err(Error::invalid("name must contain 1..120 characters"));
    }
    Ok(normalized)
}
fn taxon(conn: &Connection, id: &str, kind: TaxonomyKind) -> Result<(String, bool, bool), Error> {
    conn.query_row(
        "SELECT name,archived,protected FROM taxonomy WHERE id=?1 AND kind=?2",
        params![id, kind.key()],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .optional()?
    .ok_or_else(|| Error::invalid("taxonomy id not found"))
}
fn create(
    conn: &Connection,
    kind: TaxonomyKind,
    raw: &str,
    authority: Authority,
) -> Result<String, Error> {
    let name = name(raw)?;
    let id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO taxonomy(id,kind,name,name_key,protected) VALUES(?1,?2,?3,?4,?5)",
        params![
            id,
            kind.key(),
            name,
            name.to_lowercase(),
            authority == Authority::User
        ],
    )
    .map_err(|_| Error::conflict("active name already exists"))?;
    Ok(id)
}
fn rename(
    conn: &Connection,
    kind: TaxonomyKind,
    id: &str,
    raw: &str,
    authority: Authority,
) -> Result<(), Error> {
    let (old, archived, protected) = taxon(conn, id, kind)?;
    let name = name(raw)?;
    if old == name {
        if authority == Authority::User {
            conn.execute("UPDATE taxonomy SET protected=1 WHERE id=?1", [id])?;
        }
        return Ok(());
    }
    if archived || (protected && authority == Authority::Model) {
        return Err(Error::conflict("taxonomy name protected"));
    }
    conn.execute(
        "UPDATE taxonomy SET name=?1,name_key=?2,protected=?3 WHERE id=?4",
        params![
            name,
            name.to_lowercase(),
            protected || authority == Authority::User,
            id
        ],
    )
    .map_err(|_| Error::conflict("active name already exists"))?;
    Ok(())
}
fn archive(
    conn: &Connection,
    kind: TaxonomyKind,
    id: &str,
    authority: Authority,
) -> Result<(), Error> {
    let (_, archived, protected) = taxon(conn, id, kind)?;
    if archived {
        return Ok(());
    }
    if protected && authority == Authority::Model {
        return Err(Error::conflict("taxonomy protected"));
    }
    conn.execute("UPDATE taxonomy SET archived=1 WHERE id=?1", [id])?;
    Ok(())
}
fn merge(
    conn: &Connection,
    kind: TaxonomyKind,
    from: &str,
    into: &str,
    authority: Authority,
) -> Result<(), Error> {
    if from == into {
        taxon(conn, from, kind)?;
        return Ok(());
    }
    let (_, archived, protected) = taxon(conn, from, kind)?;
    let (_, target_archived, _) = taxon(conn, into, kind)?;
    if archived || target_archived || (protected && authority == Authority::Model) {
        return Err(Error::conflict("merge protected or archived taxonomy"));
    }
    let mut stmt = conn.prepare("SELECT body FROM assignments")?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    for body in rows {
        let mut a: Assignment = serde_json::from_str(&body)?;
        match kind {
            TaxonomyKind::Topics if a.topic_id.as_deref() == Some(from) => {
                if a.topic_locked {
                    return Err(Error::conflict("merge affects locked topic"));
                }
                a.topic_id = Some(into.into());
            }
            TaxonomyKind::Tags if a.tag_ids.iter().any(|id| id == from) => {
                if a.tags_locked {
                    return Err(Error::conflict("merge affects locked tags"));
                }
                a.tag_ids.retain(|id| id != from);
                a.tag_ids.push(into.into());
                a.tag_ids.sort();
                a.tag_ids.dedup();
            }
            _ => continue,
        }
        save(conn, &a)?;
    }
    archive(conn, kind, from, authority)
}
pub fn apply_ops(
    conn: &Connection,
    operations: &[Op],
    authority: Authority,
) -> Result<Vec<OpResult>, Error> {
    let mut results = Vec::new();
    for (op_index, op) in operations.iter().enumerate() {
        let id = match op {
            Op::CreateTopic {
                name,
            } => Some(create(conn, TaxonomyKind::Topics, name, authority)?),
            Op::CreateTag {
                name,
            } => Some(create(conn, TaxonomyKind::Tags, name, authority)?),
            Op::RenameTopic {
                id,
                name,
            } => {
                rename(conn, TaxonomyKind::Topics, id, name, authority)?;
                None
            }
            Op::RenameTag {
                id,
                name,
            } => {
                rename(conn, TaxonomyKind::Tags, id, name, authority)?;
                None
            }
            Op::ArchiveTopic {
                id,
            } => {
                archive(conn, TaxonomyKind::Topics, id, authority)?;
                None
            }
            Op::ArchiveTag {
                id,
            } => {
                archive(conn, TaxonomyKind::Tags, id, authority)?;
                None
            }
            Op::MergeTopic {
                from_id,
                into_id,
            } => {
                merge(conn, TaxonomyKind::Topics, from_id, into_id, authority)?;
                None
            }
            Op::MergeTag {
                from_id,
                into_id,
            } => {
                merge(conn, TaxonomyKind::Tags, from_id, into_id, authority)?;
                None
            }
            Op::AssignItem {
                item,
                topic_id,
                tag_ids,
                topic_locked,
                tags_locked,
            } => {
                if tag_ids.len() > 16 {
                    return Err(Error::invalid("at most 16 tags per item"));
                }
                let old = assignment(conn, item)?;
                let mut tags = tag_ids.clone();
                tags.sort();
                tags.dedup();
                if authority == Authority::Model
                    && ((topic_locked.is_some_and(|v| v != old.topic_locked))
                        || (tags_locked.is_some_and(|v| v != old.tags_locked))
                        || (old.topic_locked && old.topic_id != *topic_id)
                        || (old.tags_locked && old.tag_ids != tags))
                {
                    return Err(Error::conflict("manual assignment is protected"));
                }
                if let Some(id) = topic_id {
                    if old.topic_id.as_ref() != Some(id) && taxon(conn, id, TaxonomyKind::Topics)?.1
                    {
                        return Err(Error::conflict("archived topic"));
                    }
                }
                for id in &tags {
                    if !old.tag_ids.contains(id) && taxon(conn, id, TaxonomyKind::Tags)?.1 {
                        return Err(Error::conflict("archived tag"));
                    }
                }
                save(
                    conn,
                    &Assignment {
                        item: item.clone(),
                        topic_id: topic_id.clone(),
                        tag_ids: tags,
                        topic_locked: topic_locked.unwrap_or(old.topic_locked),
                        tags_locked: tags_locked.unwrap_or(old.tags_locked),
                    },
                )?;
                None
            }
        };
        results.push(OpResult {
            op_index,
            id,
        });
    }
    Ok(results)
}

use crate::*;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use std::{path::Path, sync::Mutex};

pub struct ReviewStore {
    pub(crate) connection: Mutex<Connection>,
    _directory: std::fs::File,
}
impl ReviewStore {
    pub fn open(trusted_root: &Path) -> Result<Self> {
        let root = std::fs::canonicalize(trusted_root)?;
        let mut directory = std::fs::File::open(&root)?;
        #[cfg(target_os = "linux")]
        let db = {
            use std::os::fd::{AsRawFd, FromRawFd};
            for part in [".refact", "reviews"] {
                let name = std::ffi::CString::new(part).unwrap();
                let fd = unsafe {
                    libc::mkdirat(directory.as_raw_fd(), name.as_ptr(), 0o700);
                    libc::openat(
                        directory.as_raw_fd(),
                        name.as_ptr(),
                        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                    )
                };
                if fd < 0 {
                    return Err(std::io::Error::last_os_error().into());
                }
                directory = unsafe { std::fs::File::from_raw_fd(fd) };
            }
            for name in [
                "reviews.sqlite",
                "reviews.sqlite-journal",
                "reviews.sqlite-wal",
                "reviews.sqlite-shm",
            ] {
                let p = format!("/proc/self/fd/{}/{}", directory.as_raw_fd(), name);
                if let Ok(m) = std::fs::symlink_metadata(&p) {
                    if !m.is_file() || m.file_type().is_symlink() {
                        return Err(invalid("unsafe store file"));
                    }
                }
            }
            format!("/proc/self/fd/{}/reviews.sqlite", directory.as_raw_fd())
        };
        #[cfg(not(target_os = "linux"))]
        let db = {
            let mut p = root;
            for part in [".refact", "reviews"] {
                p.push(part);
                if !p.exists() {
                    std::fs::create_dir(&p)?;
                }
                if std::fs::symlink_metadata(&p)?.file_type().is_symlink() {
                    return Err(invalid("symlink store"));
                }
            }
            p.push("reviews.sqlite");
            if let Ok(m) = std::fs::symlink_metadata(&p) {
                if !m.is_file() || m.file_type().is_symlink() {
                    return Err(invalid("unsafe store file"));
                }
            }
            p.to_string_lossy().to_string()
        };
        let mut c = Connection::open(db)?;
        c.busy_timeout(std::time::Duration::from_secs(10))?;
        c.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; CREATE TABLE IF NOT EXISTS reviews(id TEXT PRIMARY KEY, chat TEXT NOT NULL UNIQUE, document TEXT NOT NULL); CREATE TABLE IF NOT EXISTS requests(scope TEXT NOT NULL, key TEXT NOT NULL, fingerprint TEXT NOT NULL, response TEXT NOT NULL, PRIMARY KEY(scope,key)); CREATE TABLE IF NOT EXISTS blobs(digest TEXT PRIMARY KEY, data BLOB NOT NULL); CREATE TABLE IF NOT EXISTS captures(id TEXT PRIMARY KEY, document TEXT NOT NULL); CREATE TABLE IF NOT EXISTS object_revisions(review_id TEXT NOT NULL, revision INTEGER NOT NULL, document TEXT NOT NULL); CREATE TABLE IF NOT EXISTS deliveries(id TEXT PRIMARY KEY, review_id TEXT NOT NULL, document TEXT NOT NULL, delivered INTEGER NOT NULL);")?;
        c.execute_batch("CREATE UNIQUE INDEX IF NOT EXISTS object_revision_identity ON object_revisions(review_id, revision, json_extract(document,'$.object.kind'), json_extract(document,'$.object.value.id'));")?;
        crate::privacy::migrate(&mut c)?;
        crate::stacks::migrate(&mut c)?;
        Ok(Self {
            connection: Mutex::new(c),
            _directory: directory,
        })
    }
    pub fn create(
        &self,
        owner_chat_id: &str,
        title: &str,
        origin: Origin,
        request_key: &str,
    ) -> Result<Review> {
        self.create_internal(owner_chat_id, title, origin, request_key, vec![])
    }
    pub fn create_with_privacy(
        &self,
        owner_chat_id: &str,
        title: &str,
        origin: Origin,
        request_key: &str,
        privacy: Vec<refact_privacy::FileRecord>,
    ) -> Result<Review> {
        self.create_internal(owner_chat_id, title, origin, request_key, privacy)
    }
    fn create_internal(
        &self,
        owner_chat_id: &str,
        title: &str,
        origin: Origin,
        request_key: &str,
        privacy: Vec<refact_privacy::FileRecord>,
    ) -> Result<Review> {
        if owner_chat_id.is_empty() || request_key.is_empty() {
            return Err(invalid("missing identity/request key"));
        }
        let semantic = (owner_chat_id, title, &origin);
        let fingerprint = content_hash(&serde_json::to_vec(&semantic)?);
        let scope = format!("create:{owner_chat_id}");
        let mut c = self
            .connection
            .lock()
            .map_err(|_| invalid("poisoned store"))?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(r) = replay(&tx, &scope, request_key, &fingerprint)? {
            let r = crate::privacy::retain(&tx, r, &privacy)?;
            tx.commit()?;
            return Ok(r);
        }
        if let Some(id) = tx
            .query_row(
                "SELECT review_id FROM singleton_reviews WHERE chat=?1",
                [owner_chat_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        {
            let r = crate::privacy::retain(&tx, load(&tx, &id)?, &privacy)?;
            tx.execute(
                "INSERT INTO requests VALUES(?1,?2,?3,?4)",
                params![scope, request_key, fingerprint, format!("review:{id}")],
            )?;
            tx.commit()?;
            return Ok(r);
        }
        if crate::stacks::stack_for_chat(&tx, owner_chat_id)?.is_some() {
            return Err(invalid("stack chat requires create_member_review"));
        }
        let r = Review {
            privacy: crate::privacy::union(&[], &privacy)?,
            schema_version: SCHEMA_VERSION,
            id: uuid::Uuid::new_v4().to_string(),
            owner_chat_id: owner_chat_id.to_owned(),
            revision: 0,
            current_candidate_id: None,
            title: title.to_owned(),
            origin,
            target_reference: String::new(),
            candidates: vec![],
            tour: vec![],
            annotations: vec![],
            threads: vec![],
            evidence: vec![],
            runs: vec![],
            materials: vec![],
            activity: vec![],
            remote_links: vec![],
        };
        let document = serde_json::to_string(&r)?;
        tx.execute(
            "INSERT INTO reviews VALUES(?1,?2,?3)",
            params![r.id, owner_chat_id, document],
        )?;
        tx.execute(
            "INSERT INTO singleton_reviews VALUES(?1,?2)",
            params![owner_chat_id, r.id],
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO active_reviews VALUES(?1,?2)",
            params![owner_chat_id, r.id],
        )?;
        tx.execute(
            "INSERT INTO requests VALUES(?1,?2,?3,?4)",
            params![scope, request_key, fingerprint, document],
        )?;
        tx.commit()?;
        Ok(r)
    }
    pub fn get_for_chat(&self, owner_chat_id: &str) -> Result<Option<Review>> {
        let c = self
            .connection
            .lock()
            .map_err(|_| invalid("poisoned store"))?;
        let s: Option<String> = c
            .query_row(
                "SELECT r.document FROM active_reviews a JOIN reviews r ON r.id=a.review_id AND r.chat=a.chat WHERE a.chat=?1",
                [owner_chat_id],
                |r| r.get(0),
            )
            .optional()?;
        s.map(|s| decode(&s)).transpose()
    }
    pub fn load(&self, id: &str) -> Result<Review> {
        let c = self
            .connection
            .lock()
            .map_err(|_| invalid("poisoned store"))?;
        load(&c, id)
    }
    pub fn mutate(&self, id: &str, mutation: Mutation) -> Result<Review> {
        self.mutate_internal(id, mutation, None)
    }
    pub fn mutate_with_privacy(
        &self,
        id: &str,
        mutation: Mutation,
        privacy: Vec<refact_privacy::FileRecord>,
    ) -> Result<Review> {
        let mut c = self
            .connection
            .lock()
            .map_err(|_| invalid("poisoned store"))?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let result = mutate_with_privacy(&tx, id, mutation, None, &privacy)?;
        tx.commit()?;
        Ok(result)
    }
    /// Engine-only final activation: candidate journal and execution slot commit together.
    pub fn mutate_and_set_active(
        &self,
        owner: &str,
        id: &str,
        expected_active: Option<&str>,
        mutation: Mutation,
    ) -> Result<Review> {
        self.mutate_internal(id, mutation, Some((owner, expected_active)))
    }
    pub fn mutate_and_set_active_with_privacy(
        &self,
        owner: &str,
        id: &str,
        expected_active: Option<&str>,
        mutation: Mutation,
        privacy: Vec<refact_privacy::FileRecord>,
    ) -> Result<Review> {
        let mut c = self
            .connection
            .lock()
            .map_err(|_| invalid("poisoned store"))?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let result =
            mutate_with_privacy(&tx, id, mutation, Some((owner, expected_active)), &privacy)?;
        tx.commit()?;
        Ok(result)
    }
    fn mutate_internal(
        &self,
        id: &str,
        mutation: Mutation,
        activation: Option<(&str, Option<&str>)>,
    ) -> Result<Review> {
        let mut c = self
            .connection
            .lock()
            .map_err(|_| invalid("poisoned store"))?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let result = mutate_in_transaction(&tx, id, mutation, activation)?;
        tx.commit()?;
        Ok(result)
    }
    /// All member journals commit or roll back together; never changes execution slots.
    pub fn mutate_many(&self, mutations: Vec<(String, Mutation)>) -> Result<Vec<Review>> {
        if mutations.len() > MAX_FILES {
            return Err(Error::Limit("too many mutations".into()));
        }
        // Input order is semantic: one member may stage then activate with sequential CAS.
        let mut c = self
            .connection
            .lock()
            .map_err(|_| invalid("poisoned store"))?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let results = mutations
            .into_iter()
            .map(|(id, m)| mutate_in_transaction(&tx, &id, m, None))
            .collect::<Result<Vec<_>>>()?;
        tx.commit()?;
        Ok(results)
    }
    pub fn object_history(&self, review_id: &str) -> Result<Vec<ObjectRevision>> {
        let c = self
            .connection
            .lock()
            .map_err(|_| invalid("poisoned store"))?;
        let mut q =
            c.prepare("SELECT document FROM object_revisions WHERE review_id=?1 ORDER BY rowid")?;
        let rows = q.query_map([review_id], |row| row.get::<_, String>(0))?;
        rows.map(|r| Ok(serde_json::from_str(&r?)?)).collect()
    }
    pub fn pending_deliveries(&self, review_id: &str) -> Result<Vec<ReviewDelivery>> {
        let c = self
            .connection
            .lock()
            .map_err(|_| invalid("poisoned store"))?;
        let mut q = c.prepare(
            "SELECT document FROM deliveries WHERE review_id=?1 AND delivered=0 ORDER BY rowid",
        )?;
        let rows = q.query_map([review_id], |r| r.get::<_, String>(0))?;
        rows.map(|r| Ok(serde_json::from_str(&r?)?)).collect()
    }
    pub fn settle_delivery(&self, delivery_id: &str) -> Result<()> {
        let c = self
            .connection
            .lock()
            .map_err(|_| invalid("poisoned store"))?;
        if c.execute(
            "UPDATE deliveries SET delivered=1 WHERE id=?1",
            [delivery_id],
        )? == 0
        {
            return Err(Error::NotFound);
        }
        Ok(())
    }
    pub fn put_blob(&self, bytes: &[u8]) -> Result<BlobRef> {
        if bytes.len() > MAX_BLOB_BYTES {
            return Err(Error::Limit("blob exceeds 16MiB".into()));
        }
        let b = BlobRef {
            digest: content_hash(bytes),
            size: bytes.len() as u64,
        };
        self.connection
            .lock()
            .map_err(|_| invalid("poisoned store"))?
            .execute(
                "INSERT OR IGNORE INTO blobs VALUES(?1,?2)",
                params![b.digest, bytes],
            )?;
        Ok(b)
    }
    pub fn read_blob(&self, b: &BlobRef) -> Result<Vec<u8>> {
        let c = self
            .connection
            .lock()
            .map_err(|_| invalid("poisoned store"))?;
        blob(&c, b)
    }
}
fn decode(s: &str) -> Result<Review> {
    let r: Review = serde_json::from_str(s)?;
    if r.schema_version != SCHEMA_VERSION {
        return Err(invalid("unsupported schema version"));
    }
    Ok(r)
}
pub(crate) fn load(c: &Connection, id: &str) -> Result<Review> {
    let s: Option<String> = c
        .query_row("SELECT document FROM reviews WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .optional()?;
    decode(&s.ok_or(Error::NotFound)?)
}
fn replay(c: &Connection, scope: &str, key: &str, fingerprint: &str) -> Result<Option<Review>> {
    let row: Option<(String, String)> = c
        .query_row(
            "SELECT fingerprint,response FROM requests WHERE scope=?1 AND key=?2",
            params![scope, key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    match row {
        Some((f, s)) if f == fingerprint => Ok(Some(if let Some(id) = s.strip_prefix("review:") {
            load(c, id)?
        } else {
            decode(&s)?
        })),
        Some(_) => Err(Error::IdempotencyConflict),
        None => Ok(None),
    }
}
pub(crate) fn blob(c: &Connection, b: &BlobRef) -> Result<Vec<u8>> {
    let bytes: Option<Vec<u8>> = c
        .query_row("SELECT data FROM blobs WHERE digest=?1", [&b.digest], |r| {
            r.get(0)
        })
        .optional()?;
    let bytes = bytes.ok_or_else(|| Error::Integrity("missing referenced blob".into()))?;
    if bytes.len() as u64 != b.size || content_hash(&bytes) != b.digest {
        return Err(Error::Integrity("blob integrity failure".into()));
    }
    Ok(bytes)
}
fn put<T: Clone>(items: &mut Vec<T>, item: &T, id: impl Fn(&T) -> &str) {
    if let Some(old) = items.iter_mut().find(|x| id(x) == id(item)) {
        *old = item.clone();
    } else {
        items.push(item.clone());
    }
}
fn unique<T>(items: &[T], id: impl Fn(&T) -> &str) -> Result<()> {
    let mut ids = std::collections::HashSet::new();
    for item in items {
        if id(item).is_empty() || !ids.insert(id(item)) {
            return Err(invalid("empty/duplicate stable ID"));
        }
    }
    Ok(())
}
fn apply(c: &Connection, r: &mut Review, op: &Operation) -> Result<()> {
    match op {
        Operation::RequestRunCancel {
            run_id,
        } => {
            let run = r
                .runs
                .iter_mut()
                .find(|run| &run.id == run_id)
                .ok_or(Error::NotFound)?;
            if matches!(run.status, RunStatus::Queued | RunStatus::Running) {
                run.status = RunStatus::CancelRequested;
            }
        }

        Operation::Rename {
            title,
        } => r.title = title.clone(),
        Operation::SetTargetReference {
            target_reference,
        } => r.target_reference = target_reference.clone(),
        Operation::CreateThread {
            thread,
            contribution,
        } => {
            if r.threads.iter().any(|t| t.id == thread.id) {
                return Err(invalid("thread exists"));
            }
            apply(
                c,
                r,
                &Operation::PutThread {
                    thread: thread.clone(),
                },
            )?;
            apply(
                c,
                r,
                &Operation::AddContribution {
                    contribution: contribution.clone(),
                },
            )?;
        }
        Operation::ActivateSnapshot {
            candidate,
            imported,
            investigation,
            target_reference,
        } => {
            if imported.kind != MaterialKind::Imported
                || investigation.kind != MaterialKind::Investigation
                || imported.candidate_id != candidate.id
                || investigation.candidate_id != candidate.id
                || investigation.id != format!("activation:{}", candidate.id)
                || investigation
                    .worktree_id
                    .as_deref()
                    .is_none_or(str::is_empty)
                || investigation
                    .worktree_root
                    .as_deref()
                    .is_none_or(str::is_empty)
            {
                return Err(invalid("invalid snapshot activation binding"));
            }
            // These effects are deliberately within the caller's single SQLite transaction.
            // Any reconciliation, validation or publication failure rolls back all of them.
            apply(
                c,
                r,
                &Operation::StageCandidate {
                    candidate: candidate.clone(),
                },
            )?;
            apply(
                c,
                r,
                &Operation::AddMaterial {
                    material: imported.clone(),
                },
            )?;
            apply(
                c,
                r,
                &Operation::AddMaterial {
                    material: investigation.clone(),
                },
            )?;
            apply(
                c,
                r,
                &Operation::SetTargetReference {
                    target_reference: target_reference.clone(),
                },
            )?;
            apply(
                c,
                r,
                &Operation::ActivateCandidate {
                    candidate_id: candidate.id.clone(),
                },
            )?;
        }
        Operation::StageCandidate {
            candidate,
        } => {
            if r.candidates.iter().any(|x| x.id == candidate.id) {
                return Err(invalid("candidate immutable"));
            }
            let stored: Option<String> = c
                .query_row(
                    "SELECT document FROM captures WHERE id=?1",
                    [&candidate.id],
                    |row| row.get(0),
                )
                .optional()?;
            if stored.as_deref() != Some(&serde_json::to_string(candidate)?) {
                return Err(invalid("candidate must be captured by this store"));
            }
            r.candidates.push(candidate.clone());
        }
        Operation::ActivateCandidate {
            candidate_id,
        } => {
            let candidate = r
                .candidates
                .iter()
                .find(|x| &x.id == candidate_id)
                .ok_or(Error::NotFound)?
                .clone();
            crate::reconcile::activate(c, r, &candidate)?;
            r.current_candidate_id = Some(candidate_id.clone());
        }
        Operation::PutTourStep {
            step,
        } => {
            let mut step = step.clone();
            if let Some(old) = r.tour.iter().find(|x| x.id == step.id) {
                if Some(step.current.version) != old.current.version.checked_add(1) {
                    return Err(invalid("tour version must increment"));
                }
                preserve_anchors(&mut step.current.anchors, &old.current.anchors);
                step.history = old.history.clone();
                step.history.push(old.current.clone());
            } else {
                step.history.clear();
                step.current.version = 1;
            }
            let previous: Vec<_> = r
                .tour
                .iter()
                .filter(|s| s.id == step.id)
                .flat_map(|s| std::iter::once(&s.current).chain(s.history.iter()))
                .flat_map(|v| v.blocks.iter())
                .collect();
            crate::rich_blocks::normalize_revisions(&mut step.current.blocks, &previous)?;
            put(&mut r.tour, &step, |x| &x.id);
        }
        Operation::PutAnnotation {
            annotation,
        } => {
            let mut annotation = annotation.clone();
            if let Some(old) = r.annotations.iter().find(|x| x.id == annotation.id) {
                preserve_anchors(&mut annotation.anchors, &old.anchors);
            }
            put(&mut r.annotations, &annotation, |x| &x.id);
        }
        Operation::PutThread {
            thread,
        } => {
            if let Some(old) = r.threads.iter().find(|x| x.id == thread.id) {
                if old.target != thread.target {
                    return Err(invalid("discussion target immutable"));
                }
                if old.contributions != thread.contributions {
                    return Err(invalid("contributions append-only"));
                }
            } else if !thread.contributions.is_empty() {
                return Err(invalid("use add_contribution"));
            }
            let mut thread = thread.clone();
            if let Some(old) = r.threads.iter().find(|x| x.id == thread.id) {
                preserve_anchors(&mut thread.anchors, &old.anchors);
            }
            put(&mut r.threads, &thread, |x| &x.id);
        }
        Operation::AddContribution {
            contribution,
        } => {
            if r.threads
                .iter()
                .flat_map(|t| &t.contributions)
                .any(|x| x.id == contribution.id)
            {
                return Err(invalid("contribution immutable"));
            }
            let mut contribution = contribution.clone();
            crate::rich_blocks::normalize_revisions(&mut contribution.blocks, &[])?;
            r.threads
                .iter_mut()
                .find(|x| x.id == contribution.thread_id)
                .ok_or(Error::NotFound)?
                .contributions
                .push(contribution.clone());
            let delivery = ReviewDelivery {
                id: format!("{}:{}", r.id, contribution.id),
                review_id: r.id.clone(),
                contribution_id: contribution.id.clone(),
                owner_chat_id: r.owner_chat_id.clone(),
                delivered: false,
            };
            c.execute(
                "INSERT INTO deliveries VALUES(?1,?2,?3,0)",
                params![delivery.id, r.id, serde_json::to_string(&delivery)?],
            )?;
        }
        Operation::AddEvidence {
            evidence,
        } => {
            if r.evidence.iter().any(|x| x.id == evidence.id) {
                return Err(invalid("evidence immutable"));
            }
            r.evidence.push(evidence.clone());
        }
        Operation::AddMaterial {
            material,
        } => {
            if r.materials.iter().any(|x| x.id == material.id) {
                return Err(invalid("material immutable"));
            }
            r.materials.push(material.clone());
        }
        Operation::PutRun {
            run,
        } => {
            if let Some(old) = r.runs.iter().find(|x| x.id == run.id) {
                if old.candidate_id != run.candidate_id
                    || old.agent_id != run.agent_id
                    || old.started_at_ms != run.started_at_ms
                    || (old.status == RunStatus::CancelRequested
                        && matches!(run.status, RunStatus::Queued | RunStatus::Running))
                    || matches!(
                        old.status,
                        RunStatus::Completed
                            | RunStatus::Partial
                            | RunStatus::Failed
                            | RunStatus::Cancelled
                            | RunStatus::Interrupted
                    )
                {
                    return Err(invalid("run provenance/terminal state immutable"));
                }
            }
            put(&mut r.runs, run, |x| &x.id);
        }
        Operation::PutRemoteLink {
            link,
        } => {
            let known = match link.kind {
                RemoteLinkKind::Contribution => r
                    .threads
                    .iter()
                    .flat_map(|t| &t.contributions)
                    .any(|c| c.id == link.local_id),
                RemoteLinkKind::Annotation => r.annotations.iter().any(|a| a.id == link.local_id),
            };
            if !known || link.source.trim().is_empty() {
                return Err(invalid(
                    "remote link names an unknown local object or source",
                ));
            }
            if link.state != RemoteLinkState::Requested
                && link.remote_id.as_deref().is_none_or(str::is_empty)
            {
                return Err(invalid("remote link requires the remote identity"));
            }
            match r
                .remote_links
                .iter_mut()
                .find(|old| old.kind == link.kind && old.local_id == link.local_id)
            {
                Some(old) if old.state == RemoteLinkState::Imported => {
                    return Err(invalid("imported remote identity is immutable"))
                }
                Some(old) => *old = link.clone(),
                None => r.remote_links.push(link.clone()),
            }
        }
    }
    Ok(())
}
fn validate(c: &Connection, r: &Review) -> Result<()> {
    unique(&r.tour, |x| &x.id)?;
    unique(&r.annotations, |x| &x.id)?;
    unique(&r.threads, |x| &x.id)?;
    unique(&r.evidence, |x| &x.id)?;
    unique(&r.materials, |x| &x.id)?;
    unique(&r.runs, |x| &x.id)?;
    let candidate = |id: &str| -> Result<()> {
        if r.candidates.iter().any(|x| x.id == id) {
            Ok(())
        } else {
            Err(invalid("unknown candidate reference"))
        }
    };
    for run in &r.runs {
        candidate(&run.candidate_id)?;
    }
    for material in &r.materials {
        candidate(&material.candidate_id)?;
        for b in &material.artifacts {
            blob(c, b)?;
        }
    }
    for e in &r.evidence {
        candidate(&e.candidate_id)?;
        if e.applicability == Applicability::Current
            && (r.current_candidate_id.as_ref() != Some(&e.candidate_id)
                || matches!(&e.source, EvidenceSource::Source {anchor: Some(a), ..} if a.status != AnchorStatus::Current))
        {
            return Err(invalid("evidence cannot claim current applicability"));
        }
        if e.run_id.as_ref().is_some_and(|id| {
            !r.runs
                .iter()
                .any(|x| &x.id == id && x.candidate_id == e.candidate_id)
        }) || e.material_id.as_ref().is_some_and(|id| {
            !r.materials
                .iter()
                .any(|x| &x.id == id && x.candidate_id == e.candidate_id)
        }) {
            return Err(invalid("evidence provenance mismatch"));
        }
        if let EvidenceSource::Source {
            path,
            anchor,
        } = &e.source
        {
            crate::rich_blocks::validate_path(r, &e.candidate_id, path)?;
            if let Some(anchor) = anchor {
                if anchor.position.candidate_id != e.candidate_id || anchor.position.path != *path {
                    return Err(invalid("evidence anchor provenance mismatch"));
                }
                crate::reconcile::validate_anchor(c, r, anchor)?;
            }
        }
        if let EvidenceSource::Executed {
            stdout,
            stderr,
            ..
        } = &e.source
        {
            for b in stdout.iter().chain(stderr.iter()) {
                blob(c, b)?;
            }
        }
    }
    for a in &r.annotations {
        if a.applicability == Applicability::Current
            && (a.anchors.is_empty()
                || a.anchors.iter().any(|anchor| {
                    anchor.status != AnchorStatus::Current
                        || r.current_candidate_id.as_ref() != Some(&anchor.position.candidate_id)
                })
                || a.evidence_ids.iter().any(|id| {
                    r.evidence
                        .iter()
                        .any(|e| &e.id == id && e.applicability != Applicability::Current)
                })
                || a.run_id.as_ref().is_some_and(|id| {
                    r.runs.iter().any(|run| {
                        &run.id == id && r.current_candidate_id.as_ref() != Some(&run.candidate_id)
                    })
                }))
        {
            return Err(invalid("annotation cannot claim current applicability"));
        }

        for id in &a.evidence_ids {
            if !r.evidence.iter().any(|e| &e.id == id) {
                return Err(invalid("unknown evidence"));
            }
        }
        if a.run_id
            .as_ref()
            .is_some_and(|id| !r.runs.iter().any(|x| &x.id == id))
        {
            return Err(invalid("unknown run"));
        }
    }
    for step in &r.tour {
        for version in std::iter::once(&step.current).chain(step.history.iter()) {
            crate::rich_blocks::validate_version(c, r, version)?;
        }
    }
    for t in &r.threads {
        if let Some(target) = &t.target {
            crate::rich_blocks::resolve_tour_target(r, target)?;
            if t.anchors
                .iter()
                .any(|a| a.position.candidate_id != target.candidate_id)
            {
                return Err(invalid("discussion anchor candidate mismatch"));
            }
        }
        unique(&t.contributions, |x| &x.id)?;
        for contribution in &t.contributions {
            let bound = contribution
                .context
                .as_ref()
                .and_then(|c| c.candidate_id.as_deref())
                .or_else(|| t.target.as_ref().map(|t| t.candidate_id.as_str()));
            if let Some(target) = &t.target {
                if bound != Some(target.candidate_id.as_str()) {
                    return Err(invalid("contribution target candidate mismatch"));
                }
            }
            crate::rich_blocks::validate_blocks(c, r, &contribution.blocks, bound)?;
            if contribution.thread_id != t.id {
                return Err(invalid("contribution thread mismatch"));
            }
            if let Some(context) = &contribution.context {
                if let Some(id) = &context.candidate_id {
                    candidate(id)?;
                }
                if context.run_id.as_ref().is_some_and(|id| {
                    !r.runs.iter().any(|run| {
                        &run.id == id && context.candidate_id.as_ref() == Some(&run.candidate_id)
                    })
                }) {
                    return Err(invalid("contribution run mismatch"));
                }
            }
        }
        if t.annotation_id
            .as_ref()
            .is_some_and(|id| !r.annotations.iter().any(|x| &x.id == id))
        {
            return Err(invalid("unknown annotation"));
        }
    }
    for anchor in r
        .tour
        .iter()
        .flat_map(|s| &s.current.anchors)
        .chain(r.annotations.iter().flat_map(|a| &a.anchors))
        .chain(r.threads.iter().flat_map(|t| &t.anchors))
    {
        crate::reconcile::validate_anchor(c, r, anchor)?;
    }
    Ok(())
}

pub(crate) fn preserve_anchors(next: &mut [Anchor], previous: &[Anchor]) {
    for next in next {
        let matches: Vec<_> = previous
            .iter()
            .filter(|old| {
                old.position == next.position
                    || next.history.contains(&old.position)
                    || old.history.contains(&next.position)
            })
            .collect();
        if matches.len() == 1 {
            let old = matches[0];
            next.history = old.history.clone();
            if next.position != old.position {
                next.history.push(old.position.clone());
            }
        }
    }
}
fn changed_objects(old: &Review, next: &Review) -> Vec<HistoricalObject> {
    let mut objects = vec![];
    for a in &old.annotations {
        if !next.annotations.iter().any(|n| n == a) {
            objects.push(HistoricalObject::Annotation(a.clone()));
        }
    }
    for t in &old.threads {
        if !next.threads.iter().any(|n| {
            n.id == t.id
                && n.anchors == t.anchors
                && n.annotation_id == t.annotation_id
                && n.resolved == t.resolved
                && n.target == t.target
        }) {
            objects.push(HistoricalObject::Thread(t.clone()));
        }
    }
    for t in &old.tour {
        if !next.tour.iter().any(|n| n == t) {
            objects.push(HistoricalObject::TourStep(t.clone()));
        }
    }
    objects
}

pub(crate) fn mutate_in_transaction(
    c: &Connection,
    id: &str,
    mutation: Mutation,
    activation: Option<(&str, Option<&str>)>,
) -> Result<Review> {
    mutate_with_privacy(c, id, mutation, activation, &[])
}
fn mutate_with_privacy(
    c: &Connection,
    id: &str,
    mutation: Mutation,
    activation: Option<(&str, Option<&str>)>,
    privacy: &[refact_privacy::FileRecord],
) -> Result<Review> {
    if mutation.request_key.is_empty() {
        return Err(invalid("missing request key"));
    }
    match &mutation.operation {
        Operation::PutTourStep {
            step,
        } => {
            let stored = load(c, id)?;
            let history = stored
                .tour
                .iter()
                .find(|old| old.id == step.id)
                .map(|old| old.history.as_slice())
                .unwrap_or(&[]);
            if !history.starts_with(&step.history) {
                return Err(invalid("tour history protected"));
            }
        }
        _ => {}
    }
    let mut semantic_operation = mutation.operation.clone();
    match &mut semantic_operation {
        Operation::CreateThread {
            contribution, ..
        }
        | Operation::AddContribution {
            contribution,
        } => {
            contribution.created_at_ms = 0;
            contribution.chat_message_id = None;
            contribution.context = None;
        }
        Operation::AddEvidence {
            evidence,
        } => evidence.created_at_ms = 0,
        _ => {}
    }
    let semantic = (
        mutation.expected_revision,
        &mutation.request_key,
        &mutation.actor,
        &semantic_operation,
    );
    // Keep legacy mutation fingerprints byte-compatible when not switching slots.
    let fingerprint = content_hash(&if activation.is_some() {
        serde_json::to_vec(&(semantic, activation))?
    } else {
        serde_json::to_vec(&semantic)?
    });
    if let Some(r) = replay(c, id, &mutation.request_key, &fingerprint)? {
        return crate::privacy::retain(c, r, privacy);
    }
    let mut r = load(c, id)?;
    if let Some((owner, expected)) = activation {
        crate::stacks::activate_checked(c, owner, &r, expected)?;
    }
    if r.revision != mutation.expected_revision {
        return Err(Error::Conflict {
            expected: mutation.expected_revision,
            actual: r.revision,
        });
    }
    match &mutation.operation {
        Operation::CreateThread {
            contribution, ..
        }
        | Operation::AddContribution {
            contribution,
        } if contribution.actor != mutation.actor => {
            return Err(invalid("contribution actor must equal mutation actor"))
        }
        _ => {}
    }
    let previous = r.clone();
    r.privacy = crate::privacy::union(&r.privacy, privacy)?;
    apply(c, &mut r, &mutation.operation)?;
    validate(c, &r)?;
    r.revision = r
        .revision
        .checked_add(1)
        .ok_or_else(|| invalid("revision overflow"))?;
    r.activity.push(Activity {
        id: uuid::Uuid::new_v4().to_string(),
        at_ms: mutation.at_ms,
        actor: mutation.actor.clone(),
        action: serde_json::to_value(&mutation.operation)?["op"]
            .as_str()
            .unwrap_or("mutation")
            .to_owned(),
        candidate_id: r.current_candidate_id.clone(),
    });
    let document = serde_json::to_string(&r)?;
    for object in changed_objects(&previous, &r) {
        let record = ObjectRevision {
            review_id: id.to_owned(),
            revision: previous.revision,
            actor: mutation.actor.clone(),
            at_ms: mutation.at_ms,
            object,
        };
        c.execute(
            "INSERT INTO object_revisions VALUES(?1,?2,?3)",
            params![id, previous.revision, serde_json::to_string(&record)?],
        )?;
    }
    c.execute(
        "UPDATE reviews SET document=?1 WHERE id=?2",
        params![document, id],
    )?;
    c.execute(
        "INSERT INTO requests VALUES(?1,?2,?3,?4)",
        params![
            id,
            mutation.request_key,
            fingerprint,
            format!("review:{id}")
        ],
    )?;
    Ok(r)
}

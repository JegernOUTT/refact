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
        c.execute_batch("CREATE TABLE IF NOT EXISTS request_intakes(scope TEXT NOT NULL, key TEXT NOT NULL, document TEXT NOT NULL, PRIMARY KEY(scope,key));")?;
        c.execute_batch("CREATE TABLE IF NOT EXISTS proposal_turn_completions(proposal_id TEXT PRIMARY KEY, owner TEXT NOT NULL, document TEXT NOT NULL, consumed INTEGER NOT NULL DEFAULT 0);")?;
        crate::stacks::migrate(&mut c)?;
        Ok(Self {
            connection: Mutex::new(c),
            _directory: directory,
        })
    }
    pub fn set_completion_busy_timeout_zero(&self) -> Result<()> {
        self.connection
            .lock()
            .map_err(|_| invalid("poisoned store"))?
            .busy_timeout(std::time::Duration::ZERO)?;
        Ok(())
    }

    pub fn proposal_completion(
        &self,
        owner: &str,
        proposal_id: &str,
    ) -> Result<Option<ProposalTurnCompletion>> {
        let c = self
            .connection
            .lock()
            .map_err(|_| invalid("poisoned store"))?;
        let document: Option<String> = c
            .query_row(
                "SELECT document FROM proposal_turn_completions WHERE owner=?1 AND proposal_id=?2",
                params![owner, proposal_id],
                |r| r.get(0),
            )
            .optional()?;
        document.map(|s| Ok(serde_json::from_str(&s)?)).transpose()
    }
    pub fn proposal_completions(&self, owner: &str) -> Result<Vec<ProposalTurnCompletion>> {
        let c = self
            .connection
            .lock()
            .map_err(|_| invalid("poisoned store"))?;
        let mut stmt = c.prepare(
            "SELECT document FROM proposal_turn_completions WHERE owner=?1 AND consumed=0",
        )?;
        let rows = stmt.query_map([owner], |row| row.get::<_, String>(0))?;
        rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
    }
    pub fn acknowledge_proposal_result(
        &self,
        record: &ProposalTurnCompletion,
        ids: &[String],
    ) -> Result<()> {
        if ids.is_empty() || ids.iter().any(String::is_empty) {
            return Err(invalid("missing proposal result"));
        }
        let mut c = self
            .connection
            .lock()
            .map_err(|_| invalid("poisoned store"))?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (document, consumed): (String, bool) = tx.query_row(
            "SELECT document,consumed FROM proposal_turn_completions WHERE proposal_id=?1 AND owner=?2",
            params![record.proposal_id, record.owner_chat_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        let stored: ProposalTurnCompletion = serde_json::from_str(&document)?;
        let mut updated = record.clone();
        updated.result_message_ids = stored.result_message_ids.clone();
        if updated != stored
            || (!record.result_message_ids.is_empty() && record.result_message_ids != ids)
            || (!stored.result_message_ids.is_empty() && stored.result_message_ids != ids)
        {
            return Err(invalid("proposal completion acknowledgement mismatch"));
        }
        if stored.result_message_ids.is_empty() {
            if consumed {
                return Err(invalid("proposal completion already consumed"));
            }
            updated.result_message_ids = ids.to_vec();
            tx.execute("UPDATE proposal_turn_completions SET document=?1 WHERE proposal_id=?2 AND document=?3 AND consumed=0",
                params![serde_json::to_string(&updated)?, record.proposal_id, document])?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn consume_proposal_completion(&self, record: &ProposalTurnCompletion) -> Result<bool> {
        let mut c = self
            .connection
            .lock()
            .map_err(|_| invalid("poisoned store"))?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let review = load(&tx, &record.review_id)?;
        let active: Option<String> = tx
            .query_row(
                "SELECT review_id FROM active_reviews WHERE chat=?1",
                [&record.owner_chat_id],
                |r| r.get(0),
            )
            .optional()?;
        let proposal = review
            .tour_revision_proposals
            .iter()
            .find(|p| p.id == record.proposal_id);
        let Some(proposal) = proposal else {
            return Ok(false);
        };
        let contribution = review
            .threads
            .iter()
            .find(|t| t.id == proposal.thread_id && t.target.as_ref() == Some(&proposal.target))
            .and_then(|t| {
                t.contributions
                    .iter()
                    .find(|c| c.id == record.contribution_id)
            });
        let settled: Option<(String, String)> = tx
            .query_row(
                "SELECT review_id,document FROM deliveries WHERE id=?1 AND delivered=1",
                [&record.delivery_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let delivery_matches = if let Some((id, document)) = settled {
            let delivery: ReviewDelivery = serde_json::from_str(&document)?;
            id == review.id
                && delivery.review_id == review.id
                && delivery.owner_chat_id == record.owner_chat_id
                && delivery.contribution_id == record.contribution_id
                && delivery.id == record.delivery_id
        } else {
            false
        };
        if active.as_deref() != Some(&record.review_id)
            || review.owner_chat_id != record.owner_chat_id
            || review.current_candidate_id.as_deref() != Some(&record.candidate_id)
            || proposal.target.candidate_id != record.candidate_id
            || proposal.acceptance.is_some()
            || proposal.fulfilment.is_some()
            || require_current_target(&review, &proposal.target).is_err()
            || proposal.proposal_contribution_id != record.contribution_id
            || record.delivery_id != format!("{}:{}", review.id, record.contribution_id)
            || !contribution.is_some_and(|c| {
                c.actor.kind == ActorKind::Agent
                    && c.actor.id == record.owner_chat_id
                    && c.actor == proposal.proposed_by
            })
            || !delivery_matches
            || record.result_message_ids.is_empty()
            || record.result_message_ids.iter().any(String::is_empty)
        {
            return Ok(false);
        }
        let consumed = tx.execute("UPDATE proposal_turn_completions SET consumed=1 WHERE proposal_id=?1 AND owner=?2 AND document=?3 AND consumed=0",
            params![record.proposal_id, record.owner_chat_id, serde_json::to_string(record)?])? == 1;
        tx.commit()?;
        Ok(consumed)
    }
    pub fn intake_for_request(
        &self,
        scope: &str,
        key: &str,
    ) -> Result<Option<ContextIntakeReceipt>> {
        let c = self
            .connection
            .lock()
            .map_err(|_| invalid("poisoned store"))?;
        let doc: Option<String> = c
            .query_row(
                "SELECT document FROM request_intakes WHERE scope=?1 AND key=?2",
                params![scope, key],
                |r| r.get(0),
            )
            .optional()?;
        doc.map(|doc| Ok(serde_json::from_str(&doc)?)).transpose()
    }
    pub fn create(
        &self,
        owner_chat_id: &str,
        title: &str,
        origin: Origin,
        request_key: &str,
    ) -> Result<Review> {
        self.create_internal(owner_chat_id, title, origin, request_key, None)
    }
    pub fn create_with_intake(
        &self,
        owner_chat_id: &str,
        title: &str,
        origin: Origin,
        request_key: &str,
        receipt: ContextIntakeReceipt,
    ) -> Result<Review> {
        self.create_internal(owner_chat_id, title, origin, request_key, Some(receipt))
    }
    fn create_internal(
        &self,
        owner_chat_id: &str,
        title: &str,
        origin: Origin,
        request_key: &str,
        intake: Option<ContextIntakeReceipt>,
    ) -> Result<Review> {
        if owner_chat_id.is_empty() || request_key.is_empty() {
            return Err(invalid("missing identity/request key"));
        }
        let semantic = (owner_chat_id, title, &origin);
        let fingerprint = content_hash(&if let Some(receipt) = &intake {
            serde_json::to_vec(&(semantic, crate::intake::intake_semantic(receipt)))?
        } else {
            serde_json::to_vec(&semantic)?
        });
        let scope = format!("create:{owner_chat_id}");
        let mut c = self
            .connection
            .lock()
            .map_err(|_| invalid("poisoned store"))?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(r) = replay(&tx, &scope, request_key, &fingerprint)? {
            return Ok(r);
        }
        if let Some(receipt) = &intake {
            crate::intake::bind_intake(receipt, "before_review", owner_chat_id, None)?;
        }
        if let Some(id) = tx
            .query_row(
                "SELECT review_id FROM singleton_reviews WHERE chat=?1",
                [owner_chat_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        {
            let r = load(&tx, &id)?;
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
            context_intake: intake,
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
            tour_revision_proposals: vec![],
            annotations: vec![],
            threads: vec![],
            evidence: vec![],
            runs: vec![],
            materials: vec![],
            activity: vec![],
        };
        let document = serde_json::to_string(&r)?;
        if let Some(receipt) = &r.context_intake {
            tx.execute(
                "INSERT INTO request_intakes VALUES(?1,?2,?3)",
                params![scope, request_key, serde_json::to_string(receipt)?],
            )?;
        }
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
    pub fn mutate_with_intake(
        &self,
        id: &str,
        mutation: Mutation,
        receipt: ContextIntakeReceipt,
    ) -> Result<Review> {
        if !matches!(mutation.operation, Operation::PutTourStep { .. }) {
            return Err(invalid("context intake requires put tour step"));
        }
        self.mutate_intake_internal(None, id, mutation, None, receipt)
    }
    pub fn mutate_current_tour_revision_with_intake(
        &self,
        owner: &str,
        id: &str,
        mutation: Mutation,
        context: Option<ContributionContext>,
        receipt: ContextIntakeReceipt,
    ) -> Result<Review> {
        if !matches!(mutation.operation, Operation::ApplyTourRevision { .. }) || context.is_some() {
            return Err(invalid("context intake requires apply tour revision"));
        }
        self.mutate_intake_internal(Some(owner), id, mutation, context, receipt)
    }
    fn mutate_intake_internal(
        &self,
        owner: Option<&str>,
        id: &str,
        mutation: Mutation,
        context: Option<ContributionContext>,
        receipt: ContextIntakeReceipt,
    ) -> Result<Review> {
        let mut c = self
            .connection
            .lock()
            .map_err(|_| invalid("poisoned store"))?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(owner) = owner {
            let review = load(&tx, id)?;
            let active: Option<String> = tx
                .query_row(
                    "SELECT review_id FROM active_reviews WHERE chat=?1",
                    [owner],
                    |row| row.get(0),
                )
                .optional()?;
            if review.owner_chat_id != owner || active.as_deref() != Some(id) {
                return Err(invalid("tour revision requires active owner review"));
            }
        }
        let result = mutate_with_intake_context(&tx, id, mutation, None, context, Some(receipt))?;
        tx.commit()?;
        Ok(result)
    }
    pub fn mutate_current_tour_revision(
        &self,
        owner: &str,
        id: &str,
        mutation: Mutation,
        context: Option<ContributionContext>,
    ) -> Result<Review> {
        self.mutate_current_tour_revision_with_completion(owner, id, mutation, context, None)
    }
    pub fn mutate_current_tour_revision_with_completion(
        &self,
        owner: &str,
        id: &str,
        mutation: Mutation,
        context: Option<ContributionContext>,
        intent: Option<ProposalTurnCompletionIntent>,
    ) -> Result<Review> {
        if intent.is_some() && !matches!(mutation.operation, Operation::ProposeTourRevision { .. })
        {
            return Err(invalid("completion intent requires proposal"));
        }
        if !matches!(
            mutation.operation,
            Operation::ProposeTourRevision { .. }
                | Operation::AcceptTourRevision { .. }
                | Operation::ApplyTourRevision { .. }
        ) {
            return Err(invalid("current tour revision requires revision operation"));
        }
        if context.is_some() && !matches!(mutation.operation, Operation::ProposeTourRevision { .. })
        {
            return Err(invalid("proposal context requires propose tour revision"));
        }
        let mut c = self
            .connection
            .lock()
            .map_err(|_| invalid("poisoned store"))?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let review = load(&tx, id)?;
        let active: Option<String> = tx
            .query_row(
                "SELECT review_id FROM active_reviews WHERE chat=?1",
                [owner],
                |row| row.get(0),
            )
            .optional()?;
        if review.owner_chat_id != owner || active.as_deref() != Some(id) {
            return Err(invalid("tour revision requires active owner review"));
        }
        let replayed: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM requests WHERE scope=?1 AND key=?2)",
            params![id, mutation.request_key],
            |r| r.get(0),
        )?;
        let result = mutate_with_context(&tx, id, mutation, None, context)?;
        if !replayed {
            if let Some(intent) = intent {
                if intent.owner_chat_id != owner
                    || intent.turn_id.is_empty()
                    || intent.tool_call_id.is_empty()
                    || intent.job_id.as_ref().is_some_and(String::is_empty)
                    || intent
                        .human_message_id
                        .as_ref()
                        .is_some_and(String::is_empty)
                {
                    return Err(invalid("invalid proposal completion intent"));
                }
                let proposal = result
                    .tour_revision_proposals
                    .iter()
                    .find(|p| {
                        !review
                            .tour_revision_proposals
                            .iter()
                            .any(|old| old.id == p.id)
                    })
                    .ok_or_else(|| invalid("completion requires new proposal"))?;
                let record = ProposalTurnCompletion {
                    owner_chat_id: intent.owner_chat_id,
                    turn_id: intent.turn_id,
                    tool_call_id: intent.tool_call_id,
                    job_id: intent.job_id,
                    human_message_id: intent.human_message_id,
                    review_id: result.id.clone(),
                    candidate_id: proposal.target.candidate_id.clone(),
                    proposal_id: proposal.id.clone(),
                    contribution_id: proposal.proposal_contribution_id.clone(),
                    delivery_id: format!("{}:{}", result.id, proposal.proposal_contribution_id),
                    result_message_ids: vec![],
                };
                tx.execute("INSERT INTO proposal_turn_completions(proposal_id,owner,document) VALUES(?1,?2,?3)",
                    params![record.proposal_id, owner, serde_json::to_string(&record)?])?;
            }
        }
        tx.commit()?;
        Ok(result)
    }
    pub fn mutate_tour_revision_with_context(
        &self,
        id: &str,
        mutation: Mutation,
        context: ContributionContext,
    ) -> Result<Review> {
        if !matches!(mutation.operation, Operation::ProposeTourRevision { .. }) {
            return Err(invalid("proposal context requires propose tour revision"));
        }
        let mut c = self
            .connection
            .lock()
            .map_err(|_| invalid("poisoned store"))?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let result = mutate_with_context(&tx, id, mutation, None, Some(context))?;
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
        Operation::ProposeTourRevision {
            ..
        }
        | Operation::AcceptTourRevision {
            ..
        }
        | Operation::ApplyTourRevision {
            ..
        } => return Err(invalid("revision requires mutation context")),
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
            if r.candidates.is_empty() {
                if let Some(receipt) = &r.context_intake {
                    crate::intake::bind_intake(
                        receipt,
                        "before_review",
                        &r.owner_chat_id,
                        Some(candidate),
                    )?;
                }
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
    }
    Ok(())
}
fn apply_revision(
    c: &Connection,
    r: &mut Review,
    m: &Mutation,
    context: Option<ContributionContext>,
) -> Result<()> {
    let stable_id = |role: &str| -> Result<String> {
        Ok(format!(
            "tour-revision:{role}:{}",
            content_hash(&serde_json::to_vec(&(&r.id, &m.request_key, role))?)
        ))
    };
    let owner_agent = || -> Result<()> {
        if m.actor.kind != ActorKind::Agent || m.actor.id != r.owner_chat_id {
            return Err(invalid("tour revision requires owner agent"));
        }
        Ok(())
    };
    let contribution =
        |id: String, thread_id: String, body: String, blocks: Vec<RichBlock>| Contribution {
            id,
            thread_id,
            body,
            blocks,
            actor: m.actor.clone(),
            created_at_ms: m.at_ms,
            chat_message_id: None,
            context: None,
        };
    match &m.operation {
        Operation::ProposeTourRevision {
            target,
            thread_id,
            change_request,
            body,
            blocks,
        } => {
            owner_agent()?;
            if change_request.trim().is_empty() || (body.trim().is_empty() && blocks.is_empty()) {
                return Err(invalid("empty tour revision proposal"));
            }
            require_current_target(r, target)?;
            let thread = r
                .threads
                .iter()
                .find(|t| &t.id == thread_id)
                .ok_or(Error::NotFound)?;
            if thread.target.as_ref() != Some(target) {
                return Err(invalid("proposal thread target mismatch"));
            }
            let proposal = TourRevisionProposal {
                id: stable_id("proposal")?,
                target: target.clone(),
                thread_id: thread_id.clone(),
                proposal_contribution_id: stable_id("proposal-contribution")?,
                change_request: change_request.clone(),
                proposed_by: m.actor.clone(),
                proposed_at_ms: m.at_ms,
                acceptance: None,
                fulfilment: None,
            };
            if context.as_ref().is_some_and(|context| {
                context.candidate_id.as_ref() != Some(&target.candidate_id)
                    || context.run_id.is_some()
                    || context.applicability != Applicability::Current
            }) {
                return Err(invalid("proposal context provenance mismatch"));
            }
            let mut receipt = contribution(
                proposal.proposal_contribution_id.clone(),
                thread_id.clone(),
                body.clone(),
                blocks.clone(),
            );
            receipt.context = context;
            apply(
                c,
                r,
                &Operation::AddContribution {
                    contribution: receipt,
                },
            )?;
            r.tour_revision_proposals.push(proposal);
        }
        Operation::AcceptTourRevision {
            proposal_id,
            target,
        } => {
            if m.actor.kind != ActorKind::User {
                return Err(invalid("tour revision acceptance requires user"));
            }
            let index = r
                .tour_revision_proposals
                .iter()
                .position(|p| &p.id == proposal_id)
                .ok_or(Error::NotFound)?;
            let proposal = &r.tour_revision_proposals[index];
            if &proposal.target != target {
                return Err(invalid("acceptance target mismatch"));
            }
            require_current_target(r, target)?;
            if proposal.acceptance.is_some() || proposal.fulfilment.is_some() {
                return Err(invalid("tour revision already accepted"));
            }
            let receipt = contribution(
                stable_id("acceptance-contribution")?,
                proposal.thread_id.clone(),
                format!("Accepted Tour revision: {}", proposal.change_request),
                vec![],
            );
            let acceptance = TourRevisionAcceptance {
                actor: m.actor.clone(),
                accepted_at_ms: m.at_ms,
                contribution_id: receipt.id.clone(),
            };
            apply(
                c,
                r,
                &Operation::AddContribution {
                    contribution: receipt,
                },
            )?;
            r.tour_revision_proposals[index].acceptance = Some(acceptance);
        }
        Operation::ApplyTourRevision {
            proposal_id,
            current,
        } => {
            owner_agent()?;
            let index = r
                .tour_revision_proposals
                .iter()
                .position(|p| &p.id == proposal_id)
                .ok_or(Error::NotFound)?;
            let proposal = &r.tour_revision_proposals[index];
            if proposal.acceptance.is_none() || proposal.fulfilment.is_some() {
                return Err(invalid("tour revision must be accepted and unfulfilled"));
            }
            require_current_target(r, &proposal.target)?;
            if Some(current.version) != proposal.target.version.checked_add(1)
                || current.candidate_id.as_ref() != Some(&proposal.target.candidate_id)
            {
                return Err(invalid("tour revision authored version/candidate mismatch"));
            }
            let step_id = proposal.target.step_id.clone();
            let old = r
                .tour
                .iter()
                .find(|s| s.id == step_id)
                .ok_or(Error::NotFound)?
                .current
                .clone();
            apply(
                c,
                r,
                &Operation::PutTourStep {
                    step: TourStep {
                        id: step_id.clone(),
                        current: current.clone(),
                        history: vec![],
                    },
                },
            )?;
            let mut normalized = r
                .tour
                .iter()
                .find(|s| s.id == step_id)
                .unwrap()
                .current
                .clone();
            normalized.version = old.version;
            normalized.context_intake = old.context_intake.clone();
            // A revision number alone is not authored content.
            if normalized == old {
                return Err(invalid("tour revision content unchanged"));
            }
            r.tour_revision_proposals[index].fulfilment = Some(TourRevisionFulfilment {
                actor: m.actor.clone(),
                fulfilled_at_ms: m.at_ms,
                version: current.version,
            });
        }
        _ => apply(c, r, &m.operation)?,
    }
    Ok(())
}
fn require_current_target(r: &Review, target: &TourDiscussionTarget) -> Result<()> {
    crate::rich_blocks::resolve_tour_target(r, target)?;
    if !crate::rich_blocks::is_tour_revision_target_current(r, target) {
        return Err(invalid("stale tour revision target"));
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
    mutate_with_context(c, id, mutation, activation, None)
}
fn mutate_with_context(
    c: &Connection,
    id: &str,
    mutation: Mutation,
    activation: Option<(&str, Option<&str>)>,
    context: Option<ContributionContext>,
) -> Result<Review> {
    mutate_with_intake_context(c, id, mutation, activation, context, None)
}
fn mutate_with_intake_context(
    c: &Connection,
    id: &str,
    mut mutation: Mutation,
    activation: Option<(&str, Option<&str>)>,
    context: Option<ContributionContext>,
    intake: Option<ContextIntakeReceipt>,
) -> Result<Review> {
    if mutation.request_key.is_empty() {
        return Err(invalid("missing request key"));
    }
    match &mutation.operation {
        Operation::PutTourStep {
            step,
        } => {
            if step.current.context_intake.is_some() {
                return Err(invalid("runtime-owned context intake"));
            }
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
        Operation::ApplyTourRevision {
            current, ..
        } if current.context_intake.is_some() => {
            return Err(invalid("runtime-owned context intake"))
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
    let fingerprint = content_hash(&if let Some(receipt) = &intake {
        serde_json::to_vec(&(
            semantic,
            activation,
            crate::intake::intake_semantic(receipt),
        ))?
    } else if activation.is_some() {
        serde_json::to_vec(&(semantic, activation))?
    } else {
        serde_json::to_vec(&semantic)?
    });
    if let Some(r) = replay(c, id, &mutation.request_key, &fingerprint)? {
        return Ok(r);
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
    if let Some(receipt) = &intake {
        let candidate = r
            .current_candidate_id
            .as_ref()
            .and_then(|id| r.candidates.iter().find(|c| &c.id == id));
        crate::intake::bind_intake(&receipt, "before_tour", &r.owner_chat_id, candidate)?;
        match &mut mutation.operation {
            Operation::PutTourStep {
                step,
            } => {
                if step.current.candidate_id != receipt.candidate_id {
                    return Err(invalid("authored tour intake candidate mismatch"));
                }
                step.current.context_intake = Some(receipt.clone());
            }
            Operation::ApplyTourRevision {
                current, ..
            } => {
                if current.candidate_id != receipt.candidate_id {
                    return Err(invalid("authored tour intake candidate mismatch"));
                }
                current.context_intake = Some(receipt.clone());
            }
            _ => return Err(invalid("context intake requires tour authoring")),
        }
    }
    apply_revision(c, &mut r, &mutation, context)?;
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
    if let Some(receipt) = &intake {
        c.execute(
            "INSERT INTO request_intakes VALUES(?1,?2,?3)",
            params![id, mutation.request_key, serde_json::to_string(receipt)?],
        )?;
    }
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

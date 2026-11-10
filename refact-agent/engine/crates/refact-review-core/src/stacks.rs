use crate::*;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{de::DeserializeOwned, Serialize};
use std::collections::HashSet;

pub(crate) fn migrate(c: &mut Connection) -> Result<()> {
    let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute_batch("CREATE TABLE IF NOT EXISTS review_store_migrations(version INTEGER PRIMARY KEY);
        CREATE TABLE IF NOT EXISTS active_reviews(chat TEXT PRIMARY KEY, review_id TEXT NOT NULL UNIQUE);
        CREATE TABLE IF NOT EXISTS singleton_reviews(chat TEXT PRIMARY KEY, review_id TEXT NOT NULL UNIQUE);
        CREATE TABLE IF NOT EXISTS stacks(id TEXT PRIMARY KEY, chat TEXT NOT NULL UNIQUE, document TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS stack_revisions(stack_id TEXT NOT NULL, revision INTEGER NOT NULL, document TEXT NOT NULL, PRIMARY KEY(stack_id,revision));
        CREATE TABLE IF NOT EXISTS stack_preparations(chat TEXT PRIMARY KEY, fingerprint TEXT NOT NULL, document TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS stack_confirmations(chat TEXT PRIMARY KEY, stack_id TEXT NOT NULL, fingerprint TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS stack_requests(scope TEXT NOT NULL, key TEXT NOT NULL, fingerprint TEXT NOT NULL, response TEXT NOT NULL, PRIMARY KEY(scope,key));")?;
    let migrated: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM review_store_migrations WHERE version=1)",
        [],
        |r| r.get(0),
    )?;
    if !migrated {
        // The legacy UNIQUE(chat) guarantees exactly one unambiguous slot per owner.
        tx.execute_batch(
            "ALTER TABLE reviews RENAME TO legacy_reviews;
            CREATE TABLE reviews(id TEXT PRIMARY KEY, chat TEXT NOT NULL, document TEXT NOT NULL);
            INSERT INTO reviews SELECT id,chat,document FROM legacy_reviews;
            INSERT INTO active_reviews SELECT chat,id FROM legacy_reviews;
            INSERT INTO singleton_reviews SELECT chat,id FROM legacy_reviews;
            DROP TABLE legacy_reviews;
            CREATE INDEX reviews_by_chat ON reviews(chat);
            INSERT INTO review_store_migrations VALUES(1);",
        )?;
    }
    tx.commit()?;
    Ok(())
}

pub(crate) fn stack_for_chat(c: &Connection, chat: &str) -> Result<Option<StackManifest>> {
    let s: Option<String> = c
        .query_row("SELECT document FROM stacks WHERE chat=?1", [chat], |r| {
            r.get(0)
        })
        .optional()?;
    s.map(|s| Ok(serde_json::from_str(&s)?)).transpose()
}
fn load_stack(c: &Connection, id: &str) -> Result<StackManifest> {
    let s: Option<String> = c
        .query_row("SELECT document FROM stacks WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .optional()?;
    Ok(serde_json::from_str(&s.ok_or(Error::NotFound)?)?)
}
fn replay<T: DeserializeOwned>(
    c: &Connection,
    scope: &str,
    key: &str,
    fp: &str,
) -> Result<Option<T>> {
    if key.is_empty() {
        return Err(invalid("missing request key"));
    }
    let row: Option<(String, String)> = c
        .query_row(
            "SELECT fingerprint,response FROM stack_requests WHERE scope=?1 AND key=?2",
            params![scope, key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    match row {
        Some((stored, response)) if stored == fp => Ok(Some(serde_json::from_str(&response)?)),
        Some(_) => Err(Error::IdempotencyConflict),
        None => Ok(None),
    }
}
fn remember<T: Serialize>(
    c: &Connection,
    scope: &str,
    key: &str,
    fp: &str,
    response: &T,
) -> Result<()> {
    c.execute(
        "INSERT INTO stack_requests VALUES(?1,?2,?3,?4)",
        params![scope, key, fp, serde_json::to_string(response)?],
    )?;
    Ok(())
}
fn validate_manifest(m: &StackManifest) -> Result<()> {
    if m.schema_version != SCHEMA_VERSION
        || m.id.is_empty()
        || m.owner_chat_id.is_empty()
        || m.repository.is_empty()
        || m.source.kind.is_empty()
        || m.source.kind.len() > 256
    {
        return Err(invalid("invalid stack identity/provenance"));
    }
    if !m.scope_choice.confirmed && !m.scope_choice.explicit_request {
        return Err(invalid("explicit scope choice required before review"));
    }
    if m.members.len() > MAX_FILES {
        return Err(Error::Limit("too many stack members".into()));
    }
    let mut ids = HashSet::new();
    let mut branches = HashSet::new();
    let mut prs = HashSet::new();
    let mut reviews = HashSet::new();
    for member in &m.members {
        if uuid::Uuid::parse_str(&member.id).is_err()
            || !ids.insert(member.id.as_str())
            || member.branch_ref.is_empty()
            || !branches.insert(member.branch_ref.as_str())
        {
            return Err(invalid("duplicate/invalid stack member identity"));
        }
        if let Some(pr) = &member.pr_identity {
            if pr.host.is_empty()
                || pr.repository.is_empty()
                || pr.number == 0
                || !prs.insert((
                    pr.host.to_lowercase(),
                    pr.repository.to_lowercase(),
                    pr.number,
                ))
            {
                return Err(invalid("duplicate/invalid PR identity"));
            }
        }
        if let Some(id) = &member.review_id {
            if !reviews.insert(id) {
                return Err(invalid("duplicate member review"));
            }
        }
        if let Some(parent) = &member.parent_member_id {
            if parent == &member.id || !ids.contains(parent.as_str()) {
                return Err(invalid("parent must precede child; cycles forbidden"));
            }
        }
    }
    if !ids.contains(m.scope_choice.selected_member_id.as_str()) {
        return Err(invalid("selected stack member missing"));
    }
    Ok(())
}
fn check_revision(expected: u64, actual: u64) -> Result<()> {
    if expected != actual {
        return Err(Error::Conflict {
            expected,
            actual,
        });
    }
    Ok(())
}
pub(crate) fn activate_checked(
    c: &Connection,
    chat: &str,
    review: &Review,
    expected: Option<&str>,
) -> Result<()> {
    if !scope_confirmed(c, chat)? {
        return Err(invalid("stack preparation awaits current scope consent"));
    }
    if review.owner_chat_id != chat {
        return Err(invalid("active review owner mismatch"));
    }
    if let Some(m) = stack_for_chat(c, chat)? {
        let member = m
            .members
            .iter()
            .find(|m| m.review_id.as_deref() == Some(review.id.as_str()))
            .ok_or_else(|| invalid("active review not a stack member"))?;
        if member.state != StackMemberState::Ready
            || (m.scope_choice.scope == StackScope::Single
                && member.id != m.scope_choice.selected_member_id)
        {
            return Err(invalid("active member unavailable/outside scope"));
        }
    }
    let active: Option<String> = c
        .query_row(
            "SELECT review_id FROM active_reviews WHERE chat=?1",
            [chat],
            |r| r.get(0),
        )
        .optional()?;
    if active.as_deref() != expected {
        return Err(invalid("active review CAS conflict"));
    }
    c.execute("INSERT INTO active_reviews VALUES(?1,?2) ON CONFLICT(chat) DO UPDATE SET review_id=excluded.review_id",params![chat,review.id])?;
    Ok(())
}
fn save(c: &Connection, m: &StackManifest) -> Result<()> {
    let doc = serde_json::to_string(m)?;
    c.execute("INSERT INTO stacks VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET document=excluded.document", params![m.id,m.owner_chat_id,doc])?;
    c.execute(
        "INSERT INTO stack_revisions VALUES(?1,?2,?3)",
        params![m.id, m.revision, doc],
    )?;
    Ok(())
}

fn scope_confirmed(c: &Connection, owner: &str) -> Result<bool> {
    let confirmed = c.query_row(
            "SELECT NOT EXISTS(SELECT 1 FROM stack_preparations p WHERE p.chat=?1 AND NOT EXISTS(SELECT 1 FROM stack_confirmations f JOIN stacks s ON s.id=f.stack_id AND s.chat=f.chat WHERE f.chat=p.chat AND f.fingerprint=p.fingerprint AND s.id=json_extract(p.document,'$.id') AND json_extract(s.document,'$.scope_choice.confirmed')=1))",
            [owner], |r| r.get(0))?;
    Ok(confirmed)
}

fn update_in_transaction(
    c: &Connection,
    mut manifest: StackManifest,
    expected_revision: u64,
) -> Result<StackManifest> {
    let old = load_stack(c, &manifest.id)?;
    check_revision(expected_revision, old.revision)?;
    check_revision(expected_revision, manifest.revision)?;
    if old.owner_chat_id != manifest.owner_chat_id
        || old.repository != manifest.repository
        || old.source != manifest.source
        || old.scope_choice != manifest.scope_choice
    {
        return Err(invalid("stack owner/repository/provenance/scope immutable"));
    }
    for prior in &old.members {
        let next = manifest
            .members
            .iter()
            .find(|m| m.id == prior.id)
            .ok_or_else(|| invalid("retain removed member tombstones"))?;
        if prior.review_id != next.review_id
                || (prior.reason.as_deref().is_some_and(|r| r.starts_with("entry:")) && prior.reason != next.reason)
                || (prior.reason.as_deref().is_some_and(|r| r.starts_with("entry:recorded_comparisons:"))
                    && (prior.declared_base_ref != next.declared_base_ref || prior.resolved_base_oid != next.resolved_base_oid || prior.head_oid != next.head_oid))
                // No durable rename-consent path exists yet. Conservatively keep branch
                // identity immutable, including PR-backed members, to prevent reattribution.
                || prior.branch_ref != next.branch_ref
                || (prior.pr_identity.is_some() && prior.pr_identity != next.pr_identity)
                || (prior.state == StackMemberState::Removed
                    && next.state != StackMemberState::Removed)
        {
            return Err(invalid("member review/PR identity or tombstone immutable"));
        }
    }
    if manifest
        .members
        .iter()
        .any(|m| m.review_id.is_some() && !old.members.iter().any(|o| o.id == m.id))
    {
        return Err(invalid("untrusted member binding"));
    }
    validate_manifest(&manifest)?;
    manifest.revision = old
        .revision
        .checked_add(1)
        .ok_or_else(|| invalid("revision overflow"))?;
    save(c, &manifest)?;
    Ok(manifest)
}

impl ReviewStore {
    pub fn stack_scope_confirmed(&self, owner: &str) -> Result<bool> {
        let c = self
            .connection
            .lock()
            .map_err(|_| invalid("poisoned store"))?;
        scope_confirmed(&c, owner)
    }

    pub fn replay_stack_refresh(
        &self,
        owner: &str,
        stack_id: &str,
        request_key: &str,
        expected_revision: u64,
    ) -> Result<Option<StackManifest>> {
        let c = self
            .connection
            .lock()
            .map_err(|_| invalid("poisoned store"))?;
        if !scope_confirmed(&c, owner)? {
            return Err(invalid("stack preparation awaits current scope consent"));
        }
        let stack = load_stack(&c, stack_id)?;
        if stack.owner_chat_id != owner {
            return Err(invalid("refresh owner mismatch"));
        }
        let fp = content_hash(&serde_json::to_vec(&(owner, stack_id, expected_revision))?);
        replay(
            &c,
            &format!("refresh-stack:{owner}:{stack_id}"),
            request_key,
            &fp,
        )
    }

    pub fn publish_stack_refresh(
        &self,
        manifest: StackManifest,
        expected_revision: u64,
        request_key: &str,
        candidates: Vec<(String, Candidate)>,
    ) -> Result<StackManifest> {
        self.publish_stack_refresh_with_privacy(
            manifest,
            expected_revision,
            request_key,
            candidates,
            vec![],
        )
    }
    pub fn publish_stack_refresh_with_privacy(
        &self,
        manifest: StackManifest,
        expected_revision: u64,
        request_key: &str,
        candidates: Vec<(String, Candidate)>,
        privacy: Vec<(String, Vec<refact_privacy::FileRecord>)>,
    ) -> Result<StackManifest> {
        let mut c = self
            .connection
            .lock()
            .map_err(|_| invalid("poisoned store"))?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !scope_confirmed(&tx, &manifest.owner_chat_id)? {
            return Err(invalid("stack preparation awaits current scope consent"));
        }
        let selected: HashSet<_> = manifest
            .members
            .iter()
            .filter(|m| {
                m.state != StackMemberState::Removed
                    && (manifest.scope_choice.scope == StackScope::Whole
                        || m.id == manifest.scope_choice.selected_member_id)
            })
            .map(|m| m.id.as_str())
            .collect();
        let mut seen = HashSet::new();
        for (id, candidate) in &candidates {
            if !selected.contains(id.as_str()) || !seen.insert(id.as_str()) {
                return Err(invalid("extra/duplicate refresh candidate"));
            }
            let member = manifest
                .members
                .iter()
                .find(|m| &m.id == id)
                .ok_or(Error::NotFound)?;
            if member.state != StackMemberState::Ready
                || member.review_id.is_none()
                || member.resolved_base_oid.as_deref() != Some(candidate.base_oid.as_str())
                || member.head_oid.as_deref() != Some(candidate.head_oid.as_str())
                || candidate.repository != manifest.repository
                || candidate.policy != CapturePolicy::default()
                || candidate.origin != Origin::Imported
            {
                return Err(invalid("refresh candidate binding mismatch"));
            }
        }
        if seen != selected {
            return Err(invalid("missing refresh candidate"));
        }
        let fp = content_hash(&serde_json::to_vec(&(
            &manifest.owner_chat_id,
            &manifest.id,
            expected_revision,
        ))?);
        let scope = format!("refresh-stack:{}:{}", manifest.owner_chat_id, manifest.id);
        if let Some(response) = replay(&tx, &scope, request_key, &fp)? {
            retain_member_privacy(&tx, &response, &privacy)?;
            tx.commit()?;
            return Ok(response);
        }
        let manifest = update_in_transaction(&tx, manifest, expected_revision)?;
        for (id, candidate) in candidates {
            let member = manifest
                .members
                .iter()
                .find(|m| m.id == id)
                .ok_or(Error::NotFound)?;
            let review_id = member.review_id.as_deref().ok_or(Error::NotFound)?;
            let review = crate::store::load(&tx, review_id)?;
            if review.owner_chat_id != manifest.owner_chat_id {
                return Err(invalid("refresh owner mismatch"));
            }
            if review
                .current_candidate_id
                .as_ref()
                .and_then(|id| review.candidates.iter().find(|c| &c.id == id))
                .is_some_and(|old| {
                    old.repository == candidate.repository
                        && old.base_oid == candidate.base_oid
                        && old.head_oid == candidate.head_oid
                        && old.content_digest == candidate.content_digest
                        && old.policy == candidate.policy
                        && old.origin == candidate.origin
                })
            {
                continue;
            }
            let material = Material {
                id: format!("import:{}", candidate.id), kind: MaterialKind::Imported,
                candidate_id: candidate.id.clone(), agent_id: None, description: "Immutable refreshed stack comparison; execution remains on prior binding until explicit capture".into(),
                content_digest: candidate.content_digest.clone(), input_digests: Default::default(), environment: None, worktree_id: None, worktree_root: None, artifacts: vec![],
            };
            for (offset, operation) in vec![
                Operation::StageCandidate {
                    candidate: candidate.clone(),
                },
                Operation::AddMaterial {
                    material,
                },
                Operation::ActivateCandidate {
                    candidate_id: candidate.id,
                },
            ]
            .into_iter()
            .enumerate()
            {
                crate::store::mutate_in_transaction(
                    &tx,
                    review_id,
                    Mutation {
                        expected_revision: review.revision + offset as u64,
                        request_key: format!("refresh:{request_key}:{offset}"),
                        actor: Actor {
                            kind: ActorKind::System,
                            id: manifest.owner_chat_id.clone(),
                        },
                        at_ms: 0,
                        operation,
                    },
                    None,
                )?;
            }
        }
        retain_member_privacy(&tx, &manifest, &privacy)?;
        remember(&tx, &scope, request_key, &fp, &manifest)?;
        tx.commit()?;
        Ok(manifest)
    }

    pub fn publish_stack_import(
        &self,
        manifest: StackManifest,
        expected_preparation: &str,
        request_key: &str,
        candidates: Vec<(String, Candidate)>,
    ) -> Result<StackManifest> {
        self.publish_stack_import_inner(
            manifest,
            expected_preparation,
            request_key,
            candidates,
            false,
            None,
            vec![],
        )
    }

    pub fn replay_stack_entry_import(
        &self,
        owner: &str,
        token: &str,
        key: &str,
        acquisition_fingerprint: &str,
    ) -> Result<Option<StackManifest>> {
        let c = self
            .connection
            .lock()
            .map_err(|_| invalid("poisoned store"))?;
        let pending: Option<String> = c
            .query_row(
                "SELECT fingerprint FROM stack_preparations WHERE chat=?1",
                [owner],
                |r| r.get(0),
            )
            .optional()?;
        if pending.as_deref() != Some(token) {
            return Err(invalid("stack preparation superseded"));
        }
        replay(
            &c,
            &format!("entry-acquisition:{owner}:{token}"),
            key,
            acquisition_fingerprint,
        )
    }

    pub fn publish_stack_entry_import(
        &self,
        manifest: StackManifest,
        expected_preparation: &str,
        request_key: &str,
        candidates: Vec<(String, Candidate)>,
    ) -> Result<StackManifest> {
        self.publish_stack_import_inner(
            manifest,
            expected_preparation,
            request_key,
            candidates,
            true,
            None,
            vec![],
        )
    }

    pub fn publish_stack_entry_acquisition(
        &self,
        manifest: StackManifest,
        token: &str,
        key: &str,
        candidates: Vec<(String, Candidate)>,
        acquisition_fingerprint: &str,
    ) -> Result<StackManifest> {
        self.publish_stack_import_inner(
            manifest,
            token,
            key,
            candidates,
            true,
            Some(acquisition_fingerprint),
            vec![],
        )
    }

    pub fn publish_stack_import_with_privacy(
        &self,
        manifest: StackManifest,
        preparation: &str,
        key: &str,
        candidates: Vec<(String, Candidate)>,
        privacy: Vec<(String, Vec<refact_privacy::FileRecord>)>,
    ) -> Result<StackManifest> {
        self.publish_stack_import_inner(
            manifest,
            preparation,
            key,
            candidates,
            false,
            None,
            privacy,
        )
    }
    pub fn publish_stack_entry_acquisition_with_privacy(
        &self,
        manifest: StackManifest,
        token: &str,
        key: &str,
        candidates: Vec<(String, Candidate)>,
        fingerprint: &str,
        privacy: Vec<(String, Vec<refact_privacy::FileRecord>)>,
    ) -> Result<StackManifest> {
        self.publish_stack_import_inner(
            manifest,
            token,
            key,
            candidates,
            true,
            Some(fingerprint),
            privacy,
        )
    }
    fn publish_stack_import_inner(
        &self,
        mut manifest: StackManifest,
        expected_preparation: &str,
        request_key: &str,
        mut candidates: Vec<(String, Candidate)>,
        discovery_only: bool,
        acquisition_fingerprint: Option<&str>,
        privacy: Vec<(String, Vec<refact_privacy::FileRecord>)>,
    ) -> Result<StackManifest> {
        validate_manifest(&manifest)?;
        check_revision(0, manifest.revision)?;
        if !manifest.scope_choice.confirmed
            || manifest.members.iter().any(|m| m.review_id.is_some())
        {
            return Err(invalid("confirmed unbound import required"));
        }
        let mut c = self
            .connection
            .lock()
            .map_err(|_| invalid("poisoned store"))?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let pending: Option<(String, String)> = tx
            .query_row(
                "SELECT fingerprint,document FROM stack_preparations WHERE chat=?1",
                [&manifest.owner_chat_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let (token, document) = pending.ok_or_else(|| invalid("missing stack preparation"))?;
        if token != expected_preparation || token.is_empty() {
            return Err(invalid("stack preparation superseded"));
        }
        let acquisition_scope = format!("entry-acquisition:{}:{}", manifest.owner_chat_id, token);
        if let Some(fp) = acquisition_fingerprint {
            if let Some(response) = replay(&tx, &acquisition_scope, request_key, fp)? {
                retain_member_privacy(&tx, &response, &privacy)?;
                tx.commit()?;
                return Ok(response);
            }
        }
        let mut normalized = manifest.clone();
        normalized.scope_choice.confirmed = false;
        normalized.scope_choice.explicit_request = false;
        normalized.scope_choice.scope = StackScope::Single;
        let mut prepared: StackManifest = serde_json::from_str(&document)?;
        prepared.scope_choice.confirmed = false;
        prepared.scope_choice.explicit_request = false;
        prepared.scope_choice.scope = StackScope::Single;
        if discovery_only {
            if !expected_preparation.starts_with("entry:") {
                return Err(invalid("entry discovery token required"));
            }
            normalized.root_base_ref.clear();
            normalized.root_base_oid = None;
            for member in &mut normalized.members {
                let prior = prepared
                    .members
                    .iter()
                    .find(|p| p.id == member.id)
                    .ok_or_else(|| invalid("member outside discovery"))?;
                let selected = manifest.scope_choice.scope == StackScope::Whole
                    || member.id == manifest.scope_choice.selected_member_id;
                if !selected && member != prior {
                    return Err(invalid("nonselected discovery member changed"));
                }
                member.declared_base_ref.clear();
                member.resolved_base_oid = None;
                member.head_oid = None;
                member.state = StackMemberState::Missing;
            }
        }
        if normalized != prepared {
            return Err(invalid("publication differs from prepared comparison"));
        }
        let selected: HashSet<_> = manifest
            .members
            .iter()
            .filter(|m| {
                manifest.scope_choice.scope == StackScope::Whole
                    || m.id == manifest.scope_choice.selected_member_id
            })
            .map(|m| m.id.as_str())
            .collect();
        let mut seen = HashSet::new();
        for (member_id, candidate) in &candidates {
            if !selected.contains(member_id.as_str()) || !seen.insert(member_id.as_str()) {
                return Err(invalid("extra/duplicate captured member"));
            }
            let member = manifest
                .members
                .iter()
                .find(|m| &m.id == member_id)
                .ok_or(Error::NotFound)?;
            if member.state != StackMemberState::Ready
                || member.resolved_base_oid.as_deref() != Some(candidate.base_oid.as_str())
                || member.head_oid.as_deref() != Some(candidate.head_oid.as_str())
                || candidate.repository != manifest.repository
                || candidate.policy != CapturePolicy::default()
                || candidate.origin != Origin::Imported
            {
                return Err(invalid("captured comparison outside prepared scope"));
            }
        }
        if seen != selected {
            return Err(invalid("missing captured member"));
        }
        candidates.sort_by(|a, b| a.0.cmp(&b.0));
        let semantic: Vec<_> = candidates
            .iter()
            .map(|(id, candidate)| {
                let mut candidate = candidate.clone();
                candidate.id.clear();
                candidate.created_at_ms = 0;
                (id, candidate)
            })
            .collect();
        let semantic = (&manifest, expected_preparation, semantic);
        let fp = content_hash(&serde_json::to_vec(&semantic)?);
        let scope = format!("publish-stack:{}:{}", manifest.owner_chat_id, manifest.id);
        if let Some(response) = replay(&tx, &scope, request_key, &fp)? {
            retain_member_privacy(&tx, &response, &privacy)?;
            tx.commit()?;
            return Ok(response);
        }
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM stacks WHERE id=?1 OR chat=?2)",
            params![manifest.id, manifest.owner_chat_id],
            |r| r.get(0),
        )?;
        if exists {
            return Err(invalid("stack already exists"));
        }
        for (member_id, candidate) in candidates {
            let member = manifest
                .members
                .iter_mut()
                .find(|m| m.id == member_id)
                .ok_or(Error::NotFound)?;
            let review = Review {
                privacy: vec![],
                schema_version: SCHEMA_VERSION,
                id: uuid::Uuid::new_v4().to_string(),
                owner_chat_id: manifest.owner_chat_id.clone(),
                revision: 0,
                current_candidate_id: None,
                title: member.branch_ref.clone(),
                origin: Origin::Imported,
                target_reference: String::new(),
                candidates: vec![],
                tour: vec![],
                annotations: vec![],
                threads: vec![],
                evidence: vec![],
                runs: vec![],
                materials: vec![],
                activity: vec![],
            };
            tx.execute(
                "INSERT INTO reviews VALUES(?1,?2,?3)",
                params![
                    review.id,
                    review.owner_chat_id,
                    serde_json::to_string(&review)?
                ],
            )?;
            member.review_id = Some(review.id.clone());
            let material = Material {
                id: format!("import:{}", candidate.id),
                kind: MaterialKind::Imported,
                candidate_id: candidate.id.clone(),
                agent_id: None,
                description: "Immutable imported stack comparison".into(),
                content_digest: candidate.content_digest.clone(),
                input_digests: Default::default(),
                environment: None,
                worktree_id: None,
                worktree_root: None,
                artifacts: vec![],
            };
            let operations = vec![
                Operation::StageCandidate {
                    candidate: candidate.clone(),
                },
                Operation::AddMaterial {
                    material,
                },
                Operation::ActivateCandidate {
                    candidate_id: candidate.id,
                },
            ];
            for (revision, operation) in operations.into_iter().enumerate() {
                crate::store::mutate_in_transaction(
                    &tx,
                    &review.id,
                    Mutation {
                        expected_revision: revision as u64,
                        request_key: format!("publish:{request_key}:{revision}"),
                        actor: Actor {
                            kind: ActorKind::System,
                            id: manifest.owner_chat_id.clone(),
                        },
                        at_ms: 0,
                        operation,
                    },
                    None,
                )?;
            }
        }
        retain_member_privacy(&tx, &manifest, &privacy)?;
        save(&tx, &manifest)?;
        tx.execute("INSERT INTO stack_confirmations VALUES(?1,?2,?3) ON CONFLICT(chat) DO UPDATE SET stack_id=excluded.stack_id,fingerprint=excluded.fingerprint", params![manifest.owner_chat_id, manifest.id, expected_preparation])?;
        remember(&tx, &scope, request_key, &fp, &manifest)?;
        if let Some(fp) = acquisition_fingerprint {
            remember(&tx, &acquisition_scope, request_key, fp, &manifest)?;
        }
        tx.commit()?;
        Ok(manifest)
    }

    /// Durable discovery only: no review creation, scope confirmation or activation.
    pub fn put_stack_preparation(
        &self,
        owner: &str,
        fingerprint: &str,
        manifest: &StackManifest,
    ) -> Result<()> {
        if owner.is_empty() || owner != manifest.owner_chat_id || fingerprint.is_empty() {
            return Err(invalid("invalid preparation owner/fingerprint"));
        }
        let mut discovery = manifest.clone();
        discovery.scope_choice.confirmed = true;
        validate_manifest(&discovery)?;
        if manifest.members.iter().any(|m| m.review_id.is_some()) {
            return Err(invalid("preparation cannot bind reviews"));
        }
        let c = self
            .connection
            .lock()
            .map_err(|_| invalid("poisoned store"))?;
        c.execute("INSERT INTO stack_preparations VALUES(?1,?2,?3) ON CONFLICT(chat) DO UPDATE SET fingerprint=excluded.fingerprint,document=excluded.document",params![owner,fingerprint,serde_json::to_string(manifest)?])?;
        Ok(())
    }
    pub fn get_stack_preparation(&self, owner: &str) -> Result<Option<(String, StackManifest)>> {
        let c = self
            .connection
            .lock()
            .map_err(|_| invalid("poisoned store"))?;
        let row: Option<(String, String)> = c
            .query_row(
                "SELECT fingerprint,document FROM stack_preparations WHERE chat=?1",
                [owner],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        row.map(|(fp, doc)| Ok((fp, serde_json::from_str(&doc)?)))
            .transpose()
    }
    pub fn load_stack(&self, id: &str) -> Result<StackManifest> {
        let c = self
            .connection
            .lock()
            .map_err(|_| invalid("poisoned store"))?;
        load_stack(&c, id)
    }
    pub fn list_for_chat(&self, chat: &str) -> Result<Vec<Review>> {
        let c = self
            .connection
            .lock()
            .map_err(|_| invalid("poisoned store"))?;
        let mut q = c.prepare("SELECT id FROM reviews WHERE chat=?1 ORDER BY id")?;
        let rows = q.query_map([chat], |r| r.get::<_, String>(0))?;
        rows.map(|id| crate::store::load(&c, &id?)).collect()
    }
    pub fn find_review_for_agent(&self, chat: &str, agent_id: &str) -> Result<Option<Review>> {
        let matches: Vec<_> = self
            .list_for_chat(chat)?
            .into_iter()
            .filter(|r| r.runs.iter().any(|run| run.agent_id == agent_id))
            .collect();
        if matches.len() > 1 {
            return Err(invalid("ambiguous agent review binding"));
        }
        Ok(matches.into_iter().next())
    }
    pub fn get_stack_for_chat(&self, chat: &str) -> Result<Option<StackManifest>> {
        let c = self
            .connection
            .lock()
            .map_err(|_| invalid("poisoned store"))?;
        stack_for_chat(&c, chat)
    }
    pub fn stack_history(&self, stack_id: &str) -> Result<Vec<StackManifest>> {
        let c = self
            .connection
            .lock()
            .map_err(|_| invalid("poisoned store"))?;
        let mut q =
            c.prepare("SELECT document FROM stack_revisions WHERE stack_id=?1 ORDER BY revision")?;
        let rows = q.query_map([stack_id], |r| r.get::<_, String>(0))?;
        rows.map(|r| Ok(serde_json::from_str(&r?)?)).collect()
    }
    pub fn create_stack(
        &self,
        manifest: StackManifest,
        request_key: &str,
    ) -> Result<StackManifest> {
        let fp = content_hash(&serde_json::to_vec(&manifest)?);
        let scope = format!("create-stack:{}", manifest.owner_chat_id);
        let mut c = self
            .connection
            .lock()
            .map_err(|_| invalid("poisoned store"))?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(m) = replay(&tx, &scope, request_key, &fp)? {
            return Ok(m);
        }
        validate_manifest(&manifest)?;
        check_revision(0, manifest.revision)?;
        if manifest.members.iter().any(|m| m.review_id.is_some()) {
            return Err(invalid("bind reviews through create_member_review"));
        }
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM stacks WHERE id=?1 OR chat=?2)",
            params![manifest.id, manifest.owner_chat_id],
            |row| row.get(0),
        )?;
        if exists {
            return Err(invalid("stack already exists"));
        }
        save(&tx, &manifest)?;
        remember(&tx, &scope, request_key, &fp, &manifest)?;
        tx.commit()?;
        Ok(manifest)
    }
    pub fn update_stack(
        &self,
        mut manifest: StackManifest,
        expected_revision: u64,
        request_key: &str,
    ) -> Result<StackManifest> {
        let fp = content_hash(&serde_json::to_vec(&(&manifest, expected_revision))?);
        let scope = format!("update-stack:{}", manifest.id);
        let mut c = self
            .connection
            .lock()
            .map_err(|_| invalid("poisoned store"))?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(m) = replay(&tx, &scope, request_key, &fp)? {
            return Ok(m);
        }
        manifest = update_in_transaction(&tx, manifest, expected_revision)?;
        remember(&tx, &scope, request_key, &fp, &manifest)?;
        tx.commit()?;
        Ok(manifest)
    }
    pub fn stack_review_for_member(
        &self,
        stack_id: &str,
        member_id: &str,
    ) -> Result<Option<Review>> {
        let c = self
            .connection
            .lock()
            .map_err(|_| invalid("poisoned store"))?;
        let m = load_stack(&c, stack_id)?;
        let member = m
            .members
            .iter()
            .find(|m| m.id == member_id)
            .ok_or(Error::NotFound)?;
        member
            .review_id
            .as_deref()
            .map(|id| crate::store::load(&c, id))
            .transpose()
    }
    pub fn create_member_review(
        &self,
        stack_id: &str,
        member_id: &str,
        expected_revision: u64,
        title: &str,
        origin: Origin,
        request_key: &str,
    ) -> Result<Review> {
        let fp = content_hash(&serde_json::to_vec(&(
            stack_id,
            member_id,
            expected_revision,
            title,
            &origin,
        ))?);
        let scope = format!("member-review:{stack_id}");
        let mut c = self
            .connection
            .lock()
            .map_err(|_| invalid("poisoned store"))?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(r) = replay(&tx, &scope, request_key, &fp)? {
            let r = crate::privacy::retain(&tx, r, &[])?;
            tx.commit()?;
            return Ok(r);
        }
        let mut m = load_stack(&tx, stack_id)?;
        check_revision(expected_revision, m.revision)?;
        if m.scope_choice.scope == StackScope::Single
            && m.scope_choice.selected_member_id != member_id
        {
            return Err(invalid("member outside confirmed scope"));
        }
        let member = m
            .members
            .iter_mut()
            .find(|m| m.id == member_id)
            .ok_or(Error::NotFound)?;
        if member.state != StackMemberState::Ready {
            return Err(invalid("member unavailable"));
        }
        if let Some(id) = &member.review_id {
            let r = crate::store::load(&tx, id)?;
            remember(&tx, &scope, request_key, &fp, &r)?;
            tx.commit()?;
            return Ok(r);
        }
        // Import creates independent member histories. Never repurpose the active legacy
        // singleton: staging stack candidates must not alter its execution context.
        let r = {
            let r = Review {
                privacy: vec![],
                schema_version: SCHEMA_VERSION,
                id: uuid::Uuid::new_v4().to_string(),
                owner_chat_id: m.owner_chat_id.clone(),
                revision: 0,
                current_candidate_id: None,
                title: title.into(),
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
            };
            tx.execute(
                "INSERT INTO reviews VALUES(?1,?2,?3)",
                params![r.id, r.owner_chat_id, serde_json::to_string(&r)?],
            )?;
            r
        };
        member.review_id = Some(r.id.clone());
        m.revision = m
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("revision overflow"))?;
        save(&tx, &m)?;
        remember(&tx, &scope, request_key, &fp, &r)?;
        tx.commit()?;
        Ok(r)
    }
    /// Trusted engine activation only, after runtime context validation. Reads never call this.
    pub fn set_active_review(
        &self,
        chat: &str,
        review_id: &str,
        expected_active_review_id: Option<&str>,
        request_key: &str,
    ) -> Result<Review> {
        let fp = content_hash(&serde_json::to_vec(&(
            chat,
            review_id,
            expected_active_review_id,
        ))?);
        let scope = format!("activate-review:{chat}");
        let mut c = self
            .connection
            .lock()
            .map_err(|_| invalid("poisoned store"))?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(r) = replay(&tx, &scope, request_key, &fp)? {
            let r = crate::privacy::retain(&tx, r, &[])?;
            tx.commit()?;
            return Ok(r);
        }
        let r = crate::store::load(&tx, review_id)?;
        activate_checked(&tx, chat, &r, expected_active_review_id)?;
        remember(&tx, &scope, request_key, &fp, &r)?;
        tx.commit()?;
        Ok(r)
    }
}

fn retain_member_privacy(
    c: &Connection,
    manifest: &StackManifest,
    privacy: &[(String, Vec<refact_privacy::FileRecord>)],
) -> Result<()> {
    let mut seen = HashSet::new();
    for (id, records) in privacy {
        let member = manifest
            .members
            .iter()
            .find(|m| &m.id == id)
            .ok_or_else(|| invalid("unknown privacy member"))?;
        if !seen.insert(id)
            || (manifest.scope_choice.scope != StackScope::Whole
                && id != &manifest.scope_choice.selected_member_id)
        {
            return Err(invalid("privacy outside selected members"));
        }
        let review = crate::store::load(
            c,
            member
                .review_id
                .as_deref()
                .ok_or_else(|| invalid("unbound privacy member"))?,
        )?;
        if review.owner_chat_id != manifest.owner_chat_id {
            return Err(invalid("privacy member owner mismatch"));
        }
        crate::privacy::retain(c, review, records)?;
    }
    Ok(())
}

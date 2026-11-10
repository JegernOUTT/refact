use crate::*;
use refact_privacy::FileRecord;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

pub(crate) fn union(previous: &[FileRecord], next: &[FileRecord]) -> Result<Vec<FileRecord>> {
    let mut seen = HashSet::new();
    let mut records = Vec::new();
    for record in previous.iter().chain(next) {
        if record.path.trim().is_empty()
            || record.zone.trim().is_empty()
            || record.path.contains('\0')
            || record.zone.contains('\0')
        {
            return Err(invalid("malformed privacy provenance"));
        }
        if seen.insert(record.clone()) {
            records.push(record.clone());
        }
    }
    Ok(records)
}

pub(crate) fn retain(
    c: &Connection,
    mut response: Review,
    records: &[FileRecord],
) -> Result<Review> {
    let mut current = crate::store::load(c, &response.id)?;
    let privacy = union(&current.privacy, &union(&response.privacy, records)?)?;
    if current.privacy != privacy {
        current.privacy = privacy.clone();
        c.execute(
            "UPDATE reviews SET document=?1 WHERE id=?2",
            params![serde_json::to_string(&current)?, current.id],
        )?;
    }
    response.privacy = privacy;
    Ok(response)
}

fn receipt(value: &mut Value, records: &mut Vec<FileRecord>) -> Result<()> {
    if let Some(object) = value.as_object_mut() {
        if let Some(receipt) = object.remove("context_intake") {
            if !receipt.is_null() {
                let files = receipt
                    .get("privacy")
                    .ok_or_else(|| invalid("missing legacy privacy"))?;
                *records = union(
                    records,
                    &serde_json::from_value::<Vec<FileRecord>>(files.clone())?,
                )?;
            }
        }
    }
    Ok(())
}
fn step(value: &mut Value, records: &mut Vec<FileRecord>) -> Result<()> {
    if let Some(current) = value.get_mut("current") {
        receipt(current, records)?;
    }
    if let Some(history) = value.get_mut("history").and_then(Value::as_array_mut) {
        for version in history {
            receipt(version, records)?;
        }
    }
    Ok(())
}
fn extract(value: &mut Value, records: &mut Vec<FileRecord>) -> Result<()> {
    if value.get("tour").is_some() && value.get("owner_chat_id").is_some() {
        receipt(value, records)?;
        value
            .as_object_mut()
            .ok_or_else(|| invalid("invalid review"))?
            .remove("tour_revision_proposals");
        for item in value
            .get_mut("tour")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| invalid("invalid tour"))?
        {
            step(item, records)?;
        }
    } else if value.pointer("/object/kind").and_then(Value::as_str) == Some("tour_step") {
        step(
            value
                .pointer_mut("/object/value")
                .ok_or_else(|| invalid("invalid historical tour"))?,
            records,
        )?;
    }
    Ok(())
}

fn rows(c: &Connection, sql: &str) -> Result<Vec<(i64, String, String)>> {
    let mut q = c.prepare(sql)?;
    let rows = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

pub(crate) fn migrate(c: &mut Connection) -> Result<()> {
    let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute_batch("CREATE TABLE IF NOT EXISTS review_data_migrations(name TEXT PRIMARY KEY);")?;
    let done: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM review_data_migrations WHERE name='retained_privacy')",
        [],
        |r| r.get(0),
    )?;
    if done {
        return Ok(());
    }
    let mut reviews = HashMap::new();
    let mut owners: HashMap<String, Vec<String>> = HashMap::new();
    for (_, id, document) in rows(&tx, "SELECT rowid,id,document FROM reviews")? {
        let mut value: Value = serde_json::from_str(&document)?;
        let mut records = vec![];
        extract(&mut value, &mut records)?;
        let review: Review = serde_json::from_value(value)?;
        if review.id != id {
            return Err(invalid("review identity mismatch"));
        }
        owners
            .entry(review.owner_chat_id.clone())
            .or_default()
            .push(id.clone());
        records = union(&review.privacy, &records)?;
        reviews.insert(id, (review, records));
    }
    for (rowid, id, document) in rows(&tx, "SELECT rowid,review_id,document FROM object_revisions")?
    {
        let mut value: Value = serde_json::from_str(&document)?;
        let mut records = vec![];
        extract(&mut value, &mut records)?;
        let (_, privacy) = reviews
            .get_mut(&id)
            .ok_or_else(|| invalid("orphan object history"))?;
        *privacy = union(privacy, &records)?;
        tx.execute(
            "UPDATE object_revisions SET document=?1 WHERE rowid=?2",
            params![serde_json::to_string(&value)?, rowid],
        )?;
    }
    let mut responses = vec![];
    let mut request_reviews = HashMap::new();
    for (rowid, scope, response) in rows(&tx, "SELECT rowid,scope,response FROM requests")? {
        if let Some(id) = response.strip_prefix("review:") {
            request_reviews.insert(rowid, id.to_owned());
            continue;
        }
        let mut value: Value = serde_json::from_str(&response)?;
        let mut records = vec![];
        extract(&mut value, &mut records)?;
        if value.get("tour").is_some() && value.get("owner_chat_id").is_some() {
            let review: Review = serde_json::from_value(value.clone())?;
            let (_, privacy) = reviews
                .get_mut(&review.id)
                .ok_or_else(|| invalid("orphan request review"))?;
            *privacy = union(privacy, &union(&review.privacy, &records)?)?;
            request_reviews.insert(rowid, review.id.clone());
            responses.push((rowid, Some(review.id), value));
        } else {
            if !records.is_empty() {
                return Err(invalid("unbound request provenance"));
            }
            let _ = scope;
            responses.push((rowid, None, value));
        }
    }
    let mut stack_responses = vec![];
    let has_stack_requests: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='stack_requests')",
        [],
        |r| r.get(0),
    )?;
    if has_stack_requests {
        for (rowid, _, response) in rows(&tx, "SELECT rowid,scope,response FROM stack_requests")? {
            let mut value: Value = serde_json::from_str(&response)?;
            if value.get("tour").is_some() && value.get("owner_chat_id").is_some() {
                let mut records = vec![];
                extract(&mut value, &mut records)?;
                let review: Review = serde_json::from_value(value.clone())?;
                let (_, privacy) = reviews
                    .get_mut(&review.id)
                    .ok_or_else(|| invalid("orphan stack request review"))?;
                *privacy = union(privacy, &union(&review.privacy, &records)?)?;
                stack_responses.push((rowid, review.id, value));
            }
        }
    }
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='request_intakes')",
        [],
        |r| r.get(0),
    )?;
    if exists {
        let mut q = tx.prepare("SELECT i.scope,i.document,r.rowid FROM request_intakes i LEFT JOIN requests r ON r.scope=i.scope AND r.key=i.key")?;
        let entries = q
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(q);
        for (scope, document, rowid) in entries {
            let value: Value = serde_json::from_str(&document)?;
            let files = value
                .get("privacy")
                .ok_or_else(|| invalid("missing legacy privacy"))?;
            let records = union(
                &[],
                &serde_json::from_value::<Vec<FileRecord>>(files.clone())?,
            )?;
            let id = rowid
                .and_then(|n| request_reviews.get(&n).cloned())
                .or_else(|| reviews.contains_key(&scope).then(|| scope.clone()))
                .or_else(|| {
                    scope
                        .strip_prefix("create:")
                        .and_then(|owner| owners.get(owner))
                        .filter(|ids| ids.len() == 1)
                        .map(|ids| ids[0].clone())
                })
                .ok_or_else(|| invalid("unbound legacy privacy"))?;
            let (_, privacy) = reviews
                .get_mut(&id)
                .ok_or_else(|| invalid("orphan legacy privacy"))?;
            *privacy = union(privacy, &records)?;
        }
    }
    let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='stack_request_intakes')", [], |r| r.get(0))?;
    if exists {
        let entries = rows(
            &tx,
            "SELECT rowid,scope,document FROM stack_request_intakes",
        )?;
        for (rowid, scope, document) in entries {
            let key: String = tx.query_row(
                "SELECT key FROM stack_request_intakes WHERE rowid=?1",
                [rowid],
                |r| r.get(0),
            )?;
            let request_scope = scope.replacen("stack-intakes:", "entry-acquisition:", 1);
            let response: Option<String> = tx
                .query_row(
                    "SELECT response FROM stack_requests WHERE scope=?1 AND key=?2",
                    params![request_scope, key],
                    |r| r.get(0),
                )
                .optional()?;
            let entries: Vec<(String, Value)> = serde_json::from_str(&document)?;
            let response = if let Some(response) = response {
                response
            } else {
                let owner = entries
                    .first()
                    .and_then(|(_, r)| r.get("owner_chat_id"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| invalid("unbound stack privacy owner"))?;
                tx.query_row("SELECT document FROM stacks WHERE chat=?1", [owner], |r| {
                    r.get(0)
                })?
            };
            let manifest: StackManifest = serde_json::from_str(&response)?;
            for (member, receipt) in entries {
                if receipt.get("owner_chat_id").and_then(Value::as_str)
                    != Some(manifest.owner_chat_id.as_str())
                {
                    return Err(invalid("stack privacy owner mismatch"));
                }
                let id = manifest
                    .members
                    .iter()
                    .find(|m| m.id == member)
                    .and_then(|m| m.review_id.as_ref())
                    .ok_or_else(|| invalid("unbound stack privacy"))?;
                let files = receipt
                    .get("privacy")
                    .ok_or_else(|| invalid("missing legacy privacy"))?;
                let records: Vec<FileRecord> = serde_json::from_value(files.clone())?;
                let (_, privacy) = reviews
                    .get_mut(id)
                    .ok_or_else(|| invalid("orphan stack privacy"))?;
                *privacy = union(privacy, &records)?;
            }
        }
    }
    for (rowid, id, mut value) in responses {
        if let Some(id) = id {
            let (_, privacy) = &reviews[&id];
            value
                .as_object_mut()
                .ok_or_else(|| invalid("invalid review response"))?
                .insert("privacy".into(), serde_json::to_value(privacy)?);
        }
        tx.execute(
            "UPDATE requests SET response=?1 WHERE rowid=?2",
            params![serde_json::to_string(&value)?, rowid],
        )?;
    }
    for (rowid, id, mut value) in stack_responses {
        let (_, privacy) = &reviews[&id];
        value
            .as_object_mut()
            .ok_or_else(|| invalid("invalid stack review response"))?
            .insert("privacy".into(), serde_json::to_value(privacy)?);
        tx.execute(
            "UPDATE stack_requests SET response=?1 WHERE rowid=?2",
            params![serde_json::to_string(&value)?, rowid],
        )?;
    }
    for (id, (mut review, privacy)) in reviews {
        review.privacy = privacy;
        tx.execute(
            "UPDATE reviews SET document=?1 WHERE id=?2",
            params![serde_json::to_string(&review)?, id],
        )?;
    }
    tx.execute_batch("DROP TABLE IF EXISTS request_intakes; DROP TABLE IF EXISTS proposal_turn_completions; DROP TABLE IF EXISTS stack_request_intakes; INSERT INTO review_data_migrations VALUES('retained_privacy');")?;
    tx.commit()?;
    Ok(())
}

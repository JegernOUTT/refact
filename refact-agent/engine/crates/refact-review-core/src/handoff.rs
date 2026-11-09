use crate::*;
use rusqlite::Connection;

fn history(c: &Connection, id: &str) -> Result<Vec<ObjectRevision>> {
    let mut q =
        c.prepare("SELECT document FROM object_revisions WHERE review_id=?1 ORDER BY rowid")?;
    let rows = q.query_map([id], |r| r.get::<_, String>(0))?;
    rows.map(|r| Ok(serde_json::from_str(&r?)?)).collect()
}
fn deliveries(c: &Connection, id: &str) -> Result<Vec<ReviewDelivery>> {
    let mut q =
        c.prepare("SELECT document,delivered FROM deliveries WHERE review_id=?1 ORDER BY rowid")?;
    let rows = q.query_map([id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?)))?;
    rows.map(|r| {
        let (document, delivered) = r?;
        let mut delivery: ReviewDelivery = serde_json::from_str(&document)?;
        // The settlement column is authoritative; the immutable outbox payload is not.
        delivery.delivered = delivered;
        Ok(delivery)
    })
    .collect()
}
impl ReviewStore {
    /// Local internal data, not privacy clearance for any destination. One SQLite read
    /// transaction pins current payloads, object histories and outbox settlement state.
    pub fn handoff_snapshot(&self, review_id: &str) -> Result<HandoffSnapshot> {
        let mut c = self
            .connection
            .lock()
            .map_err(|_| invalid("poisoned store"))?;
        let tx = c.transaction()?;
        let review = crate::store::load(&tx, review_id)?;
        let stack_manifest = crate::stacks::stack_for_chat(&tx, &review.owner_chat_id)?;
        let mut stack_member_reviews = vec![];
        let mut stack_history = vec![];
        if let Some(manifest) = &stack_manifest {
            let mut q = tx.prepare(
                "SELECT document FROM stack_revisions WHERE stack_id=?1 ORDER BY revision",
            )?;
            for row in q.query_map([&manifest.id], |r| r.get::<_, String>(0))? {
                stack_history.push(serde_json::from_str(&row?)?);
            }
            for member in &manifest.members {
                if let Some(id) = &member.review_id {
                    let member_review = crate::store::load(&tx, id)?;
                    if member_review.owner_chat_id != manifest.owner_chat_id {
                        return Err(Error::Integrity(
                            "stack member review owner mismatch".into(),
                        ));
                    }
                    stack_member_reviews.push(StackMemberReviewSnapshot {
                        member_id: member.id.clone(),
                        object_history: history(&tx, id)?,
                        delivery_statuses: deliveries(&tx, id)?,
                        review: member_review,
                    });
                }
            }
        }
        let snapshot = HandoffSnapshot {
            object_history: history(&tx, review_id)?,
            delivery_statuses: deliveries(&tx, review_id)?,
            review,
            stack_manifest,
            stack_history,
            stack_member_reviews,
        };
        tx.commit()?;
        Ok(snapshot)
    }
}

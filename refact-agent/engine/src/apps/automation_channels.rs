//! Trusted project-automation -> App Internal-source handoff.
//!
//! This is an adapter to the project's existing AutomationService, not a second
//! broker. An App declares Internal { topic: delivery.channel }, the matching
//! Internal permission, a confirmed source grant and live demand. Publication
//! confers no collector, cross-project, chat, or model-egress authority.
use refact_scheduler_core::sources::AutomationService;
use serde_json::{json, Value};

pub(crate) const MAX_CHANNEL_BYTES: usize = 262_144;

/// Only the approved generic outbox drain calls this host port. `event_id` is an
/// occurrence identity, never a payload hash: equal independent updates survive.
pub(crate) async fn publish(
    service: &AutomationService,
    channel: &str,
    data: Value,
    event_id: &str,
    definition_id: &str,
    source_id: Option<&str>,
    config_digest: &str,
    original_event_seq: i64,
) -> Result<(), String> {
    let original_event_seq = u64::try_from(original_event_seq)
        .ok()
        .filter(|seq| *seq > 0)
        .ok_or("automation channel journal event sequence must be positive")?;
    if channel.is_empty() || channel.len() > 256 || event_id.is_empty() || event_id.len() > 512 {
        return Err("automation channel identity bounds".into());
    }
    let envelope = json!({
        "schema_version": 1,
        "channel": channel,
        "data": data,
        "provenance": {
            "namespace": "project_automation",
            "producer": definition_id,
            "source_id": source_id,
            "config_digest": config_digest,
            "original_event_seq": original_event_seq,
            "event_id": event_id,
        }
    });
    if serde_json::to_vec(&envelope)
        .map_err(|e| e.to_string())?
        .len()
        > MAX_CHANNEL_BYTES
    {
        return Err("automation channel envelope bounds".into());
    }
    // Exclude the project source owner so Internal collectors cannot subscribe
    // to their own handoff and create an unbounded source->outbox feedback loop.
    // The service durably ingests each eligible App subscriber before success.
    service
        .publish_internal_id(
            channel,
            envelope,
            event_id,
            crate::scheduler::generic_runtime::PROJECT_SOURCE_OWNER,
        )
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_apps_core::{Permission, SourceDefinition, SourceDriver};

    fn internal(id: &str, topic: &str, permission: &str) -> SourceDefinition {
        SourceDefinition {
            source_id: id.into(),
            channel: "display".into(),
            driver: SourceDriver::Internal {
                topic: topic.into(),
            },
            permissions: vec![Permission::Internal {
                topic: permission.into(),
            }],
        }
    }

    #[tokio::test]
    async fn collector_outbox_reaches_only_granted_app_topic_without_model() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let host = gcx.apps.get(temp.path().into(), gcx.clone()).await.unwrap();
        host.automation
            .configure(
                "dashboard",
                "1".into(),
                vec![
                    internal("approved", "metrics", "metrics"),
                    internal("ungranted", "metrics", "metrics"),
                    internal("wrong_topic", "other", "other"),
                    internal("wrong_permission", "metrics", "other"),
                ],
            )
            .await
            .unwrap();
        for id in ["approved", "ungranted", "wrong_topic", "wrong_permission"] {
            host.automation
                .set_demand("dashboard", id, "view", true)
                .await
                .unwrap();
        }
        for id in ["approved", "wrong_topic", "wrong_permission"] {
            // Granting a mismatched permission may itself be rejected by service validation.
            let _ = host.automation.set_grant("dashboard", id, true).await;
        }
        let definition: refact_scheduler_core::automation::Definition =
            serde_json::from_value(json!({
                "schema_version":1,"id":"collector_bridge","revision":1,"owner":{"kind":"project"},
                "description":"bridge test","enabled":true,"trigger":{"kind":"continuous"},
                "action":{"kind":"collect","driver":{"type":"interval","every_ms":1000}},
                "deliveries":[{"id":"metrics","channel":"metrics","credential_ref":null}],
                "lifecycle":{"kind":"on_demand"},
                "policy":{"idempotent":true,"max_pending_events":16,"max_event_bytes":1024}
            }))
            .unwrap();
        let catalog = host
            .generic
            .save(0, vec![definition.clone()])
            .await
            .unwrap();
        host.generic
            .apply_exact(
                &definition.id,
                catalog.revision,
                1,
                &definition.security_digest().unwrap(),
            )
            .await
            .unwrap();
        let value = host
            .generic
            .run_exact(&definition.id, catalog.revision, 1)
            .await
            .unwrap();
        host.generic.drain_deliveries().await.unwrap();
        let snapshot = host.automation.snapshot("dashboard").await;
        let delivered = snapshot.iter().find(|e| e.source_id == "approved").unwrap();
        let envelope = delivered
            .data
            .as_ref()
            .expect("collector did not reach App internal source");
        assert_eq!(envelope["data"], value["data"]);
        assert_eq!(envelope["provenance"]["producer"], "collector_bridge");
        assert_eq!(envelope["channel"], "metrics");
        assert!(envelope["provenance"]["original_event_seq"]
            .as_u64()
            .is_some());
        for e in &snapshot {
            if e.source_id != "approved" {
                assert!(
                    e.data.is_none(),
                    "unauthorized source received data: {}",
                    e.source_id
                );
            }
        }
        let seq = delivered.seq;
        host.generic.drain_deliveries().await.unwrap();
        assert_eq!(
            host.automation
                .snapshot("dashboard")
                .await
                .iter()
                .find(|e| e.source_id == "approved")
                .unwrap()
                .seq,
            seq
        );
        gcx.apps.shutdown().await;
    }

    #[tokio::test]
    async fn pending_collector_payload_cannot_be_retargeted_by_reapproval() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let host = gcx.apps.get(temp.path().into(), gcx.clone()).await.unwrap();
        // Both exact-topic consumers exist and are eligible before the old event
        // is produced; absence of delivery must not be a missing-subscriber oracle.
        host.automation
            .configure(
                "dashboard",
                "1".into(),
                vec![
                    internal("old", "old_metrics", "old_metrics"),
                    internal("new", "new_metrics", "new_metrics"),
                ],
            )
            .await
            .unwrap();
        for id in ["old", "new"] {
            host.automation
                .set_grant("dashboard", id, true)
                .await
                .unwrap();
            host.automation
                .set_demand("dashboard", id, "view", true)
                .await
                .unwrap();
        }
        let mut definition: refact_scheduler_core::automation::Definition =
            serde_json::from_value(json!({
                "schema_version":1,"id":"retarget_bridge","revision":1,"owner":{"kind":"project"},
                "description":"retarget regression","enabled":true,"trigger":{"kind":"continuous"},
                "action":{"kind":"collect","driver":{"type":"interval","every_ms":1000}},
                "deliveries":[{"id":"metrics","channel":"old_metrics","credential_ref":null}],
                "lifecycle":{"kind":"on_demand"},
                "policy":{"idempotent":true,"max_pending_events":16,"max_event_bytes":1024}
            }))
            .unwrap();
        let catalog = host
            .generic
            .save(0, vec![definition.clone()])
            .await
            .unwrap();
        host.generic
            .apply_exact(
                &definition.id,
                catalog.revision,
                1,
                &definition.security_digest().unwrap(),
            )
            .await
            .unwrap();
        let owner = crate::scheduler::generic_runtime::PROJECT_SOURCE_OWNER;
        let source =
            crate::scheduler::generic_runtime::GenericAutomationRuntime::source_id(&definition)
                .unwrap();
        let old_digest = host
            .automation
            .source_config_digest(owner, &source)
            .await
            .unwrap();
        let old_value = host
            .generic
            .run_exact(&definition.id, catalog.revision, 1)
            .await
            .unwrap();
        assert_eq!(old_value["data"], json!({"tick":true}));
        let journal =
            refact_scheduler_core::automation::AutomationJournal::open(temp.path()).unwrap();
        let pending = journal
            .pending_deliveries_for_targets(
                &["automation_source_channels".into()],
                64,
                chrono::Utc::now().timestamp_millis().max(0) as u64,
            )
            .unwrap();
        assert!(
            !pending.is_empty(),
            "old event must actually be pending before retarget"
        );
        assert!(pending
            .iter()
            .any(|d| d.data["source_config_digest"] == old_digest
                && d.data["data"] == json!({"tick":true})));
        definition.revision = 2;
        definition.deliveries[0].channel = "new_metrics".into();
        let catalog = host
            .generic
            .save(catalog.revision, vec![definition.clone()])
            .await
            .unwrap();
        host.generic
            .apply_exact(
                &definition.id,
                catalog.revision,
                2,
                &definition.security_digest().unwrap(),
            )
            .await
            .unwrap();
        assert_ne!(
            host.automation
                .source_config_digest(owner, &source)
                .await
                .unwrap(),
            old_digest,
            "compiled source digest must bind the approved delivery targets"
        );
        host.generic.drain_deliveries().await.unwrap();
        for consumer in host.automation.snapshot("dashboard").await {
            assert!(
                consumer.data.is_none(),
                "pending old payload escaped to {}",
                consumer.source_id
            );
        }
        // Positive control: the very same new consumer receives a newly admitted
        // occurrence under the replacement approval, rather than being inert.
        host.generic
            .run_exact(&definition.id, catalog.revision, 2)
            .await
            .unwrap();
        host.generic.drain_deliveries().await.unwrap();
        let snapshot = host.automation.snapshot("dashboard").await;
        assert!(snapshot
            .iter()
            .find(|e| e.source_id == "old")
            .unwrap()
            .data
            .is_none());
        let new = snapshot
            .iter()
            .find(|e| e.source_id == "new")
            .unwrap()
            .data
            .as_ref()
            .unwrap();
        assert_eq!(new["data"], json!({"tick":true}));
        assert_eq!(new["channel"], "new_metrics");
        gcx.apps.shutdown().await;
    }

    #[tokio::test]
    async fn transform_outbox_projects_fields_into_same_app_broker() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let host = gcx.apps.get(temp.path().into(), gcx.clone()).await.unwrap();
        host.automation
            .configure(
                "dashboard",
                "1".into(),
                vec![internal("input", "public_metrics", "public_metrics")],
            )
            .await
            .unwrap();
        host.automation
            .set_grant("dashboard", "input", true)
            .await
            .unwrap();
        host.automation
            .set_demand("dashboard", "input", "view", true)
            .await
            .unwrap();
        let definition: refact_scheduler_core::automation::Definition = serde_json::from_value(json!({
            "schema_version":1,"id":"transform_bridge","revision":1,"owner":{"kind":"project"},
            "description":"project selected data","enabled":true,
            "trigger":{"kind":"channel_event","channel":"raw_metrics","filter":null},
            "action":{"kind":"transform","input_channel":"raw_metrics","output_channel":"public_metrics","select_fields":["public"],"max_output_bytes":1024},
            "deliveries":[{"id":"public","channel":"public_metrics","credential_ref":null}],
            "lifecycle":{"kind":"on_demand"},
            "policy":{"idempotent":true,"max_pending_events":16,"max_event_bytes":1024}
        })).unwrap();
        let catalog = host
            .generic
            .save(0, vec![definition.clone()])
            .await
            .unwrap();
        host.generic
            .apply_exact(
                &definition.id,
                catalog.revision,
                1,
                &definition.security_digest().unwrap(),
            )
            .await
            .unwrap();
        host.generic
            .emit_event_id(
                "raw_metrics",
                json!({"public":7,"secret":"not selected"}),
                "original-one",
                0,
            )
            .await
            .unwrap();
        host.generic.drain_deliveries().await.unwrap();
        let source = host.automation.snapshot("dashboard").await.remove(0);
        assert_eq!(source.data.as_ref().unwrap()["data"], json!({"public":7}));
        assert_eq!(
            source.data.as_ref().unwrap()["provenance"]["producer"],
            "transform_bridge"
        );
        host.generic
            .emit_event_id(
                "raw_metrics",
                json!({"public":7,"secret":"not selected"}),
                "original-one",
                0,
            )
            .await
            .unwrap();
        host.generic.drain_deliveries().await.unwrap();
        assert_eq!(
            host.automation.snapshot("dashboard").await[0].seq,
            source.seq
        );
        gcx.apps.shutdown().await;
    }

    #[tokio::test]
    async fn trusted_publication_replay_is_durable_but_equal_independent_updates_survive() {
        let temp = tempfile::tempdir().unwrap();
        let service = AutomationService::new(temp.path()).unwrap();
        for invalid_seq in [i64::MIN, -1, 0] {
            assert!(publish(
                &service,
                "metrics",
                json!({"value":7}),
                "invalid",
                "collector",
                None,
                "digest",
                invalid_seq
            )
            .await
            .is_err());
        }
        service
            .configure(
                "dashboard",
                "1".into(),
                vec![internal("input", "metrics", "metrics")],
            )
            .await
            .unwrap();
        service.set_grant("dashboard", "input", true).await.unwrap();
        service
            .set_demand("dashboard", "input", "view", true)
            .await
            .unwrap();
        let owner = crate::scheduler::generic_runtime::PROJECT_SOURCE_OWNER;
        service
            .configure(
                owner,
                "1".into(),
                vec![internal("loop", "metrics", "metrics")],
            )
            .await
            .unwrap();
        service.set_grant(owner, "loop", true).await.unwrap();
        service
            .set_demand(owner, "loop", "background", true)
            .await
            .unwrap();
        let db = rusqlite::Connection::open(
            refact_scheduler_core::automation::journal_path(temp.path()).unwrap(),
        )
        .unwrap();
        db.execute_batch("CREATE TRIGGER fail_bridge BEFORE INSERT ON events BEGIN SELECT RAISE(FAIL, 'bridge admission'); END;").unwrap();
        assert!(publish(
            &service,
            "metrics",
            json!({"value":7}),
            "one",
            "collector",
            None,
            "digest",
            1
        )
        .await
        .is_err());
        assert!(service.snapshot("dashboard").await[0].data.is_none());
        db.execute_batch("DROP TRIGGER fail_bridge;").unwrap();
        publish(
            &service,
            "metrics",
            json!({"value":7}),
            "one",
            "collector",
            None,
            "digest",
            1,
        )
        .await
        .unwrap();
        let first = service.snapshot("dashboard").await[0].clone();
        publish(
            &service,
            "metrics",
            json!({"value":7}),
            "one",
            "collector",
            None,
            "digest",
            1,
        )
        .await
        .unwrap();
        assert_eq!(service.snapshot("dashboard").await[0].seq, first.seq);
        publish(
            &service,
            "metrics",
            json!({"value":7}),
            "two",
            "collector",
            None,
            "digest",
            2,
        )
        .await
        .unwrap();
        assert_eq!(service.snapshot("dashboard").await[0].seq, first.seq + 1);
        assert_eq!(
            service.snapshot("dashboard").await[0]
                .data
                .as_ref()
                .unwrap()["data"],
            json!({"value":7})
        );
        assert!(
            service.snapshot(owner).await[0].data.is_none(),
            "project collector consumed its own handoff"
        );
        assert!(publish(
            &service,
            "metrics",
            json!("x".repeat(MAX_CHANNEL_BYTES)),
            "large",
            "collector",
            None,
            "digest",
            3
        )
        .await
        .is_err());
        service.shutdown().await;
    }
}

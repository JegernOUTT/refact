use std::fmt;
use std::ops::Deref;

use refact_core::chat_types::ChatMessage;

use crate::destination::Destination;
use crate::matching::CompiledPolicy;
use crate::record::{FileRecord, PrivacyRecord};

pub const PRIVACY_CONTEXT_CHANGED: &str = "privacy context changed";

/// A value that passed the privacy policy for one destination.
///
/// The inner value cannot be constructed outside this crate.
///
/// ```compile_fail
/// use refact_privacy::Cleared;
///
/// let _ = Cleared("unchecked");
/// ```
pub struct Cleared<T>(T);

impl<T> Deref for Cleared<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> Cleared<T> {
    #[cfg(feature = "test-util")]
    pub(crate) fn for_testing(value: T) -> Self {
        Self(value)
    }
}

pub trait PrivacyAudited {
    fn privacy_records(&self) -> Result<Vec<(usize, FileRecord)>, PrivacyAuditError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivacyAuditError {
    pub message_index: usize,
    pub message: String,
}

impl fmt::Display for PrivacyAuditError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for PrivacyAuditError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub destination: Destination,
    pub offending: Vec<(usize, FileRecord)>,
    pub message: String,
}

impl Refusal {
    pub fn model_facing(&self) -> String {
        let target = self.destination.id.0.as_str();
        if self.offending.is_empty() {
            return if self.message == PRIVACY_CONTEXT_CHANGED {
                "Request not sent: user privacy policy — privacy context kept changing, so verification could not complete. Try again once the privacy context is stable.".to_string()
            } else {
                "Request not sent: user privacy policy — privacy metadata or policy could not be verified. Resolve the privacy verification problem before trying again.".to_string()
            };
        }
        let mut listed = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        let mut zones = std::collections::BTreeSet::new();
        let mut hidden = 0usize;
        for (_, record) in &self.offending {
            zones.insert(format!("\"{}\"", record.zone));
            if !seen.insert((&record.path, &record.zone)) {
                continue;
            }
            if listed.len() < 5 {
                let attribution = match record.attribution {
                    crate::record::Attribution::Declared => "declared",
                    crate::record::Attribution::Observed => "observed",
                    crate::record::Attribution::Heuristic => "heuristic",
                };
                listed.push(format!(
                    "  - {} (zone \"{}\", {attribution})",
                    record.path, record.zone
                ));
            } else {
                hidden += 1;
            }
        }
        if hidden > 0 {
            listed.push(format!("  - ...and {hidden} more"));
        }
        let zones = zones.into_iter().collect::<Vec<_>>().join(", ");
        format!(
            "Request not sent: user privacy policy does not allow \"{target}\" to receive data from these guarded files present in this chat's context:\n{}\nRetrying the same request will be refused again. Ask the user to allow \"{target}\" for zone(s) {zones} in Privacy settings, or to clear this chat's privacy record for these files.",
            listed.join("\n")
        )
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for Refusal {}

pub fn records_from_messages(
    messages: &[ChatMessage],
) -> Result<Vec<(usize, FileRecord)>, PrivacyAuditError> {
    let mut records = Vec::new();
    for (message_index, message) in messages.iter().enumerate() {
        if shell_output_decision_applied(message) {
            continue;
        }
        if message.extra.contains_key("privacy_ref") {
            return Err(PrivacyAuditError {
                message_index,
                message: format!("message {message_index} requires scoped provenance resolution"),
            });
        }
        let Some(value) = message.extra.get("privacy") else {
            continue;
        };
        let privacy = serde_json::from_value::<PrivacyRecord>(value.clone()).map_err(|error| {
            PrivacyAuditError {
                message_index,
                message: format!(
                    "message {message_index} contains malformed privacy metadata: {error}"
                ),
            }
        })?;
        records.extend(
            privacy
                .files
                .into_iter()
                .map(|record| (message_index, record)),
        );
    }
    Ok(records)
}

fn shell_output_decision_applied(message: &ChatMessage) -> bool {
    let shell = message.extra.get("privacy_shell");
    let decided = shell
        .and_then(|value| value.get("withheld"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
        || shell
            .and_then(|value| value.get("approved"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
    decided
        || message
            .extra
            .get("privacy_observation")
            .and_then(|value| value.get("degraded"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
}

pub fn clear<T: PrivacyAudited>(
    value: T,
    destination: &Destination,
    policy: &CompiledPolicy,
) -> Result<Cleared<T>, Refusal> {
    let records = value.privacy_records().map_err(|error| Refusal {
        destination: destination.clone(),
        offending: Vec::new(),
        message: error.message,
    })?;
    let offending: Vec<_> = records
        .into_iter()
        .filter(|(_, record)| !record_is_allowed(record, destination, policy))
        .collect();

    if let Some((index, record)) = offending.first() {
        return Err(Refusal {
            destination: destination.clone(),
            message: format!(
                "destination {} cannot receive message {index} path {}",
                destination.id.0, record.path
            ),
            offending,
        });
    }

    Ok(Cleared(value))
}

fn record_is_allowed(
    record: &FileRecord,
    destination: &Destination,
    policy: &CompiledPolicy,
) -> bool {
    if record.path.is_empty() || record.path.contains('\0') || record.zone == "blocked" {
        return false;
    }
    if let Some(zone) = policy.zone_named(&record.zone) {
        return destination.matches_send_to(&zone.send_to);
    }
    let current = policy.zone_for_path(std::path::Path::new(&record.path));
    if !destination.matches_send_to(&current.send_to) {
        return false;
    }
    if let Some(zone_names) = record.zone.strip_prefix("effective:") {
        return !zone_names.is_empty()
            && zone_names.split('+').all(|zone_name| {
                !zone_name.is_empty()
                    && zone_name != "blocked"
                    && policy
                        .zone_named(zone_name)
                        .is_none_or(|zone| destination.matches_send_to(&zone.send_to))
            });
    }
    true
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use refact_core::chat_types::ChatMessage;

    use super::*;
    use crate::destination::{DestinationId, DestinationKind};
    use crate::policy::{PrivacyPolicy, ShellBehavior, SubagentPolicy, Zone};
    use crate::record::Attribution;

    struct AuditedRecords {
        value: String,
        records: Vec<FileRecord>,
    }

    impl PrivacyAudited for AuditedRecords {
        fn privacy_records(&self) -> Result<Vec<(usize, FileRecord)>, PrivacyAuditError> {
            Ok(self.records.iter().cloned().enumerate().collect())
        }
    }

    struct AuditedMessages {
        messages: Arc<Vec<ChatMessage>>,
    }

    impl PrivacyAudited for AuditedMessages {
        fn privacy_records(&self) -> Result<Vec<(usize, FileRecord)>, PrivacyAuditError> {
            records_from_messages(&self.messages)
        }
    }

    fn destination(id: &str) -> Destination {
        Destination {
            id: DestinationId(id.to_string()),
            kind: DestinationKind::Provider,
            display_name: id.to_string(),
        }
    }

    fn policy() -> PrivacyPolicy {
        PrivacyPolicy {
            blocked: Vec::new(),
            zones: vec![
                Zone {
                    name: "secrets".to_string(),
                    patterns: vec![".env*".to_string()],
                    send_to: vec!["trusted".to_string()],
                    on_shell_read: ShellBehavior::Withhold,
                },
                Zone {
                    name: "normal".to_string(),
                    patterns: vec!["*".to_string()],
                    send_to: vec!["*".to_string()],
                    on_shell_read: ShellBehavior::Withhold,
                },
            ],
            subagents: SubagentPolicy::default(),
            ..Default::default()
        }
    }

    fn record(path: &str, zone: &str) -> FileRecord {
        FileRecord {
            path: path.to_string(),
            zone: zone.to_string(),
            attribution: Attribution::Declared,
        }
    }

    fn compiled(policy: &PrivacyPolicy) -> CompiledPolicy {
        policy.compile().expect("policy should compile")
    }

    fn message_with_record(path: &str, zone: &str) -> ChatMessage {
        message_with_records(vec![record(path, zone)])
    }

    fn message_with_records(files: Vec<FileRecord>) -> ChatMessage {
        let mut message = ChatMessage::new("tool".to_string(), "result".to_string());
        message.extra.insert(
            "privacy".to_string(),
            serde_json::to_value(PrivacyRecord {
                files,
            })
            .expect("privacy record should serialize"),
        );
        message
    }

    #[test]
    fn clear_returns_token_when_all_records_are_allowed() {
        let audited = AuditedRecords {
            value: "payload".to_string(),
            records: vec![record("src/main.rs", "normal"), record(".env", "secrets")],
        };

        let policy = compiled(&policy());
        let cleared = clear(audited, &destination("trusted"), &policy)
            .expect("trusted destination should be allowed");

        assert_eq!(cleared.value, "payload");
    }

    #[test]
    fn synthesized_normal_zone_is_allowed_by_clearance() {
        let policy = compiled(&PrivacyPolicy::default());
        let classified = policy.zone_for_path(std::path::Path::new("src/main.rs"));
        let audited = AuditedRecords {
            value: "payload".to_string(),
            records: vec![record("src/main.rs", &classified.name)],
        };

        assert_eq!(classified.name, "normal");
        assert!(clear(audited, &destination("untrusted"), &policy).is_ok());
    }

    #[test]
    fn synthetic_effective_zone_keeps_the_intersected_destination_set() {
        let policy = compiled(&PrivacyPolicy {
            blocked: Vec::new(),
            zones: vec![
                Zone {
                    name: "a".to_string(),
                    patterns: vec!["a.txt".to_string()],
                    send_to: vec!["trusted".to_string(), "a-only".to_string()],
                    on_shell_read: ShellBehavior::Withhold,
                },
                Zone {
                    name: "b".to_string(),
                    patterns: vec!["b.txt".to_string()],
                    send_to: vec!["trusted".to_string(), "b-only".to_string()],
                    on_shell_read: ShellBehavior::Withhold,
                },
            ],
            subagents: SubagentPolicy::default(),
            ..Default::default()
        });
        let effective = policy.strictest_zone_for_paths([
            std::path::Path::new("a.txt"),
            std::path::Path::new("b.txt"),
        ]);
        let effective_record = record("a.txt", &effective.name);

        assert_eq!(effective.name, "effective:a+b");
        assert_eq!(effective.send_to, ["trusted"]);
        assert!(clear(
            AuditedRecords {
                value: "payload".to_string(),
                records: vec![effective_record.clone()],
            },
            &destination("trusted"),
            &policy,
        )
        .is_ok());
        assert!(clear(
            AuditedRecords {
                value: "payload".to_string(),
                records: vec![effective_record.clone()],
            },
            &destination("a-only"),
            &policy,
        )
        .is_err());
        assert!(clear(
            AuditedRecords {
                value: "payload".to_string(),
                records: vec![effective_record],
            },
            &destination("untrusted"),
            &policy,
        )
        .is_err());
    }

    #[test]
    fn zone_without_destinations_is_still_refused() {
        let policy = compiled(&PrivacyPolicy {
            blocked: Vec::new(),
            zones: vec![Zone {
                name: "secrets".to_string(),
                patterns: vec!["**/*.pem".to_string()],
                send_to: Vec::new(),
                on_shell_read: ShellBehavior::Withhold,
            }],
            subagents: SubagentPolicy::default(),
            ..Default::default()
        });
        let audited = AuditedRecords {
            value: "payload".to_string(),
            records: vec![record("keys/private.pem", "secrets")],
        };

        assert!(clear(audited, &destination("untrusted"), &policy).is_err());
    }

    #[test]
    fn refusal_names_the_source_message_index_and_path() {
        let messages = Arc::new(vec![
            message_with_records(vec![
                record("src/main.rs", "normal"),
                record("src/lib.rs", "normal"),
            ]),
            message_with_records(vec![
                record(".env", "secrets"),
                record("keys.txt", "secrets"),
            ]),
        ]);
        let audited = AuditedMessages {
            messages,
        };

        let policy = compiled(&policy());
        let refusal = match clear(audited, &destination("untrusted"), &policy) {
            Ok(_) => panic!("untrusted destination should be refused"),
            Err(refusal) => refusal,
        };

        assert_eq!(refusal.offending[0], (1, record(".env", "secrets")));
        assert!(refusal.message.contains("message 1"));
        assert!(refusal.message.contains(".env"));
    }

    #[test]
    fn refusal_does_not_mutate_messages() {
        let messages = Arc::new(vec![
            message_with_record("src/main.rs", "normal"),
            message_with_record(".env", "secrets"),
        ]);
        let before = serde_json::to_value(messages.as_ref()).expect("messages should serialize");
        let audited = AuditedMessages {
            messages: Arc::clone(&messages),
        };

        let policy = compiled(&policy());
        let refusal = match clear(audited, &destination("untrusted"), &policy) {
            Ok(_) => panic!("untrusted destination should be refused"),
            Err(refusal) => refusal,
        };

        let after = serde_json::to_value(messages.as_ref()).expect("messages should serialize");
        assert_eq!(refusal.offending[0].0, 1);
        assert_eq!(before, after);
    }

    #[test]
    fn malformed_message_privacy_metadata_fails_closed() {
        let mut malformed = ChatMessage::new("tool".to_string(), "result".to_string());
        malformed.extra.insert(
            "privacy".to_string(),
            serde_json::json!({ "files": "not-an-array" }),
        );
        let audited = AuditedMessages {
            messages: Arc::new(vec![
                ChatMessage::new("user".to_string(), "hello".to_string()),
                malformed,
            ]),
        };
        let policy = compiled(&PrivacyPolicy::default());

        let refusal = match clear(audited, &destination("untrusted"), &policy) {
            Ok(_) => panic!("malformed privacy metadata must fail closed"),
            Err(refusal) => refusal,
        };

        assert!(refusal.message.contains("message 1"));
        assert!(refusal.message.contains("malformed privacy metadata"));
    }

    #[test]
    fn verification_failure_display_is_safe_and_distinct_from_denial() {
        let transient = Refusal {
            destination: destination("private/internal/provider"),
            offending: Vec::new(),
            message: PRIVACY_CONTEXT_CHANGED.into(),
        };
        let generic = Refusal {
            message: "malformed metadata at /private/internal/path: secret contents".into(),
            ..transient.clone()
        };
        assert_eq!(transient.model_facing(), "Request not sent: user privacy policy — privacy context kept changing, so verification could not complete. Try again once the privacy context is stable.");
        assert_eq!(generic.model_facing(), "Request not sent: user privacy policy — privacy metadata or policy could not be verified. Resolve the privacy verification problem before trying again.");
        let refusal = clear(
            AuditedRecords {
                value: "payload".into(),
                records: vec![record(".env", "secrets")],
            },
            &destination("untrusted"),
            &compiled(&policy()),
        )
        .err()
        .unwrap();
        assert_eq!(refusal.model_facing(), "Request not sent: user privacy policy does not allow \"untrusted\" to receive data from these guarded files present in this chat's context:\n  - .env (zone \"secrets\", declared)\nRetrying the same request will be refused again. Ask the user to allow \"untrusted\" for zone(s) \"secrets\" in Privacy settings, or to clear this chat's privacy record for these files.");
    }

    #[test]
    fn refusal_message_index_is_not_the_flattened_record_ordinal() {
        let messages = Arc::new(vec![
            message_with_records(vec![
                record("src/first.rs", "normal"),
                record("src/second.rs", "normal"),
            ]),
            message_with_records(vec![record("keys/private.pem", "secrets")]),
        ]);
        let audited = AuditedMessages {
            messages,
        };
        let policy = compiled(&policy());

        let refusal = match clear(audited, &destination("untrusted"), &policy) {
            Ok(_) => panic!("guarded file must be refused"),
            Err(refusal) => refusal,
        };

        assert_eq!(refusal.offending[0].0, 1);
        assert!(refusal.message.contains("message 1"));
    }

    #[test]
    fn removed_zones_reclassify_paths_and_preserve_remaining_obligations() {
        for (path, zone, target, allowed) in [
            ("guarded", "removed", "untrusted", true),
            (".env", "removed", "untrusted", false),
            (".env", "removed", "trusted", true),
            ("guarded", "effective:removed+secrets", "untrusted", false),
            (
                "guarded",
                "effective:removed+also-removed",
                "untrusted",
                true,
            ),
            ("guarded", "blocked", "trusted", false),
            ("guarded", "effective:removed+blocked", "trusted", false),
            ("", "removed", "trusted", false),
            ("bad\0path", "removed", "trusted", false),
            ("guarded", "effective:", "trusted", false),
        ] {
            let audited = AuditedRecords {
                value: "payload".to_string(),
                records: vec![record(path, zone)],
            };

            let policy = compiled(&policy());
            assert_eq!(
                clear(audited, &destination(target), &policy).is_ok(),
                allowed,
                "path={path:?}, zone={zone}, target={target}"
            );
        }
    }
}

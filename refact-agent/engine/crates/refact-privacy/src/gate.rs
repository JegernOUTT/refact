use std::fmt;
use std::ops::Deref;

use refact_core::chat_types::ChatMessage;

use crate::destination::Destination;
use crate::policy::PrivacyPolicy;
use crate::record::{FileRecord, PrivacyRecord};

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
    fn privacy_records(&self) -> Vec<FileRecord>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub destination: Destination,
    pub offending: Vec<(usize, FileRecord)>,
    pub message: String,
}

impl fmt::Display for Refusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for Refusal {}

pub fn records_from_messages(messages: &[ChatMessage]) -> Vec<FileRecord> {
    messages
        .iter()
        .filter_map(|message| message.extra.get("privacy"))
        .filter_map(|value| serde_json::from_value::<PrivacyRecord>(value.clone()).ok())
        .flat_map(|record| record.files)
        .collect()
}

pub fn clear<T: PrivacyAudited>(
    value: T,
    destination: &Destination,
    policy: &PrivacyPolicy,
) -> Result<Cleared<T>, Refusal> {
    let offending: Vec<_> = value
        .privacy_records()
        .into_iter()
        .enumerate()
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
    policy: &PrivacyPolicy,
) -> bool {
    record.zone != "blocked"
        && policy
            .zones
            .iter()
            .find(|zone| zone.name == record.zone)
            .is_some_and(|zone| destination.matches_send_to(&zone.send_to))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use refact_core::chat_types::ChatMessage;

    use super::*;
    use crate::destination::{DestinationId, DestinationKind};
    use crate::policy::{ShellBehavior, SubagentPolicy, Zone};
    use crate::record::Attribution;

    struct AuditedRecords {
        value: String,
        records: Vec<FileRecord>,
    }

    impl PrivacyAudited for AuditedRecords {
        fn privacy_records(&self) -> Vec<FileRecord> {
            self.records.clone()
        }
    }

    struct AuditedMessages {
        messages: Arc<Vec<ChatMessage>>,
    }

    impl PrivacyAudited for AuditedMessages {
        fn privacy_records(&self) -> Vec<FileRecord> {
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
        }
    }

    fn record(path: &str, zone: &str) -> FileRecord {
        FileRecord {
            path: path.to_string(),
            zone: zone.to_string(),
            attribution: Attribution::Declared,
        }
    }

    fn message_with_record(path: &str, zone: &str) -> ChatMessage {
        let mut message = ChatMessage::new("tool".to_string(), "result".to_string());
        message.extra.insert(
            "privacy".to_string(),
            serde_json::to_value(PrivacyRecord {
                files: vec![record(path, zone)],
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

        let cleared = clear(audited, &destination("trusted"), &policy())
            .expect("trusted destination should be allowed");

        assert_eq!(cleared.value, "payload");
    }

    #[test]
    fn refusal_names_the_first_offending_index_and_path() {
        let audited = AuditedRecords {
            value: "payload".to_string(),
            records: vec![
                record("src/main.rs", "normal"),
                record(".env", "secrets"),
                record("keys.txt", "secrets"),
            ],
        };

        let refusal = match clear(audited, &destination("untrusted"), &policy()) {
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

        let refusal = match clear(audited, &destination("untrusted"), &policy()) {
            Ok(_) => panic!("untrusted destination should be refused"),
            Err(refusal) => refusal,
        };

        let after = serde_json::to_value(messages.as_ref()).expect("messages should serialize");
        assert_eq!(refusal.offending[0].0, 1);
        assert_eq!(before, after);
    }

    #[test]
    fn records_are_derived_from_message_privacy_metadata() {
        let messages = vec![
            ChatMessage::new("user".to_string(), "hello".to_string()),
            message_with_record(".env", "secrets"),
        ];

        assert_eq!(
            records_from_messages(&messages),
            vec![record(".env", "secrets")]
        );
    }

    #[test]
    fn unknown_and_blocked_zones_fail_closed() {
        for zone in ["unknown", "blocked"] {
            let audited = AuditedRecords {
                value: "payload".to_string(),
                records: vec![record("guarded", zone)],
            };

            assert!(clear(audited, &destination("trusted"), &policy()).is_err());
        }
    }
}

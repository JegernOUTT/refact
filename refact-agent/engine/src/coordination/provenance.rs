use std::collections::HashSet;

use refact_privacy::FileRecord;
use refact_work_core::CoordinationSettings;

use crate::call_validation::ChatMessage;

pub struct WorkProvenance {
    records: Vec<FileRecord>,
    seen: HashSet<FileRecord>,
    bytes: usize,
    message_count: usize,
    complete: bool,
}

impl Default for WorkProvenance {
    fn default() -> Self {
        Self {
            records: Vec::new(),
            seen: HashSet::new(),
            bytes: 0,
            message_count: 0,
            complete: true,
        }
    }
}

impl WorkProvenance {
    pub fn matches_message_count(&self, count: usize) -> bool {
        self.message_count == count
    }

    pub fn from_messages(messages: &[ChatMessage], settings: &CoordinationSettings) -> Self {
        let mut value = Self::default();
        for message in messages {
            value.observe(message, messages.len(), settings);
            if !value.complete {
                break;
            }
        }
        value.message_count = messages.len();
        value
    }

    fn invalidate(&mut self) {
        self.records.clear();
        self.seen.clear();
        self.bytes = 0;
        self.complete = false;
    }

    pub fn observe(
        &mut self,
        message: &ChatMessage,
        message_count: usize,
        settings: &CoordinationSettings,
    ) {
        self.message_count = message_count;
        if !self.complete {
            return;
        }
        let Ok(records) = crate::privacy::records::records_to_carry(std::slice::from_ref(message))
        else {
            self.invalidate();
            return;
        };
        for record in records {
            if self.seen.contains(&record) {
                continue;
            }
            let bytes = record.path.len().saturating_add(record.zone.len());
            if self.records.len() >= settings.max_work_provenance_records
                || self.bytes.saturating_add(bytes) > settings.max_work_provenance_bytes
            {
                self.invalidate();
                return;
            }
            self.bytes += bytes;
            self.seen.insert(record.clone());
            self.records.push(record);
        }
    }

    pub fn snapshot(
        &self,
        message_count: usize,
        settings: &CoordinationSettings,
    ) -> (Vec<FileRecord>, bool) {
        if !self.complete
            || message_count != self.message_count
            || self.records.len() > settings.max_work_provenance_records
            || self.bytes > settings.max_work_provenance_bytes
        {
            return (Vec::new(), false);
        }
        (self.records.clone(), true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn message(path: &str) -> ChatMessage {
        let mut message = ChatMessage::new("tool".into(), "observed".into());
        message.extra.insert(
            "privacy".into(),
            json!({"files":[{
                "path":path,"zone":"guarded","attribution":"declared"
            }]}),
        );
        message
    }

    #[test]
    fn repeated_sources_are_deduplicated_and_unknown_mutations_fail_closed() {
        let settings = CoordinationSettings::default();
        let mut value = WorkProvenance::default();
        value.observe(&message("/p/secret"), 1, &settings);
        value.observe(&message("/p/secret"), 2, &settings);
        assert_eq!(value.snapshot(2, &settings).0.len(), 1);
        assert!(value.snapshot(2, &settings).1);
        assert!(!value.snapshot(3, &settings).1);
    }

    #[test]
    fn overflow_and_malformed_provenance_stay_closed_until_authoritative_rebuild() {
        let settings = CoordinationSettings {
            max_work_provenance_records: 1,
            ..Default::default()
        };
        let mut value = WorkProvenance::default();
        value.observe(&message("/p/a"), 1, &settings);
        value.observe(&message("/p/b"), 2, &settings);
        assert_eq!(value.snapshot(2, &settings), (Vec::new(), false));
        value.observe(&ChatMessage::default(), 3, &settings);
        assert!(!value.snapshot(3, &settings).1);
        value = WorkProvenance::from_messages(&[message("/p/a")], &settings);
        assert!(value.snapshot(1, &settings).1);
        let mut malformed = message("/p/a");
        malformed
            .extra
            .insert("privacy".into(), json!({"files":"not an array"}));
        value.observe(&malformed, 2, &settings);
        assert!(!value.snapshot(2, &settings).1);
    }

    #[test]
    fn byte_caps_and_live_reduction_never_return_truncated_clearance() {
        let settings = CoordinationSettings::default();
        let value = WorkProvenance::from_messages(&[message("/p/secret")], &settings);
        let narrow = CoordinationSettings {
            max_work_provenance_bytes: 1,
            ..settings
        };
        assert_eq!(value.snapshot(1, &narrow), (Vec::new(), false));
        let value = WorkProvenance::from_messages(&[message("/p/secret")], &narrow);
        assert_eq!(value.snapshot(1, &narrow), (Vec::new(), false));
    }
}

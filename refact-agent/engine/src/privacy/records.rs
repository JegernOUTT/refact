use std::path::{Path, PathBuf};
use std::sync::Arc;

use refact_privacy::{Attribution, FileRecord, PrivacyRecord};

use crate::call_validation::ChatMessage;
use crate::files_correction::registered_worktree_path_mappings;
use crate::files_in_workspace::strictest_zone_for_path;
use crate::global_context::GlobalContext;

pub fn attach_record(message: &mut ChatMessage, record: FileRecord) {
    merge_records(message, std::iter::once(record));
}

pub fn merge_records(message: &mut ChatMessage, records: impl IntoIterator<Item = FileRecord>) {
    let mut privacy = message
        .extra
        .remove("privacy")
        .and_then(|value| serde_json::from_value::<PrivacyRecord>(value).ok())
        .unwrap_or_default();
    for record in records {
        if !privacy.files.contains(&record) {
            privacy.files.push(record);
        }
    }
    if !privacy.files.is_empty() {
        message.extra.insert(
            "privacy".to_string(),
            serde_json::to_value(privacy).expect("privacy records should serialize"),
        );
    }
}

pub fn merge_message_records<'a>(
    message: &mut ChatMessage,
    sources: impl IntoIterator<Item = &'a ChatMessage>,
) {
    let records = sources
        .into_iter()
        .filter_map(|source| source.extra.get("privacy"))
        .filter_map(|value| serde_json::from_value::<PrivacyRecord>(value.clone()).ok())
        .flat_map(|privacy| privacy.files);
    merge_records(message, records);
}

pub fn declared_file_record(gcx: &Arc<GlobalContext>, path: &Path) -> Result<FileRecord, String> {
    let policy = gcx.privacy_policy_load.read().unwrap().policy.clone();
    let compiled = policy.compile().map_err(|error| error.to_string())?;
    let mappings = registered_worktree_path_mappings(gcx.cache_dir.as_path());
    Ok(FileRecord {
        path: refact_core::chat_types::normalize_file_name(path.to_string_lossy().into_owned()),
        zone: strictest_zone_for_path(&compiled, path, &mappings)
            .name
            .clone(),
        attribution: Attribution::Declared,
    })
}

pub fn declared_file_records(
    gcx: &Arc<GlobalContext>,
    paths: impl IntoIterator<Item = PathBuf>,
) -> Result<Vec<FileRecord>, String> {
    let mut records = Vec::new();
    for path in paths {
        let record = declared_file_record(gcx, &path)?;
        if !records.contains(&record) {
            records.push(record);
        }
    }
    Ok(records)
}

pub fn attach_declared_output_files(
    gcx: &Arc<GlobalContext>,
    messages: &mut [ChatMessage],
) -> Result<(), String> {
    for message in messages {
        let paths = match &message.content {
            crate::call_validation::ChatContent::ContextFiles(files) => files
                .iter()
                .map(|file| PathBuf::from(&file.file_name))
                .collect::<Vec<_>>(),
            crate::call_validation::ChatContent::SimpleText(text) if message.role == "diff" => {
                serde_json::from_str::<Vec<crate::call_validation::DiffChunk>>(text)
                    .unwrap_or_default()
                    .into_iter()
                    .flat_map(|chunk| {
                        std::iter::once(PathBuf::from(chunk.file_name)).chain(
                            chunk
                                .file_name_rename
                                .filter(|path| !path.is_empty())
                                .map(PathBuf::from),
                        )
                    })
                    .collect()
            }
            _ => continue,
        };
        let records = declared_file_records(gcx, paths)?;
        merge_records(message, records);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_records_preserves_existing_records_and_deduplicates() {
        let mut message = ChatMessage::default();
        let first = FileRecord {
            path: "a.rs".to_string(),
            zone: "normal".to_string(),
            attribution: Attribution::Declared,
        };
        let second = FileRecord {
            path: ".env".to_string(),
            zone: "secrets".to_string(),
            attribution: Attribution::Declared,
        };

        attach_record(&mut message, first.clone());
        merge_records(&mut message, [first, second.clone()]);

        let privacy: PrivacyRecord =
            serde_json::from_value(message.extra["privacy"].clone()).unwrap();
        assert_eq!(privacy.files.len(), 2);
        assert_eq!(privacy.files[1], second);
    }

    #[test]
    fn merge_message_records_unions_source_records() {
        let first = FileRecord {
            path: "a.rs".to_string(),
            zone: "normal".to_string(),
            attribution: Attribution::Declared,
        };
        let second = FileRecord {
            path: ".env".to_string(),
            zone: "secrets".to_string(),
            attribution: Attribution::Observed,
        };
        let mut source_a = ChatMessage::default();
        let mut source_b = ChatMessage::default();
        merge_records(&mut source_a, [first.clone(), second.clone()]);
        merge_records(&mut source_b, [second.clone()]);
        let mut target = ChatMessage::default();

        merge_message_records(&mut target, [&source_a, &source_b]);

        let privacy: PrivacyRecord =
            serde_json::from_value(target.extra["privacy"].clone()).unwrap();
        assert_eq!(privacy.files, vec![first, second]);
    }
}

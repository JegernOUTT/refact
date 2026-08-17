use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use refact_privacy::{
    Attribution, Destination, DestinationId, DestinationKind, FileRecord, PrivacyRecord,
    ShellBehavior,
};
use refact_exec::ObservationStatus;

use crate::call_validation::ChatMessage;
use crate::files_correction::registered_worktree_path_mappings;
use crate::files_in_workspace::registered_alias_paths;
use crate::global_context::GlobalContext;

pub const SHELL_WITHHELD_MESSAGE: &str = "Output withheld by user privacy policy — this command read guarded files. Other tools will refuse identically. Do not retry.";
pub const SHELL_APPROVAL_MESSAGE: &str =
    "Output awaiting user approval — this command read guarded files.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellReadDecision {
    Pass,
    Ask,
}

pub type DerivedPrivacyZones = Arc<RwLock<HashMap<PathBuf, String>>>;

pub fn new_derived_privacy_zones() -> DerivedPrivacyZones {
    Arc::new(RwLock::new(HashMap::new()))
}

pub fn provider_destination(model_id: &str) -> Destination {
    Destination {
        id: DestinationId(
            model_id
                .split_once('/')
                .map_or(model_id, |(provider, _)| provider)
                .to_string(),
        ),
        kind: DestinationKind::Provider,
        display_name: model_id.to_string(),
    }
}

pub fn shell_observation_needed(gcx: &Arc<GlobalContext>, destination: &Destination) -> bool {
    let policy = gcx.privacy_policy_load.read().unwrap().policy.clone();
    if policy.blocked.is_empty()
        && policy
            .zones
            .iter()
            .all(|zone| destination.matches_send_to(&zone.send_to))
    {
        return false;
    }
    let workspace_files = gcx.documents_state.workspace_files.lock().unwrap().clone();
    if workspace_files.is_empty() {
        return false;
    }
    let Ok(compiled) = policy.compile() else {
        return true;
    };
    let mappings = registered_worktree_path_mappings(gcx.cache_dir.as_path());
    let roots = privacy_roots(gcx, &mappings);
    workspace_files.into_iter().any(|path| {
        let candidates = record_path_candidates(gcx, &path, &mappings);
        let zone = compiled.strictest_zone_for_paths_with_roots(candidates, &roots);
        zone.name == "blocked" || !destination.matches_send_to(&zone.send_to)
    })
}

pub fn shell_observation_needed_for_session(
    gcx: &Arc<GlobalContext>,
    destination: &Destination,
    derived_zones: &DerivedPrivacyZones,
) -> bool {
    if shell_observation_needed(gcx, destination) {
        return true;
    }
    let derived_zone_names = derived_zones
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .values()
        .cloned()
        .collect::<Vec<_>>();
    let policy = gcx.privacy_policy_load.read().unwrap().policy.clone();
    derived_zone_names.iter().any(|name| {
        name == "blocked"
            || policy
                .zones
                .iter()
                .find(|zone| zone.name == *name)
                .is_some_and(|zone| !destination.matches_send_to(&zone.send_to))
    })
}

fn zone_for_record_path<'a>(
    gcx: &Arc<GlobalContext>,
    policy: &refact_privacy::PrivacyPolicy,
    compiled: &'a refact_privacy::CompiledPolicy,
    path: &Path,
    mappings: &[crate::files_correction::RegisteredWorktreePathMapping],
    derived_zones: &DerivedPrivacyZones,
) -> String {
    let candidates = record_path_candidates(gcx, path, mappings);
    let static_zone =
        compiled.strictest_zone_for_paths_with_roots(&candidates, privacy_roots(gcx, mappings));
    let derived_zone_name = {
        let derived_zones = derived_zones
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        candidates
            .iter()
            .filter_map(|candidate| derived_zones.get(candidate))
            .min_by_key(|name| zone_destination_count(policy, name))
            .cloned()
    };
    let Some(derived_zone_name) = derived_zone_name else {
        return static_zone.name.clone();
    };
    if zone_destination_count(policy, &derived_zone_name) < destination_count(&static_zone.send_to)
    {
        derived_zone_name
    } else {
        static_zone.name.clone()
    }
}

fn record_path_candidates(
    gcx: &Arc<GlobalContext>,
    path: &Path,
    mappings: &[crate::files_correction::RegisteredWorktreePathMapping],
) -> Vec<PathBuf> {
    let mut candidates = registered_alias_paths(path, mappings);
    let workspaces = gcx
        .documents_state
        .workspace_folders
        .lock()
        .unwrap()
        .clone();
    let aliases = candidates.clone();
    candidates.extend(workspaces.iter().flat_map(|workspace| {
        aliases
            .iter()
            .filter_map(move |alias| alias.strip_prefix(workspace).ok().map(Path::to_path_buf))
    }));
    candidates.sort();
    candidates.dedup();
    candidates
}

fn privacy_roots(
    gcx: &Arc<GlobalContext>,
    mappings: &[crate::files_correction::RegisteredWorktreePathMapping],
) -> Vec<PathBuf> {
    let mut roots = gcx
        .documents_state
        .workspace_folders
        .lock()
        .unwrap()
        .clone();
    roots.extend(
        mappings
            .iter()
            .flat_map(|mapping| [mapping.root.clone(), mapping.source_root.clone()]),
    );
    roots.sort();
    roots.dedup();
    roots
}

fn zone_destination_count(policy: &refact_privacy::PrivacyPolicy, name: &str) -> usize {
    if name == "blocked" {
        return 0;
    }
    policy
        .zones
        .iter()
        .find(|zone| zone.name == name)
        .map(|zone| destination_count(&zone.send_to))
        .unwrap_or_else(|| {
            if name == "normal" {
                usize::MAX
            } else {
                usize::MAX - 1
            }
        })
}

fn destination_count(destinations: &[String]) -> usize {
    if destinations.iter().any(|destination| destination == "*") {
        usize::MAX
    } else {
        destinations.len()
    }
}

pub async fn apply_shell_observation(
    gcx: &Arc<GlobalContext>,
    command: &str,
    cwd: &Path,
    destination: &Destination,
    observation: ObservationStatus,
    derived_zones: &DerivedPrivacyZones,
    message: &mut ChatMessage,
) -> Result<ShellReadDecision, String> {
    let policy = gcx.privacy_policy_load.read().unwrap().policy.clone();
    let records = match observation {
        ObservationStatus::Observed(access) => {
            let records = observed_file_records_with_derived(gcx, access.reads, derived_zones)?;
            inherit_observed_write_zones(gcx, &policy, &records, access.writes, derived_zones)?;
            records
        }
        ObservationStatus::Unavailable(reason) => {
            let compiled = policy.compile().map_err(|error| error.to_string())?;
            let heuristic =
                crate::privacy::heuristic::attribute_shell_command(command, cwd, &compiled);
            crate::privacy::warn_observation_degraded_once(gcx.clone(), &reason).await;
            message.extra.insert(
                "privacy_observation".to_string(),
                serde_json::json!({
                    "status": "unavailable",
                    "reason": reason,
                    "degraded": true,
                    "incomplete": heuristic.incomplete,
                }),
            );
            merge_records(message, heuristic.files);
            return Ok(ShellReadDecision::Pass);
        }
    };
    merge_records(message, records.clone());

    let behaviors = records.iter().filter_map(|record| {
        if record.zone == "blocked" {
            return Some(ShellBehavior::Deny);
        }
        policy
            .zones
            .iter()
            .find(|zone| zone.name == record.zone)
            .filter(|zone| !destination.matches_send_to(&zone.send_to))
            .map(|zone| zone.on_shell_read)
    });
    let behavior = behaviors.fold(None, |decision, behavior| match (decision, behavior) {
        (_, ShellBehavior::Deny) => Some(ShellBehavior::Deny),
        (Some(ShellBehavior::Deny), _) => Some(ShellBehavior::Deny),
        (_, ShellBehavior::Withhold) => Some(ShellBehavior::Withhold),
        (Some(ShellBehavior::Withhold), _) => Some(ShellBehavior::Withhold),
        (_, ShellBehavior::Ask) => Some(ShellBehavior::Ask),
    });
    match behavior {
        Some(ShellBehavior::Deny) => {
            Err("Denied by user privacy policy — this command read guarded files".to_string())
        }
        Some(ShellBehavior::Withhold) => {
            retain_local_shell_output(message, SHELL_WITHHELD_MESSAGE, false);
            Ok(ShellReadDecision::Pass)
        }
        Some(ShellBehavior::Ask) => {
            retain_local_shell_output(message, SHELL_APPROVAL_MESSAGE, true);
            Ok(ShellReadDecision::Ask)
        }
        None => Ok(ShellReadDecision::Pass),
    }
}

fn retain_local_shell_output(message: &mut ChatMessage, replacement: &str, ask_pending: bool) {
    let full_output = message.content.content_text_only();
    message.extra.insert(
        "privacy_shell".to_string(),
        serde_json::json!({
            "withheld": !ask_pending,
            "ask_pending": ask_pending,
            "local_only_output": full_output,
        }),
    );
    message.content = crate::call_validation::ChatContent::SimpleText(replacement.to_string());
}

pub fn shell_ask_pending(message: &ChatMessage) -> bool {
    message
        .extra
        .get("privacy_shell")
        .and_then(|value| value.get("ask_pending"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

pub fn resolve_shell_ask(message: &mut ChatMessage, accepted: bool) -> bool {
    if !shell_ask_pending(message) {
        return false;
    }
    let full_output = message
        .extra
        .get("privacy_shell")
        .and_then(|value| value.get("local_only_output"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    let Some(shell) = message
        .extra
        .get_mut("privacy_shell")
        .and_then(serde_json::Value::as_object_mut)
    else {
        return false;
    };
    shell.insert("ask_pending".to_string(), serde_json::Value::Bool(false));
    shell.insert("approved".to_string(), serde_json::Value::Bool(accepted));
    shell.insert("withheld".to_string(), serde_json::Value::Bool(!accepted));
    message.content = crate::call_validation::ChatContent::SimpleText(if accepted {
        full_output
    } else {
        SHELL_WITHHELD_MESSAGE.to_string()
    });
    true
}

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
        .filter(|source| !shell_result_is_locally_resolved(source))
        .filter_map(|source| source.extra.get("privacy"))
        .filter_map(|value| serde_json::from_value::<PrivacyRecord>(value.clone()).ok())
        .flat_map(|privacy| privacy.files);
    merge_records(message, records);
}

fn shell_result_is_locally_resolved(message: &ChatMessage) -> bool {
    message
        .extra
        .get("privacy_observation")
        .and_then(|value| value.get("degraded"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
        || message.extra.get("privacy_shell").is_some_and(|shell| {
            shell
                .get("withheld")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
                || shell
                    .get("approved")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false)
        })
}

fn file_record(
    gcx: &Arc<GlobalContext>,
    path: &Path,
    attribution: Attribution,
    derived_zones: &DerivedPrivacyZones,
) -> Result<FileRecord, String> {
    let policy = gcx.privacy_policy_load.read().unwrap().policy.clone();
    let compiled = policy.compile().map_err(|error| error.to_string())?;
    let mappings = registered_worktree_path_mappings(gcx.cache_dir.as_path());
    Ok(FileRecord {
        path: refact_core::chat_types::normalize_file_name(path.to_string_lossy().into_owned()),
        zone: zone_for_record_path(gcx, &policy, &compiled, path, &mappings, derived_zones),
        attribution,
    })
}

pub fn declared_file_record(gcx: &Arc<GlobalContext>, path: &Path) -> Result<FileRecord, String> {
    file_record(
        gcx,
        path,
        Attribution::Declared,
        &new_derived_privacy_zones(),
    )
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

pub fn observed_file_records(
    gcx: &Arc<GlobalContext>,
    paths: impl IntoIterator<Item = PathBuf>,
) -> Result<Vec<FileRecord>, String> {
    observed_file_records_with_derived(gcx, paths, &new_derived_privacy_zones())
}

fn observed_file_records_with_derived(
    gcx: &Arc<GlobalContext>,
    paths: impl IntoIterator<Item = PathBuf>,
    derived_zones: &DerivedPrivacyZones,
) -> Result<Vec<FileRecord>, String> {
    let mut records = Vec::new();
    for path in paths {
        let record = file_record(gcx, &path, Attribution::Observed, derived_zones)?;
        if !records.contains(&record) {
            records.push(record);
        }
    }
    Ok(records)
}

fn inherit_observed_write_zones(
    gcx: &Arc<GlobalContext>,
    policy: &refact_privacy::PrivacyPolicy,
    reads: &[FileRecord],
    writes: impl IntoIterator<Item = PathBuf>,
    derived_zones: &DerivedPrivacyZones,
) -> Result<(), String> {
    let Some(zone_name) = reads
        .iter()
        .filter(|record| record.zone != "normal")
        .map(|record| record.zone.as_str())
        .min_by_key(|name| zone_destination_count(policy, name))
    else {
        return Ok(());
    };
    let mappings = registered_worktree_path_mappings(gcx.cache_dir.as_path());
    let mut derived_zones = derived_zones
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    for path in writes {
        for candidate in record_path_candidates(gcx, &path, &mappings) {
            let replace = derived_zones
                .get(&candidate)
                .map(|current| {
                    zone_destination_count(policy, zone_name)
                        < zone_destination_count(policy, current)
                })
                .unwrap_or(true);
            if replace {
                derived_zones.insert(candidate, zone_name.to_string());
            }
        }
    }
    Ok(())
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
    use refact_exec::ObservedAccess;
    use refact_privacy::{PrivacyPolicy, SubagentPolicy, Zone};

    async fn gcx_with_policy(
        workspace_file: &Path,
        send_to: &[&str],
        behavior: ShellBehavior,
    ) -> Arc<GlobalContext> {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_files.lock().unwrap() = vec![workspace_file.to_path_buf()];
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![workspace_file
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf()];
        gcx.privacy_policy_load.write().unwrap().policy = Arc::new(PrivacyPolicy {
            blocked: Vec::new(),
            zones: vec![
                Zone {
                    name: "secrets".to_string(),
                    patterns: vec![workspace_file
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .into_owned()],
                    send_to: send_to.iter().map(|value| (*value).to_string()).collect(),
                    on_shell_read: behavior,
                },
                Zone {
                    name: "normal".to_string(),
                    patterns: vec!["*".to_string()],
                    send_to: vec!["*".to_string()],
                    on_shell_read: ShellBehavior::Withhold,
                },
            ],
            subagents: SubagentPolicy::default(),
        });
        gcx
    }

    fn tool_message(content: &str) -> ChatMessage {
        ChatMessage::new("tool".to_string(), content.to_string())
    }

    #[tokio::test]
    async fn observed_guarded_read_withholds_output_and_keeps_local_copy() {
        let temp = tempfile::tempdir().unwrap();
        let secret = temp.path().join("secret.txt");
        std::fs::write(&secret, "secret").unwrap();
        let gcx = gcx_with_policy(&secret, &[], ShellBehavior::Withhold).await;
        let derived_zones = new_derived_privacy_zones();
        let mut message = tool_message("secret output");

        let decision = apply_shell_observation(
            &gcx,
            "cat secret.txt",
            temp.path(),
            &provider_destination("untrusted/model"),
            ObservationStatus::Observed(ObservedAccess {
                reads: vec![secret.clone()],
                writes: Vec::new(),
            }),
            &derived_zones,
            &mut message,
        )
        .await
        .unwrap();

        assert_eq!(decision, ShellReadDecision::Pass);
        assert_eq!(message.content.content_text_only(), SHELL_WITHHELD_MESSAGE);
        assert_eq!(
            message.extra["privacy_shell"]["local_only_output"],
            "secret output"
        );
        assert_eq!(message.extra["privacy"]["files"][0]["zone"], "secrets");
        assert_eq!(
            message.extra["privacy"]["files"][0]["attribution"],
            "observed"
        );
        assert!(refact_privacy::records_from_messages(&[message])
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn observed_read_allowed_for_destination_passes_through() {
        let temp = tempfile::tempdir().unwrap();
        let secret = temp.path().join("secret.txt");
        std::fs::write(&secret, "secret").unwrap();
        let gcx = gcx_with_policy(&secret, &["trusted"], ShellBehavior::Withhold).await;
        let destination = provider_destination("trusted/model");
        let derived_zones = new_derived_privacy_zones();
        let mut message = tool_message("allowed output");

        assert!(!shell_observation_needed(&gcx, &destination));
        let decision = apply_shell_observation(
            &gcx,
            "cat secret.txt",
            temp.path(),
            &destination,
            ObservationStatus::Observed(ObservedAccess {
                reads: vec![secret],
                writes: Vec::new(),
            }),
            &derived_zones,
            &mut message,
        )
        .await
        .unwrap();

        assert_eq!(decision, ShellReadDecision::Pass);
        assert_eq!(message.content.content_text_only(), "allowed output");
        assert!(!message.extra.contains_key("privacy_shell"));
    }

    #[tokio::test]
    async fn tool_shell_derived_write_inherits_secret_zone_for_session() {
        let temp = tempfile::tempdir().unwrap();
        let secret = temp.path().join("secret.txt");
        let derived = temp.path().join("derived.txt");
        std::fs::write(&secret, "secret").unwrap();
        std::fs::write(&derived, "copy").unwrap();
        let gcx = gcx_with_policy(&secret, &[], ShellBehavior::Withhold).await;
        let derived_zones = new_derived_privacy_zones();
        let destination = provider_destination("untrusted/model");
        let mut copy_message = tool_message("copied");

        apply_shell_observation(
            &gcx,
            "cat secret.txt > derived.txt",
            temp.path(),
            &destination,
            ObservationStatus::Observed(ObservedAccess {
                reads: vec![secret],
                writes: vec![derived.clone()],
            }),
            &derived_zones,
            &mut copy_message,
        )
        .await
        .unwrap();

        gcx.documents_state.workspace_files.lock().unwrap().clear();
        assert!(shell_observation_needed_for_session(
            &gcx,
            &destination,
            &derived_zones
        ));

        let mut read_message = tool_message("derived secret");
        apply_shell_observation(
            &gcx,
            "cat derived.txt",
            temp.path(),
            &destination,
            ObservationStatus::Observed(ObservedAccess {
                reads: vec![derived],
                writes: Vec::new(),
            }),
            &derived_zones,
            &mut read_message,
        )
        .await
        .unwrap();

        assert_eq!(read_message.extra["privacy"]["files"][0]["zone"], "secrets");
        assert_eq!(
            read_message.content.content_text_only(),
            SHELL_WITHHELD_MESSAGE
        );

        let mut other_session_message = tool_message("ordinary output");
        apply_shell_observation(
            &gcx,
            "cat derived.txt",
            temp.path(),
            &destination,
            ObservationStatus::Observed(ObservedAccess {
                reads: vec![temp.path().join("derived.txt")],
                writes: Vec::new(),
            }),
            &new_derived_privacy_zones(),
            &mut other_session_message,
        )
        .await
        .unwrap();

        assert_eq!(
            other_session_message.extra["privacy"]["files"][0]["zone"],
            "normal"
        );
        assert_eq!(
            other_session_message.content.content_text_only(),
            "ordinary output"
        );
    }

    #[tokio::test]
    async fn unavailable_observation_is_degraded_and_fail_open() {
        let temp = tempfile::tempdir().unwrap();
        let secret = temp.path().join("secret.txt");
        std::fs::write(&secret, "secret").unwrap();
        let gcx = gcx_with_policy(&secret, &[], ShellBehavior::Withhold).await;
        let derived_zones = new_derived_privacy_zones();
        let mut message = tool_message("heuristic output");

        let decision = apply_shell_observation(
            &gcx,
            "cat secret.txt",
            temp.path(),
            &provider_destination("untrusted/model"),
            ObservationStatus::Unavailable("ptrace unavailable".to_string()),
            &derived_zones,
            &mut message,
        )
        .await
        .unwrap();

        assert_eq!(decision, ShellReadDecision::Pass);
        assert_eq!(message.content.content_text_only(), "heuristic output");
        assert_eq!(message.extra["privacy_observation"]["degraded"], true);
        assert_eq!(message.extra["privacy"]["files"][0]["zone"], "secrets");
        assert_eq!(
            message.extra["privacy"]["files"][0]["attribution"],
            "heuristic"
        );
        assert!(!message.extra.contains_key("privacy_shell"));
        assert!(refact_privacy::records_from_messages(&[message])
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn ask_read_stays_hidden_until_decided_without_rerunning_command() {
        let temp = tempfile::tempdir().unwrap();
        let secret = temp.path().join("secret.txt");
        std::fs::write(&secret, "secret").unwrap();
        let gcx = gcx_with_policy(&secret, &[], ShellBehavior::Ask).await;
        let derived_zones = new_derived_privacy_zones();
        let mut message = tool_message("approval output");

        let decision = apply_shell_observation(
            &gcx,
            "cat secret.txt",
            temp.path(),
            &provider_destination("untrusted/model"),
            ObservationStatus::Observed(ObservedAccess {
                reads: vec![secret],
                writes: Vec::new(),
            }),
            &derived_zones,
            &mut message,
        )
        .await
        .unwrap();

        assert_eq!(decision, ShellReadDecision::Ask);
        assert!(shell_ask_pending(&message));
        assert_eq!(message.content.content_text_only(), SHELL_APPROVAL_MESSAGE);
        assert!(resolve_shell_ask(&mut message, true));
        assert_eq!(message.content.content_text_only(), "approval output");
        assert!(!shell_ask_pending(&message));
        assert!(refact_privacy::records_from_messages(&[message])
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn no_workspace_files_skips_observation() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        gcx.privacy_policy_load.write().unwrap().policy = Arc::new(PrivacyPolicy {
            zones: vec![Zone {
                name: "secrets".to_string(),
                patterns: vec![".env".to_string()],
                send_to: Vec::new(),
                on_shell_read: ShellBehavior::Withhold,
            }],
            ..PrivacyPolicy::default()
        });

        assert!(!shell_observation_needed(
            &gcx,
            &provider_destination("untrusted/model")
        ));
    }

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

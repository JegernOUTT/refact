use std::cmp::Ordering;
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

fn zone_for_record_path(
    gcx: &Arc<GlobalContext>,
    compiled: &refact_privacy::CompiledPolicy,
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
        strictest_named_zone(
            compiled,
            candidates
                .iter()
                .filter_map(|candidate| derived_zones.get(candidate))
                .map(String::as_str),
        )
        .map(str::to_string)
    };
    let Some(derived_zone_name) = derived_zone_name else {
        return static_zone.name.clone();
    };
    if compare_zone_strictness(
        compiled,
        &derived_zone_name,
        zone_destinations(compiled, &derived_zone_name),
        &static_zone.name,
        &static_zone.send_to,
    ) == Ordering::Less
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

fn compare_named_zones(
    compiled: &refact_privacy::CompiledPolicy,
    left: &str,
    right: &str,
) -> Ordering {
    compare_zone_strictness(
        compiled,
        left,
        zone_destinations(compiled, left),
        right,
        zone_destinations(compiled, right),
    )
}

fn compare_zone_strictness(
    compiled: &refact_privacy::CompiledPolicy,
    left_name: &str,
    left_destinations: &[String],
    right_name: &str,
    right_destinations: &[String],
) -> Ordering {
    match (
        destinations_strict_subset(left_destinations, right_destinations),
        destinations_strict_subset(right_destinations, left_destinations),
    ) {
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        _ => compare_zone_order(compiled, left_name, right_name),
    }
}

fn compare_zone_order(
    compiled: &refact_privacy::CompiledPolicy,
    left_name: &str,
    right_name: &str,
) -> Ordering {
    compiled
        .zone_index_named(left_name)
        .unwrap_or(usize::MAX)
        .cmp(&compiled.zone_index_named(right_name).unwrap_or(usize::MAX))
        .then_with(|| left_name.cmp(right_name))
}

fn strictest_named_zone<'a, I>(
    compiled: &refact_privacy::CompiledPolicy,
    names: I,
) -> Option<&'a str>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut names = names.into_iter();
    let first = names.next()?;
    let minimums = names.fold(vec![first], |mut minimums, candidate| {
        let candidate_destinations = zone_destinations(compiled, candidate);
        if minimums.iter().any(|minimum| {
            destinations_strict_subset(zone_destinations(compiled, minimum), candidate_destinations)
        }) {
            return minimums;
        }
        minimums.retain(|minimum| {
            !destinations_strict_subset(
                candidate_destinations,
                zone_destinations(compiled, minimum),
            )
        });
        minimums.push(candidate);
        minimums
    });
    minimums
        .into_iter()
        .min_by(|left, right| compare_zone_order(compiled, left, right))
}

fn zone_destinations<'a>(compiled: &'a refact_privacy::CompiledPolicy, name: &str) -> &'a [String] {
    compiled
        .zone_named(name)
        .map(|zone| zone.send_to.as_slice())
        .unwrap_or(&[])
}

fn destinations_strict_subset(left: &[String], right: &[String]) -> bool {
    let left_wildcard = left.iter().any(|destination| destination == "*");
    let right_wildcard = right.iter().any(|destination| destination == "*");
    if left_wildcard {
        return false;
    }
    if right_wildcard {
        return true;
    }
    left.iter().all(|destination| right.contains(destination))
        && right.iter().any(|destination| !left.contains(destination))
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
    crate::privacy::record_observation_status(gcx, &observation);
    let policy = gcx.privacy_policy_load.read().unwrap().policy.clone();
    let records = match observation {
        ObservationStatus::Observed(access) => {
            let records = observed_file_records_with_derived(gcx, access.reads, derived_zones)?;
            inherit_observed_write_zones(gcx, &policy, &records, access.writes, derived_zones)?;
            records
        }
        ObservationStatus::Pending(access) => {
            message.extra.insert(
                "privacy_observation".to_string(),
                serde_json::json!({
                    "status": "pending",
                    "degraded": false,
                    "incomplete": true,
                }),
            );
            let records = observed_file_records_with_derived(gcx, access.reads, derived_zones)?;
            inherit_observed_write_zones(gcx, &policy, &records, access.writes, derived_zones)?;
            records
        }
        ObservationStatus::Incomplete(access) => {
            message.extra.insert(
                "privacy_observation".to_string(),
                serde_json::json!({
                    "status": "incomplete",
                    "degraded": false,
                    "incomplete": true,
                }),
            );
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

pub fn records_to_carry(
    sources: &[ChatMessage],
) -> Result<Vec<FileRecord>, refact_privacy::PrivacyAuditError> {
    refact_privacy::records_from_messages(sources).map(|indexed| {
        indexed
            .into_iter()
            .fold(Vec::new(), |mut records, (_, record)| {
                if !records.contains(&record) {
                    records.push(record);
                }
                records
            })
    })
}

pub fn merge_message_records(
    message: &mut ChatMessage,
    sources: &[ChatMessage],
) -> Result<(), refact_privacy::PrivacyAuditError> {
    let records = records_to_carry(sources)?;
    merge_records(message, records);
    Ok(())
}

pub fn carry_records_into(
    message: &mut ChatMessage,
    sources: &[ChatMessage],
) -> Result<(), refact_privacy::PrivacyAuditError> {
    merge_message_records(message, sources)
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
        zone: zone_for_record_path(gcx, &compiled, path, &mappings, derived_zones),
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
    let compiled = policy.compile().map_err(|error| error.to_string())?;
    let Some(zone_name) = strictest_named_zone(
        &compiled,
        reads
            .iter()
            .filter(|record| record.zone != "normal")
            .map(|record| record.zone.as_str()),
    ) else {
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
                .map(|current| compare_named_zones(&compiled, zone_name, current) == Ordering::Less)
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
            ..Default::default()
        });
        gcx
    }

    fn tool_message(content: &str) -> ChatMessage {
        ChatMessage::new("tool".to_string(), content.to_string())
    }

    async fn gcx_with_zones(workspace: &Path, zones: Vec<Zone>) -> Arc<GlobalContext> {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![workspace.to_path_buf()];
        gcx.privacy_policy_load.write().unwrap().policy = Arc::new(PrivacyPolicy {
            blocked: Vec::new(),
            zones,
            subagents: SubagentPolicy::default(),
            ..Default::default()
        });
        gcx
    }

    #[test]
    fn named_zone_strictness_uses_inclusion_then_zone_order() {
        let policy = PrivacyPolicy {
            blocked: Vec::new(),
            zones: vec![
                Zone {
                    name: "z-earlier".to_string(),
                    patterns: vec!["earlier.txt".to_string()],
                    send_to: vec!["provider-a".to_string(), "provider-b".to_string()],
                    on_shell_read: ShellBehavior::Withhold,
                },
                Zone {
                    name: "a-later-equal".to_string(),
                    patterns: vec!["equal.txt".to_string()],
                    send_to: vec!["provider-b".to_string(), "provider-a".to_string()],
                    on_shell_read: ShellBehavior::Withhold,
                },
                Zone {
                    name: "later-narrow".to_string(),
                    patterns: vec!["narrow.txt".to_string()],
                    send_to: vec!["provider-a".to_string()],
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
        };
        let compiled = policy.compile().expect("policy should compile");

        assert_eq!(
            compare_named_zones(&compiled, "a-later-equal", "z-earlier"),
            Ordering::Greater
        );
        assert_eq!(
            compare_named_zones(&compiled, "later-narrow", "z-earlier"),
            Ordering::Less
        );
        for names in [
            ["z-earlier", "a-later-equal", "later-narrow"],
            ["later-narrow", "a-later-equal", "z-earlier"],
        ] {
            assert_eq!(strictest_named_zone(&compiled, names), Some("later-narrow"));
        }
    }

    #[tokio::test]
    async fn derived_static_tie_uses_zone_order() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("file.txt");
        std::fs::write(&file, "value").unwrap();
        let gcx = gcx_with_zones(
            temp.path(),
            vec![
                Zone {
                    name: "derived-earlier".to_string(),
                    patterns: vec!["derived-only.txt".to_string()],
                    send_to: vec!["provider-a".to_string()],
                    on_shell_read: ShellBehavior::Withhold,
                },
                Zone {
                    name: "static-later".to_string(),
                    patterns: vec!["file.txt".to_string()],
                    send_to: vec!["provider-a".to_string()],
                    on_shell_read: ShellBehavior::Withhold,
                },
                Zone {
                    name: "normal".to_string(),
                    patterns: vec!["*".to_string()],
                    send_to: vec!["*".to_string()],
                    on_shell_read: ShellBehavior::Withhold,
                },
            ],
        )
        .await;
        let derived_zones = new_derived_privacy_zones();
        derived_zones
            .write()
            .unwrap()
            .insert(file.clone(), "derived-earlier".to_string());

        let record = file_record(&gcx, &file, Attribution::Observed, &derived_zones).unwrap();

        assert_eq!(record.zone, "derived-earlier");
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
    async fn pending_observation_is_not_degraded() {
        let temp = tempfile::tempdir().unwrap();
        let secret = temp.path().join("secret.txt");
        std::fs::write(&secret, "secret").unwrap();
        let gcx = gcx_with_policy(&secret, &[], ShellBehavior::Withhold).await;
        let derived_zones = new_derived_privacy_zones();
        let mut message = tool_message("background started");

        let decision = apply_shell_observation(
            &gcx,
            "sleep 30",
            temp.path(),
            &provider_destination("untrusted/model"),
            ObservationStatus::Pending(ObservedAccess::default()),
            &derived_zones,
            &mut message,
        )
        .await
        .unwrap();

        assert_eq!(decision, ShellReadDecision::Pass);
        assert_eq!(message.extra["privacy_observation"]["status"], "pending");
        assert_eq!(message.extra["privacy_observation"]["degraded"], false);
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

        merge_message_records(&mut target, &[source_a, source_b]).unwrap();

        let privacy: PrivacyRecord =
            serde_json::from_value(target.extra["privacy"].clone()).unwrap();
        assert_eq!(privacy.files, vec![first, second]);
    }
}

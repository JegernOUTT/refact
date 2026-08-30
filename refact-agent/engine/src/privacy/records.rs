use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use refact_privacy::{
    Attribution, Destination, DestinationId, DestinationKind, FileRecord, PrivacyRecord,
    ShellBehavior,
};
use refact_exec::ObservationStatus;
use refact_chat_api::{
    ToolEnrichment, ToolEnrichmentKind, ToolEnrichmentProvenance, ToolEnrichmentReference,
};

use crate::call_validation::ChatMessage;
use crate::exec::path_enrichment::{CollectedPathEnrichment, PathEnrichment};
use crate::files_correction::registered_worktree_path_mappings;
use crate::files_in_workspace::{check_file_privacy_for_model_context, registered_alias_paths};
use crate::global_context::GlobalContext;

pub const SHELL_WITHHELD_MESSAGE: &str = "Output withheld by user privacy policy — the command ran, but its output read guarded files and cannot be shown.";
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
    !policy.blocked.is_empty()
        || policy
            .zones
            .iter()
            .any(|zone| !destination.matches_send_to(&zone.send_to))
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

pub(crate) async fn filter_path_enrichment_for_model_context(
    gcx: Arc<GlobalContext>,
    destination: &Destination,
    derived_zones: &DerivedPrivacyZones,
    collected: CollectedPathEnrichment,
) -> PathEnrichment {
    let mut metadata = collected.metadata;
    let mut references = Vec::new();
    let mut seen = HashSet::new();
    let compiled_policy = {
        let policy = gcx.privacy_policy_load.read().unwrap().policy.clone();
        policy.compile().ok()
    };
    let worktree_mappings = registered_worktree_path_mappings(gcx.cache_dir.as_path());
    for candidate in collected.candidates {
        if check_file_privacy_for_model_context(gcx.clone(), &candidate.canonical_path)
            .await
            .is_err()
            || !compiled_policy.as_ref().is_some_and(|compiled| {
                path_allowed_for_destination(
                    &gcx,
                    compiled,
                    &worktree_mappings,
                    &candidate.canonical_path,
                    destination,
                    derived_zones,
                )
            })
        {
            metadata.withheld_count += 1;
            continue;
        }
        let key = (
            candidate.reference.path.clone(),
            candidate.reference.line1,
            candidate.reference.line2,
            candidate.reference.column1,
            candidate.reference.column2,
        );
        if seen.insert(key) {
            references.push(candidate.reference);
        }
    }
    metadata.references = references;
    metadata
}

pub(crate) fn tool_enrichment_from_path_references(enrichment: PathEnrichment) -> ToolEnrichment {
    let references = enrichment
        .references
        .into_iter()
        .map(|path| {
            let mut reference = ToolEnrichmentReference::new(ToolEnrichmentKind::Path, path.path);
            reference.provenance = ToolEnrichmentProvenance::Heuristic;
            reference.line1 = path.line1.map(|line| line as usize);
            reference.line2 = path.line2.map(|line| line as usize);
            reference.column1 = path.column1.map(|column| column as usize);
            reference.column2 = path.column2.map(|column| column as usize);
            reference.source = Some(path.source);
            reference.confidence = match path.confidence.as_str() {
                "high" => Some(0.9),
                "medium" => Some(0.6),
                "low" => Some(0.3),
                _ => None,
            };
            reference
        })
        .collect();
    ToolEnrichment {
        references,
        truncated: enrichment.truncated
            || enrichment.omitted_count > 0
            || enrichment.withheld_count > 0,
        ..Default::default()
    }
}

fn path_allowed_for_destination(
    gcx: &Arc<GlobalContext>,
    compiled: &refact_privacy::CompiledPolicy,
    mappings: &[crate::files_correction::RegisteredWorktreePathMapping],
    path: &Path,
    destination: &Destination,
    derived_zones: &DerivedPrivacyZones,
) -> bool {
    let zone = zone_for_record_path(gcx, compiled, path, mappings, derived_zones);
    zone != "blocked"
        && compiled
            .zone_named(&zone)
            .map(|candidate| destination.matches_send_to(&candidate.send_to))
            .unwrap_or(false)
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
    candidates.push(path.to_path_buf());
    if let Some(facts) = gcx.file_index.get(path) {
        candidates.extend(gcx.file_index.aliases_of(&facts, path));
    }
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
            classify_observed_access(gcx, &policy, access, derived_zones).await?
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
            classify_observed_access(gcx, &policy, access, derived_zones).await?
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
            classify_observed_access(gcx, &policy, access, derived_zones).await?
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

    let offending: Vec<(&FileRecord, ShellBehavior)> = records
        .iter()
        .filter_map(|record| {
            if record.zone == "blocked" {
                return Some((record, ShellBehavior::Deny));
            }
            policy
                .zones
                .iter()
                .find(|zone| zone.name == record.zone)
                .filter(|zone| !destination.matches_send_to(&zone.send_to))
                .map(|zone| (record, zone.on_shell_read))
        })
        .collect();
    let behavior =
        offending
            .iter()
            .map(|(_, behavior)| *behavior)
            .fold(None, |decision, behavior| match (decision, behavior) {
                (_, ShellBehavior::Deny) => Some(ShellBehavior::Deny),
                (Some(ShellBehavior::Deny), _) => Some(ShellBehavior::Deny),
                (_, ShellBehavior::Withhold) => Some(ShellBehavior::Withhold),
                (Some(ShellBehavior::Withhold), _) => Some(ShellBehavior::Withhold),
                (_, ShellBehavior::Ask) => Some(ShellBehavior::Ask),
            });
    let guarded = guarded_read_list(&offending);
    let target = destination.id.0.as_str();
    match behavior {
        Some(ShellBehavior::Deny) => Err(format!(
            "Denied by user privacy policy — the command read guarded files, so its output was discarded:\n{guarded}\nThe command already ran; re-running it unchanged will be denied again. Scope it so it does not read those paths, or ask the user to relax the zone."
        )),
        Some(ShellBehavior::Withhold) => {
            retain_local_shell_output(
                message,
                &format!(
                    "Output withheld by user privacy policy — the command ran, but its output cannot be sent to \"{target}\" because it read guarded files:\n{guarded}\nAny side effects already happened, so do not re-run it just to retry. To see output, re-run it scoped so it does not read those paths, or ask the user to allow these zones for \"{target}\"."
                ),
                false,
            );
            Ok(ShellReadDecision::Pass)
        }
        Some(ShellBehavior::Ask) => {
            retain_local_shell_output(
                message,
                &format!(
                    "Output awaiting user approval — the command ran, but its output stays hidden until the user approves it, because it read guarded files:\n{guarded}"
                ),
                true,
            );
            Ok(ShellReadDecision::Ask)
        }
        None => Ok(ShellReadDecision::Pass),
    }
}

async fn classify_observed_access(
    gcx: &Arc<GlobalContext>,
    policy: &refact_privacy::PrivacyPolicy,
    access: refact_exec::ObservedAccess,
    derived_zones: &DerivedPrivacyZones,
) -> Result<Vec<FileRecord>, String> {
    let gcx = gcx.clone();
    let policy = policy.clone();
    let derived_zones = derived_zones.clone();
    tokio::task::spawn_blocking(move || {
        let records = observed_file_records_with_derived(&gcx, access.reads, &derived_zones)?;
        inherit_observed_write_zones(&gcx, &policy, &records, access.writes, &derived_zones)?;
        Ok(records)
    })
    .await
    .map_err(|error| format!("privacy classification task failed: {error}"))?
}

const MAX_LISTED_GUARDED_READS: usize = 5;

fn guarded_read_list(offending: &[(&FileRecord, ShellBehavior)]) -> String {
    let mut listed: Vec<String> = Vec::new();
    let mut seen: Vec<(&str, &str)> = Vec::new();
    let mut hidden = 0usize;
    for (record, _) in offending {
        let key = (record.path.as_str(), record.zone.as_str());
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        if listed.len() < MAX_LISTED_GUARDED_READS {
            listed.push(format!("  - {} (zone \"{}\")", record.path, record.zone));
        } else {
            hidden += 1;
        }
    }
    if hidden > 0 {
        listed.push(format!("  - ...and {hidden} more"));
    }
    listed.join("\n")
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

pub(crate) fn record_is_persistable(record: &FileRecord) -> bool {
    record.zone != "normal" || matches!(record.attribution, Attribution::Declared)
}

pub fn merge_records(message: &mut ChatMessage, records: impl IntoIterator<Item = FileRecord>) {
    let mut privacy = message
        .extra
        .remove("privacy")
        .and_then(|value| serde_json::from_value::<PrivacyRecord>(value).ok())
        .unwrap_or_default();
    let mut seen = privacy.files.iter().cloned().collect::<HashSet<_>>();
    for record in records {
        if !record_is_persistable(&record) {
            continue;
        }
        if seen.insert(record.clone()) {
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
        let mut seen = HashSet::with_capacity(indexed.len());
        indexed
            .into_iter()
            .filter_map(|(_, record)| {
                (record_is_persistable(&record) && seen.insert(record.clone())).then_some(record)
            })
            .collect()
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
    Ok(file_record_with(
        gcx,
        &compiled,
        &mappings,
        path,
        attribution,
        derived_zones,
    ))
}

fn file_record_with(
    gcx: &Arc<GlobalContext>,
    compiled: &refact_privacy::CompiledPolicy,
    mappings: &[crate::files_correction::RegisteredWorktreePathMapping],
    path: &Path,
    attribution: Attribution,
    derived_zones: &DerivedPrivacyZones,
) -> FileRecord {
    FileRecord {
        path: refact_core::chat_types::normalize_file_name(path.to_string_lossy().into_owned()),
        zone: zone_for_record_path(gcx, compiled, path, mappings, derived_zones),
        attribution,
    }
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
    let policy = gcx.privacy_policy_load.read().unwrap().policy.clone();
    let compiled = policy.compile().map_err(|error| error.to_string())?;
    let mappings = registered_worktree_path_mappings(gcx.cache_dir.as_path());
    let derived_zones = new_derived_privacy_zones();
    let mut records = Vec::new();
    let mut seen = HashSet::new();
    for path in paths {
        let record = file_record_with(
            gcx,
            &compiled,
            &mappings,
            &path,
            Attribution::Declared,
            &derived_zones,
        );
        if seen.insert(record.clone()) {
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
    let policy = gcx.privacy_policy_load.read().unwrap().policy.clone();
    let compiled = policy.compile().map_err(|error| error.to_string())?;
    let mappings = registered_worktree_path_mappings(gcx.cache_dir.as_path());
    let mut records = Vec::new();
    let mut seen = HashSet::new();
    for path in paths {
        let record = file_record_with(
            gcx,
            &compiled,
            &mappings,
            &path,
            Attribution::Observed,
            derived_zones,
        );
        if seen.insert(record.clone()) {
            records.push(record);
        }
    }
    Ok(records)
}

fn is_taintable_write_target(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|metadata| metadata.is_file())
        .unwrap_or(false)
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
        if !is_taintable_write_target(&path) {
            continue;
        }
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

    #[tokio::test]
    async fn path_enrichment_omits_destination_guarded_candidates() {
        let temp = tempfile::tempdir().unwrap();
        let public = temp.path().join("public.rs");
        let secret = temp.path().join("secret.rs");
        std::fs::write(&public, "pub fn visible() {}\n").unwrap();
        std::fs::write(&secret, "pub fn guarded() {}\n").unwrap();
        let gcx = gcx_with_zones(
            temp.path(),
            vec![
                Zone {
                    name: "secrets".to_string(),
                    patterns: vec!["secret.rs".to_string()],
                    send_to: vec![],
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
        let collected = crate::exec::path_enrichment::collect(
            "cat public.rs secret.rs",
            temp.path(),
            temp.path(),
            "",
        );

        let enrichment = filter_path_enrichment_for_model_context(
            gcx,
            &provider_destination("untrusted/model"),
            &new_derived_privacy_zones(),
            collected,
        )
        .await;

        assert_eq!(enrichment.references.len(), 1);
        assert_eq!(enrichment.references[0].path, "public.rs");
        assert_eq!(enrichment.withheld_count, 1);
        let envelope = tool_enrichment_from_path_references(enrichment);
        assert_eq!(envelope.references.len(), 1);
        assert_eq!(envelope.references[0].kind, ToolEnrichmentKind::Path);
        assert_eq!(envelope.references[0].target, "public.rs");
        assert_eq!(
            envelope.references[0].provenance,
            ToolEnrichmentProvenance::Heuristic
        );
        let expected_confidence = if cfg!(windows) { Some(0.3) } else { Some(0.9) };
        assert_eq!(envelope.references[0].confidence, expected_confidence);
        assert!(envelope.truncated);
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

    #[cfg(unix)]
    #[tokio::test]
    async fn shared_sink_writes_do_not_inherit_guarded_zone() {
        let temp = tempfile::tempdir().unwrap();
        let secret = temp.path().join("secret.txt");
        std::fs::write(&secret, "secret").unwrap();
        let gcx = gcx_with_policy(&secret, &[], ShellBehavior::Withhold).await;
        let derived_zones = new_derived_privacy_zones();
        let mut message = tool_message("secret output");

        apply_shell_observation(
            &gcx,
            "cat secret.txt 2>/dev/null",
            temp.path(),
            &provider_destination("untrusted/model"),
            ObservationStatus::Observed(ObservedAccess {
                reads: vec![secret.clone()],
                writes: vec![PathBuf::from("/dev/null")],
            }),
            &derived_zones,
            &mut message,
        )
        .await
        .unwrap();

        let tainted = derived_zones.read().unwrap();
        assert!(
            !tainted.keys().any(|path| path == Path::new("/dev/null")),
            "shared character devices must never inherit a guarded zone"
        );
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
        let withheld = message.content.content_text_only();
        assert!(withheld.starts_with("Output withheld by user privacy policy"));
        assert!(withheld.contains("secret.txt"));
        assert!(withheld.contains("zone \"secrets\""));
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
        let withheld = read_message.content.content_text_only();
        assert!(withheld.starts_with("Output withheld by user privacy policy"));
        assert!(withheld.contains("zone \"secrets\""));

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

        assert!(
            other_session_message.extra.get("privacy").is_none(),
            "a fresh session must not inherit the derived secret zone, so the read stays \
             unguarded and its inert normal record is not persisted"
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
        let pending = message.content.content_text_only();
        assert!(pending.starts_with("Output awaiting user approval"));
        assert!(pending.contains("zone \"secrets\""));
        assert!(resolve_shell_ask(&mut message, true));
        assert_eq!(message.content.content_text_only(), "approval output");
        assert!(!shell_ask_pending(&message));
        assert!(refact_privacy::records_from_messages(&[message])
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn restrictive_policy_requires_observation_outside_the_workspace() {
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

        assert!(shell_observation_needed(
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
        merge_records(&mut message, [first.clone(), second.clone()]);

        let privacy: PrivacyRecord =
            serde_json::from_value(message.extra["privacy"].clone()).unwrap();
        assert_eq!(privacy.files, vec![first, second]);
    }

    #[test]
    fn merge_records_deduplicates_large_input_in_first_seen_order() {
        let all = (0..5_000)
            .map(|index| FileRecord {
                path: format!("file-{index}.rs"),
                zone: if index % 2 == 0 {
                    "normal".to_string()
                } else {
                    "secrets".to_string()
                },
                attribution: Attribution::Observed,
            })
            .collect::<Vec<_>>();
        let expected = all
            .iter()
            .filter(|record| record.zone != "normal")
            .cloned()
            .collect::<Vec<_>>();
        let records = all
            .iter()
            .cloned()
            .flat_map(|record| [record.clone(), record])
            .collect::<Vec<_>>();
        let mut message = ChatMessage::default();

        merge_records(&mut message, records);

        let privacy: PrivacyRecord =
            serde_json::from_value(message.extra["privacy"].clone()).unwrap();
        assert_eq!(privacy.files, expected);
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
        let third = FileRecord {
            path: "later.rs".to_string(),
            zone: "normal".to_string(),
            attribution: Attribution::Heuristic,
        };
        let mut source_a = ChatMessage::default();
        let mut source_b = ChatMessage::default();
        merge_records(&mut source_a, [first.clone(), second.clone()]);
        merge_records(
            &mut source_b,
            [second.clone(), third.clone(), first.clone()],
        );
        let mut target = ChatMessage::default();

        merge_message_records(&mut target, &[source_a, source_b]).unwrap();

        let privacy: PrivacyRecord =
            serde_json::from_value(target.extra["privacy"].clone()).unwrap();
        assert_eq!(privacy.files, vec![first, second]);
        assert!(!privacy.files.contains(&third));
    }

    #[test]
    fn merge_records_drops_inert_normal_observed_records() {
        let observed_normal = FileRecord {
            path: "/proj/.venv/lib/python3.11/site-packages/mod.py".to_string(),
            zone: "normal".to_string(),
            attribution: Attribution::Observed,
        };
        let mut message = ChatMessage::default();

        merge_records(&mut message, [observed_normal]);

        assert!(
            message.extra.get("privacy").is_none(),
            "normal+observed records are inert for the gate and must not be persisted"
        );
    }

    #[test]
    fn merge_records_keeps_every_guarded_record_regardless_of_attribution() {
        let guarded = [
            Attribution::Observed,
            Attribution::Declared,
            Attribution::Heuristic,
        ]
        .into_iter()
        .enumerate()
        .map(|(index, attribution)| FileRecord {
            path: format!("secret-{index}.env"),
            zone: "secrets".to_string(),
            attribution,
        })
        .collect::<Vec<_>>();
        let mut message = ChatMessage::default();

        merge_records(&mut message, guarded.clone());

        let privacy: PrivacyRecord =
            serde_json::from_value(message.extra["privacy"].clone()).unwrap();
        assert_eq!(privacy.files, guarded);
    }

    #[test]
    fn merge_records_keeps_blocked_zone_records() {
        let blocked = FileRecord {
            path: "/etc/shadow".to_string(),
            zone: "blocked".to_string(),
            attribution: Attribution::Observed,
        };
        let mut message = ChatMessage::default();

        merge_records(&mut message, [blocked.clone()]);

        let privacy: PrivacyRecord =
            serde_json::from_value(message.extra["privacy"].clone()).unwrap();
        assert_eq!(privacy.files, vec![blocked]);
    }

    #[test]
    fn merge_records_keeps_effective_zone_records() {
        let effective = FileRecord {
            path: "derived.txt".to_string(),
            zone: "effective:secrets+internal".to_string(),
            attribution: Attribution::Observed,
        };
        let mut message = ChatMessage::default();

        merge_records(&mut message, [effective.clone()]);

        let privacy: PrivacyRecord =
            serde_json::from_value(message.extra["privacy"].clone()).unwrap();
        assert_eq!(privacy.files, vec![effective]);
    }

    #[test]
    fn records_to_carry_does_not_amplify_normal_observed_records() {
        let guarded = FileRecord {
            path: ".env".to_string(),
            zone: "secrets".to_string(),
            attribution: Attribution::Observed,
        };
        let declared_normal = FileRecord {
            path: "src/main.rs".to_string(),
            zone: "normal".to_string(),
            attribution: Attribution::Declared,
        };
        let mut source = ChatMessage::default();
        merge_records(&mut source, [guarded.clone(), declared_normal.clone()]);
        source.extra.insert(
            "privacy".to_string(),
            serde_json::to_value(PrivacyRecord {
                files: vec![
                    guarded.clone(),
                    declared_normal.clone(),
                    FileRecord {
                        path: "/legacy/observed.rs".to_string(),
                        zone: "normal".to_string(),
                        attribution: Attribution::Observed,
                    },
                ],
            })
            .unwrap(),
        );

        let carried = records_to_carry(&[source]).unwrap();

        assert_eq!(carried, vec![guarded, declared_normal]);
    }
}

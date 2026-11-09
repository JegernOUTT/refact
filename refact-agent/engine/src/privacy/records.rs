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

const INHERITED_ZONE_NOTE: &str = "\nEntries marked inherited carry a session-derived label taken from an earlier guarded read in this chat rather than from a configured rule, and they clear when the session ends or via POST /v1/privacy/derived/clear.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellReadDecision {
    Pass,
    Ask,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DerivedZone {
    pub zone: String,
    pub origin: Option<String>,
}

pub type DerivedPrivacyZones = Arc<RwLock<HashMap<PathBuf, DerivedZone>>>;

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

/// Counts only: never carries paths, patterns or zone names, so it is safe to log.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ObservationReason {
    pub blocked_patterns: usize,
    pub guarded_zones: usize,
    pub derived_labels: usize,
}

impl ObservationReason {
    pub fn needed(&self) -> bool {
        self.blocked_patterns > 0 || self.guarded_zones > 0 || self.derived_labels > 0
    }
}

pub fn shell_observation_reason(
    gcx: &Arc<GlobalContext>,
    destination: &Destination,
) -> ObservationReason {
    let policy = gcx.privacy_policy_load.read().unwrap().policy.clone();
    ObservationReason {
        blocked_patterns: policy.blocked.len(),
        guarded_zones: policy
            .zones
            .iter()
            .filter(|zone| !destination.matches_send_to(&zone.send_to))
            .count(),
        derived_labels: 0,
    }
}

pub fn shell_observation_reason_for_session(
    gcx: &Arc<GlobalContext>,
    destination: &Destination,
    derived_zones: &DerivedPrivacyZones,
) -> ObservationReason {
    let mut reason = shell_observation_reason(gcx, destination);
    let derived_zone_names = derived_zones
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .values()
        .map(|derived| derived.zone.clone())
        .collect::<Vec<_>>();
    let policy = gcx.privacy_policy_load.read().unwrap().policy.clone();
    reason.derived_labels = derived_zone_names
        .iter()
        .filter(|name| {
            *name == "blocked"
                || policy
                    .zones
                    .iter()
                    .find(|zone| zone.name == **name)
                    .map(|zone| !destination.matches_send_to(&zone.send_to))
                    .unwrap_or(true)
        })
        .count();
    reason
}

pub fn shell_observation_needed(gcx: &Arc<GlobalContext>, destination: &Destination) -> bool {
    shell_observation_reason(gcx, destination).needed()
}

pub fn shell_observation_needed_for_session(
    gcx: &Arc<GlobalContext>,
    destination: &Destination,
    derived_zones: &DerivedPrivacyZones,
) -> bool {
    shell_observation_reason_for_session(gcx, destination, derived_zones).needed()
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
    finish_record_zone(compiled, &candidates, static_zone, derived_zones)
}

fn finish_record_zone(
    compiled: &refact_privacy::CompiledPolicy,
    candidates: &[PathBuf],
    static_zone: refact_privacy::Zone,
    derived_zones: &DerivedPrivacyZones,
) -> String {
    let derived_zone_name = {
        let derived_zones = derived_zones
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        strictest_named_zone(
            compiled,
            candidates
                .iter()
                .filter_map(|candidate| derived_zones.get(candidate))
                .map(|derived| derived.zone.as_str()),
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

pub fn derived_zone_for_path(
    gcx: &Arc<GlobalContext>,
    path: &Path,
    derived_zones: &DerivedPrivacyZones,
) -> Option<DerivedZone> {
    let mappings = registered_worktree_path_mappings(gcx.cache_dir.as_path());
    derived_zone_for_path_with(gcx, path, &mappings, derived_zones)
}

fn derived_zone_for_path_with(
    gcx: &Arc<GlobalContext>,
    path: &Path,
    mappings: &[crate::files_correction::RegisteredWorktreePathMapping],
    derived_zones: &DerivedPrivacyZones,
) -> Option<DerivedZone> {
    let candidates = record_path_candidates(gcx, path, mappings);
    let derived_zones = derived_zones
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    candidates
        .iter()
        .find_map(|candidate| derived_zones.get(candidate).cloned())
}

pub(crate) fn record_path_candidates(
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

fn record_path_candidates_with_workspaces(
    gcx: &Arc<GlobalContext>,
    path: &Path,
    mappings: &[crate::files_correction::RegisteredWorktreePathMapping],
    workspaces: &[PathBuf],
) -> Vec<PathBuf> {
    let mut candidates = registered_alias_paths(path, mappings);
    candidates.push(path.to_path_buf());
    if let Some(facts) = gcx.file_index.get(path) {
        candidates.extend(gcx.file_index.aliases_of(&facts, path));
    }
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

fn privacy_roots_with_workspaces(
    workspaces: &[PathBuf],
    mappings: &[crate::files_correction::RegisteredWorktreePathMapping],
) -> Vec<PathBuf> {
    let mut roots = workspaces.to_vec();
    roots.extend(
        mappings
            .iter()
            .flat_map(|mapping| [mapping.root.clone(), mapping.source_root.clone()]),
    );
    roots.sort();
    roots.dedup();
    roots
}

pub(crate) fn privacy_roots(
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
            let snapshot = super::classification_cache::snapshot(gcx)?;
            let compiled = &snapshot.compiled;
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

    let current = super::classification_cache::snapshot(gcx)?;
    let offending: Vec<(&FileRecord, ShellBehavior)> = records
        .iter()
        .filter_map(|record| {
            if record.zone == "blocked" {
                return Some((record, ShellBehavior::Deny));
            }
            match current.compiled.zone_named(&record.zone) {
                Some(zone) => (!destination.matches_send_to(&zone.send_to))
                    .then_some((record, zone.on_shell_read)),
                None => Some((record, ShellBehavior::Withhold)),
            }
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
    let origins = offending_record_origins(gcx, &offending, derived_zones);
    let guarded = guarded_read_list(&offending, &origins);
    let inherited_note = if origins.is_empty() {
        ""
    } else {
        INHERITED_ZONE_NOTE
    };
    let target = destination.id.0.as_str();
    match behavior {
        Some(ShellBehavior::Deny) => Err(format!(
            "Denied by user privacy policy — the command read guarded files, so its output was discarded:\n{guarded}\nThe command already ran; re-running it unchanged will be denied again. Scope it so it does not read those paths, or ask the user to relax the zone.{inherited_note}"
        )),
        Some(ShellBehavior::Withhold) => {
            retain_local_shell_output(
                message,
                &format!(
                    "Output withheld by user privacy policy — the command ran, but its output cannot be sent to \"{target}\" because it read guarded files:\n{guarded}\nAny side effects already happened, so do not re-run it just to retry. To see output, re-run it scoped so it does not read those paths, or ask the user to allow these zones for \"{target}\".{inherited_note}"
                ),
                false,
            );
            Ok(ShellReadDecision::Pass)
        }
        Some(ShellBehavior::Ask) => {
            retain_local_shell_output(
                message,
                &format!(
                    "Output awaiting user approval — the command ran, but its output stays hidden until the user approves it, because it read guarded files:\n{guarded}{inherited_note}"
                ),
                true,
            );
            Ok(ShellReadDecision::Ask)
        }
        None => Ok(ShellReadDecision::Pass),
    }
}

fn offending_record_origins(
    gcx: &Arc<GlobalContext>,
    offending: &[(&FileRecord, ShellBehavior)],
    derived_zones: &DerivedPrivacyZones,
) -> HashMap<String, String> {
    if offending.is_empty()
        || derived_zones
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_empty()
    {
        return HashMap::new();
    }
    let mappings = registered_worktree_path_mappings(gcx.cache_dir.as_path());
    let mut origins = HashMap::new();
    for (record, _) in offending {
        if origins.contains_key(&record.path) {
            continue;
        }
        let derived = derived_zone_for_path_with(
            gcx,
            Path::new(record.path.as_str()),
            &mappings,
            derived_zones,
        );
        if let Some(origin) = derived
            .filter(|derived| derived.zone == record.zone)
            .and_then(|derived| derived.origin)
        {
            origins.insert(record.path.clone(), origin);
        }
    }
    origins
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

fn guarded_read_list(
    offending: &[(&FileRecord, ShellBehavior)],
    origins: &HashMap<String, String>,
) -> String {
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
            listed.push(match origins.get(&record.path) {
                Some(origin) => format!(
                    "  - {} (zone \"{}\", inherited from \"{origin}\")",
                    record.path, record.zone
                ),
                None => format!("  - {} (zone \"{}\")", record.path, record.zone),
            });
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
    let mut privacy = match message.extra.get("privacy") {
        Some(value) => match serde_json::from_value::<PrivacyRecord>(value.clone()) {
            Ok(privacy) => privacy,
            Err(_) => return,
        },
        None => PrivacyRecord::default(),
    };
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

fn message_records(message: &ChatMessage) -> Option<Vec<FileRecord>> {
    let value = message.extra.get("privacy")?;
    serde_json::from_value::<PrivacyRecord>(value.clone())
        .ok()
        .map(|privacy| privacy.files)
}

/// Drops duplicate records covered by the first reconstructed user carrier.
pub fn compact_reconstruction_privacy(messages: &mut [ChatMessage]) -> bool {
    let Ok(audited) = refact_privacy::records_from_messages(messages) else {
        return false;
    };
    let carrier = messages.iter().position(|message| message.role == "user");
    let carried = audited
        .into_iter()
        .filter(|(index, _)| Some(*index) == carrier)
        .map(|(_, record)| record)
        .collect::<HashSet<_>>();
    if carried.is_empty() {
        return false;
    }
    let mut changed = false;
    for (index, message) in messages.iter_mut().enumerate() {
        if Some(index) == carrier {
            continue;
        }
        let Some(records) = message_records(message) else {
            continue;
        };
        if !records.is_empty() && records.iter().all(|record| carried.contains(record)) {
            message.extra.remove("privacy");
            changed = true;
        }
    }
    changed
}

/// Repairs reconstruction payloads written by engines that stamped the whole source union
/// onto every reconstructed message.
pub fn compact_stored_reconstruction_privacy(messages: &mut [ChatMessage]) -> bool {
    use refact_core::active_context::{
        COMPRESSION_REPORT_KEY, RECONSTRUCTED_HISTORY_KIND, RECONSTRUCTION_SCHEMA_VERSION,
    };
    let mut changed = false;
    for message in messages
        .iter_mut()
        .filter(|message| message.role == COMPRESSION_REPORT_KEY)
    {
        let Some(payload) = message
            .extra
            .get_mut(COMPRESSION_REPORT_KEY)
            .filter(|report| {
                report.get("kind").and_then(serde_json::Value::as_str)
                    == Some(RECONSTRUCTED_HISTORY_KIND)
                    && report
                        .get("schema_version")
                        .and_then(serde_json::Value::as_u64)
                        == Some(u64::from(RECONSTRUCTION_SCHEMA_VERSION))
            })
            .and_then(|report| report.get_mut("payload"))
            .and_then(|payload| payload.get_mut("messages"))
        else {
            continue;
        };
        let mut nested = match <Vec<ChatMessage> as serde::Deserialize>::deserialize(&*payload) {
            Ok(nested) => nested,
            Err(error) => {
                tracing::warn!("skipping reconstruction payload privacy repair: {error}");
                continue;
            }
        };
        if !compact_reconstruction_privacy(&mut nested) {
            continue;
        }
        if let Some(originals) = payload.as_array_mut() {
            for (original, compacted) in originals.iter_mut().zip(nested) {
                if !compacted.extra.contains_key("privacy") {
                    if let Some(object) = original.as_object_mut() {
                        object.remove("privacy");
                    }
                }
            }
            changed = true;
        }
    }
    changed
}

fn file_record(
    gcx: &Arc<GlobalContext>,
    path: &Path,
    attribution: Attribution,
    derived_zones: &DerivedPrivacyZones,
) -> Result<FileRecord, String> {
    let snapshot = super::classification_cache::snapshot(gcx)?;
    let classified = snapshot.classify(gcx, path);
    let record = FileRecord {
        path: refact_core::chat_types::normalize_file_name(path.to_string_lossy().into_owned()),
        zone: finish_record_zone(
            &snapshot.compiled,
            &classified.candidates,
            classified.zone,
            derived_zones,
        ),
        attribution,
    };
    snapshot.finish(gcx)?;
    Ok(record)
}

pub fn declared_file_record(gcx: &Arc<GlobalContext>, path: &Path) -> Result<FileRecord, String> {
    file_record(
        gcx,
        path,
        Attribution::Declared,
        &new_derived_privacy_zones(),
    )
}

pub(crate) struct BatchDeclaredRecords {
    snapshot: Arc<super::classification_cache::Snapshot>,
    pub(crate) compiled: Arc<refact_privacy::CompiledPolicy>,
    mappings: crate::files_correction::WorktreePathMappings,
}

impl BatchDeclaredRecords {
    pub(crate) fn new(gcx: &Arc<GlobalContext>) -> Result<Self, String> {
        let snapshot = super::classification_cache::snapshot(gcx)?;
        Ok(Self {
            compiled: snapshot.compiled.clone(),
            mappings: snapshot.mappings.clone(),
            snapshot,
        })
    }

    pub(crate) fn classify_declared_paths(
        &mut self,
        gcx: &Arc<GlobalContext>,
        paths: impl IntoIterator<Item = PathBuf>,
    ) -> Vec<FileRecord> {
        let mut records = Vec::new();
        let mut seen = HashSet::new();
        for path in paths {
            let classified = self.snapshot.classify(gcx, &path);
            let record = FileRecord {
                path: refact_core::chat_types::normalize_file_name(
                    path.to_string_lossy().into_owned(),
                ),
                zone: classified.zone.name,
                attribution: Attribution::Declared,
            };
            if seen.insert(record.clone()) {
                records.push(record);
            }
        }
        records
    }

    pub(crate) fn finish(&self, gcx: &Arc<GlobalContext>) -> Result<(), String> {
        if *self.mappings != *self.snapshot.mappings {
            return Err("privacy mappings changed".into());
        }
        self.snapshot.finish(gcx)
    }
}

pub fn declared_file_records(
    gcx: &Arc<GlobalContext>,
    paths: impl IntoIterator<Item = PathBuf>,
) -> Result<Vec<FileRecord>, String> {
    let mut batch = BatchDeclaredRecords::new(gcx)?;
    let records = batch.classify_declared_paths(gcx, paths);
    batch.finish(gcx)?;
    Ok(records)
}

/// Private request-local configuration, not a permission or topology cache.
/// Admission must call `still_current` before publishing the resulting labels.
#[derive(Clone)]
pub(crate) struct ReviewPrivacyInputs {
    read_context: Arc<crate::files_in_workspace::FileReadContext>,
    policy: Arc<refact_privacy::PrivacyPolicy>,
    compiled: Arc<refact_privacy::CompiledPolicy>,
    workspaces: Vec<PathBuf>,
}

pub(crate) async fn prepare_review_privacy_inputs(
    gcx: Arc<GlobalContext>,
) -> Result<ReviewPrivacyInputs, String> {
    let mut read_context = crate::files_in_workspace::prepare_file_read_context(gcx.clone()).await;
    read_context.refresh_review_mappings(&gcx.cache_dir);
    let load = gcx.privacy_policy_load.read().unwrap().clone();
    if load.error.is_some() {
        return Err("Review privacy configuration is unavailable".into());
    }
    let compiled = load.policy.compile().map_err(|error| error.to_string())?;
    let workspaces = gcx
        .documents_state
        .workspace_folders
        .lock()
        .unwrap()
        .clone();
    Ok(ReviewPrivacyInputs {
        read_context: Arc::new(read_context),
        policy: load.policy,
        compiled: Arc::new(compiled),
        workspaces,
    })
}

impl ReviewPrivacyInputs {
    pub(crate) fn check_origin(&self, path: &Path) -> Result<(), String> {
        crate::files_in_workspace::check_file_privacy_for_read_preparation_with_context(
            &self.read_context,
            path,
        )
    }

    pub(crate) fn records(
        &self,
        gcx: &Arc<GlobalContext>,
        paths: impl IntoIterator<Item = PathBuf>,
    ) -> Result<Vec<FileRecord>, String> {
        self.records_cancellable(gcx, paths, &std::sync::atomic::AtomicBool::new(false))
    }

    pub(crate) fn records_cancellable(
        &self,
        gcx: &Arc<GlobalContext>,
        paths: impl IntoIterator<Item = PathBuf>,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<Vec<FileRecord>, String> {
        let mut records = Vec::new();
        let mut seen = HashSet::new();
        for path in paths {
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                return Err("Review privacy preparation cancelled".into());
            }
            // Retain config values only. Resolve roots, aliases and indexed
            // identities anew for every record, including topology ABA changes.
            let mappings = self.read_context.worktree_mappings();
            let candidates =
                record_path_candidates_with_workspaces(gcx, &path, mappings, &self.workspaces);
            let zone = self.compiled.strictest_zone_for_paths_with_roots(
                candidates,
                privacy_roots_with_workspaces(&self.workspaces, mappings),
            );
            let record = FileRecord {
                path: refact_core::chat_types::normalize_file_name(
                    path.to_string_lossy().into_owned(),
                ),
                zone: zone.name.clone(),
                attribution: Attribution::Declared,
            };
            if seen.insert(record.clone()) {
                records.push(record);
            }
        }
        Ok(records)
    }

    /// Actual configuration value equality, not an atomic filesystem guarantee.
    pub(crate) async fn still_current(&self, gcx: Arc<GlobalContext>) -> bool {
        let mut current = crate::files_in_workspace::prepare_file_read_context(gcx.clone()).await;
        current.refresh_review_mappings(&gcx.cache_dir);
        if !self.read_context.same_inputs(&current)
            || *gcx.documents_state.workspace_folders.lock().unwrap() != self.workspaces
        {
            return false;
        }
        let load = gcx.privacy_policy_load.read().unwrap().clone();
        if load.error.is_some() || load.policy != self.policy {
            return false;
        }
        // Bypass the normal loader's TTL: malformed/changed source must not be
        // admitted because the installed last-good policy is still equal.
        let global = super::global_privacy_path(&gcx);
        let projects = super::project_privacy_paths(
            crate::files_correction::get_project_dirs(gcx.clone()).await,
        );
        let fresh = refact_privacy::load_policy(&global, &projects, None).await;
        fresh.error.is_none() && fresh.policy == self.policy
    }
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
    let snapshot = super::classification_cache::snapshot(gcx)?;
    let mut records = Vec::new();
    let mut seen = HashSet::new();
    for path in paths {
        let classified = snapshot.classify(gcx, &path);
        let record = FileRecord {
            path: refact_core::chat_types::normalize_file_name(path.to_string_lossy().into_owned()),
            zone: finish_record_zone(
                &snapshot.compiled,
                &classified.candidates,
                classified.zone,
                derived_zones,
            ),
            attribution: Attribution::Observed,
        };
        if seen.insert(record.clone()) {
            records.push(record);
        }
    }
    snapshot.finish(gcx)?;
    Ok(records)
}

fn is_taintable_write_target(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|metadata| metadata.is_file())
        .unwrap_or(false)
        && !is_vcs_metadata_churn(path)
}

fn is_vcs_metadata_churn(path: &Path) -> bool {
    let components = path
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(name) => name.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>();
    components
        .iter()
        .enumerate()
        .any(|(index, name)| match *name {
            ".hg" | ".svn" => true,
            ".git" => components.get(index + 1) != Some(&"objects"),
            _ => false,
        })
}

fn inherit_observed_write_zones(
    gcx: &Arc<GlobalContext>,
    _policy: &refact_privacy::PrivacyPolicy,
    reads: &[FileRecord],
    writes: impl IntoIterator<Item = PathBuf>,
    derived_zones: &DerivedPrivacyZones,
) -> Result<(), String> {
    let snapshot = super::classification_cache::snapshot(gcx)?;
    let compiled = &snapshot.compiled;
    let Some(zone_name) = strictest_named_zone(
        &compiled,
        reads
            .iter()
            .filter(|record| record.zone != "normal")
            .map(|record| record.zone.as_str()),
    ) else {
        return Ok(());
    };
    let origin = reads
        .iter()
        .find(|record| record.zone == zone_name)
        .map(|record| record.path.clone());
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
                .map(|current| {
                    compare_named_zones(&compiled, zone_name, &current.zone) == Ordering::Less
                })
                .unwrap_or(true);
            if replace {
                derived_zones.insert(
                    candidate,
                    DerivedZone {
                        zone: zone_name.to_string(),
                        origin: origin.clone(),
                    },
                );
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

    #[tokio::test]
    async fn removed_session_label_still_requires_observation_and_withholds() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("derived.txt");
        std::fs::write(&path, "fixture").unwrap();
        let gcx = gcx_with_zones(temp.path(), Vec::new()).await;
        let derived = new_derived_privacy_zones();
        derived.write().unwrap().insert(
            path.clone(),
            DerivedZone {
                zone: "removed-guarded-zone".into(),
                origin: None,
            },
        );
        let destination = provider_destination("other/model");
        assert!(shell_observation_needed_for_session(
            &gcx,
            &destination,
            &derived
        ));
        let mut message = tool_message("guarded derived output");
        apply_shell_observation(
            &gcx,
            "cat derived.txt",
            temp.path(),
            &destination,
            ObservationStatus::Observed(ObservedAccess {
                reads: vec![path],
                writes: Vec::new(),
            }),
            &derived,
            &mut message,
        )
        .await
        .unwrap();
        assert_eq!(message.extra["privacy_shell"]["withheld"], true);
    }

    #[tokio::test]
    async fn declared_batch_matches_sequential_and_finish_revokes_policy_swap() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ["normal.txt", "guarded.txt", "blocked.txt", "normal.txt"]
            .into_iter()
            .map(|name| temp.path().join(name))
            .collect::<Vec<_>>();
        for path in &paths {
            std::fs::write(path, "fixture").unwrap();
        }
        let gcx = gcx_with_zones(
            temp.path(),
            vec![
                Zone {
                    name: "guarded".into(),
                    patterns: vec!["guarded.txt".into()],
                    send_to: vec!["trusted".into()],
                    on_shell_read: ShellBehavior::Ask,
                },
                Zone {
                    name: "normal".into(),
                    patterns: vec!["*".into()],
                    send_to: vec!["*".into()],
                    on_shell_read: ShellBehavior::Withhold,
                },
            ],
        )
        .await;
        let zones = gcx.privacy_policy_load.read().unwrap().policy.zones.clone();
        gcx.privacy_policy_load.write().unwrap().policy = Arc::new(PrivacyPolicy {
            blocked: vec!["blocked.txt".into()],
            zones,
            ..Default::default()
        });
        let mut batch = BatchDeclaredRecords::new(&gcx).unwrap();
        let expected = declared_file_records(&gcx, paths.clone()).unwrap();
        assert_eq!(batch.classify_declared_paths(&gcx, paths.clone()), expected);
        assert_eq!(batch.classify_declared_paths(&gcx, paths.clone()), expected);
        batch.finish(&gcx).unwrap();
        let old = gcx.privacy_policy_load.read().unwrap().policy.clone();
        gcx.privacy_policy_load.write().unwrap().policy = Arc::new((*old).clone());
        assert!(
            batch.finish(&gcx).is_err(),
            "policy identity swap must revoke in-flight batch"
        );
        let malformed = Arc::new(PrivacyPolicy {
            blocked: vec!["[".into()],
            ..Default::default()
        });
        gcx.privacy_policy_load.write().unwrap().policy = malformed;
        assert!(
            BatchDeclaredRecords::new(&gcx).is_err(),
            "invalid policy must fail closed"
        );
    }

    #[tokio::test]
    async fn declared_batch_finish_rejects_load_error_poison_and_mapping_snapshot_difference() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = gcx_with_zones(temp.path(), Vec::new()).await;
        let mut batch = BatchDeclaredRecords::new(&gcx).unwrap();
        batch.classify_declared_paths(&gcx, [temp.path().join("fixture")]);
        batch.finish(&gcx).unwrap();
        // Exercise the mapping fence without relying on the registry TTL or sleeps.
        batch.mappings = Arc::new(vec![
            crate::files_correction::RegisteredWorktreePathMapping {
                root: temp.path().join("different-root"),
                source_root: temp.path().join("different-source"),
            },
        ]);
        assert!(batch.finish(&gcx).is_err());
        let batch = BatchDeclaredRecords::new(&gcx).unwrap();
        gcx.privacy_policy_load.write().unwrap().error = Some("fixture load failure".into());
        assert!(batch.finish(&gcx).is_err());
        assert!(BatchDeclaredRecords::new(&gcx).is_err());
        gcx.privacy_policy_load.write().unwrap().error = None;
        let batch = BatchDeclaredRecords::new(&gcx).unwrap();
        let poisoned = gcx.clone();
        assert!(std::thread::spawn(move || {
            let _guard = poisoned.privacy_policy_load.write().unwrap();
            panic!("fixture poison");
        })
        .join()
        .is_err());
        assert!(batch.finish(&gcx).is_err());
        assert!(BatchDeclaredRecords::new(&gcx).is_err());
    }

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
    async fn review_inputs_validate_values_not_timestamps_and_fail_closed_on_sources() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        super::super::save_privacy_policy(gcx.clone(), PrivacyPolicy::default())
            .await
            .unwrap();
        let inputs = prepare_review_privacy_inputs(gcx.clone()).await.unwrap();
        assert!(inputs.still_current(gcx.clone()).await);
        // Equal settings with a different allocation/refresh timestamp remain equal.
        let settings = gcx.privacy_settings.read().unwrap().clone();
        *gcx.privacy_settings.write().unwrap() = Arc::new(crate::privacy::PrivacySettings {
            privacy_rules: crate::privacy::FilePrivacySettings {
                blocked: settings.privacy_rules.blocked.clone(),
                only_send_to_servers_I_control: settings
                    .privacy_rules
                    .only_send_to_servers_I_control
                    .clone(),
            },
            loaded_ts: u64::MAX / 2,
        });
        assert!(inputs.still_current(gcx.clone()).await);
        gcx.documents_state
            .workspace_folders
            .lock()
            .unwrap()
            .push(PathBuf::from("new-root"));
        assert!(!inputs.still_current(gcx.clone()).await);
        gcx.documents_state.workspace_folders.lock().unwrap().pop();
        let path = super::super::global_privacy_path(&gcx);
        std::fs::write(&path, "privacy_rules: [malformed").unwrap();
        assert!(!inputs.still_current(gcx.clone()).await);
        std::fs::write(&path, "privacy_rules:\n  blocked: ['*.rs']\n").unwrap();
        assert!(!inputs.still_current(gcx).await);
    }

    #[tokio::test]
    async fn review_inputs_use_retained_legacy_rules_and_cancel_between_records() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        super::super::save_privacy_policy(gcx.clone(), PrivacyPolicy::default())
            .await
            .unwrap();
        let inputs = prepare_review_privacy_inputs(gcx.clone()).await.unwrap();
        let path = PathBuf::from("example.rs");
        assert!(inputs.check_origin(&path).is_ok());
        *gcx.privacy_settings.write().unwrap() = Arc::new(crate::privacy::PrivacySettings {
            privacy_rules: crate::privacy::FilePrivacySettings {
                blocked: vec!["[".into()],
                only_send_to_servers_I_control: Vec::new(),
            },
            loaded_ts: u64::MAX / 2,
        });
        assert!(!inputs.still_current(gcx.clone()).await);
        assert!(inputs.check_origin(&path).is_ok());
        let malformed = prepare_review_privacy_inputs(gcx.clone()).await.unwrap();
        assert!(malformed.check_origin(&path).is_err());
        let cancel = std::sync::atomic::AtomicBool::new(true);
        assert!(inputs.records_cancellable(&gcx, [path], &cancel).is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn review_inputs_resolve_live_root_topology_for_each_record() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        let root = temp.path().join("root");
        std::fs::create_dir(&first).unwrap();
        std::fs::create_dir_all(second.join("secrets")).unwrap();
        let public = first.join("public.txt");
        let secret = second.join("secrets/file.txt");
        std::fs::write(&public, "public").unwrap();
        std::fs::write(&secret, "guarded").unwrap();
        std::os::unix::fs::symlink(&first, &root).unwrap();
        let gcx = gcx_with_zones(&root, Vec::new()).await;
        let mut policy = (*gcx.privacy_policy_load.read().unwrap().policy).clone();
        policy.blocked = vec!["secrets/*".into()];
        gcx.privacy_policy_load.write().unwrap().policy = Arc::new(policy);
        // Keep this fixture's installed policy instead of asking the TTL loader to refresh it.
        let settings = gcx.privacy_settings.read().unwrap().clone();
        *gcx.privacy_settings.write().unwrap() = Arc::new(crate::privacy::PrivacySettings {
            privacy_rules: crate::privacy::FilePrivacySettings {
                blocked: settings.privacy_rules.blocked.clone(),
                only_send_to_servers_I_control: settings
                    .privacy_rules
                    .only_send_to_servers_I_control
                    .clone(),
            },
            loaded_ts: u64::MAX / 2,
        });
        let inputs = prepare_review_privacy_inputs(gcx.clone()).await.unwrap();
        let paths = [public.clone(), secret, public]
            .into_iter()
            .enumerate()
            .map(|(i, path)| {
                std::fs::remove_file(&root).unwrap();
                std::os::unix::fs::symlink(if i == 1 { &second } else { &first }, &root).unwrap();
                path
            });
        let records = inputs.records(&gcx, paths).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].zone, "normal");
        assert_eq!(records[1].zone, "blocked");
        assert_eq!(std::fs::read_link(root).unwrap(), first);
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

    #[cfg(unix)]
    #[tokio::test]
    async fn declared_batch_refreshes_symlink_root_after_lazy_input_retarget() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        let root = temp.path().join("root");
        std::fs::create_dir(&first).unwrap();
        std::fs::create_dir_all(second.join("secrets")).unwrap();
        let secret = second.join("secrets/file.txt");
        let public = first.join("public.txt");
        std::fs::write(&secret, "guarded").unwrap();
        std::fs::write(&public, "public").unwrap();
        std::os::unix::fs::symlink(&first, &root).unwrap();
        let gcx = gcx_with_zones(&root, Vec::new()).await;
        let mut policy = (*gcx.privacy_policy_load.read().unwrap().policy).clone();
        policy.blocked = vec!["secrets/*".into()];
        gcx.privacy_policy_load.write().unwrap().policy = Arc::new(policy);
        let paths = [secret.clone(), public]
            .into_iter()
            .enumerate()
            .map(|(index, path)| {
                std::fs::remove_file(&root).unwrap();
                std::os::unix::fs::symlink(if index == 0 { &second } else { &first }, &root)
                    .unwrap();
                path
            });

        let records = declared_file_records(&gcx, paths).unwrap();

        assert_eq!(records.len(), 2);
        assert_eq!(records[0].zone, "blocked");
        assert_eq!(records[1].zone, "normal");
        assert_eq!(std::fs::read_link(&root).unwrap(), first);
    }

    #[tokio::test]
    async fn declared_batch_rejects_workspace_roots_changed_by_lazy_input() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        std::fs::create_dir(&first).unwrap();
        std::fs::create_dir_all(second.join("secrets")).unwrap();
        let secret = second.join("secrets/file.txt");
        std::fs::write(&secret, "guarded").unwrap();
        let gcx = gcx_with_zones(&first, Vec::new()).await;
        let mut policy = (*gcx.privacy_policy_load.read().unwrap().policy).clone();
        policy.blocked = vec!["secrets/*".into()];
        gcx.privacy_policy_load.write().unwrap().policy = Arc::new(policy);
        let paths = std::iter::once_with(|| {
            *gcx.documents_state.workspace_folders.lock().unwrap() = vec![second];
            secret
        });

        assert_eq!(
            declared_file_records(&gcx, paths).unwrap_err(),
            "privacy context changed"
        );
    }

    #[tokio::test]
    async fn declared_batch_matches_single_records_and_reloads_changed_policy() {
        let temp = tempfile::tempdir().unwrap();
        let secret = temp.path().join("secret.txt");
        let public = temp.path().join("public.txt");
        std::fs::write(&secret, "guarded").unwrap();
        std::fs::write(&public, "public").unwrap();
        let gcx = gcx_with_policy(&secret, &[], ShellBehavior::Withhold).await;
        let paths = vec![
            public.clone(),
            secret.clone(),
            public.clone(),
            secret.clone(),
        ];
        let expected = vec![
            declared_file_record(&gcx, &public).unwrap(),
            declared_file_record(&gcx, &secret).unwrap(),
        ];
        assert_eq!(
            declared_file_records(&gcx, paths.clone()).unwrap(),
            expected
        );
        let mut policy = (*gcx.privacy_policy_load.read().unwrap().policy).clone();
        policy.blocked.push("public.txt".into());
        gcx.privacy_policy_load.write().unwrap().policy = Arc::new(policy);
        let changed = declared_file_records(&gcx, paths).unwrap();
        assert_eq!(changed[0], declared_file_record(&gcx, &public).unwrap());
        assert_eq!(changed[0].zone, "blocked");
        assert_eq!(changed[1], expected[1]);
    }

    #[tokio::test]
    async fn live_record_candidates_preserve_registered_alias_fanout() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        std::fs::create_dir(&source).unwrap();
        let source = dunce::canonicalize(source).unwrap();
        let file = source.join("file.txt");
        std::fs::write(&file, "value").unwrap();
        let mappings = (0..16)
            .map(
                |index| crate::files_correction::RegisteredWorktreePathMapping {
                    root: temp.path().join(format!("worktree-{index}")),
                    source_root: source.clone(),
                },
            )
            .collect::<Vec<_>>();
        let gcx = gcx_with_policy(&file, &[], ShellBehavior::Withhold).await;
        let mut policy = (*gcx.privacy_policy_load.read().unwrap().policy).clone();
        policy.blocked.push(
            mappings[15]
                .root
                .join("file.txt")
                .to_string_lossy()
                .into_owned(),
        );
        let compiled = policy.compile().unwrap();
        let candidates = record_path_candidates(&gcx, &file, &mappings);
        for mapping in &mappings {
            assert!(candidates.contains(&mapping.root.join("file.txt")));
        }
        assert_eq!(
            compiled
                .strictest_zone_for_paths_with_roots(&candidates, privacy_roots(&gcx, &mappings))
                .name,
            "blocked"
        );
        policy.blocked = vec![file.to_string_lossy().into_owned()];
        let compiled = policy.compile().unwrap();
        for mapping in &mappings {
            let candidates =
                record_path_candidates(&gcx, &mapping.root.join("file.txt"), &mappings);
            assert!(candidates.contains(&file));
            assert_eq!(
                compiled
                    .strictest_zone_for_paths_with_roots(
                        &candidates,
                        privacy_roots(&gcx, &mappings)
                    )
                    .name,
                "blocked"
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn declared_batch_preserves_indexed_hardlink_and_symlink_classification() {
        let temp = tempfile::tempdir().unwrap();
        let secret = temp.path().join("secret.txt");
        let hardlink = temp.path().join("hardlink.txt");
        let symlink = temp.path().join("symlink.txt");
        std::fs::write(&secret, "guarded").unwrap();
        std::fs::hard_link(&secret, &hardlink).unwrap();
        std::os::unix::fs::symlink(&secret, &symlink).unwrap();
        let gcx = gcx_with_policy(&secret, &[], ShellBehavior::Withhold).await;
        gcx.file_index.lookup_or_stat(&secret).unwrap();
        gcx.file_index.lookup_or_stat(&hardlink).unwrap();
        let paths = vec![hardlink, symlink, secret];
        let expected = paths
            .iter()
            .map(|path| declared_file_record(&gcx, path).unwrap())
            .collect::<Vec<_>>();
        let actual = declared_file_records(&gcx, paths).unwrap();
        assert_eq!(actual, expected);
        assert!(actual.iter().all(|record| record.zone == "secrets"));
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
        derived_zones.write().unwrap().insert(
            file.clone(),
            DerivedZone {
                zone: "derived-earlier".to_string(),
                origin: None,
            },
        );

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
    async fn vcs_metadata_writes_do_not_inherit_guarded_zone() {
        let temp = tempfile::tempdir().unwrap();
        let secret = temp.path().join("secret.txt");
        std::fs::write(&secret, "secret").unwrap();
        let commit_editmsg = temp.path().join(".git").join("COMMIT_EDITMSG");
        let object = temp
            .path()
            .join(".git")
            .join("objects")
            .join("ab")
            .join("cd");
        std::fs::create_dir_all(commit_editmsg.parent().unwrap()).unwrap();
        std::fs::create_dir_all(object.parent().unwrap()).unwrap();
        std::fs::write(&commit_editmsg, "message").unwrap();
        std::fs::write(&object, "object").unwrap();
        let gcx = gcx_with_policy(&secret, &[], ShellBehavior::Withhold).await;
        let derived_zones = new_derived_privacy_zones();
        let destination = provider_destination("untrusted/model");
        let mut commit_message = tool_message("committed");

        apply_shell_observation(
            &gcx,
            "git commit",
            temp.path(),
            &destination,
            ObservationStatus::Observed(ObservedAccess {
                reads: vec![secret.clone()],
                writes: vec![commit_editmsg.clone(), object.clone()],
            }),
            &derived_zones,
            &mut commit_message,
        )
        .await
        .unwrap();

        {
            let tainted = derived_zones.read().unwrap();
            assert!(
                !tainted.keys().any(|path| path == commit_editmsg.as_path()),
                "version-control churn must never inherit a guarded zone"
            );
            assert!(
                tainted.keys().any(|path| path == object.as_path()),
                "git object files can hold real content and stay taintable"
            );
        }

        let mut next_commit = tool_message("second commit");
        apply_shell_observation(
            &gcx,
            "git commit",
            temp.path(),
            &destination,
            ObservationStatus::Observed(ObservedAccess {
                reads: vec![commit_editmsg],
                writes: Vec::new(),
            }),
            &derived_zones,
            &mut next_commit,
        )
        .await
        .unwrap();

        assert_eq!(next_commit.content.content_text_only(), "second commit");
        assert!(next_commit.extra.get("privacy").is_none());
    }

    #[tokio::test]
    async fn inherited_zone_is_reported_with_its_origin() {
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
                reads: vec![secret.clone()],
                writes: vec![derived.clone()],
            }),
            &derived_zones,
            &mut copy_message,
        )
        .await
        .unwrap();

        assert_eq!(
            derived_zone_for_path(&gcx, &derived, &derived_zones)
                .and_then(|zone| zone.origin)
                .unwrap(),
            refact_core::chat_types::normalize_file_name(secret.to_string_lossy().into_owned())
        );

        let mut read_message = tool_message("derived secret");
        apply_shell_observation(
            &gcx,
            "cat derived.txt",
            temp.path(),
            &destination,
            ObservationStatus::Observed(ObservedAccess {
                reads: vec![derived.clone()],
                writes: Vec::new(),
            }),
            &derived_zones,
            &mut read_message,
        )
        .await
        .unwrap();

        let withheld = read_message.content.content_text_only();
        assert!(withheld.starts_with("Output withheld by user privacy policy"));
        assert!(withheld.contains(&format!(
            "(zone \"secrets\", inherited from \"{}\")",
            refact_core::chat_types::normalize_file_name(secret.to_string_lossy().into_owned())
        )));
        assert!(withheld.contains("session-derived label"));

        let mut plain_message = tool_message("secret output");
        apply_shell_observation(
            &gcx,
            "cat secret.txt",
            temp.path(),
            &destination,
            ObservationStatus::Observed(ObservedAccess {
                reads: vec![secret.clone()],
                writes: Vec::new(),
            }),
            &derived_zones,
            &mut plain_message,
        )
        .await
        .unwrap();

        let plain = plain_message.content.content_text_only();
        assert!(plain.contains(&format!(
            "  - {} (zone \"secrets\")",
            refact_core::chat_types::normalize_file_name(secret.to_string_lossy().into_owned())
        )));
        assert!(!plain.contains("inherited from"));
        assert!(!plain.contains("session-derived label"));
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

    #[tokio::test]
    async fn observation_reason_counts_match_the_boolean_decision() {
        let temp = tempfile::tempdir().unwrap();
        let secret = temp.path().join("secret.txt");
        std::fs::write(&secret, "secret").unwrap();
        let gcx = gcx_with_policy(&secret, &["trusted"], ShellBehavior::Withhold).await;
        let allowed = provider_destination("trusted/model");
        let guarded = provider_destination("untrusted/model");
        let derived_zones = new_derived_privacy_zones();

        let allowed_reason = shell_observation_reason(&gcx, &allowed);
        assert_eq!(allowed_reason.blocked_patterns, 0);
        assert_eq!(allowed_reason.guarded_zones, 0);
        assert_eq!(allowed_reason.derived_labels, 0);
        assert!(!allowed_reason.needed());
        assert_eq!(
            allowed_reason.needed(),
            shell_observation_needed(&gcx, &allowed)
        );

        let guarded_reason = shell_observation_reason(&gcx, &guarded);
        assert_eq!(guarded_reason.guarded_zones, 1);
        assert!(guarded_reason.needed());
        assert_eq!(
            guarded_reason.needed(),
            shell_observation_needed(&gcx, &guarded)
        );

        derived_zones.write().unwrap().insert(
            temp.path().join("derived.txt"),
            DerivedZone {
                zone: "blocked".to_string(),
                origin: None,
            },
        );
        let session_reason = shell_observation_reason_for_session(&gcx, &allowed, &derived_zones);
        assert_eq!(session_reason.guarded_zones, 0);
        assert_eq!(session_reason.derived_labels, 1);
        assert!(session_reason.needed());
        assert_eq!(
            session_reason.needed(),
            shell_observation_needed_for_session(&gcx, &allowed, &derived_zones)
        );
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
    fn malformed_or_future_privacy_is_not_erased_by_merge() {
        let mut message = tool_message("guarded output");
        let future = serde_json::json!({"files": [], "future_obligation": "restricted"});
        message.extra.insert("privacy".into(), future.clone());
        merge_records(&mut message, [guarded_record("another.env")]);
        assert_eq!(message.extra["privacy"], future);
        assert!(records_to_carry(&[message]).is_err());
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

    fn guarded_record(path: &str) -> FileRecord {
        FileRecord {
            path: path.to_string(),
            zone: "secrets".to_string(),
            attribution: Attribution::Declared,
        }
    }

    fn control_metadata(role: &str) -> Option<serde_json::Value> {
        match role {
            "goal" | "plan" => Some(serde_json::json!({"version": 1, "mode": "agent"})),
            "event" => Some(serde_json::json!({
                "subkind": "system_notice",
                "source": "test",
                "payload": {},
            })),
            _ => None,
        }
    }

    fn message_with_records(id: &str, role: &str, records: Vec<FileRecord>) -> ChatMessage {
        let mut message = ChatMessage {
            message_id: id.to_string(),
            role: role.to_string(),
            content: crate::call_validation::ChatContent::SimpleText(format!("{role} body")),
            ..Default::default()
        };
        if let Some(metadata) = control_metadata(role) {
            message.extra.insert(role.to_string(), metadata);
        }
        merge_records(&mut message, records);
        message
    }

    fn record_identity(messages: &[ChatMessage]) -> HashSet<(String, String, String)> {
        refact_privacy::records_from_messages(messages)
            .unwrap()
            .into_iter()
            .map(|(_, record)| {
                (
                    record.zone,
                    record.path,
                    serde_json::to_value(record.attribution)
                        .unwrap()
                        .as_str()
                        .unwrap()
                        .to_string(),
                )
            })
            .collect()
    }

    #[test]
    fn compact_reconstruction_privacy_drops_records_covered_by_user_union() {
        let first = guarded_record("a.env");
        let second = guarded_record("b.env");
        let third = guarded_record("c.env");
        let mut payload = vec![
            message_with_records("m-system", "system", Vec::new()),
            message_with_records("m-user", "user", vec![first.clone(), second.clone()]),
            message_with_records("m-event", "event", vec![first.clone()]),
            message_with_records("m-goal", "goal", vec![first.clone(), second.clone()]),
            message_with_records("m-context", "context_file", vec![third.clone()]),
        ];
        let before = record_identity(&payload);

        assert!(compact_reconstruction_privacy(&mut payload));

        assert!(payload[0].extra.get("privacy").is_none());
        assert_eq!(
            message_records(&payload[1]).unwrap(),
            vec![first, second.clone()]
        );
        assert!(payload[2].extra.get("privacy").is_none());
        assert!(payload[3].extra.get("privacy").is_none());
        assert_eq!(message_records(&payload[4]).unwrap(), vec![third]);
        assert_eq!(record_identity(&payload), before);
    }

    #[test]
    fn compact_reconstruction_privacy_is_idempotent_and_noop_without_user_carrier() {
        let record = guarded_record("a.env");
        let mut without_user = vec![
            message_with_records("m-event", "event", vec![record.clone()]),
            message_with_records("m-goal", "goal", vec![record.clone()]),
        ];
        let unchanged = serde_json::to_value(&without_user).unwrap();

        assert!(!compact_reconstruction_privacy(&mut without_user));
        assert_eq!(serde_json::to_value(&without_user).unwrap(), unchanged);

        let mut with_user = vec![
            message_with_records("m-user", "user", vec![record.clone()]),
            message_with_records("m-event", "event", vec![record]),
        ];

        assert!(compact_reconstruction_privacy(&mut with_user));
        let compacted = serde_json::to_value(&with_user).unwrap();
        assert!(!compact_reconstruction_privacy(&mut with_user));
        assert_eq!(serde_json::to_value(&with_user).unwrap(), compacted);
    }

    #[test]
    fn compact_stored_reconstruction_privacy_rewrites_legacy_payload() {
        use refact_core::active_context::{
            COMPRESSION_REPORT_KEY, ReconstructionMetadata, STATIC_COMPRESSION_REPORT_KIND,
            active_context, make_reconstruction_report,
        };

        let first = guarded_record("a.env");
        let second = guarded_record("b.env");
        let union = vec![first.clone(), second];
        let legacy_payload = vec![
            message_with_records("p-user", "user", union.clone()),
            message_with_records("p-event", "event", union.clone()),
            message_with_records("p-goal", "goal", union.clone()),
            message_with_records("p-user-two", "user", union.clone()),
        ];
        let boundary =
            make_reconstruction_report(legacy_payload.clone(), ReconstructionMetadata::default())
                .unwrap();
        let mut stored = vec![
            message_with_records("archived", "user", vec![guarded_record("archived.env")]),
            boundary,
            message_with_records("tail", "user", vec![first]),
        ];
        let before = record_identity(&active_context(&stored).unwrap().messages);

        assert!(compact_stored_reconstruction_privacy(&mut stored));

        let active = active_context(&stored).unwrap();
        for message in &active.messages {
            assert_eq!(
                message.extra.contains_key("privacy"),
                message.message_id == "p-user" || message.message_id == "tail",
                "only the first reconstruction carrier and tail keep provenance"
            );
        }
        assert_eq!(record_identity(&active.messages), before);
        assert!(!compact_stored_reconstruction_privacy(&mut stored));

        let mut static_report = ChatMessage {
            message_id: "static".to_string(),
            role: COMPRESSION_REPORT_KEY.to_string(),
            content: crate::call_validation::ChatContent::SimpleText("Static trim".to_string()),
            ..Default::default()
        };
        static_report.extra.insert(
            COMPRESSION_REPORT_KEY.to_string(),
            serde_json::json!({
                "kind": STATIC_COMPRESSION_REPORT_KIND,
                "schema_version": 1,
                "payload": { "messages": serde_json::to_value(&legacy_payload).unwrap() },
            }),
        );
        let mut static_stored = vec![static_report];
        let static_before = serde_json::to_value(&static_stored).unwrap();

        assert!(!compact_stored_reconstruction_privacy(&mut static_stored));
        assert_eq!(serde_json::to_value(&static_stored).unwrap(), static_before);

        let mut malformed =
            make_reconstruction_report(legacy_payload, ReconstructionMetadata::default()).unwrap();
        malformed.extra.get_mut(COMPRESSION_REPORT_KEY).unwrap()["payload"]["messages"] =
            serde_json::json!("not an array");
        let mut malformed_stored = vec![malformed];
        let malformed_before = serde_json::to_value(&malformed_stored).unwrap();

        assert!(!compact_stored_reconstruction_privacy(
            &mut malformed_stored
        ));
        assert_eq!(
            serde_json::to_value(&malformed_stored).unwrap(),
            malformed_before
        );
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

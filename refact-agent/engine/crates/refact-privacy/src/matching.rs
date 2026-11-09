use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};

use glob::Pattern;
use unicode_normalization::UnicodeNormalization;

use crate::policy::{PrivacyPolicy, ShellBehavior, Zone};

#[cfg(test)]
thread_local! {
    static MATCH_WORK: std::cell::Cell<Option<[usize; 4]>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
fn count_work(index: usize) {
    MATCH_WORK.with(|counts| {
        if let Some(mut values) = counts.get() {
            values[index] += 1;
            counts.set(Some(values));
        }
    });
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyError {
    InvalidGlob { pattern: String, message: String },
    InvalidZoneName { name: String },
}

impl fmt::Display for PolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidGlob {
                pattern,
                message,
            } => {
                write!(formatter, "invalid glob pattern {pattern:?}: {message}")
            }
            Self::InvalidZoneName {
                name,
            } => {
                write!(formatter, "invalid privacy zone name {name:?}")
            }
        }
    }
}

impl std::error::Error for PolicyError {}

struct PrefixedPattern {
    pattern: Pattern,
    literal_end: usize,
}

impl PrefixedPattern {
    fn new(pattern: Pattern) -> Self {
        let literal_end = pattern
            .as_str()
            .find(['*', '?', '['])
            .unwrap_or(pattern.as_str().len());
        Self {
            pattern,
            literal_end,
        }
    }

    fn matches(&self, candidate: &str) -> bool {
        #[cfg(test)]
        count_work(2);
        candidate.starts_with(&self.pattern.as_str()[..self.literal_end])
            && self.pattern.matches(candidate)
    }
}

fn compile_zone_patterns(patterns: &[String]) -> Result<Vec<PrefixedPattern>, PolicyError> {
    compile_patterns(patterns)
        .map(|patterns| patterns.into_iter().map(PrefixedPattern::new).collect())
}

struct CompiledZone {
    zone: Zone,
    patterns: Vec<PrefixedPattern>,
}

pub struct CompiledPolicy {
    zones: Vec<CompiledZone>,
    normal_index: usize,
}

/// An operation-local owner of one immutable policy and its pure lexical memo.
///
/// Every call still observes roots and each lazily consumed path afresh. Only
/// string-to-earliest-pattern-match work is reused, never filesystem observations
/// or path-to-zone verdicts. The combined maps retain at most 4096 entries and
/// 1 MiB of key bytes; this is not a bound on total heap allocation.
pub struct ClassificationBatch {
    compiled: CompiledPolicy,
    memo: LexicalMemo,
}

impl ClassificationBatch {
    pub fn new(compiled: CompiledPolicy) -> Self {
        Self {
            compiled,
            memo: LexicalMemo::new(4096, 1024 * 1024),
        }
    }

    pub fn strictest_zone_for_paths_with_roots<I, P, R, Q>(&mut self, paths: I, roots: R) -> Zone
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
        R: IntoIterator<Item = Q>,
        Q: AsRef<Path>,
    {
        self.memo.reset_after_admission_failure();
        let roots = roots
            .into_iter()
            .map(|root| absolute_path(root.as_ref()))
            .collect::<Vec<_>>();
        let canonical_roots = canonical_roots_of(&roots);
        let root_index = PreparedRootIndex::new(&roots, &canonical_roots);
        let indices = self
            .compiled
            .zone_indices_with_memo(paths, &root_index, &mut self.memo);
        let zones = indices
            .into_iter()
            .map(|index| {
                let zone = &self.compiled.zones[index].zone;
                // Keep matched permissions, but FIRST name index for ordering.
                (self.compiled.zone_index_named(&zone.name).unwrap(), zone)
            })
            .collect::<Vec<_>>();
        effective_zone(&zones)
            .unwrap_or_else(|| self.compiled.zones[self.compiled.normal_index].zone.clone())
    }
}

// Deliberately no DerefMut: replacing the policy must never retain its memo.
impl std::ops::Deref for ClassificationBatch {
    type Target = CompiledPolicy;

    fn deref(&self) -> &Self::Target {
        &self.compiled
    }
}

pub fn compile_patterns(patterns: &[String]) -> Result<Vec<Pattern>, PolicyError> {
    let mut compiled = Vec::with_capacity(patterns.len());
    for pattern in patterns {
        let normalized = normalize(pattern);
        compiled.push(
            Pattern::new(&normalized).map_err(|error| PolicyError::InvalidGlob {
                pattern: pattern.clone(),
                message: error.to_string(),
            })?,
        );
        for alias in canonical_pattern_aliases(&normalized) {
            if let Ok(alias) = Pattern::new(&alias) {
                compiled.push(alias);
            }
        }
    }
    Ok(compiled)
}

#[cfg(windows)]
fn canonical_pattern_aliases(normalized: &str) -> Vec<String> {
    let literal_end = normalized.find(['*', '?', '[']).unwrap_or(normalized.len());
    let boundary = normalized[..literal_end]
        .rfind('/')
        .map_or(0, |index| index + 1);
    let (base, tail) = normalized.split_at(boundary);
    if base.is_empty() {
        return Vec::new();
    }
    let Ok(canonical) = dunce::canonicalize(base) else {
        return Vec::new();
    };
    let canonical = normalize(&canonical.to_string_lossy());
    let alias = format!("{}/{tail}", canonical.trim_end_matches('/'));
    if alias == normalized {
        Vec::new()
    } else {
        vec![alias]
    }
}

#[cfg(not(windows))]
fn canonical_pattern_aliases(_normalized: &str) -> Vec<String> {
    Vec::new()
}

impl PrivacyPolicy {
    pub fn compile(&self) -> Result<CompiledPolicy, PolicyError> {
        let mut zones =
            Vec::with_capacity(self.zones.len() + usize::from(!self.blocked.is_empty()) + 1);
        if !self.blocked.is_empty() {
            zones.push(CompiledZone {
                zone: Zone {
                    name: "blocked".to_string(),
                    patterns: self.blocked.clone(),
                    send_to: Vec::new(),
                    on_shell_read: ShellBehavior::Deny,
                },
                patterns: compile_zone_patterns(&self.blocked)?,
            });
        }

        for zone in &self.zones {
            if zone.name.starts_with("effective:") || zone.name.contains('+') {
                return Err(PolicyError::InvalidZoneName {
                    name: zone.name.clone(),
                });
            }
            zones.push(CompiledZone {
                zone: zone.clone(),
                patterns: compile_zone_patterns(&zone.patterns)?,
            });
        }

        let normal_index = match zones
            .iter()
            .position(|compiled| compiled.zone.name == "normal")
        {
            Some(index) => index,
            None => {
                let zone = Zone {
                    name: "normal".to_string(),
                    patterns: vec!["**".to_string()],
                    send_to: vec!["*".to_string()],
                    on_shell_read: ShellBehavior::Withhold,
                };
                let index = zones.len();
                zones.push(CompiledZone {
                    patterns: compile_zone_patterns(&zone.patterns)?,
                    zone,
                });
                index
            }
        };

        Ok(CompiledPolicy {
            zones,
            normal_index,
        })
    }
}

impl CompiledPolicy {
    pub fn zone_named(&self, name: &str) -> Option<&Zone> {
        self.zones
            .iter()
            .find(|compiled| compiled.zone.name == name)
            .map(|compiled| &compiled.zone)
    }

    pub fn zone_index_named(&self, name: &str) -> Option<usize> {
        self.zones
            .iter()
            .position(|compiled| compiled.zone.name == name)
    }

    pub fn zone_for_path(&self, path: &Path) -> &Zone {
        self.zone_for_path_with_roots(path, std::iter::empty::<&Path>())
    }

    pub fn zone_for_path_with_roots<I, P>(&self, path: &Path, roots: I) -> &Zone
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        let roots = roots
            .into_iter()
            .map(|root| absolute_path(root.as_ref()))
            .collect::<Vec<_>>();
        let canonical_roots = canonical_roots_of(&roots);
        let root_index = PreparedRootIndex::new(&roots, &canonical_roots);
        self.zone_for_path_prepared(path, &root_index)
    }

    fn zone_for_path_prepared(&self, path: &Path, root_index: &PreparedRootIndex) -> &Zone {
        let candidates = normalized_candidates(&path_candidates(path, root_index));
        if let Some((index, _)) = self.zones.iter().enumerate().find(|(_, compiled)| {
            compiled.patterns.iter().any(|pattern| {
                candidates
                    .iter()
                    .any(|candidate| pattern.matches(candidate))
            })
        }) {
            return &self.zones[index].zone;
        }

        &self.zones[self.normal_index].zone
    }

    pub fn strictest_zone_for_paths<I, P>(&self, paths: I) -> Zone
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        self.strictest_zone_for_paths_with_roots(paths, std::iter::empty::<&Path>())
    }

    pub fn strictest_zone_for_paths_with_roots<I, P, R, Q>(&self, paths: I, roots: R) -> Zone
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
        R: IntoIterator<Item = Q>,
        Q: AsRef<Path>,
    {
        let roots = roots
            .into_iter()
            .map(|root| absolute_path(root.as_ref()))
            .collect::<Vec<_>>();
        let canonical_roots = canonical_roots_of(&roots);
        let root_index = PreparedRootIndex::new(&roots, &canonical_roots);
        let paths = paths.into_iter();
        if paths.size_hint().1.is_some_and(|upper| upper <= 1) {
            let zones = paths
                .map(|path| {
                    let zone = self.zone_for_path_prepared(path.as_ref(), &root_index);
                    (self.zone_index_named(&zone.name).unwrap(), zone)
                })
                .collect::<Vec<_>>();
            return effective_zone(&zones)
                .unwrap_or_else(|| self.zones[self.normal_index].zone.clone());
        }
        let indices = self.zone_indices_for_paths(paths, &root_index);
        let zones = indices
            .into_iter()
            .map(|index| {
                let zone = &self.zones[index].zone;
                (self.zone_index_named(&zone.name).unwrap(), zone)
            })
            .collect::<Vec<_>>();
        effective_zone(&zones).unwrap_or_else(|| self.zones[self.normal_index].zone.clone())
    }

    fn zone_indices_for_paths<I, P>(&self, paths: I, root_index: &PreparedRootIndex) -> Vec<usize>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        self.zone_indices_with_budget(paths, root_index, 4096, 1024 * 1024)
    }

    fn zone_indices_with_budget<I, P>(
        &self,
        paths: I,
        root_index: &PreparedRootIndex,
        entries: usize,
        bytes: usize,
    ) -> Vec<usize>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        self.zone_indices_with_memo(paths, root_index, &mut LexicalMemo::new(entries, bytes))
    }

    fn zone_indices_with_memo<I, P>(
        &self,
        paths: I,
        root_index: &PreparedRootIndex,
        memo: &mut LexicalMemo,
    ) -> Vec<usize>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        let LexicalMemo {
            raw_matches,
            normalized_matches,
            budget,
            admission_failed,
            ..
        } = memo;
        let mut indices = Vec::new();
        for path in paths {
            let mut index = self.zones.len();
            for candidate in path_candidates(path.as_ref(), root_index) {
                let raw = candidate.to_string_lossy();
                let matched = if let Some(matched) = raw_matches.get(raw.as_ref()) {
                    *matched
                } else {
                    let normalized = normalize(&raw);
                    let mut matched = self.zones.len();
                    if !normalized.is_empty() {
                        let basename = normalized.rsplit('/').find(|part| !part.is_empty());
                        for candidate in [basename, Some(normalized.as_str())].into_iter().flatten()
                        {
                            let index = if let Some(index) = normalized_matches.get(candidate) {
                                *index
                            } else {
                                let index = self
                                    .zones
                                    .iter()
                                    .position(|zone| {
                                        zone.patterns
                                            .iter()
                                            .any(|pattern| pattern.matches(candidate))
                                    })
                                    .unwrap_or(self.zones.len());
                                if budget.reserve(candidate.len()) {
                                    normalized_matches.insert(candidate.to_owned(), index);
                                } else {
                                    *admission_failed = true;
                                }
                                index
                            };
                            matched = matched.min(index);
                            if matched == 0 {
                                break;
                            }
                        }
                    }
                    if budget.reserve(raw.len()) {
                        raw_matches.insert(raw.into_owned(), matched);
                    } else {
                        *admission_failed = true;
                    }
                    matched
                };
                index = index.min(matched);
                if index == 0 {
                    break;
                }
            }
            indices.push(if index == self.zones.len() {
                self.normal_index
            } else {
                index
            });
        }
        indices
    }
}

// Values are earliest enum indices, or zones.len() for NO match. In particular,
// the sentinel is not normal_index: another representation may match later.
struct LexicalMemo {
    raw_matches: HashMap<String, usize>,
    normalized_matches: HashMap<String, usize>,
    budget: MemoBudget,
    capacity: MemoBudget,
    admission_failed: bool,
}

impl LexicalMemo {
    fn new(entries: usize, bytes: usize) -> Self {
        Self {
            raw_matches: HashMap::new(),
            normalized_matches: HashMap::new(),
            budget: MemoBudget {
                entries,
                bytes,
            },
            capacity: MemoBudget {
                entries,
                bytes,
            },
            admission_failed: false,
        }
    }

    fn reset_after_admission_failure(&mut self) {
        // Reset only between calls; a full memo with no rejected miss stays warm.
        if self.admission_failed {
            self.raw_matches.clear();
            self.normalized_matches.clear();
            self.budget = self.capacity;
            self.admission_failed = false;
        }
    }
}

#[derive(Clone, Copy)]
struct MemoBudget {
    entries: usize,
    bytes: usize,
}

impl MemoBudget {
    fn reserve(&mut self, bytes: usize) -> bool {
        if self.entries == 0 || bytes > self.bytes {
            return false;
        }
        self.entries -= 1;
        self.bytes -= bytes;
        true
    }
}

fn effective_zone(zones: &[(usize, &Zone)]) -> Option<Zone> {
    let first = zones.first()?.1;
    let send_to = zones
        .iter()
        .skip(1)
        .fold(first.send_to.clone(), |allowed, (_, zone)| {
            intersect_destinations(&allowed, &zone.send_to)
        });
    let on_shell_read = zones
        .iter()
        .fold(ShellBehavior::Withhold, |strictest, (_, zone)| {
            strictest_shell_behavior(strictest, zone.on_shell_read)
        });
    let name = zones
        .iter()
        .filter(|(_, zone)| {
            same_destinations(&zone.send_to, &send_to) && zone.on_shell_read == on_shell_read
        })
        .min_by_key(|(index, _)| *index)
        .map(|(_, zone)| zone.name.clone())
        .unwrap_or_else(|| {
            let names = zones
                .iter()
                .map(|(_, zone)| zone.name.as_str())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
                .join("+");
            format!("effective:{names}")
        });
    Some(Zone {
        name,
        patterns: Vec::new(),
        send_to,
        on_shell_read,
    })
}

fn intersect_destinations(left: &[String], right: &[String]) -> Vec<String> {
    let left_wildcard = left.iter().any(|destination| destination == "*");
    let right_wildcard = right.iter().any(|destination| destination == "*");
    if left_wildcard && right_wildcard {
        return vec!["*".to_string()];
    }
    let values = if left_wildcard {
        right.iter()
    } else if right_wildcard {
        left.iter()
    } else {
        return left
            .iter()
            .filter(|destination| right.contains(destination))
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
    };
    values
        .filter(|destination| destination.as_str() != "*")
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn same_destinations(left: &[String], right: &[String]) -> bool {
    let canonical = |values: &[String]| {
        if values.iter().any(|value| value == "*") {
            BTreeSet::from(["*".to_string()])
        } else {
            values.iter().cloned().collect()
        }
    };
    canonical(left) == canonical(right)
}

fn strictest_shell_behavior(left: ShellBehavior, right: ShellBehavior) -> ShellBehavior {
    use ShellBehavior::{Ask, Deny, Withhold};

    match (left, right) {
        (Deny, _) | (_, Deny) => Deny,
        (Ask, _) | (_, Ask) => Ask,
        (Withhold, Withhold) => Withhold,
    }
}

fn normalize(value: &str) -> String {
    #[cfg(test)]
    count_work(1);
    let slashed = value.replace('\\', "/");
    if slashed.is_ascii() {
        return slashed.to_ascii_lowercase();
    }
    slashed.nfc().collect::<String>().to_lowercase()
}

fn absolute_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|current| current.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    }
}

fn canonical_roots_of(roots: &[PathBuf]) -> Vec<PathBuf> {
    roots
        .iter()
        .map(|root| canonicalize_or_original(root))
        .collect()
}

fn canonicalize_or_original(path: &Path) -> PathBuf {
    #[cfg(test)]
    count_work(0);
    dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

struct PreparedRootIndex {
    roots: HashSet<PathBuf>,
}

impl PreparedRootIndex {
    fn new(roots: &[PathBuf], canonical_roots: &[PathBuf]) -> Self {
        Self {
            roots: roots
                .iter()
                .zip(canonical_roots)
                .flat_map(|(root, canonical_root)| [root.clone(), canonical_root.clone()])
                .collect(),
        }
    }
}

fn path_candidates(path: &Path, root_index: &PreparedRootIndex) -> Vec<PathBuf> {
    let absolute = absolute_path(path);
    let canonical = canonicalize_or_original(&absolute);
    let mut candidates = vec![path.to_path_buf()];
    for representation in [absolute, canonical] {
        if !candidates
            .iter()
            .any(|candidate| candidate.as_os_str() == representation.as_os_str())
        {
            candidates.push(representation);
        }
    }
    append_relative_candidates(&mut candidates, root_index);
    candidates
}

fn append_relative_candidates(candidate_paths: &mut Vec<PathBuf>, root_index: &PreparedRootIndex) {
    if root_index.roots.is_empty() {
        return;
    }
    let paths = candidate_paths.clone();
    for path in &paths {
        let prefixes = path.ancestors().chain(std::iter::once(Path::new("")));
        #[cfg(windows)]
        let prefixes = prefixes.chain(path.components().next().and_then(|component| {
            if let std::path::Component::Prefix(prefix) = component {
                Some(Path::new(prefix.as_os_str()))
            } else {
                None
            }
        }));
        for prefix in prefixes {
            #[cfg(test)]
            count_work(3);
            if let Some(root) = root_index.roots.get(prefix) {
                if let Ok(relative) = path.strip_prefix(root) {
                    candidate_paths.push(relative.to_path_buf());
                }
            }
        }
    }
}

fn normalized_candidates(paths: &[PathBuf]) -> BTreeSet<String> {
    let mut candidates = BTreeSet::new();
    let mut seen = HashSet::new();
    for path in paths {
        let raw = path.to_string_lossy();
        if !seen.insert(raw.clone()) {
            continue;
        }
        let normalized = normalize(&raw);
        if !normalized.is_empty() {
            if let Some(basename) = normalized
                .rsplit('/')
                .find(|component| !component.is_empty())
            {
                candidates.insert(basename.to_string());
            }
            candidates.insert(normalized);
        }
    }
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::destination::{Destination, DestinationId, DestinationKind};
    use crate::policy::{ShellBehavior, SubagentPolicy};

    fn owner_oracle(batch: &mut ClassificationBatch, paths: &[PathBuf], roots: &[PathBuf]) {
        let absolute_roots = roots
            .iter()
            .map(|root| absolute_path(root))
            .collect::<Vec<_>>();
        let indices = original_indices(&batch.compiled, paths, &absolute_roots);
        let expected = {
            let zones = indices
                .iter()
                .map(|index| {
                    let zone = &batch.compiled.zones[*index].zone;
                    (batch.compiled.zone_index_named(&zone.name).unwrap(), zone)
                })
                .collect::<Vec<_>>();
            effective_zone(&zones).unwrap_or_else(|| {
                batch.compiled.zones[batch.compiled.normal_index]
                    .zone
                    .clone()
            })
        };
        assert_eq!(
            batch.strictest_zone_for_paths_with_roots(paths, roots),
            expected
        );
    }

    #[test]
    fn owner_memo_reuses_strings_across_calls_and_keeps_policy_namespace() {
        let temp = tempfile::tempdir().unwrap();
        let roots = vec![temp.path().to_path_buf()];
        let paths = [
            "late.txt",
            "other.txt",
            "missing.txt",
            "cafe\u{301}.txt",
            "",
        ]
        .map(|name| temp.path().join(name));
        for normal_patterns in [vec!["never"], vec!["**"]] {
            for (entries, bytes) in [(0, 0), (1, 1), (3, 32), (4096, 1024 * 1024)] {
                for reversed in [false, true] {
                    let mut zones = vec![
                        zone("duplicate", &["early.txt"], &["a", "b"]),
                        zone("normal", &normal_patterns, &["*"]),
                        zone_with_shell_behavior(
                            "other",
                            &["other.txt"],
                            &["b"],
                            ShellBehavior::Deny,
                        ),
                        zone_with_shell_behavior(
                            "duplicate",
                            &["late.txt", "café.txt"],
                            &["b"],
                            ShellBehavior::Deny,
                        ),
                    ];
                    if reversed {
                        zones.reverse();
                    }
                    let mut batch = ClassificationBatch::new(policy(zones).compile().unwrap());
                    batch.memo = LexicalMemo::new(entries, bytes);
                    for round in 0..8 {
                        for len in 0..=paths.len() {
                            owner_oracle(&mut batch, &paths[..len], &roots);
                        }
                        let unique = vec![temp.path().join(format!("unique-{round}.txt"))];
                        owner_oracle(&mut batch, &unique, &roots);
                        let count =
                            batch.memo.raw_matches.len() + batch.memo.normalized_matches.len();
                        let key_bytes = batch
                            .memo
                            .raw_matches
                            .keys()
                            .chain(batch.memo.normalized_matches.keys())
                            .map(String::len)
                            .sum::<usize>();
                        assert_eq!(
                            count + batch.memo.budget.entries,
                            batch.memo.capacity.entries
                        );
                        assert_eq!(
                            key_bytes + batch.memo.budget.bytes,
                            batch.memo.capacity.bytes
                        );
                    }
                }
            }
        }
        // Warm hits remain useful even after insertion is disabled.
        let mut batch = ClassificationBatch::new(
            policy(vec![zone("late", &["late.txt"], &[])])
                .compile()
                .unwrap(),
        );
        owner_oracle(&mut batch, &paths, &roots);
        batch.memo.budget = MemoBudget {
            entries: 0,
            bytes: 0,
        };
        MATCH_WORK.with(|counts| counts.set(Some([0; 4])));
        batch.strictest_zone_for_paths_with_roots(&paths, &roots);
        let work = MATCH_WORK.with(|counts| counts.replace(None).unwrap());
        assert_eq!(
            work[0],
            roots.len() + paths.len(),
            "fresh root AND each path canonicalization"
        );
        assert_eq!(work[1], 0, "raw memo hits skip normalization");
        assert_eq!(
            work[2], 0,
            "warm hits skip pattern probes despite full budget"
        );
        let mut different = ClassificationBatch::new(
            policy(vec![zone("different", &["late.txt"], &["x"])])
                .compile()
                .unwrap(),
        );
        assert_eq!(
            different
                .strictest_zone_for_paths_with_roots(&paths[..1], &roots)
                .name,
            "different"
        );
    }

    #[test]
    fn owner_memo_recovers_on_call_after_admission_failure() {
        let temp = tempfile::tempdir().unwrap();
        let compiled = policy(vec![
            zone("first", &["never-first"], &[]),
            zone("second", &["never-second"], &[]),
            zone("normal", &["never-normal"], &["*"]),
        ])
        .compile()
        .unwrap();
        let mut batch = ClassificationBatch::new(compiled);
        batch.memo = LexicalMemo::new(3, 1024 * 1024);
        let mut work = Vec::new();
        for (name, repeats) in [("owner-a", 1), ("owner-b", 50), ("owner-c", 50)] {
            let paths = vec![temp.path().join(name); repeats];
            let expected = batch.compiled.strictest_zone_for_paths(&paths);
            MATCH_WORK.with(|counts| counts.set(Some([0; 4])));
            let actual = batch.strictest_zone_for_paths_with_roots(&paths, [] as [&Path; 0]);
            let counts = MATCH_WORK.with(|counts| counts.replace(None).unwrap());
            assert_eq!(actual, expected);
            assert_eq!(counts[0], repeats, "every path is freshly canonicalized");
            work.push(counts);
        }
        assert_eq!(work[0][2], 6);
        assert_eq!(
            work[1][2], 300,
            "no mid-call reset rescues first saturation"
        );
        assert_eq!(work[2][2], 6, "next record recovers bounded lexical reuse");
        assert_eq!(work[2][1], 1);
    }

    #[test]
    fn owner_exact_full_warm_hits_do_not_reset() {
        let temp = tempfile::tempdir().unwrap();
        let paths = [temp.path().join("missing")];
        let mut batch = ClassificationBatch::new(
            policy(vec![zone("normal", &["never"], &["*"])])
                .compile()
                .unwrap(),
        );
        let raw_len = paths[0].to_string_lossy().len();
        let key_bytes = "missing".len() + raw_len * 2;
        batch.memo = LexicalMemo::new(3, key_bytes);
        owner_oracle(&mut batch, &paths, &[]);
        assert_eq!((batch.memo.budget.entries, batch.memo.budget.bytes), (0, 0));
        for _ in 0..5 {
            assert!(!batch.memo.admission_failed);
            MATCH_WORK.with(|counts| counts.set(Some([0; 4])));
            let result = batch.strictest_zone_for_paths_with_roots(&paths, [] as [&Path; 0]);
            let work = MATCH_WORK.with(|counts| counts.replace(None).unwrap());
            assert_eq!(result.name, "normal");
            assert_eq!(work, [1, 0, 0, 0]);
            assert_eq!(batch.memo.raw_matches.len(), 1);
            assert_eq!(batch.memo.normalized_matches.len(), 2);
            assert_eq!((batch.memo.budget.entries, batch.memo.budget.bytes), (0, 0));
        }
    }

    #[test]
    fn owner_rejected_oversized_zero_and_tiny_budgets_reset_exactly() {
        let temp = tempfile::tempdir().unwrap();
        let paths = [temp.path().join("missing-long-key")];
        for (entries, bytes) in [(0, 0), (0, 1024), (3, 0), (1, 1), (3, 16)] {
            let mut batch = ClassificationBatch::new(
                policy(vec![zone("normal", &["never"], &["*"])])
                    .compile()
                    .unwrap(),
            );
            batch.memo = LexicalMemo::new(entries, bytes);
            for _ in 0..3 {
                owner_oracle(&mut batch, &paths, &[]);
                assert!(batch.memo.admission_failed);
                let count = batch.memo.raw_matches.len() + batch.memo.normalized_matches.len();
                let key_bytes = batch
                    .memo
                    .raw_matches
                    .keys()
                    .chain(batch.memo.normalized_matches.keys())
                    .map(String::len)
                    .sum::<usize>();
                assert_eq!(count + batch.memo.budget.entries, entries);
                assert_eq!(key_bytes + batch.memo.budget.bytes, bytes);
                owner_oracle(&mut batch, &[], &[]);
                assert!(!batch.memo.admission_failed);
                assert!(batch.memo.raw_matches.is_empty());
                assert!(batch.memo.normalized_matches.is_empty());
                assert_eq!(
                    (batch.memo.budget.entries, batch.memo.budget.bytes),
                    (entries, bytes)
                );
            }
        }
    }

    #[test]
    fn owner_single_alias_matches_existing_fast_path_with_duplicate_permissions() {
        let temp = tempfile::tempdir().unwrap();
        let compiled = policy(vec![
            zone("duplicate", &["early.txt"], &["a", "b"]),
            zone_with_shell_behavior("duplicate", &["late.txt"], &["b"], ShellBehavior::Deny),
            zone("normal", &["never"], &["*"]),
        ])
        .compile()
        .unwrap();
        let mut batch = ClassificationBatch::new(compiled);
        for (entries, bytes) in [(0, 0), (1, 1), (4096, 1024 * 1024)] {
            batch.memo = LexicalMemo::new(entries, bytes);
            for name in ["late.txt", "early.txt", "absent.txt", "late.txt"] {
                let paths = [temp.path().join(name)];
                let expected = batch
                    .compiled
                    .strictest_zone_for_paths_with_roots(&paths, [temp.path()]);
                assert_eq!(
                    batch.strictest_zone_for_paths_with_roots(&paths, [temp.path()]),
                    expected
                );
                if name == "late.txt" {
                    assert_eq!(expected.send_to, ["b"]);
                    assert_eq!(expected.on_shell_read, ShellBehavior::Deny);
                }
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn owner_memo_reobserves_file_and_root_symlinks_a_b_a_and_lazy_aliases() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        for dir in [&first, &second] {
            std::fs::create_dir(dir).unwrap();
            std::fs::create_dir(dir.join("private")).unwrap();
            std::fs::write(dir.join("private/x.txt"), "fixture").unwrap();
        }
        let alias = temp.path().join("alias");
        let root_alias = temp.path().join("root_alias");
        symlink(first.join("private/x.txt"), &alias).unwrap();
        symlink(&first, &root_alias).unwrap();
        let mut files = ClassificationBatch::new(
            policy(vec![
                zone("normal", &["never"], &["*"]),
                zone("first", &["first/private/*"], &["a"]),
                zone("second", &["second/private/*"], &["b"]),
            ])
            .compile()
            .unwrap(),
        );
        let mut roots = ClassificationBatch::new(
            policy(vec![
                zone("normal", &["never"], &["*"]),
                zone("private", &["private/*"], &[]),
            ])
            .compile()
            .unwrap(),
        );
        for (target, name, root_name) in [
            (&first, "first", "private"),
            (&second, "second", "normal"),
            (&first, "first", "private"),
        ] {
            std::fs::remove_file(&alias).unwrap();
            std::fs::remove_file(&root_alias).unwrap();
            symlink(target.join("private/x.txt"), &alias).unwrap();
            symlink(target, &root_alias).unwrap();
            owner_oracle(&mut files, &[alias.clone()], &[temp.path().to_path_buf()]);
            assert_eq!(
                files
                    .strictest_zone_for_paths_with_roots([&alias], [temp.path()])
                    .name,
                name
            );
            // Lexical alias itself has no match; canonical representation does.
            assert_eq!(
                files.memo.raw_matches.get(alias.to_string_lossy().as_ref()),
                Some(&files.compiled.zones.len())
            );
            let physical = first.join("private/x.txt");
            owner_oracle(&mut roots, &[physical.clone()], &[root_alias.clone()]);
            assert_eq!(
                roots
                    .strictest_zone_for_paths_with_roots([&physical], [&root_alias])
                    .name,
                root_name
            );
        }
        let lazy = (0..2).map(|index| {
            if index == 1 {
                std::fs::remove_file(&alias).unwrap();
                symlink(second.join("private/x.txt"), &alias).unwrap();
            }
            alias.clone()
        });
        let result = files.strictest_zone_for_paths_with_roots(lazy, [temp.path()]);
        assert_eq!(result.name, "effective:first+second");
        assert!(result.send_to.is_empty());
    }

    fn original_indices(
        compiled: &CompiledPolicy,
        paths: &[PathBuf],
        roots: &[PathBuf],
    ) -> Vec<usize> {
        let canonical = canonical_roots_of(roots);
        paths
            .iter()
            .map(|path| {
                let base = [
                    path.clone(),
                    absolute_path(path),
                    canonicalize_or_original(&absolute_path(path)),
                ];
                let mut candidates = BTreeSet::new();
                for candidate in &base {
                    let mut values = vec![candidate.clone()];
                    for (root, canonical) in roots.iter().zip(&canonical) {
                        for prefix in [root, canonical] {
                            if let Ok(relative) = candidate.strip_prefix(prefix) {
                                values.push(relative.to_path_buf());
                            }
                        }
                    }
                    for value in values {
                        let value = value
                            .to_string_lossy()
                            .replace('\\', "/")
                            .nfc()
                            .collect::<String>()
                            .to_lowercase();
                        if !value.is_empty() {
                            if let Some(basename) = value.rsplit('/').find(|part| !part.is_empty())
                            {
                                candidates.insert(basename.to_string());
                            }
                            candidates.insert(value);
                        }
                    }
                }
                compiled
                    .zones
                    .iter()
                    .position(|zone| {
                        zone.patterns.iter().any(|pattern| {
                            candidates
                                .iter()
                                .any(|candidate| pattern.pattern.matches(candidate))
                        })
                    })
                    .unwrap_or(compiled.normal_index)
            })
            .collect()
    }

    fn assert_batch_oracle(compiled: &CompiledPolicy, paths: &[PathBuf], roots: &[PathBuf]) {
        let roots = roots
            .iter()
            .map(|root| absolute_path(root))
            .collect::<Vec<_>>();
        let expected = original_indices(compiled, paths, &roots);
        let index = PreparedRootIndex::new(&roots, &canonical_roots_of(&roots));
        assert_eq!(
            compiled.zone_indices_for_paths(paths, &index),
            expected,
            "paths={paths:?}"
        );
        for (entries, bytes) in [(0, 0), (1, 1), (3, 32), (4096, 1024 * 1024)] {
            let actual = compiled.zone_indices_with_budget(paths, &index, entries, bytes);
            assert_eq!(actual, expected, "budget={entries}/{bytes}");
        }
        let zones = expected
            .iter()
            .map(|index| {
                let zone = &compiled.zones[*index].zone;
                (compiled.zone_index_named(&zone.name).unwrap(), zone)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            compiled.strictest_zone_for_paths_with_roots(paths, &roots),
            effective_zone(&zones)
                .unwrap_or_else(|| compiled.zones[compiled.normal_index].zone.clone())
        );
    }

    #[test]
    fn shared_candidates_preserve_each_alias_first_zone_and_fallback() {
        let temp = tempfile::tempdir().unwrap();
        let roots = vec![temp.path().to_path_buf(), temp.path().join("nested")];
        let mut paths = vec![
            temp.path().join("first/x.txt"),
            temp.path().join("second/x.txt"),
            temp.path().join("nested/second/x.txt"),
            temp.path().join("missing/./cafe\u{301}.txt"),
            PathBuf::from("second/x.txt"),
            PathBuf::from(""),
            PathBuf::from(r"second\x.txt"),
        ];
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            paths.push(PathBuf::from(std::ffi::OsString::from_vec(vec![
                b'x', 0xff,
            ])));
            paths.push(PathBuf::from("x\u{fffd}"));
        }
        for normal_patterns in [vec!["never-matches"], vec!["first/*"], vec!["**"]] {
            let compiled = policy(vec![
                zone_with_shell_behavior("first", &["first/*"], &["a", "b"], ShellBehavior::Ask),
                zone("normal", &normal_patterns, &["*"]),
                zone_with_shell_behavior(
                    "second",
                    &["second/*", "café.txt"],
                    &["b", "c"],
                    ShellBehavior::Deny,
                ),
                zone("duplicate", &["x.txt"], &["a"]),
                zone("duplicate", &["never"], &[]),
            ])
            .compile()
            .unwrap();
            for len in 0..=paths.len() {
                assert_batch_oracle(&compiled, &paths[..len], &roots);
            }
            paths.reverse();
            let mut reversed_roots = roots.clone();
            reversed_roots.reverse();
            assert_batch_oracle(&compiled, &paths, &reversed_roots);
        }
        let compiled = policy(vec![
            zone_with_shell_behavior("first", &["first/*"], &["a", "b"], ShellBehavior::Ask),
            zone_with_shell_behavior("second", &["second/*"], &["b", "c"], ShellBehavior::Deny),
        ])
        .compile()
        .unwrap();
        let result = compiled.strictest_zone_for_paths_with_roots(
            [
                temp.path().join("first/x.txt"),
                temp.path().join("second/x.txt"),
            ],
            &roots,
        );
        assert_eq!(result.name, "effective:first+second");
        assert_eq!(result.send_to, ["b"]);
        assert_eq!(result.on_shell_read, ShellBehavior::Deny);
    }

    #[cfg(unix)]
    #[test]
    fn shared_candidates_reobserve_symlinks_and_keep_hardlink_aliases() {
        let temp = tempfile::tempdir().unwrap();
        for name in ["first", "second"] {
            std::fs::create_dir(temp.path().join(name)).unwrap();
            std::fs::write(temp.path().join(name).join("x.txt"), name).unwrap();
        }
        let alias = temp.path().join("alias");
        let root_alias = temp.path().join("root_alias");
        std::os::unix::fs::symlink(temp.path().join("first/x.txt"), &alias).unwrap();
        std::os::unix::fs::symlink(temp.path().join("first"), &root_alias).unwrap();
        let hardlink = temp.path().join("hardlink");
        std::fs::hard_link(temp.path().join("second/x.txt"), &hardlink).unwrap();
        let compiled = policy(vec![
            zone("first", &["first/*"], &["a"]),
            zone("second", &["second/*"], &["b"]),
        ])
        .compile()
        .unwrap();
        let paths = vec![
            alias.clone(),
            alias.clone(),
            hardlink,
            temp.path().join("second/x.txt"),
        ];
        let roots = vec![temp.path().to_path_buf(), root_alias.clone()];
        assert_batch_oracle(&compiled, &paths, &roots);
        std::fs::remove_file(&alias).unwrap();
        std::fs::remove_file(&root_alias).unwrap();
        std::os::unix::fs::symlink(temp.path().join("second/x.txt"), &alias).unwrap();
        std::os::unix::fs::symlink(temp.path().join("second"), &root_alias).unwrap();
        assert_batch_oracle(&compiled, &paths, &roots);
        assert_eq!(
            compiled
                .strictest_zone_for_paths_with_roots(&paths, &roots)
                .name,
            "second"
        );
    }

    #[cfg(unix)]
    #[test]
    fn lazy_alias_iterator_retargets_between_fresh_observations() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first.txt");
        let second = temp.path().join("second.txt");
        let alias = temp.path().join("alias");
        std::fs::write(&first, "first").unwrap();
        std::fs::write(&second, "second").unwrap();
        std::os::unix::fs::symlink(&first, &alias).unwrap();
        let compiled = policy(vec![
            zone("first", &["first.txt"], &["a"]),
            zone("second", &["second.txt"], &["b"]),
        ])
        .compile()
        .unwrap();
        let paths = (0..2).map(|index| {
            if index == 1 {
                std::fs::remove_file(&alias).unwrap();
                std::os::unix::fs::symlink(&second, &alias).unwrap();
            }
            alias.clone()
        });
        let effective = compiled.strictest_zone_for_paths_with_roots(paths, [temp.path()]);
        assert_eq!(effective.name, "effective:first+second");
        assert!(effective.send_to.is_empty());
    }

    #[test]
    fn later_duplicate_name_keeps_matched_zone_and_first_name_order() {
        let compiled = policy(vec![
            zone_with_shell_behavior(
                "duplicate",
                &["early.txt"],
                &["a", "b"],
                ShellBehavior::Withhold,
            ),
            zone_with_shell_behavior("other", &["other.txt"], &["b"], ShellBehavior::Deny),
            zone_with_shell_behavior("duplicate", &["late.txt"], &["b"], ShellBehavior::Deny),
        ])
        .compile()
        .unwrap();
        let paths = vec![PathBuf::from("late.txt"), PathBuf::from("other.txt")];
        assert_batch_oracle(&compiled, &paths, &[]);
        let actual = compiled.strictest_zone_for_paths(&paths);
        assert_eq!(
            actual,
            Zone {
                name: "duplicate".into(),
                patterns: Vec::new(),
                send_to: vec!["b".into()],
                on_shell_read: ShellBehavior::Deny,
            }
        );
        assert_eq!(compiled.zone_for_path(Path::new("late.txt")).send_to, ["b"]);
        assert_eq!(
            compiled.zone_named("duplicate").unwrap().send_to,
            ["a", "b"]
        );
    }

    #[test]
    fn memo_budget_caps_entries_and_key_bytes_without_saturation() {
        let mut budget = MemoBudget {
            entries: 2,
            bytes: 3,
        };
        assert!(!budget.reserve(4));
        assert!(budget.reserve(2));
        assert!(!budget.reserve(2));
        assert!(budget.reserve(1));
        assert!(!budget.reserve(0));
        assert_eq!((budget.entries, budget.bytes), (0, 0));
    }

    #[test]
    fn thousands_of_missing_aliases_match_independent_oracle() {
        let temp = tempfile::tempdir().unwrap();
        let paths = (0..3000)
            .map(|index| temp.path().join(format!("alias-{index}/second/x.txt")))
            .collect::<Vec<_>>();
        let compiled = policy(vec![
            zone("first", &["alias-1/*"], &["a"]),
            zone("second", &["*/second/*"], &["b"]),
        ])
        .compile()
        .unwrap();
        assert_batch_oracle(&compiled, &paths, &[temp.path().to_path_buf()]);
    }

    #[test]
    fn literal_prefix_matches_glob_oracle() {
        let prefixes = [
            "",
            "/repo/",
            "c:/repo/",
            "café/",
            "cafe\u{301}/",
            "δ/",
            "a]",
            "a!",
            "a/./",
        ];
        let tails = [
            "", "*", "?", "[ab]", "[!a]", "[[]", "[]]", "[?]", "[*]", "**/x", "literal",
        ];
        let mut candidates = vec![String::new(), "a".into(), "café/x".into(), "δ/x".into()];
        for prefix in prefixes {
            for suffix in ["", "a", "b", "x", "?", "*", "[", "]", "nested/x", "literal"] {
                candidates.push(format!("{prefix}{suffix}"));
            }
        }
        for prefix in prefixes {
            for tail in tails {
                let source = format!("{prefix}{tail}");
                let Ok(pattern) = Pattern::new(&source) else {
                    continue;
                };
                let fast = PrefixedPattern::new(pattern);
                for candidate in &candidates {
                    assert_eq!(
                        fast.matches(candidate),
                        fast.pattern.matches(candidate),
                        "pattern={source:?} candidate={candidate:?}"
                    );
                }
            }
        }
    }

    #[test]
    #[ignore]
    fn alias_pipeline_benchmark() {
        use std::{hint::black_box, time::Instant};
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        std::fs::create_dir(&source).unwrap();
        let baseline = |compiled: &CompiledPolicy, paths: &[PathBuf], roots: &[PathBuf]| {
            let roots = roots
                .iter()
                .map(|root| absolute_path(root))
                .collect::<Vec<_>>();
            let canonical = canonical_roots_of(&roots);
            let root_index = PreparedRootIndex::new(&roots, &canonical);
            let zones = paths
                .iter()
                .map(|path| {
                    let mut candidates = vec![
                        path.clone(),
                        absolute_path(path),
                        canonicalize_or_original(&absolute_path(path)),
                    ];
                    append_relative_candidates(&mut candidates, &root_index);
                    let candidates = normalized_candidates(&candidates);
                    let index = compiled
                        .zones
                        .iter()
                        .position(|zone| {
                            zone.patterns.iter().any(|pattern| {
                                candidates
                                    .iter()
                                    .any(|candidate| pattern.matches(candidate))
                            })
                        })
                        .unwrap_or(compiled.normal_index);
                    let zone = &compiled.zones[index].zone;
                    (compiled.zone_index_named(&zone.name).unwrap(), zone)
                })
                .collect::<Vec<_>>();
            effective_zone(&zones)
                .unwrap_or_else(|| compiled.zones[compiled.normal_index].zone.clone())
        };
        for mappings in [0, 50] {
            let mut roots = vec![source.clone()];
            for mapping in 0..mappings {
                roots.push(temp.path().join(format!("worktree-{mapping}")));
            }
            let mut paths = roots
                .iter()
                .map(|root| root.join("src/public.rs"))
                .collect::<Vec<_>>();
            paths.push(PathBuf::from("src/public.rs"));
            if mappings != 0 {
                paths.extend(roots.iter().map(|root| root.join("../relative/public.rs")));
            }
            for case in ["early", "late", "none"] {
                let mut zones = (0..20)
                    .map(|index| {
                        zone(
                            &format!("zone-{index}"),
                            &[&format!("private-{index}/**")],
                            &["provider"],
                        )
                    })
                    .collect::<Vec<_>>();
                match case {
                    "early" => zones[0].patterns = vec!["public.rs".into()],
                    "late" => zones[19].patterns = vec!["public.rs".into()],
                    _ => (),
                }
                zones.push(zone("normal", &["**"], &["*"]));
                let compiled = policy(zones).compile().unwrap();
                assert_eq!(
                    baseline(&compiled, &paths, &roots),
                    compiled.strictest_zone_for_paths_with_roots(&paths, &roots)
                );
                for optimized in [false, true] {
                    MATCH_WORK.with(|counts| counts.set(Some([0; 4])));
                    if optimized {
                        black_box(compiled.strictest_zone_for_paths_with_roots(&paths, &roots));
                    } else {
                        black_box(baseline(&compiled, &paths, &roots));
                    }
                    let counts = MATCH_WORK.with(|counts| counts.replace(None).unwrap());
                    eprintln!("alias_work mappings={mappings} case={case} optimized={optimized} canonical={} normalize={} prefix_compare={} ancestor_visit={}", counts[0], counts[1], counts[2], counts[3]);
                }
                for records in [100, 5000] {
                    for round in 0..3 {
                        for optimized in if round % 2 == 0 {
                            [false, true]
                        } else {
                            [true, false]
                        } {
                            let start = Instant::now();
                            for _ in 0..records {
                                black_box(if optimized {
                                    compiled.strictest_zone_for_paths_with_roots(
                                        black_box(&paths),
                                        black_box(&roots),
                                    )
                                } else {
                                    baseline(&compiled, black_box(&paths), black_box(&roots))
                                });
                            }
                            eprintln!("alias_pipeline mappings={mappings} aliases={} case={case} records={records} round={round} optimized={optimized} elapsed_us={}", paths.len(), start.elapsed().as_micros());
                        }
                    }
                }
            }
        }
    }

    #[test]
    #[ignore]
    fn literal_prefix_microbench() {
        use std::{hint::black_box, time::Instant};
        let absolute: Vec<_> = (0..20)
            .map(|i| format!("/workspace/private-{i}/**"))
            .collect();
        let wildcard: Vec<_> = (0..20).map(|i| format!("*private-{i}*")).collect();
        for (name, sources) in [("absolute", absolute), ("wildcard", wildcard)] {
            let patterns: Vec<_> = sources
                .iter()
                .map(|s| PrefixedPattern::new(Pattern::new(s).unwrap()))
                .collect();
            let candidates = [
                "/workspace/public/src/main.rs",
                "src/main.rs",
                "main.rs",
                "/workspace/private-4/key",
                "café/main.rs",
            ];
            for round in 0..4 {
                for fast in if round % 2 == 0 {
                    [false, true]
                } else {
                    [true, false]
                } {
                    let start = Instant::now();
                    let mut hits = 0usize;
                    for _ in 0..20_000 {
                        for p in &patterns {
                            for candidate in candidates {
                                hits += usize::from(if fast {
                                    black_box(p).matches(black_box(candidate))
                                } else {
                                    black_box(&p.pattern).matches(black_box(candidate))
                                });
                            }
                        }
                    }
                    eprintln!("prefix_microbench case={name} round={round} fast={fast} elapsed_us={} hits={}",
                        start.elapsed().as_micros(), black_box(hits));
                }
            }
        }
    }

    fn zone_with_shell_behavior(
        name: &str,
        patterns: &[&str],
        send_to: &[&str],
        on_shell_read: ShellBehavior,
    ) -> Zone {
        Zone {
            name: name.to_string(),
            patterns: patterns.iter().map(|value| (*value).to_string()).collect(),
            send_to: send_to.iter().map(|value| (*value).to_string()).collect(),
            on_shell_read,
        }
    }

    fn zone(name: &str, patterns: &[&str], send_to: &[&str]) -> Zone {
        zone_with_shell_behavior(name, patterns, send_to, ShellBehavior::Withhold)
    }

    fn policy(zones: Vec<Zone>) -> PrivacyPolicy {
        PrivacyPolicy {
            blocked: Vec::new(),
            zones,
            subagents: SubagentPolicy::default(),
            ..Default::default()
        }
    }

    #[test]
    fn normalized_candidates_match_reference_dedup_algorithm() {
        let paths = vec![
            PathBuf::from("A//./cafe\u{301}.txt"),
            PathBuf::from("a//./cafe\u{301}.txt"),
            PathBuf::from("café.txt"),
            PathBuf::from("A//./cafe\u{301}.txt"),
            PathBuf::from(r"C:\\Work\\..\\Secret.TXT"),
        ];
        let actual = normalized_candidates(&paths);
        let mut expected = BTreeSet::new();
        for path in &paths {
            let normalized = normalize(&path.to_string_lossy());
            if !normalized.is_empty() {
                if let Some(base) = normalized.rsplit('/').find(|part| !part.is_empty()) {
                    expected.insert(base.to_string());
                }
                expected.insert(normalized);
            }
        }
        assert_eq!(actual, expected);
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            let non_utf8 = PathBuf::from(std::ffi::OsString::from_vec(vec![b'x', 0xff]));
            let replacement = non_utf8.to_string_lossy().into_owned();
            assert_eq!(
                normalized_candidates(&[non_utf8]),
                BTreeSet::from([normalize(&replacement)])
            );
        }
    }

    #[test]
    fn ordered_zones_use_the_first_match() {
        let policy = policy(vec![
            zone("first", &["src/*.rs"], &[]),
            zone("second", &["src/main.*"], &["provider-b"]),
            zone("normal", &["*"], &["*"]),
        ]);

        let compiled = policy.compile().expect("policy should compile");

        assert_eq!(
            compiled.zone_for_path(Path::new("src/main.rs")).name,
            "first"
        );
        assert_eq!(
            compiled.zone_for_path(Path::new("README.md")).name,
            "normal"
        );
    }

    #[test]
    fn reserved_effective_zone_name_syntax_is_rejected() {
        for name in ["effective:custom", "internal+shared"] {
            let policy = policy(vec![zone(name, &["guarded.txt"], &["trusted"])]);
            assert!(matches!(
                policy.compile(),
                Err(PolicyError::InvalidZoneName { .. })
            ));
        }
    }

    #[test]
    fn strictest_zone_uses_the_smallest_destination_set() {
        let policy = policy(vec![
            zone("restricted", &["restricted/*"], &["provider-a"]),
            zone("secrets", &["secrets/*"], &[]),
            zone("normal", &["*"], &["*"]),
        ]);
        let compiled = policy.compile().expect("policy should compile");

        assert_eq!(
            compiled
                .strictest_zone_for_paths([
                    Path::new("normal/file.rs"),
                    Path::new("restricted/file.rs"),
                    Path::new("secrets/file.rs"),
                ])
                .name,
            "secrets"
        );
    }

    #[test]
    fn wildcard_destination_set_is_the_weakest() {
        let policy = policy(vec![
            zone("wildcard", &["wildcard/*"], &["*", "provider-a"]),
            zone(
                "restricted",
                &["restricted/*"],
                &["provider-a", "provider-b"],
            ),
            zone("normal", &["*"], &["*"]),
        ]);
        let compiled = policy.compile().expect("policy should compile");

        assert_eq!(
            compiled
                .strictest_zone_for_paths([
                    Path::new("wildcard/file.rs"),
                    Path::new("restricted/file.rs"),
                ])
                .name,
            "restricted"
        );
    }

    #[test]
    fn identical_destination_sets_use_zone_order_in_both_path_orders() {
        let policy = policy(vec![
            zone("z-first", &["first.txt"], &["*"]),
            zone("a-second", &["second.txt"], &["*"]),
            zone("normal", &["*"], &["*"]),
        ]);
        let compiled = policy.compile().expect("policy should compile");

        for paths in [
            [Path::new("first.txt"), Path::new("second.txt")],
            [Path::new("second.txt"), Path::new("first.txt")],
        ] {
            assert_eq!(compiled.strictest_zone_for_paths(paths).name, "z-first");
        }
    }

    #[test]
    fn later_strict_subset_beats_zone_order() {
        let policy = policy(vec![
            zone("broad-first", &["broad.txt"], &["provider-a", "provider-b"]),
            zone("narrow-second", &["narrow.txt"], &["provider-a"]),
            zone("normal", &["*"], &["*"]),
        ]);
        let compiled = policy.compile().expect("policy should compile");

        for paths in [
            [Path::new("broad.txt"), Path::new("narrow.txt")],
            [Path::new("narrow.txt"), Path::new("broad.txt")],
        ] {
            let effective = compiled.strictest_zone_for_paths(paths);
            assert_eq!(effective.name, "narrow-second");
            assert_eq!(effective.send_to, ["provider-a"]);
        }
    }

    #[test]
    fn incomparable_destination_sets_intersect_in_both_orders() {
        let policy = policy(vec![
            zone("a", &["a.txt"], &["provider-a"]),
            zone("b", &["b.txt"], &["provider-b"]),
            zone("normal", &["*"], &["*"]),
        ]);
        let compiled = policy.compile().expect("policy should compile");

        for paths in [
            [Path::new("a.txt"), Path::new("b.txt")],
            [Path::new("b.txt"), Path::new("a.txt")],
        ] {
            let effective = compiled.strictest_zone_for_paths(paths);
            assert_eq!(
                effective,
                Zone {
                    name: "effective:a+b".to_string(),
                    patterns: Vec::new(),
                    send_to: Vec::new(),
                    on_shell_read: ShellBehavior::Withhold,
                }
            );
        }
    }

    #[test]
    fn strictest_zone_combines_shell_behavior() {
        let policy = policy(vec![
            zone_with_shell_behavior(
                "withhold",
                &["withhold.txt"],
                &["provider-a"],
                ShellBehavior::Withhold,
            ),
            zone_with_shell_behavior("ask", &["ask.txt"], &["provider-a"], ShellBehavior::Ask),
            zone_with_shell_behavior("deny", &["deny.txt"], &["provider-a"], ShellBehavior::Deny),
            zone("normal", &["*"], &["*"]),
        ]);
        let compiled = policy.compile().expect("policy should compile");

        assert_eq!(
            compiled
                .strictest_zone_for_paths([Path::new("withhold.txt"), Path::new("ask.txt")])
                .on_shell_read,
            ShellBehavior::Ask
        );
        assert_eq!(
            compiled
                .strictest_zone_for_paths([Path::new("ask.txt"), Path::new("deny.txt")])
                .on_shell_read,
            ShellBehavior::Deny
        );
    }

    #[test]
    fn zone_default_allows_no_destinations() {
        let zone = Zone::default();
        let destination = Destination {
            id: DestinationId("provider-a".to_string()),
            kind: DestinationKind::Provider,
            display_name: "Provider A".to_string(),
        };

        assert!(zone.send_to.is_empty());
        assert!(!destination.matches_send_to(&zone.send_to));
        assert!(destination.matches_send_to(&["*".to_string()]));
    }

    #[test]
    fn policy_defaults_are_restrictive_except_for_subagent_reports() {
        let policy: PrivacyPolicy = serde_yaml::from_str("{}").expect("policy should deserialize");

        assert!(policy.blocked.is_empty());
        assert!(policy.zones.is_empty());
        assert!(policy.subagents.report_declassifies);
        assert_eq!(Zone::default().on_shell_read, ShellBehavior::Withhold);
    }

    #[test]
    fn matching_is_case_insensitive() {
        let policy = policy(vec![
            zone("secrets", &[".env*"], &[]),
            zone("normal", &["*"], &["*"]),
        ]);

        let compiled = policy.compile().expect("policy should compile");

        assert_eq!(
            compiled.zone_for_path(Path::new(".ENV.local")).name,
            "secrets"
        );
    }

    #[test]
    fn matching_normalizes_unicode_to_nfc() {
        let policy = policy(vec![
            zone("accented", &["café.txt"], &[]),
            zone("normal", &["*"], &["*"]),
        ]);

        let compiled = policy.compile().expect("policy should compile");

        assert_eq!(
            compiled.zone_for_path(Path::new("cafe\u{301}.txt")).name,
            "accented"
        );
    }

    #[test]
    fn shipped_example_patterns_classify_absolute_paths() {
        let policy = policy(vec![
            zone("secrets", &["secrets.yaml", ".env*", "*.pem"], &[]),
            zone("normal", &["*"], &["*"]),
        ]);
        let compiled = policy.compile().expect("policy should compile");

        for path in ["/repo/.env", "/repo/secrets.yaml", "/repo/key.pem"] {
            assert_eq!(compiled.zone_for_path(Path::new(path)).name, "secrets");
        }
    }

    #[test]
    fn absolute_workspace_relative_and_basename_candidates_match() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let root = temp.path();
        let absolute = root.join("absolute.txt");
        let relative = root.join("src").join("relative.txt");
        let basename = root.join("nested").join("basename.txt");
        let absolute_pattern = absolute.to_string_lossy().into_owned();
        let policy = policy(vec![
            zone("absolute", &[&absolute_pattern], &[]),
            zone("relative", &["src/relative.txt"], &[]),
            zone("basename", &["basename.txt"], &[]),
            zone("normal", &["*"], &["*"]),
        ]);
        let compiled = policy.compile().expect("policy should compile");

        assert_eq!(
            compiled.zone_for_path_with_roots(&absolute, [root]).name,
            "absolute"
        );
        assert_eq!(
            compiled.zone_for_path_with_roots(&relative, [root]).name,
            "relative"
        );
        assert_eq!(
            compiled.zone_for_path_with_roots(&basename, [root]).name,
            "basename"
        );
    }

    #[test]
    fn windows_separators_match_slash_patterns() {
        let policy = policy(vec![
            zone("secrets", &["*/repo/.env"], &[]),
            zone("normal", &["*"], &["*"]),
        ]);
        let compiled = policy.compile().expect("policy should compile");

        assert_eq!(
            compiled.zone_for_path(Path::new(r"C:\repo\.env")).name,
            "secrets"
        );
    }

    #[test]
    fn malformed_glob_returns_policy_error() {
        let policy = policy(vec![
            zone("broken", &["["], &[]),
            zone("normal", &["*"], &["*"]),
        ]);

        assert!(matches!(
            policy.compile(),
            Err(PolicyError::InvalidGlob { pattern, .. }) if pattern == "["
        ));
    }

    #[test]
    fn compile_synthesizes_a_normal_fallback_zone() {
        let compiled = PrivacyPolicy::default()
            .compile()
            .expect("default policy should compile");
        let normal = compiled.zone_for_path(Path::new("/anything"));

        assert_eq!(normal.name, "normal");
        assert_eq!(normal.send_to, vec!["*"]);
    }

    #[cfg(unix)]
    #[cfg(unix)]
    #[test]
    fn engine_supplied_alias_candidate_wins_the_strict_zone() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let secret_dir = temp.path().join("secrets");
        std::fs::create_dir(&secret_dir).expect("secret dir should be created");
        let secret = secret_dir.join("secret.txt");
        let alias = temp.path().join("alias.txt");
        std::fs::write(&secret, "secret").expect("secret should be written");
        std::fs::hard_link(&secret, &alias).expect("hard link should be created");
        let policy = policy(vec![
            zone("secrets", &["secrets/*"], &[]),
            zone("normal", &["*"], &["*"]),
        ]);
        let compiled = policy.compile().expect("policy should compile");

        assert_eq!(
            compiled
                .zone_for_path_with_roots(&alias, [temp.path()])
                .name,
            "normal",
            "the matching library must not touch the filesystem on its own"
        );
        assert_eq!(
            compiled
                .strictest_zone_for_paths_with_roots([&alias, &secret], [temp.path()])
                .name,
            "secrets",
            "when the engine supplies the hard-link alias, the strict zone wins"
        );
    }

    #[cfg(unix)]
    #[cfg(unix)]
    #[test]
    fn symlink_into_secrets_is_classified_when_queried_first() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let secret_dir = temp.path().join("secrets");
        std::fs::create_dir(&secret_dir).expect("secret dir should be created");
        let secret = secret_dir.join("secret.txt");
        let alias = temp.path().join("alias.txt");
        std::fs::write(&secret, "secret").expect("secret should be written");
        std::os::unix::fs::symlink(&secret, &alias).expect("symlink should be created");
        let policy = policy(vec![
            zone("secrets", &["secrets/*"], &[]),
            zone("normal", &["*"], &["*"]),
        ]);
        let compiled = policy.compile().expect("policy should compile");

        assert_eq!(
            compiled
                .zone_for_path_with_roots(&alias, [temp.path()])
                .name,
            "secrets"
        );
    }

    #[test]
    fn root_candidates_match_the_old_nested_loop() {
        let atoms = [
            "",
            ".",
            "./",
            "a",
            "b",
            "a/./b",
            "a/../b",
            "a/a/a",
            "café",
            "cafe\u{301}",
            "Case",
            r"back\slash",
        ];
        let mut paths = vec![
            PathBuf::from("/tmp/a/./b/file"),
            PathBuf::from("a/a/a"),
            PathBuf::from("./Case/file"),
            PathBuf::from("café/file"),
            PathBuf::from(r"back\slash/file"),
        ];
        for a in atoms {
            for b in ["", "a", "./", "b", "a/../b"] {
                paths.push(PathBuf::from(format!("{a}/{b}/leaf")));
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            paths.push(PathBuf::from(std::ffi::OsString::from_vec(vec![
                b'x', 0xff,
            ])));
        }
        let roots = [
            PathBuf::from(""),
            PathBuf::from("."),
            PathBuf::from("./"),
            PathBuf::from("a/"),
            PathBuf::from("a//"),
            PathBuf::from("a/./b"),
            PathBuf::from("a/../b"),
            PathBuf::from("a/a/a"),
            PathBuf::from("/tmp/a"),
            PathBuf::from(r"C:\foo"),
            PathBuf::from(r"C:\foo\"),
            PathBuf::from(r"C:foo"),
            PathBuf::from(r"\foo"),
            PathBuf::from(r"\server\share"),
            PathBuf::from("café"),
            PathBuf::from("cafe\u{301}"),
            PathBuf::from("Case"),
        ];
        let canonical: Vec<_> = roots
            .iter()
            .map(|root| canonicalize_or_original(root))
            .collect();
        for path in &paths {
            let candidates = [
                path.clone(),
                absolute_path(path),
                canonicalize_or_original(&absolute_path(path)),
            ];
            let mut old = Vec::new();
            for candidate in &candidates {
                old.push(candidate.clone());
                for (root, canon) in roots.iter().zip(&canonical) {
                    for prefix in [root, canon] {
                        if let Ok(relative) = candidate.strip_prefix(prefix) {
                            old.push(relative.to_path_buf());
                        }
                    }
                }
            }
            let mut actual = candidates.to_vec();
            let index = PreparedRootIndex::new(&roots, &canonical);
            append_relative_candidates(&mut actual, &index);
            let raw = |values: &[PathBuf]| {
                values
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect::<BTreeSet<_>>()
            };
            assert_eq!(raw(&actual), raw(&old), "path={path:?}");
            assert_eq!(
                normalized_candidates(&actual),
                normalized_candidates(&old),
                "path={path:?}"
            );
        }
        let unequal_canonical = PreparedRootIndex::new(&roots, &canonical[..canonical.len() - 1]);
        let path = PathBuf::from("a/a/a/file");
        let mut actual = vec![path.clone()];
        append_relative_candidates(&mut actual, &unequal_canonical);
        let mut old = vec![path.clone()];
        for (root, canon) in roots.iter().zip(&canonical[..canonical.len() - 1]) {
            for prefix in [root, canon] {
                if let Ok(relative) = path.strip_prefix(prefix) {
                    old.push(relative.to_path_buf());
                }
            }
        }
        assert_eq!(
            actual
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect::<BTreeSet<_>>(),
            old.iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect::<BTreeSet<_>>()
        );
    }

    #[test]
    fn prepared_root_policy_matches_fresh_reference_classification() {
        let compiled = policy(vec![
            zone("private", &["secret/*", "*/secret.txt"], &[]),
            zone("other", &["other/*"], &["provider"]),
            zone("normal", &["**"], &["*"]),
        ])
        .compile()
        .unwrap();
        let roots = [
            PathBuf::from("."),
            PathBuf::from("./"),
            PathBuf::from("secret/.."),
            PathBuf::from("/tmp"),
        ];
        let canonical: Vec<_> = roots
            .iter()
            .map(|root| canonicalize_or_original(root))
            .collect();
        let index = PreparedRootIndex::new(&roots, &canonical);
        let paths = [
            PathBuf::from("secret/secret.txt"),
            PathBuf::from("other/file"),
            PathBuf::from("unmatched"),
            PathBuf::from("/tmp/secret/secret.txt"),
        ];
        let mut actual_zones = Vec::new();
        let mut reference_zones = Vec::new();
        for path in paths {
            let actual = compiled.zone_for_path_prepared(&path, &index);
            let base_candidates = [
                path.clone(),
                absolute_path(&path),
                canonicalize_or_original(&absolute_path(&path)),
            ];
            let mut reference_candidates = base_candidates.to_vec();
            for candidate in &base_candidates {
                for (root, canon) in roots.iter().zip(&canonical) {
                    for prefix in [root, canon] {
                        if let Ok(relative) = candidate.strip_prefix(prefix) {
                            reference_candidates.push(relative.to_path_buf());
                        }
                    }
                }
            }
            let reference_candidates = normalized_candidates(&reference_candidates);
            let expected = compiled
                .zones
                .iter()
                .find(|compiled_zone| {
                    compiled_zone.patterns.iter().any(|pattern| {
                        reference_candidates
                            .iter()
                            .any(|candidate| pattern.pattern.matches(candidate))
                    })
                })
                .map(|compiled_zone| &compiled_zone.zone)
                .unwrap_or(&compiled.zones[compiled.normal_index].zone);
            assert_eq!(actual, expected);
            let actual_index = compiled.zone_index_named(actual.name.as_str()).unwrap();
            let expected_index = compiled.zone_index_named(expected.name.as_str()).unwrap();
            actual_zones.push((actual_index, actual));
            reference_zones.push((expected_index, expected));
        }
        let actual = effective_zone(&actual_zones).unwrap();
        let expected = effective_zone(&reference_zones).unwrap();
        assert_eq!(actual, expected);
    }
}

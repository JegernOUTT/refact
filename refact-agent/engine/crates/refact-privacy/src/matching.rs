use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::{collections::HashMap, sync::RwLock};

use glob::Pattern;
use unicode_normalization::UnicodeNormalization;

use crate::policy::{PrivacyPolicy, ShellBehavior, Zone};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyError {
    InvalidGlob { pattern: String, message: String },
}

impl fmt::Display for PolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidGlob { pattern, message } => {
                write!(formatter, "invalid glob pattern {pattern:?}: {message}")
            }
        }
    }
}

impl std::error::Error for PolicyError {}

struct CompiledZone {
    zone: Zone,
    patterns: Vec<Pattern>,
}

#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct FileIdentity {
    device: u64,
    inode: u64,
}

pub struct CompiledPolicy {
    zones: Vec<CompiledZone>,
    normal_index: usize,
    #[cfg(unix)]
    secret_identities: RwLock<HashMap<FileIdentity, usize>>,
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
                patterns: compile_patterns(&self.blocked)?,
            });
        }

        for zone in &self.zones {
            zones.push(CompiledZone {
                zone: zone.clone(),
                patterns: compile_patterns(&zone.patterns)?,
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
                    patterns: compile_patterns(&zone.patterns)?,
                    zone,
                });
                index
            }
        };

        Ok(CompiledPolicy {
            zones,
            normal_index,
            #[cfg(unix)]
            secret_identities: RwLock::new(HashMap::new()),
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
        let canonical_path = canonicalize_or_original(&absolute_path(path));

        let mut candidate_paths = vec![path.to_path_buf(), absolute_path(path), canonical_path];
        append_relative_candidates(&mut candidate_paths, &roots);

        #[cfg(unix)]
        if let Some(identity) = hard_linked_file_identity(path) {
            append_identity_aliases(&mut candidate_paths, identity, &roots);
            append_relative_candidates(&mut candidate_paths, &roots);
        }

        let candidates = normalized_candidates(&candidate_paths);
        #[cfg(unix)]
        if let Some(index) = self.cached_secret_zone(path) {
            if let Some((earlier, _)) =
                self.zones[..=index]
                    .iter()
                    .enumerate()
                    .find(|(_, compiled)| {
                        compiled.patterns.iter().any(|pattern| {
                            candidates
                                .iter()
                                .any(|candidate| pattern.matches(candidate))
                        })
                    })
            {
                return &self.zones[earlier].zone;
            }
        }
        if let Some((index, _)) = self.zones.iter().enumerate().find(|(_, compiled)| {
            compiled.patterns.iter().any(|pattern| {
                candidates
                    .iter()
                    .any(|candidate| pattern.matches(candidate))
            })
        }) {
            #[cfg(unix)]
            if self.zones[index].zone.name == "secrets" {
                self.remember_secret_identity(path, index);
            }
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
            .map(|root| root.as_ref().to_path_buf())
            .collect::<Vec<_>>();
        let zones = paths
            .into_iter()
            .map(|path| {
                let zone = self.zone_for_path_with_roots(path.as_ref(), &roots);
                let index = self
                    .zone_index_named(&zone.name)
                    .expect("matched zones should belong to the compiled policy");
                (index, zone)
            })
            .collect::<Vec<_>>();
        effective_zone(&zones).unwrap_or_else(|| self.zones[self.normal_index].zone.clone())
    }

    #[cfg(unix)]
    fn cached_secret_zone(&self, path: &Path) -> Option<usize> {
        let identity = file_identity(path)?;
        match self.secret_identities.read() {
            Ok(cache) => cache.get(&identity).copied(),
            Err(poisoned) => {
                drop(poisoned.into_inner());
                let mut cache = self
                    .secret_identities
                    .write()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                cache.clear();
                self.secret_identities.clear_poison();
                None
            }
        }
    }

    #[cfg(unix)]
    fn remember_secret_identity(&self, path: &Path, zone_index: usize) {
        let Some(identity) = file_identity(path) else {
            return;
        };
        match self.secret_identities.write() {
            Ok(mut cache) => {
                cache.insert(identity, zone_index);
            }
            Err(poisoned) => {
                let mut cache = poisoned.into_inner();
                cache.clear();
                cache.insert(identity, zone_index);
                self.secret_identities.clear_poison();
            }
        }
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
    value
        .replace('\\', "/")
        .nfc()
        .collect::<String>()
        .to_lowercase()
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

fn canonicalize_or_original(path: &Path) -> PathBuf {
    dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn append_relative_candidates(candidate_paths: &mut Vec<PathBuf>, roots: &[PathBuf]) {
    let paths = candidate_paths.clone();
    for root in roots {
        let canonical_root = canonicalize_or_original(root);
        for path in &paths {
            for candidate_root in [root, &canonical_root] {
                if let Ok(relative) = path.strip_prefix(candidate_root) {
                    candidate_paths.push(relative.to_path_buf());
                }
            }
        }
    }
}

fn normalized_candidates(paths: &[PathBuf]) -> BTreeSet<String> {
    let mut candidates = BTreeSet::new();
    for path in paths {
        let normalized = normalize(&path.to_string_lossy());
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

#[cfg(unix)]
fn file_identity(path: &Path) -> Option<FileIdentity> {
    use std::os::unix::fs::MetadataExt;

    let metadata = std::fs::metadata(path).ok()?;
    Some(FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(unix)]
fn hard_linked_file_identity(path: &Path) -> Option<FileIdentity> {
    use std::os::unix::fs::MetadataExt;

    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.nlink() <= 1 {
        return None;
    }
    Some(FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(unix)]
fn append_identity_aliases(paths: &mut Vec<PathBuf>, identity: FileIdentity, roots: &[PathBuf]) {
    let mut pending = roots
        .iter()
        .map(|root| canonicalize_or_original(root))
        .collect::<Vec<_>>();
    while let Some(path) = pending.pop() {
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            let Ok(entries) = std::fs::read_dir(path) else {
                continue;
            };
            pending.extend(entries.filter_map(Result::ok).map(|entry| entry.path()));
        } else if file_identity(&path) == Some(identity) {
            paths.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::destination::{Destination, DestinationId, DestinationKind};
    use crate::policy::{ShellBehavior, SubagentPolicy};
    use crate::record::{Attribution, FileRecord, PrivacyRecord};

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
    fn rooted_identical_destination_sets_use_zone_order_in_both_path_orders() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let first = temp.path().join("first.txt");
        let second = temp.path().join("second.txt");
        let first_pattern = first.to_string_lossy().into_owned();
        let policy = policy(vec![
            zone("z-first", &[&first_pattern], &["*"]),
            zone("a-second", &["second.txt"], &["*"]),
            zone("normal", &["*"], &["*"]),
        ]);
        let compiled = policy.compile().expect("policy should compile");

        for paths in [
            [first.as_path(), second.as_path()],
            [second.as_path(), first.as_path()],
        ] {
            assert_eq!(
                compiled
                    .strictest_zone_for_paths_with_roots(paths, [temp.path()])
                    .name,
                "z-first"
            );
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
            assert!(effective.send_to.is_empty());
            assert_eq!(effective.name, "effective:a+b");
        }
    }

    #[test]
    fn subset_destination_set_is_the_effective_set() {
        let policy = policy(vec![
            zone("a", &["a.txt"], &["provider-a"]),
            zone("a-and-b", &["both.txt"], &["provider-a", "provider-b"]),
            zone("normal", &["*"], &["*"]),
        ]);
        let compiled = policy.compile().expect("policy should compile");

        let effective =
            compiled.strictest_zone_for_paths([Path::new("a.txt"), Path::new("both.txt")]);

        assert_eq!(effective.name, "a");
        assert_eq!(effective.send_to, ["provider-a"]);
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
    fn traversal_path_matches_canonical_path() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let secret_dir = temp.path().join("secrets");
        std::fs::create_dir_all(&secret_dir).expect("secret dir should be created");
        let secret = secret_dir.join("value.txt");
        std::fs::write(&secret, "secret").expect("secret should be written");
        let traversal = secret_dir.join("..").join("secrets").join("value.txt");
        let policy = policy(vec![
            zone("secrets", &["secrets/*"], &[]),
            zone("normal", &["*"], &["*"]),
        ]);
        let compiled = policy.compile().expect("policy should compile");

        assert_eq!(
            compiled
                .zone_for_path_with_roots(&traversal, [temp.path()])
                .name,
            compiled
                .zone_for_path_with_roots(&secret, [temp.path()])
                .name
        );
        assert_eq!(
            compiled
                .zone_for_path_with_roots(&traversal, [temp.path()])
                .name,
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

    #[test]
    fn compile_keeps_declared_normal_zone_at_its_position() {
        let policy = policy(vec![
            zone("secrets", &[".env*"], &[]),
            zone("normal", &["**"], &["*"]),
            zone("later", &["later/**"], &[]),
        ]);

        let compiled = policy.compile().expect("policy should compile");

        assert_eq!(compiled.normal_index, 1);
        assert_eq!(
            compiled
                .zones
                .iter()
                .filter(|compiled| compiled.zone.name == "normal")
                .count(),
            1
        );
        assert_eq!(compiled.zones[1].zone, policy.zones[1]);
    }

    #[test]
    fn record_serializes_as_files_object() {
        let record = PrivacyRecord {
            files: vec![FileRecord {
                path: ".env".to_string(),
                zone: "secrets".to_string(),
                attribution: Attribution::Declared,
            }],
        };

        assert_eq!(
            serde_json::to_value(record).expect("record should serialize"),
            serde_json::json!({
                "files": [{
                    "path": ".env",
                    "zone": "secrets",
                    "attribution": "declared"
                }]
            })
        );
    }

    #[cfg(unix)]
    #[test]
    fn directories_are_not_treated_as_hard_linked_files() {
        use std::os::unix::fs::MetadataExt;

        let temp = tempfile::tempdir().expect("tempdir should be created");
        std::fs::create_dir(temp.path().join("nested")).expect("nested dir should be created");
        assert!(std::fs::metadata(temp.path()).unwrap().nlink() > 1);
        assert_eq!(hard_linked_file_identity(temp.path()), None);

        let file = temp.path().join("file.txt");
        let alias = temp.path().join("alias.txt");
        std::fs::write(&file, "content").expect("file should be written");
        std::fs::hard_link(&file, &alias).expect("hard link should be created");
        assert_eq!(hard_linked_file_identity(&file), file_identity(&file));
    }

    #[cfg(unix)]
    #[test]
    fn hardlink_into_secrets_is_classified_when_queried_first() {
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
            "secrets"
        );
    }

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

    #[cfg(unix)]
    #[test]
    fn secret_identity_cache_does_not_override_an_earlier_zone() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let secret_dir = temp.path().join("secrets");
        std::fs::create_dir(&secret_dir).expect("secret dir should be created");
        let secret = secret_dir.join("secret.txt");
        let alias = temp.path().join("public.txt");
        std::fs::write(&secret, "secret").expect("secret should be written");
        std::fs::hard_link(&secret, &alias).expect("hard link should be created");
        let policy = policy(vec![
            zone("public", &["public.txt"], &["*"]),
            zone("secrets", &["secrets/*"], &[]),
            zone("normal", &["*"], &["*"]),
        ]);
        let compiled = policy.compile().expect("policy should compile");

        assert_eq!(
            compiled
                .zone_for_path_with_roots(&secret, [temp.path()])
                .name,
            "public"
        );
        assert_eq!(
            compiled
                .zone_for_path_with_roots(&alias, [temp.path()])
                .name,
            "public"
        );
    }

    #[cfg(unix)]
    #[test]
    fn poisoned_secret_identity_cache_is_cleared_before_classification() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let secret_dir = temp.path().join("secrets");
        std::fs::create_dir(&secret_dir).expect("secret dir should be created");
        let secret = secret_dir.join("secret.txt");
        let alias = temp.path().join("alias.txt");
        std::fs::write(&secret, "secret").expect("secret should be written");
        std::fs::hard_link(&secret, &alias).expect("hard link should be created");
        let compiled = std::sync::Arc::new(
            policy(vec![
                zone("secrets", &["secrets/*"], &[]),
                zone("normal", &["*"], &["*"]),
            ])
            .compile()
            .expect("policy should compile"),
        );
        let identity = file_identity(&secret).expect("secret identity should be available");
        let poison = compiled.clone();
        let _ = std::thread::spawn(move || {
            let mut cache = poison
                .secret_identities
                .write()
                .expect("cache should lock before poisoning");
            cache.insert(identity, 1);
            panic!("poison secret cache");
        })
        .join();

        assert_eq!(
            compiled
                .zone_for_path_with_roots(&alias, [temp.path()])
                .name,
            "secrets"
        );
        assert!(!compiled.secret_identities.is_poisoned());
        assert_eq!(
            compiled
                .secret_identities
                .read()
                .expect("cache should be usable after recovery")
                .get(&identity),
            Some(&0)
        );
    }
}

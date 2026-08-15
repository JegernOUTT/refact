use std::fmt;
use std::path::Path;
#[cfg(unix)]
use std::{collections::HashMap, sync::RwLock};

use glob::Pattern;
use unicode_normalization::UnicodeNormalization;

use crate::policy::{PrivacyPolicy, ShellBehavior, Zone};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyError {
    InvalidGlob { pattern: String, message: String },
    MissingNormalZone,
}

impl fmt::Display for PolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidGlob { pattern, message } => {
                write!(formatter, "invalid glob pattern {pattern:?}: {message}")
            }
            Self::MissingNormalZone => write!(formatter, "privacy policy has no normal zone"),
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
    patterns
        .iter()
        .map(|pattern| {
            Pattern::new(&normalize(pattern)).map_err(|error| PolicyError::InvalidGlob {
                pattern: pattern.clone(),
                message: error.to_string(),
            })
        })
        .collect()
}

impl PrivacyPolicy {
    pub fn compile(&self) -> Result<CompiledPolicy, PolicyError> {
        let mut zones =
            Vec::with_capacity(self.zones.len() + usize::from(!self.blocked.is_empty()));
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

        let normal_index = zones
            .iter()
            .position(|compiled| compiled.zone.name == "normal")
            .ok_or(PolicyError::MissingNormalZone)?;

        Ok(CompiledPolicy {
            zones,
            normal_index,
            #[cfg(unix)]
            secret_identities: RwLock::new(HashMap::new()),
        })
    }
}

impl CompiledPolicy {
    pub fn zone_for_path(&self, path: &Path) -> &Zone {
        #[cfg(unix)]
        if let Some(index) = self.cached_secret_zone(path) {
            return &self.zones[index].zone;
        }

        let normalized_path = normalize(&path.to_string_lossy());
        if let Some((index, compiled)) = self.zones.iter().enumerate().find(|(_, compiled)| {
            compiled
                .patterns
                .iter()
                .any(|pattern| pattern.matches(&normalized_path))
        }) {
            #[cfg(unix)]
            if compiled.zone.name == "secrets" {
                self.remember_secret_identity(path, index);
            }
            return &compiled.zone;
        }

        &self.zones[self.normal_index].zone
    }

    #[cfg(unix)]
    fn cached_secret_zone(&self, path: &Path) -> Option<usize> {
        let identity = file_identity(path)?;
        let cache = self
            .secret_identities
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        cache.get(&identity).copied()
    }

    #[cfg(unix)]
    fn remember_secret_identity(&self, path: &Path, zone_index: usize) {
        let Some(identity) = file_identity(path) else {
            return;
        };
        let mut cache = self
            .secret_identities
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        cache.insert(identity, zone_index);
    }
}

fn normalize(value: &str) -> String {
    value.nfc().collect::<String>().to_lowercase()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::destination::{Destination, DestinationId, DestinationKind};
    use crate::policy::{ShellBehavior, SubagentPolicy};
    use crate::record::{Attribution, FileRecord, PrivacyRecord};

    fn zone(name: &str, patterns: &[&str], send_to: &[&str]) -> Zone {
        Zone {
            name: name.to_string(),
            patterns: patterns.iter().map(|value| (*value).to_string()).collect(),
            send_to: send_to.iter().map(|value| (*value).to_string()).collect(),
            on_shell_read: ShellBehavior::Withhold,
        }
    }

    fn policy(zones: Vec<Zone>) -> PrivacyPolicy {
        PrivacyPolicy {
            blocked: Vec::new(),
            zones,
            subagents: SubagentPolicy::default(),
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
    fn compile_requires_a_normal_fallback_zone() {
        let policy = policy(vec![zone("secrets", &[".env*"], &[])]);

        assert!(matches!(
            policy.compile(),
            Err(PolicyError::MissingNormalZone)
        ));
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
    fn secrets_zone_remembers_matching_file_identity() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let secret = temp.path().join("secret.txt");
        let alias = temp.path().join("alias.txt");
        std::fs::write(&secret, "secret").expect("secret should be written");
        std::fs::hard_link(&secret, &alias).expect("hard link should be created");
        let pattern = format!("{}/*secret.txt", temp.path().display());
        let policy = policy(vec![
            zone("secrets", &[&pattern], &[]),
            zone("normal", &["*"], &["*"]),
        ]);
        let compiled = policy.compile().expect("policy should compile");

        assert_eq!(compiled.zone_for_path(&secret).name, "secrets");
        assert_eq!(compiled.zone_for_path(&alias).name, "secrets");
    }
}

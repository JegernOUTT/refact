//! Declared path scope versus enforced path scope.
//!
//! `target_files` is declared intent. It only becomes enforcement when the run
//! declares strict mode, and strict mode is decided on canonical file identity —
//! never on a string prefix or suffix, and never on a lowercased path.

use std::path::Path;

use crate::tasks::types::ScopeGuardMode;

/// `entries: None` means the run declared no scope at all and is unrestricted.
/// `entries: Some(vec![])` means every declared entry was narrowed away and
/// nothing is in scope.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PathScope {
    pub entries: Option<Vec<String>>,
    pub strict: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScopeOutcome {
    Allowed,
    Warned { path: String, scope: Vec<String> },
    Refused { path: String, scope: Vec<String> },
}

impl PathScope {
    pub fn declared(entries: Vec<String>, strict: bool) -> Self {
        Self {
            entries: (!entries.is_empty()).then_some(entries),
            strict,
        }
    }

    pub fn is_enforced(&self) -> bool {
        self.strict && self.entries.is_some()
    }

    /// The sentence every reporting surface must use, so that no description
    /// claims enforcement the code does not perform.
    pub fn describe(&self) -> String {
        match (&self.entries, self.strict) {
            (None, _) => "no path scope is declared for this run".to_string(),
            (Some(entries), false) => format!(
                "declared intent only, not enforced ({} path(s): {})",
                entries.len(),
                entries.join(", ")
            ),
            (Some(entries), true) => format!(
                "enforced on every built-in mutation ({} path(s): {})",
                entries.len(),
                entries.join(", ")
            ),
        }
    }
}

pub fn scope_for_mode(
    mode: ScopeGuardMode,
    target_files: &[String],
    limits: &refact_work_core::CoordinationSettings,
) -> PathScope {
    let strict = match mode {
        ScopeGuardMode::Strict => true,
        ScopeGuardMode::Reject => limits.strict_path_scope_default,
        ScopeGuardMode::Off | ScopeGuardMode::Warn => false,
    };
    PathScope::declared(target_files.to_vec(), strict)
}

/// A resume may narrow the accepted scope and may turn enforcement on. Dropping
/// entries the accepted scope never granted, or turning strict off, would widen
/// authority, so both are discarded rather than applied.
pub fn narrow(accepted: &PathScope, current: &PathScope) -> PathScope {
    let entries = match (&accepted.entries, &current.entries) {
        (None, current) => current.clone(),
        (Some(accepted), None) => Some(accepted.clone()),
        (Some(accepted), Some(current)) => Some(
            current
                .iter()
                .filter(|entry| accepted.contains(entry))
                .cloned()
                .collect(),
        ),
    };
    PathScope {
        entries,
        strict: accepted.strict || current.strict,
    }
}

/// Canonical identity of `path`, reusing the file-activity key so that symlink
/// aliases and hard links of one physical file collapse onto one identity.
/// A path that cannot be resolved has no identity and is refused, not guessed.
fn identity(path: &Path) -> Result<String, String> {
    crate::coordination::scope::canonical_candidate(path)
        .map_err(|error| format!("{}: {}", path.display(), error))?;
    Ok(crate::file_activity::resolve_key(path).0)
}

fn contains(root: &Path, entries: &[String], candidate: &Path) -> Result<bool, String> {
    let resolved: Vec<String> = entries
        .iter()
        .filter_map(|entry| {
            identity(&root.join(Path::new(entry.trim_start_matches(['/', '\\'])))).ok()
        })
        .collect();
    let candidate = identity(candidate)?;
    Ok(resolved
        .iter()
        .any(|entry| candidate == *entry || Path::new(&candidate).starts_with(Path::new(entry))))
}

pub fn decide(
    mode: ScopeGuardMode,
    scope: &PathScope,
    root: &Path,
    candidate: &Path,
    limits: &refact_work_core::CoordinationSettings,
) -> Result<ScopeOutcome, String> {
    let Some(entries) = scope.entries.as_ref() else {
        return Ok(ScopeOutcome::Allowed);
    };
    if matches!(mode, ScopeGuardMode::Off) {
        return Ok(ScopeOutcome::Allowed);
    }
    if scope.strict {
        if entries.len() > limits.max_path_scope_entries {
            return Err(format!(
                "declared path scope has {} entries but max_path_scope_entries is {}; refusing to enforce an unbounded scope",
                entries.len(),
                limits.max_path_scope_entries
            ));
        }
        let path = candidate.display().to_string();
        return Ok(if contains(root, entries, candidate)? {
            ScopeOutcome::Allowed
        } else {
            ScopeOutcome::Refused {
                path,
                scope: entries.clone(),
            }
        });
    }
    Ok(legacy_outcome(mode, entries, candidate))
}

/// Pre-strict behaviour, preserved verbatim so runs that never opted into strict
/// mode behave exactly as they did before: suffix matching plus a test-file
/// exemption. This is best effort, which is why it is reported as declared
/// intent rather than as enforcement.
fn legacy_outcome(mode: ScopeGuardMode, entries: &[String], candidate: &Path) -> ScopeOutcome {
    let canon_path = dunce::canonicalize(candidate)
        .map(|path| dunce::simplified(&path).to_path_buf())
        .unwrap_or_else(|_| candidate.to_path_buf());
    let canon = canon_path.to_string_lossy().to_string();
    let allowed = entries
        .iter()
        .any(|target| canon_path.ends_with(Path::new(target.trim_start_matches(['/', '\\']))));
    let is_test = canon.contains("/tests/")
        || canon.ends_with("_test.rs")
        || canon.ends_with(".test.ts")
        || canon.ends_with(".test.tsx");
    if allowed || is_test {
        return ScopeOutcome::Allowed;
    }
    match mode {
        ScopeGuardMode::Reject => ScopeOutcome::Refused {
            path: canon,
            scope: entries.to_vec(),
        },
        _ => ScopeOutcome::Warned {
            path: canon,
            scope: entries.to_vec(),
        },
    }
}

pub fn refusal_message(path: &str, scope: &[String], strict: bool) -> String {
    if strict {
        format!(
            "Path {path} is outside the enforced path scope for this run. Violated scope: [{}]. Strict scope is resolved on canonical file identity, so symlink and hard-link aliases cannot widen it.",
            scope.join(", ")
        )
    } else {
        format!(
            "Path {path} is outside the card's target_files scope. Add it to target_files or change scope_guard_mode to 'warn'."
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn limits() -> refact_work_core::CoordinationSettings {
        refact_work_core::CoordinationSettings::default()
    }

    #[test]
    fn declared_scope_says_it_is_not_enforced_until_strict() {
        let none = PathScope::declared(vec![], false);
        assert!(!none.is_enforced());
        assert!(none.describe().contains("no path scope"));

        let declared = PathScope::declared(vec!["src/a.rs".into()], false);
        assert!(!declared.is_enforced());
        assert!(declared.describe().contains("not enforced"));
        assert!(!declared.describe().contains("enforced on every"));

        let enforced = PathScope::declared(vec!["src/a.rs".into()], true);
        assert!(enforced.is_enforced());
        assert!(enforced.describe().contains("enforced on every built-in"));
    }

    #[test]
    fn strict_mode_follows_the_card_and_the_configured_default() {
        let files = vec!["src/a.rs".to_string()];
        let mut limits = limits();
        assert!(!scope_for_mode(ScopeGuardMode::Reject, &files, &limits).strict);
        assert!(scope_for_mode(ScopeGuardMode::Strict, &files, &limits).strict);
        limits.strict_path_scope_default = true;
        assert!(scope_for_mode(ScopeGuardMode::Reject, &files, &limits).strict);
        assert!(!scope_for_mode(ScopeGuardMode::Warn, &files, &limits).strict);
        assert!(scope_for_mode(ScopeGuardMode::Strict, &[], &limits)
            .entries
            .is_none());
    }

    #[test]
    fn resume_may_narrow_the_accepted_scope_but_never_widen_it() {
        let accepted = PathScope::declared(vec!["src/a.rs".into(), "src/b.rs".into()], true);

        let narrowed = narrow(
            &accepted,
            &PathScope::declared(vec!["src/a.rs".into()], true),
        );
        assert_eq!(narrowed.entries, Some(vec!["src/a.rs".to_string()]));
        assert!(narrowed.strict);

        let widened = narrow(
            &accepted,
            &PathScope::declared(vec!["src/a.rs".into(), "src/evil.rs".into()], true),
        );
        assert_eq!(widened.entries, Some(vec!["src/a.rs".to_string()]));

        let dropped = narrow(&accepted, &PathScope::declared(vec![], true));
        assert_eq!(
            dropped.entries,
            Some(vec!["src/a.rs".to_string(), "src/b.rs".to_string()])
        );

        let unstrict = narrow(
            &accepted,
            &PathScope::declared(vec!["src/a.rs".into()], false),
        );
        assert!(unstrict.strict);

        let tightened = narrow(
            &PathScope::declared(vec![], false),
            &PathScope::declared(vec!["src/a.rs".into()], true),
        );
        assert_eq!(tightened.entries, Some(vec!["src/a.rs".to_string()]));
        assert!(tightened.strict);
    }

    struct Fixture {
        _temp: tempfile::TempDir,
        root: PathBuf,
        outside: PathBuf,
    }

    fn fixture() -> Fixture {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        let outside = temp.path().join("outside");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(root.join("src/allowed.rs"), "allowed\n").unwrap();
        std::fs::write(root.join("src/other.rs"), "other\n").unwrap();
        std::fs::write(outside.join("secret.rs"), "secret\n").unwrap();
        Fixture {
            root: dunce::simplified(&std::fs::canonicalize(&root).unwrap()).to_path_buf(),
            outside: dunce::simplified(&std::fs::canonicalize(&outside).unwrap()).to_path_buf(),
            _temp: temp,
        }
    }

    #[test]
    fn strict_scope_admits_declared_paths_and_names_the_scope_on_refusal() {
        let f = fixture();
        let scope = PathScope::declared(vec!["src/allowed.rs".into()], true);
        assert_eq!(
            decide(
                ScopeGuardMode::Strict,
                &scope,
                &f.root,
                &f.root.join("src/allowed.rs"),
                &limits()
            )
            .unwrap(),
            ScopeOutcome::Allowed
        );
        let refused = decide(
            ScopeGuardMode::Strict,
            &scope,
            &f.root,
            &f.root.join("src/other.rs"),
            &limits(),
        )
        .unwrap();
        let ScopeOutcome::Refused { scope: named, .. } = refused else {
            panic!("expected a refusal: {refused:?}");
        };
        assert_eq!(named, vec!["src/allowed.rs".to_string()]);
    }

    #[test]
    fn strict_scope_admits_a_declared_directory_subtree() {
        let f = fixture();
        std::fs::create_dir_all(f.root.join("src/deep")).unwrap();
        let scope = PathScope::declared(vec!["src".into()], true);
        for path in ["src/other.rs", "src/deep/new.rs"] {
            assert_eq!(
                decide(
                    ScopeGuardMode::Strict,
                    &scope,
                    &f.root,
                    &f.root.join(path),
                    &limits()
                )
                .unwrap(),
                ScopeOutcome::Allowed,
                "{path}"
            );
        }
        assert!(matches!(
            decide(
                ScopeGuardMode::Strict,
                &scope,
                &f.root,
                &f.outside.join("secret.rs"),
                &limits()
            )
            .unwrap(),
            ScopeOutcome::Refused { .. }
        ));
    }

    #[test]
    fn strict_scope_is_not_a_suffix_match_and_has_no_test_file_exemption() {
        let f = fixture();
        std::fs::create_dir_all(f.outside.join("src")).unwrap();
        std::fs::write(f.outside.join("src/allowed.rs"), "twin\n").unwrap();
        std::fs::create_dir_all(f.root.join("tests")).unwrap();
        std::fs::write(f.root.join("tests/escape.rs"), "test\n").unwrap();
        let scope = PathScope::declared(vec!["src/allowed.rs".into()], true);

        for path in [
            f.outside.join("src/allowed.rs"),
            f.root.join("tests/escape.rs"),
        ] {
            assert!(
                matches!(
                    decide(ScopeGuardMode::Strict, &scope, &f.root, &path, &limits()).unwrap(),
                    ScopeOutcome::Refused { .. }
                ),
                "{}",
                path.display()
            );
        }

        assert_eq!(
            decide(
                ScopeGuardMode::Reject,
                &PathScope::declared(vec!["src/allowed.rs".into()], false),
                &f.root,
                &f.outside.join("src/allowed.rs"),
                &limits()
            )
            .unwrap(),
            ScopeOutcome::Allowed
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_out_of_the_scope_cannot_be_used_to_escape_it() {
        let f = fixture();
        std::os::unix::fs::symlink(f.outside.join("secret.rs"), f.root.join("src/alias.rs"))
            .unwrap();
        std::os::unix::fs::symlink(&f.outside, f.root.join("src/gate")).unwrap();
        let scope = PathScope::declared(vec!["src".into()], true);

        for path in [
            f.root.join("src/alias.rs"),
            f.root.join("src/gate/secret.rs"),
            f.root.join("src/gate/not-yet-created.rs"),
        ] {
            assert!(
                matches!(
                    decide(ScopeGuardMode::Strict, &scope, &f.root, &path, &limits()).unwrap(),
                    ScopeOutcome::Refused { .. }
                ),
                "{} escaped the scope",
                path.display()
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_hard_link_is_the_same_file_identity_as_its_out_of_scope_original() {
        let f = fixture();
        let original = f.outside.join("secret.rs");
        // Register the out-of-scope spelling first so the shared inode resolves
        // to it; the in-scope spelling must not launder the same physical file.
        assert!(identity(&original).is_ok());
        std::fs::hard_link(&original, f.root.join("src/laundered.rs")).unwrap();
        let scope = PathScope::declared(vec!["src".into()], true);

        assert!(matches!(
            decide(
                ScopeGuardMode::Strict,
                &scope,
                &f.root,
                &f.root.join("src/laundered.rs"),
                &limits()
            )
            .unwrap(),
            ScopeOutcome::Refused { .. }
        ));
    }

    #[test]
    fn traversal_has_no_identity_and_is_refused_rather_than_normalised() {
        let f = fixture();
        let scope = PathScope::declared(vec!["src".into()], true);
        assert!(decide(
            ScopeGuardMode::Strict,
            &scope,
            &f.root,
            &f.root.join("src/../../outside/secret.rs"),
            &limits()
        )
        .is_err());
    }

    #[test]
    fn an_over_large_declared_scope_is_refused_instead_of_silently_enforced() {
        let f = fixture();
        let scope = PathScope::declared(
            (0..4).map(|index| format!("src/f{index}.rs")).collect(),
            true,
        );
        let limits = refact_work_core::CoordinationSettings {
            max_path_scope_entries: 3,
            ..Default::default()
        };
        let error = decide(
            ScopeGuardMode::Strict,
            &scope,
            &f.root,
            &f.root.join("src/allowed.rs"),
            &limits,
        )
        .unwrap_err();
        assert!(error.contains("max_path_scope_entries"));
    }

    #[test]
    fn a_run_without_a_declared_scope_is_unrestricted_in_every_mode() {
        let f = fixture();
        let scope = PathScope::declared(vec![], true);
        for mode in [
            ScopeGuardMode::Off,
            ScopeGuardMode::Warn,
            ScopeGuardMode::Reject,
            ScopeGuardMode::Strict,
        ] {
            assert_eq!(
                decide(
                    mode,
                    &scope,
                    &f.root,
                    &f.outside.join("secret.rs"),
                    &limits()
                )
                .unwrap(),
                ScopeOutcome::Allowed
            );
        }
        assert_eq!(
            decide(
                ScopeGuardMode::Off,
                &PathScope::declared(vec!["src/allowed.rs".into()], true),
                &f.root,
                &f.outside.join("secret.rs"),
                &limits()
            )
            .unwrap(),
            ScopeOutcome::Allowed
        );
    }

    #[test]
    fn refusal_text_names_the_scope_and_never_claims_unimplemented_enforcement() {
        let strict = refusal_message("/root/src/evil.rs", &["src/a.rs".to_string()], true);
        assert!(strict.contains("enforced path scope"));
        assert!(strict.contains("src/a.rs"));
        let declared = refusal_message("/root/src/evil.rs", &["src/a.rs".to_string()], false);
        assert!(declared.contains("target_files scope"));
        assert!(!declared.contains("enforced"));
    }
}

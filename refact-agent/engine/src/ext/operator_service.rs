//! Extension-owned versioned persistence. No command execution, trust writes or network access.
use std::path::{Component, Path, PathBuf};
use crate::buddy::resources::ResourceError;
use refact_buddy_core::operator_types::ProjectIdentity;

static MUTATION: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn version(bytes: &Option<Vec<u8>>) -> String {
    bytes
        .as_ref()
        .map(|b| crate::yaml_configs::customization_service::version(b))
        .unwrap_or_else(|| "absent".into())
}

pub fn checked_path(project: &ProjectIdentity, relative: &str) -> Result<PathBuf, ResourceError> {
    let root = Path::new(&project.canonical_root);
    if std::fs::canonicalize(root)?.to_string_lossy() != project.canonical_root {
        return Err(ResourceError::ProjectScopeMismatch);
    }
    let relative = Path::new(relative);
    if relative
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(ResourceError::InvalidResourceId);
    }
    let path = root.join(".refact").join(relative);
    for ancestor in path.ancestors() {
        match std::fs::symlink_metadata(ancestor) {
            Ok(m) if m.file_type().is_symlink() => return Err(ResourceError::SymlinkNotAllowed),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(path)
}
pub fn read_path(path: &Path) -> Result<Option<Vec<u8>>, ResourceError> {
    match std::fs::read(path) {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}
pub fn snapshot(
    project: &ProjectIdentity,
    relative: &str,
) -> Result<Option<Vec<u8>>, ResourceError> {
    read_path(&checked_path(project, relative)?)
}

/// Shared by native editor writes and operator CAS; preserves absence during recovery.
pub fn replace_path(
    path: &Path,
    expected: &str,
    next: &Option<Vec<u8>>,
) -> Result<(), ResourceError> {
    let _guard = MUTATION
        .lock()
        .map_err(|_| ResourceError::Io("extension mutation lock poisoned".into()))?;
    replace_path_locked(path, expected, next)
}

/// The caller must hold MUTATION through its snapshot and replacement.
fn replace_path_locked(
    path: &Path,
    expected: &str,
    next: &Option<Vec<u8>>,
) -> Result<(), ResourceError> {
    use std::io::Write;
    let check = || -> Result<(), ResourceError> {
        for ancestor in path.ancestors() {
            if let Ok(m) = std::fs::symlink_metadata(ancestor) {
                if m.file_type().is_symlink() {
                    return Err(ResourceError::SymlinkNotAllowed);
                }
            }
        }
        if version(&read_path(path)?) != expected {
            return Err(ResourceError::VersionMismatch);
        }
        Ok(())
    };
    check()?;
    let parent = path.parent().ok_or(ResourceError::InvalidResourceId)?;
    if let Some(bytes) = next {
        std::fs::create_dir_all(parent)?;
        let temp = parent.join(format!(".ext-{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temp)?;
            if let Ok(metadata) = std::fs::metadata(path) {
                file.set_permissions(metadata.permissions())?;
            }
            file.write_all(bytes)?;
            file.sync_all()?;
            check()?;
            std::fs::rename(&temp, path)?;
            Ok::<_, ResourceError>(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(temp);
        }
        result?;
    } else if read_path(path)?.is_some() {
        check()?;
        std::fs::remove_file(path)?;
    }
    #[cfg(unix)]
    if parent.exists() {
        std::fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}
pub fn replace(
    project: &ProjectIdentity,
    relative: &str,
    expected: &str,
    next: &Option<Vec<u8>>,
) -> Result<(), ResourceError> {
    replace_path(&checked_path(project, relative)?, expected, next)
}
pub async fn write_file_atomic(path: &Path, content: &str) -> std::io::Result<()> {
    let path = path.to_owned();
    let bytes = content.as_bytes().to_vec();
    tokio::task::spawn_blocking(move || {
        // Native editor writes have no caller-supplied version constraint: serialize
        // their snapshot with the write, while explicit adapter CAS stays strict.
        let _guard = MUTATION
            .lock()
            .map_err(|_| std::io::Error::other("extension mutation lock poisoned"))?;
        let before = read_path(&path).map_err(|e| std::io::Error::other(format!("{e:?}")))?;
        replace_path_locked(&path, &version(&before), &Some(bytes))
            .map_err(|e| std::io::Error::other(format!("{e:?}")))
    })
    .await
    .map_err(std::io::Error::other)?
}

/// Authorization is supplied by the native policy/grant owner, never by a draft.
/// Implementations must resolve opaque command/auth references and compare exact project,
/// action revision, target, endpoint and tools; no wildcard or self-issued grants.
pub trait ResourceRights: Send + Sync {
    fn authorize(
        &self,
        spec: &refact_buddy_core::operator_types::ActionSpec,
        proposed_version: &str,
        requirement: &serde_json::Value,
    ) -> Result<(), ResourceError>;
}
pub struct DenyResourceRights;
impl ResourceRights for DenyResourceRights {
    fn authorize(
        &self,
        _: &refact_buddy_core::operator_types::ActionSpec,
        _: &str,
        _: &serde_json::Value,
    ) -> Result<(), ResourceError> {
        Err(ResourceError::ValidationFailed(
            "exact native resource grant required".into(),
        ))
    }
}

pub fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err("invalid extension name".into());
    }
    Ok(())
}

pub fn exact_names(names: &[String]) -> bool {
    !names.is_empty()
        && names.iter().all(|n| {
            !n.is_empty()
                && n.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b':'))
        })
        && names.iter().collect::<std::collections::HashSet<_>>().len() == names.len()
}
pub fn opaque_reference(reference: &str) -> bool {
    !reference.is_empty()
        && reference.len() <= 256
        && reference
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b':'))
}
pub fn prepare_hook(
    before: Option<&[u8]>,
    draft: &crate::buddy::resources::hooks::HookDraft,
) -> Result<Vec<u8>, ResourceError> {
    let invalid = || ResourceError::ValidationFailed("invalid scoped hook configuration".into());
    if draft.command.trim().is_empty()
        || draft.command.contains('\0')
        || draft.command.len() > 16384
        || !opaque_reference(&draft.command_reference)
        || !exact_names(&draft.tool_access_scope)
        || draft.timeout_seconds == 0
        || draft.timeout_seconds > 3600
    {
        return Err(invalid());
    }
    // Restrict matching to precisely the authorized tools, not arbitrary regular expressions.
    let expected = format!(
        "^({})$",
        draft
            .tool_access_scope
            .iter()
            .map(|s| regex::escape(s))
            .collect::<Vec<_>>()
            .join("|")
    );
    if draft.matcher != expected {
        return Err(ResourceError::ValidationFailed(
            "matcher must exactly enumerate tool scope".into(),
        ));
    }
    if !matches!(
        draft.event,
        crate::ext::hooks::HookEvent::PreToolUse | crate::ext::hooks::HookEvent::PostToolUse
    ) {
        return Err(invalid());
    }
    let mut root = match before {
        Some(b) => serde_yaml::from_slice::<serde_yaml::Mapping>(b).map_err(|_| invalid())?,
        None => serde_yaml::Mapping::new(),
    };
    let hooks = root
        .entry("hooks".into())
        .or_insert_with(|| serde_yaml::Value::Mapping(Default::default()))
        .as_mapping_mut()
        .ok_or_else(invalid)?;
    let entries = hooks
        .entry(serde_yaml::to_value(&draft.event).map_err(|_| invalid())?)
        .or_insert_with(|| serde_yaml::Value::Sequence(vec![]))
        .as_sequence_mut()
        .ok_or_else(invalid)?;
    entries.push(serde_yaml::to_value(serde_json::json!({"matcher":draft.matcher,"hooks":[{"type":"command","command":draft.command,"timeout":draft.timeout_seconds}]})).map_err(|_| invalid())?);
    serde_yaml::to_string(&root)
        .map(|s| s.into_bytes())
        .map_err(|_| invalid())
}

/// Fixture trigger is injected by tests. No production executor or implicit egress fallback.
pub trait HookProbe: Send + Sync {
    fn trigger(
        &self,
        event: &crate::ext::hooks::HookEvent,
        tool: &str,
        command_reference: &str,
    ) -> Result<(), ResourceError>;
}
pub fn probe_hook(
    probe: &dyn HookProbe,
    rights: &dyn ResourceRights,
    spec: &refact_buddy_core::operator_types::ActionSpec,
    post: &str,
    draft: &crate::buddy::resources::hooks::HookDraft,
    tool: &str,
) -> Result<(), ResourceError> {
    prepare_hook(None, draft)?;
    if spec.project != draft.project
        || spec.adapter != "hooks"
        || spec.resource.resource_kind != "hooks"
        || spec.resource.resource_id != "project/hooks"
    {
        return Err(ResourceError::DraftDoesNotMatchResource);
    }
    if !draft.tool_access_scope.iter().any(|s| s == tool) {
        return Err(ResourceError::ValidationFailed(
            "probe tool outside scope".into(),
        ));
    }
    rights.authorize(
        spec,
        post,
        &serde_json::json!({"operation":"probe_hook","draft":draft,"tool":tool}),
    )?;
    probe.trigger(&draft.event, tool, &draft.command_reference)
}

pub fn prepare_markdown(
    before: Option<&[u8]>,
    kind: &str,
    name: &str,
    description: Option<&str>,
    body: &str,
) -> Result<Vec<u8>, ResourceError> {
    validate_name(name).map_err(ResourceError::ValidationFailed)?;
    if body.contains('\0') || body.len() > 1024 * 1024 {
        return Err(ResourceError::ValidationFailed(
            "invalid Markdown body".into(),
        ));
    }
    let text = std::str::from_utf8(before.unwrap_or_default())
        .map_err(|_| ResourceError::ValidationFailed("invalid UTF-8".into()))?;
    // Strict parse: the permissive runtime reader must not silently drop malformed metadata.
    let mut fm = if let Some(rest) = text.strip_prefix("---\n") {
        let (yaml, _) = rest
            .split_once("\n---")
            .ok_or_else(|| ResourceError::ValidationFailed("unterminated frontmatter".into()))?;
        serde_yaml::from_str::<serde_yaml::Mapping>(yaml)
            .map_err(|_| ResourceError::ValidationFailed("invalid frontmatter".into()))?
    } else {
        serde_yaml::Mapping::new()
    };
    if kind == "skills" {
        fm.insert("name".into(), name.into());
        if before.is_none() {
            fm.insert("user-invocable".into(), false.into());
        }
    }
    if let Some(description) = description {
        fm.insert("description".into(), description.into());
    }
    if kind == "skills"
        && fm
            .get("description")
            .and_then(serde_yaml::Value::as_str)
            .is_none_or(|d| d.trim().is_empty())
    {
        return Err(ResourceError::ValidationFailed(
            "skill description required".into(),
        ));
    }
    let yaml = serde_yaml::to_string(&fm)
        .map_err(|_| ResourceError::ValidationFailed("frontmatter serialization failed".into()))?;
    Ok(format!("---\n{yaml}---\n{body}").into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hook_owner_preserves_other_events_and_exact_recovery() {
        let temp = tempfile::tempdir().unwrap();
        let project = ProjectIdentity {
            project_id: "fixture".into(),
            canonical_root: dunce::canonicalize(temp.path())
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        };
        let before = Some(b"unknown: retained\nhooks:\n  SessionStart: []\n".to_vec());
        replace(&project, "hooks.yaml", "absent", &before).unwrap();
        let mut draft = crate::buddy::resources::hooks::HookDraft {
            project: project.clone(),
            event: crate::ext::hooks::HookEvent::PreToolUse,
            command: "fixture-command".into(),
            command_reference: "fixture-ref".into(),
            matcher: "^(read)$".into(),
            timeout_seconds: 30,
            tool_access_scope: vec!["read".into()],
        };
        let after = Some(prepare_hook(before.as_deref(), &draft).unwrap());
        assert_eq!(snapshot(&project, "hooks.yaml").unwrap(), before);
        let yaml: serde_yaml::Value = serde_yaml::from_slice(after.as_ref().unwrap()).unwrap();
        assert_eq!(yaml["unknown"].as_str(), Some("retained"));
        assert!(yaml["hooks"]["SessionStart"]
            .as_sequence()
            .unwrap()
            .is_empty());
        assert_eq!(
            yaml["hooks"]["PreToolUse"][0]["hooks"][0]["command"].as_str(),
            Some("fixture-command")
        );
        replace(&project, "hooks.yaml", &version(&before), &after).unwrap();
        replace(&project, "hooks.yaml", &version(&after), &before).unwrap();
        assert_eq!(snapshot(&project, "hooks.yaml").unwrap(), before);
        draft.matcher = ".*".into();
        assert!(prepare_hook(before.as_deref(), &draft).is_err());
        draft.matcher = "^(read)$".into();
        draft.command_reference = "https://inline-secret".into();
        assert!(prepare_hook(before.as_deref(), &draft).is_err());
    }
    #[test]
    fn markdown_owner_retains_authority_and_unknown_metadata() {
        let before = b"---\nname: fixture\ndescription: old\nallowed-tools: [read]\ncustom:\n  nested: true\n---\nOriginal";
        let after =
            prepare_markdown(Some(before), "skills", "fixture", Some("new"), "New body").unwrap();
        let text = String::from_utf8(after).unwrap();
        let (fm, body) = crate::ext::slash_commands::parse_frontmatter_and_body(&text);
        assert_eq!(fm["allowed-tools"][0].as_str(), Some("read"));
        assert_eq!(fm["custom"]["nested"].as_bool(), Some(true));
        assert_eq!(body.trim(), "New body");
        assert!(prepare_markdown(
            Some(b"---\nmalformed"),
            "skills",
            "fixture",
            Some("new"),
            "body"
        )
        .is_err());
    }
    #[tokio::test]
    async fn native_write_invalidates_explicit_cas_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("command.md");
        write_file_atomic(&path, "initial").await.unwrap();
        let before = read_path(&path).unwrap();
        write_file_atomic(&path, "native edit").await.unwrap();
        assert_eq!(
            replace_path(&path, &version(&before), &Some(b"adapter edit".to_vec())),
            Err(ResourceError::VersionMismatch)
        );
        assert_eq!(read_path(&path).unwrap(), Some(b"native edit".to_vec()));
    }

    #[test]
    fn owner_cas_preserves_absence_and_refuses_intervening_edit() {
        let temp = tempfile::tempdir().unwrap();
        let p = ProjectIdentity {
            project_id: "fixture".into(),
            canonical_root: dunce::canonicalize(temp.path())
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        };
        let next = Some(b"unknown: retained\n".to_vec());
        assert_eq!(snapshot(&p, "hooks.yaml").unwrap(), None);
        replace(&p, "hooks.yaml", "absent", &next).unwrap();
        assert_eq!(snapshot(&p, "hooks.yaml").unwrap(), next);
        assert_eq!(
            replace(&p, "hooks.yaml", "absent", &None),
            Err(ResourceError::VersionMismatch)
        );
        replace(&p, "hooks.yaml", &version(&next), &None).unwrap();
        assert_eq!(snapshot(&p, "hooks.yaml").unwrap(), None);
        assert!(checked_path(&p, "../escape").is_err());
    }
}

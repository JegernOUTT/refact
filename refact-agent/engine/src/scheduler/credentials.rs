//! Host-only credential storage. Values are PLAINTEXT protected by OS permissions,
//! not encrypted. Never expose mutation ports to model tools or serialize `State` in
//! API responses. The private-cache threat boundary excludes arbitrary same-UID
//! access. Open/mutations do disk IO; collector resolution only reads memory.
use refact_scheduler_core::sources::{CredentialBinding, CredentialContext, SourceCredentialResolver};
use reqwest::header::{HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

const DENIED: &str = "credential capability denied";
const STORAGE: &str = "private credential storage unavailable";
const LIMIT: u64 = 8 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialIdentity {
    pub id: String,
    pub origin: String,
    pub header: String,
    pub tenant: String,
    pub principal: String,
    pub scope: Vec<String>,
}

/// Safe to return to the operator. Contains no credential value or value hash.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BindingMetadata {
    #[serde(flatten)]
    pub identity: CredentialIdentity,
    pub identity_revision: String,
    pub secret_revision: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct Entry {
    metadata: BindingMetadata,
    secret: String,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Authorization {
    app_id: String,
    source_id: String,
    config_digest: String,
    origin: String,
    reference: String,
    header: String,
    scheme: Option<String>,
    identity_revision: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    version: u32,
    revision: String,
    entries: BTreeMap<String, Entry>,
    grants: Vec<Authorization>,
}
struct Inner {
    directory: File,
    _lock: File,
    owner_pid: u32,
    state: Mutex<State>,
    failed: AtomicBool,
}

impl Drop for Inner {
    fn drop(&mut self) {
        #[cfg(unix)]
        if self.owner_pid == std::process::id() {
            use std::os::fd::AsRawFd;
            // CLOEXEC does not close descriptors between fork and exec. Release
            // the host capability here, rather than waiting for unrelated children
            // to close their inherited copy of this flock's open file description.
            // A fork-child destructor must not unlock its still-live parent's lock.
            unsafe { libc::flock(self._lock.as_raw_fd(), libc::LOCK_UN) };
        }
    }
}
/// Possession is a trusted host capability, NOT an app permission or JSON flag.
/// Share clones with host control code; inject only the resolver trait into collectors.
#[derive(Clone)]
pub struct AutomationCredentialStore(Arc<Inner>);

impl AutomationCredentialStore {
    /// `private_storage_base` is a trusted host override (e.g. an injected tempdir),
    /// never a manifest path. Default credentials.json is beside runtime.sqlite.
    pub fn open(project_root: &Path, private_storage_base: Option<&Path>) -> Result<Self, String> {
        #[cfg(not(unix))]
        {
            let _ = (project_root, private_storage_base);
            return Err(STORAGE.into());
        }
        #[cfg(unix)]
        {
            let root = std::fs::canonicalize(project_root).map_err(|_| STORAGE)?;
            let journal =
                refact_scheduler_core::automation::journal_path(&root).map_err(|_| STORAGE)?;
            let default_dir = journal.parent().ok_or(STORAGE)?;
            let directory_path = if let Some(base) = private_storage_base {
                let scope = default_dir.file_name().ok_or(STORAGE)?;
                base.join("refact-automation-runtime")
                    .join("v1")
                    .join(scope)
            } else {
                default_dir.to_path_buf()
            };
            if !directory_path.is_absolute() || directory_path.starts_with(&root) {
                return Err(STORAGE.into());
            }
            static OPEN: OnceLock<Mutex<BTreeMap<PathBuf, Weak<Inner>>>> = OnceLock::new();
            let mut open = OPEN
                .get_or_init(Default::default)
                .lock()
                .map_err(|_| STORAGE)?;
            if let Some(inner) = open.get(&directory_path).and_then(Weak::upgrade) {
                return Ok(Self(inner));
            }
            let directory = private_directory(&directory_path)?;
            let lock = open_private(&directory, "credentials.lock", true)?;
            use std::os::fd::AsRawFd;
            // Lifetime exclusive process lock: another daemon cannot retain a stale
            // in-memory secret/grant after an operator revokes it in this process.
            if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
                return Err(STORAGE.into());
            }
            let state = match read_state(&directory)? {
                Some(state) => state,
                None => {
                    let state = State {
                        version: 1,
                        revision: revision(),
                        entries: BTreeMap::new(),
                        grants: vec![],
                    };
                    persist(&directory, &state)?;
                    state
                }
            };
            validate_state(&state)?;
            let inner = Arc::new(Inner {
                directory,
                _lock: lock,
                owner_pid: std::process::id(),
                state: Mutex::new(state),
                failed: AtomicBool::new(false),
            });
            open.retain(|_, item| item.strong_count() > 0);
            open.insert(directory_path, Arc::downgrade(&inner));
            Ok(Self(inner))
        }
    }

    pub fn list_metadata(&self) -> Result<Vec<BindingMetadata>, String> {
        Ok(self
            .0
            .state
            .lock()
            .map_err(|_| STORAGE)?
            .entries
            .values()
            .map(|e| e.metadata.clone())
            .collect())
    }

    /// Trusted operator only. None creates; Some performs compare-and-swap update.
    /// ANY identity change invalidates grants (including a narrowing of scope).
    pub fn set(
        &self,
        mut identity: CredentialIdentity,
        secret: String,
        expected_secret_revision: Option<&str>,
    ) -> Result<BindingMetadata, String> {
        identity.header = identity.header.to_ascii_lowercase();
        identity.scope.sort();
        identity.scope.dedup();
        validate_identity(&identity)?;
        validate_secret(&secret)?;
        self.change(|state| {
            let old = state.entries.get(&identity.id);
            if old.map(|e| e.metadata.secret_revision.as_str()) != expected_secret_revision {
                return Err("credential revision conflict".into());
            }
            if old.is_none() && state.entries.len() >= 1024 {
                return Err("credential capacity limit".into());
            }
            let unchanged = old.is_some_and(|e| e.metadata.identity == identity);
            let identity_revision = if unchanged {
                old.unwrap().metadata.identity_revision.clone()
            } else {
                revision()
            };
            if !unchanged {
                state.grants.retain(|g| g.reference != identity.id);
            }
            let metadata = BindingMetadata {
                identity,
                identity_revision,
                secret_revision: revision(),
            };
            state.entries.insert(
                metadata.identity.id.clone(),
                Entry {
                    metadata: metadata.clone(),
                    secret,
                },
            );
            Ok(metadata)
        })
    }

    /// Rotation preserves exactly the already-approved identity and scope.
    pub fn rotate(
        &self,
        id: &str,
        expected_secret_revision: &str,
        secret: String,
    ) -> Result<BindingMetadata, String> {
        validate_secret(&secret)?;
        self.change(|state| {
            let entry = state.entries.get_mut(id).ok_or(DENIED)?;
            if entry.metadata.secret_revision != expected_secret_revision {
                return Err("credential revision conflict".into());
            }
            entry.secret = secret;
            entry.metadata.secret_revision = revision();
            Ok(entry.metadata.clone())
        })
    }

    pub fn remove(&self, id: &str, expected_secret_revision: &str) -> Result<(), String> {
        self.change(|state| {
            let entry = state.entries.get(id).ok_or(DENIED)?;
            if entry.metadata.secret_revision != expected_secret_revision {
                return Err("credential revision conflict".into());
            }
            state.entries.remove(id);
            state.grants.retain(|g| g.reference != id);
            Ok(())
        })
    }

    /// Call ONLY after explicit host approval of the source's compiled policy.
    /// References, metadata listing, and source demand do not call this port.
    pub fn authorize_binding(
        &self,
        context: &CredentialContext<'_>,
        binding: &CredentialBinding,
        expected_identity_revision: &str,
    ) -> Result<(), String> {
        let grant = authorization(context, binding, expected_identity_revision)?;
        self.change(|state| {
            let entry = state.entries.get(&binding.reference).ok_or(DENIED)?;
            check_entry(entry, &grant)?;
            if !state.grants.contains(&grant) {
                if state.grants.len() >= 4096 {
                    return Err("credential capacity limit".into());
                }
                state.grants.push(grant);
            }
            Ok(())
        })
    }

    pub fn revoke_binding(
        &self,
        context: &CredentialContext<'_>,
        binding: &CredentialBinding,
    ) -> Result<(), String> {
        self.change(|state| {
            state.grants.retain(|g| {
                !(g.app_id == context.app_id
                    && g.source_id == context.source_id
                    && g.config_digest == context.config_digest
                    && g.reference == binding.reference)
            });
            Ok(())
        })
    }

    fn change<T>(&self, change: impl FnOnce(&mut State) -> Result<T, String>) -> Result<T, String> {
        let mut current = self.0.state.lock().map_err(|_| STORAGE)?;
        if self.0.failed.load(Ordering::Acquire) {
            return Err(STORAGE.into());
        }
        // The exclusive lifetime lock prevents cooperative writers; this extra CAS
        // detects out-of-band replacement rather than overwriting newer state.
        #[cfg(unix)]
        if read_state(&self.0.directory)?.as_ref().map(|s| &s.revision) != Some(&current.revision) {
            return Err("credential revision conflict".into());
        }
        let mut next = current.clone();
        let result = change(&mut next)?;
        next.revision = revision();
        #[cfg(unix)]
        if let Err(error) = persist(&self.0.directory, &next) {
            // Rename may have committed before directory fsync failed. Do not
            // resolve stale cached authority after an indeterminate write.
            self.0.failed.store(true, Ordering::Release);
            return Err(error);
        }
        *current = next;
        Ok(result)
    }
}

impl SourceCredentialResolver for AutomationCredentialStore {
    fn resolve(
        &self,
        context: &CredentialContext<'_>,
        binding: &CredentialBinding,
    ) -> Result<HeaderValue, String> {
        let expected = binding
            .expected_identity_revision
            .as_deref()
            .ok_or(DENIED)?;
        let grant = authorization(context, binding, expected)?;
        let state = self.0.state.lock().map_err(|_| DENIED)?;
        if self.0.failed.load(Ordering::Acquire) {
            return Err(DENIED.into());
        }
        if !state.grants.contains(&grant) {
            return Err(DENIED.into());
        }
        let entry = state.entries.get(&binding.reference).ok_or(DENIED)?;
        check_entry(entry, &grant)?;
        // Core adds the approved scheme, so return the raw value exactly once.
        let mut value = HeaderValue::from_str(&entry.secret).map_err(|_| DENIED)?;
        value.set_sensitive(true);
        Ok(value)
    }
}
fn revision() -> String {
    uuid::Uuid::new_v4().to_string()
}
fn validate_secret(secret: &str) -> Result<(), String> {
    if secret.is_empty() || secret.len() > 4096 || HeaderValue::from_str(secret).is_err() {
        return Err("invalid credential value".into());
    }
    Ok(())
}
fn validate_identity(i: &CredentialIdentity) -> Result<(), String> {
    let url = reqwest::Url::parse(&i.origin).map_err(|_| DENIED)?;
    let name = HeaderName::from_bytes(i.header.as_bytes()).map_err(|_| DENIED)?;
    if !matches!(url.scheme(), "http" | "https" | "ws" | "wss")
        || url.origin().ascii_serialization() != i.origin
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || matches!(
            name.as_str(),
            "host"
                | "connection"
                | "content-length"
                | "transfer-encoding"
                | "upgrade"
                | "accept"
                | "last-event-id"
        )
        || name.as_str().starts_with("sec-websocket-")
        || i.id.is_empty()
        || i.id.len() > 256
        || !i
            .id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-:.".contains(&b))
        || i.tenant.len() > 1024
        || i.principal.is_empty()
        || i.principal.len() > 1024
        || i.scope.len() > 128
        || i.scope.iter().any(|s| s.is_empty() || s.len() > 1024)
    {
        return Err(DENIED.into());
    }
    Ok(())
}
fn authorization(
    context: &CredentialContext<'_>,
    binding: &CredentialBinding,
    expected: &str,
) -> Result<Authorization, String> {
    if expected.is_empty()
        || binding.expected_identity_revision.as_deref() != Some(expected)
        || [context.app_id, context.source_id, context.config_digest]
            .iter()
            .any(|s| s.is_empty() || s.len() > 1024)
        || binding.scheme.as_ref().is_some_and(|s| {
            s.is_empty()
                || s.len() > 32
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
        })
    {
        return Err(DENIED.into());
    }
    Ok(Authorization {
        app_id: context.app_id.into(),
        source_id: context.source_id.into(),
        config_digest: context.config_digest.into(),
        origin: context.origin.into(),
        reference: binding.reference.clone(),
        header: binding.header.to_ascii_lowercase(),
        scheme: binding.scheme.clone(),
        identity_revision: expected.into(),
    })
}
fn check_entry(entry: &Entry, grant: &Authorization) -> Result<(), String> {
    if entry.metadata.identity_revision != grant.identity_revision
        || entry.metadata.identity.origin != grant.origin
        || entry.metadata.identity.header != grant.header
    {
        return Err(DENIED.into());
    }
    Ok(())
}
fn validate_state(state: &State) -> Result<(), String> {
    if state.version != 1 || state.entries.len() > 1024 || state.grants.len() > 4096 {
        return Err(STORAGE.into());
    }
    for (id, entry) in &state.entries {
        if id != &entry.metadata.identity.id {
            return Err(STORAGE.into());
        }
        validate_identity(&entry.metadata.identity).map_err(|_| STORAGE)?;
        validate_secret(&entry.secret).map_err(|_| STORAGE)?;
    }
    Ok(())
}

#[cfg(unix)]
fn private_directory(path: &Path) -> Result<File, String> {
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::{ffi::OsStrExt, fs::MetadataExt};
    let mut dir = File::open("/").map_err(|_| STORAGE)?;
    let components: Vec<_> = path.components().collect();
    for (index, component) in components.iter().enumerate().skip(1) {
        let std::path::Component::Normal(name) = component else {
            return Err(STORAGE.into());
        };
        let name = std::ffi::CString::new(name.as_bytes()).map_err(|_| STORAGE)?;
        let rc = unsafe { libc::mkdirat(dir.as_raw_fd(), name.as_ptr(), 0o700) };
        if rc != 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists {
            return Err(STORAGE.into());
        }
        let fd = unsafe {
            libc::openat(
                dir.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(STORAGE.into());
        }
        dir = unsafe { File::from_raw_fd(fd) };
        let meta = dir.metadata().map_err(|_| STORAGE)?;
        // All three private suffix directories must already be owned and private;
        // never chmod an attacker-created directory into apparent safety.
        if index + 3 >= components.len()
            && (meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o777 != 0o700)
        {
            return Err(STORAGE.into());
        }
    }
    Ok(dir)
}
#[cfg(unix)]
fn open_private(dir: &File, name: &str, create: bool) -> Result<File, String> {
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::fs::MetadataExt;
    let name = std::ffi::CString::new(name).map_err(|_| STORAGE)?;
    let flags = libc::O_RDWR
        | libc::O_NOFOLLOW
        | libc::O_CLOEXEC
        | libc::O_NONBLOCK
        | if create { libc::O_CREAT } else { 0 };
    let fd = unsafe { libc::openat(dir.as_raw_fd(), name.as_ptr(), flags, 0o600) };
    if fd < 0 {
        return Err(STORAGE.into());
    }
    let file = unsafe { File::from_raw_fd(fd) };
    let meta = file.metadata().map_err(|_| STORAGE)?;
    if !meta.is_file()
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.mode() & 0o777 != 0o600
        || meta.nlink() != 1
    {
        return Err(STORAGE.into());
    }
    Ok(file)
}
#[cfg(unix)]
fn read_state(dir: &File) -> Result<Option<State>, String> {
    use std::os::fd::AsRawFd;
    // Distinguish missing from unsafe/inaccessible; never seed over a bad file.
    let name = std::ffi::CString::new("credentials.json").unwrap();
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe {
        libc::fstatat(
            dir.as_raw_fd(),
            name.as_ptr(),
            stat.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } != 0
    {
        if std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound {
            return Ok(None);
        }
        return Err(STORAGE.into());
    }
    let file = open_private(dir, "credentials.json", false)?;
    if file.metadata().map_err(|_| STORAGE)?.len() > LIMIT {
        return Err(STORAGE.into());
    }
    let mut bytes = Vec::new();
    file.take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| STORAGE)?;
    if bytes.len() as u64 > LIMIT {
        return Err(STORAGE.into());
    }
    let state = serde_json::from_slice(&bytes).map_err(|_| STORAGE)?;
    validate_state(&state)?;
    Ok(Some(state))
}
#[cfg(unix)]
fn persist(dir: &File, state: &State) -> Result<(), String> {
    use std::os::fd::{AsRawFd, FromRawFd};
    let bytes = serde_json::to_vec(state).map_err(|_| STORAGE)?;
    if bytes.len() as u64 > LIMIT {
        return Err("credential capacity limit".into());
    }
    let temp = std::ffi::CString::new(format!("credentials-{}.tmp", revision())).unwrap();
    let target = std::ffi::CString::new("credentials.json").unwrap();
    let fd = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            temp.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    };
    if fd < 0 {
        return Err(STORAGE.into());
    }
    let mut file = unsafe { File::from_raw_fd(fd) };
    let result = (|| {
        file.write_all(&bytes).map_err(|_| STORAGE)?;
        file.sync_all().map_err(|_| STORAGE)?;
        if unsafe {
            libc::renameat(
                dir.as_raw_fd(),
                temp.as_ptr(),
                dir.as_raw_fd(),
                target.as_ptr(),
            )
        } != 0
        {
            return Err(STORAGE.into());
        }
        dir.sync_all().map_err(|_| STORAGE)?;
        Ok(())
    })();
    if result.is_err() {
        unsafe {
            libc::unlinkat(dir.as_raw_fd(), temp.as_ptr(), 0);
        }
    }
    result
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};

    fn identity() -> CredentialIdentity {
        CredentialIdentity {
            id: "fixture-token".into(),
            origin: "https://example.com".into(),
            header: "authorization".into(),
            tenant: "fixture-tenant".into(),
            principal: "fixture-reader".into(),
            scope: vec!["read".into()],
        }
    }
    fn context() -> CredentialContext<'static> {
        CredentialContext {
            app_id: "app",
            source_id: "source",
            config_digest: "compiled-v1",
            origin: "https://example.com",
        }
    }
    fn binding(metadata: &BindingMetadata) -> CredentialBinding {
        CredentialBinding {
            reference: metadata.identity.id.clone(),
            header: metadata.identity.header.clone(),
            scheme: Some("Bearer".into()),
            expected_identity_revision: Some(metadata.identity_revision.clone()),
        }
    }
    fn path(project: &Path, base: &Path) -> PathBuf {
        let journal = refact_scheduler_core::automation::journal_path(project).unwrap();
        base.join("refact-automation-runtime/v1")
            .join(journal.parent().unwrap().file_name().unwrap())
            .join("credentials.json")
    }

    #[test]
    fn persistent_rotation_is_identity_bound_and_metadata_is_value_free() {
        let project = tempfile::tempdir().unwrap();
        let private = tempfile::tempdir().unwrap();
        let store = AutomationCredentialStore::open(project.path(), Some(private.path())).unwrap();
        let m = store
            .set(identity(), "fixture-secret-first".into(), None)
            .unwrap();
        let b = binding(&m);
        assert!(store.resolve(&context(), &b).is_err());
        store
            .authorize_binding(&context(), &b, &m.identity_revision)
            .unwrap();
        let value = store.resolve(&context(), &b).unwrap();
        assert_eq!(value, "fixture-secret-first");
        assert!(value.is_sensitive());
        assert!(!format!("{value:?}").contains("fixture-secret-first"));
        let serialized = serde_json::to_string(&store.list_metadata().unwrap()).unwrap();
        assert!(!serialized.contains("fixture-secret-first"));
        assert!(!serialized.contains("\"secret\":"));
        let rotated = store
            .rotate(
                &m.identity.id,
                &m.secret_revision,
                "fixture-secret-second".into(),
            )
            .unwrap();
        assert_eq!(m.identity_revision, rotated.identity_revision);
        assert_ne!(m.secret_revision, rotated.secret_revision);
        assert!(store
            .rotate(&m.identity.id, &m.secret_revision, "stale".into())
            .is_err());
        drop(store);
        let store = AutomationCredentialStore::open(project.path(), Some(private.path())).unwrap();
        assert_eq!(
            store.resolve(&context(), &b).unwrap(),
            "fixture-secret-second"
        );
        let mut broader = identity();
        broader.scope.push("write".into());
        let updated = store
            .set(
                broader,
                "fixture-secret-third".into(),
                Some(&rotated.secret_revision),
            )
            .unwrap();
        assert_ne!(updated.identity_revision, m.identity_revision);
        assert!(store.resolve(&context(), &b).is_err());
        assert!(store
            .authorize_binding(&context(), &b, &m.identity_revision)
            .is_err());
        let new_binding = binding(&updated);
        assert!(store.resolve(&context(), &new_binding).is_err());
        store
            .authorize_binding(&context(), &new_binding, &updated.identity_revision)
            .unwrap();
        assert!(store.resolve(&context(), &new_binding).is_ok());
        store
            .remove(&updated.identity.id, &updated.secret_revision)
            .unwrap();
        assert!(store.resolve(&context(), &new_binding).is_err());
        // Reusing the textual ID never resurrects the previous identity grant.
        store
            .set(identity(), "fixture-secret-fourth".into(), None)
            .unwrap();
        assert!(store.resolve(&context(), &b).is_err());
        assert!(store.resolve(&context(), &new_binding).is_err());
    }

    #[test]
    fn tenant_and_principal_changes_require_new_explicit_approval() {
        for change_tenant in [false, true] {
            let project = tempfile::tempdir().unwrap();
            let private = tempfile::tempdir().unwrap();
            let store =
                AutomationCredentialStore::open(project.path(), Some(private.path())).unwrap();
            let m = store
                .set(identity(), "fixture-original".into(), None)
                .unwrap();
            let b = binding(&m);
            store
                .authorize_binding(&context(), &b, &m.identity_revision)
                .unwrap();
            let mut changed = identity();
            if change_tenant {
                changed.tenant = "other-tenant".into();
            } else {
                changed.principal = "other-principal".into();
            }
            let next = store
                .set(changed, "fixture-next".into(), Some(&m.secret_revision))
                .unwrap();
            assert_ne!(m.identity_revision, next.identity_revision);
            assert!(store.resolve(&context(), &b).is_err());
            assert!(store.resolve(&context(), &binding(&next)).is_err());
            drop(store);
            let store =
                AutomationCredentialStore::open(project.path(), Some(private.path())).unwrap();
            assert!(store.resolve(&context(), &b).is_err());
            assert!(store.resolve(&context(), &binding(&next)).is_err());
        }
    }

    #[test]
    fn exact_context_header_scheme_and_revision_required() {
        let project = tempfile::tempdir().unwrap();
        let private = tempfile::tempdir().unwrap();
        let store = AutomationCredentialStore::open(project.path(), Some(private.path())).unwrap();
        let m = store.set(identity(), "fixture-only".into(), None).unwrap();
        let b = binding(&m);
        store
            .authorize_binding(&context(), &b, &m.identity_revision)
            .unwrap();
        for field in 0..4 {
            let mut c = context();
            match field {
                0 => c.app_id = "other",
                1 => c.source_id = "other",
                2 => c.config_digest = "other",
                _ => c.origin = "https://other.example",
            }
            assert_eq!(store.resolve(&c, &b).unwrap_err(), DENIED);
        }
        for field in 0..4 {
            let mut changed = b.clone();
            match field {
                0 => changed.header = "x-api-key".into(),
                1 => changed.scheme = None,
                2 => changed.expected_identity_revision = None,
                _ => changed.reference = "unknown".into(),
            }
            assert!(store.resolve(&context(), &changed).is_err());
        }
        store.revoke_binding(&context(), &b).unwrap();
        assert!(store.resolve(&context(), &b).is_err());
        drop(store);
        let store = AutomationCredentialStore::open(project.path(), Some(private.path())).unwrap();
        assert!(store.resolve(&context(), &b).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reopen_after_drop_does_not_wait_for_fork_child_to_exec() {
        use std::os::fd::AsRawFd;
        use std::time::{Duration, Instant};

        struct ChildGuard {
            ready: [libc::c_int; 2],
            release: [libc::c_int; 2],
            pid: libc::pid_t,
            status: Option<libc::c_int>,
        }
        impl ChildGuard {
            fn close(fd: &mut libc::c_int) {
                if *fd >= 0 {
                    unsafe { libc::close(*fd) };
                    *fd = -1;
                }
            }
            fn running(&mut self) -> Result<bool, String> {
                if self.status.is_some() {
                    return Ok(false);
                }
                loop {
                    let mut status = 0;
                    let rc = unsafe { libc::waitpid(self.pid, &mut status, libc::WNOHANG) };
                    if rc == self.pid {
                        self.status = Some(status);
                        return Ok(false);
                    }
                    if rc == 0 {
                        return Ok(true);
                    }
                    let error = std::io::Error::last_os_error();
                    if error.raw_os_error() != Some(libc::EINTR) {
                        if error.raw_os_error() == Some(libc::ECHILD) {
                            self.pid = -1;
                        }
                        return Err(error.to_string());
                    }
                }
            }
            fn ready(&mut self) -> Result<(), String> {
                let deadline = Instant::now() + Duration::from_secs(5);
                loop {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        return Err("child READY timeout".into());
                    }
                    let mut fd = libc::pollfd {
                        fd: self.ready[0],
                        events: libc::POLLIN,
                        revents: 0,
                    };
                    let rc = unsafe { libc::poll(&mut fd, 1, remaining.as_millis().max(1) as i32) };
                    if rc < 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR)
                    {
                        continue;
                    }
                    if rc <= 0 || fd.revents & libc::POLLIN == 0 {
                        return Err("child READY unavailable".into());
                    }
                    let mut byte = 0u8;
                    let rc = unsafe { libc::read(self.ready[0], (&mut byte as *mut u8).cast(), 1) };
                    if rc < 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR)
                    {
                        continue;
                    }
                    return if rc == 1 && byte == b'R' && self.running()? {
                        Ok(())
                    } else {
                        Err("invalid READY or exited child".into())
                    };
                }
            }
            fn finish(&mut self) -> Result<(), String> {
                let byte = b'G';
                let sent = loop {
                    let rc =
                        unsafe { libc::write(self.release[1], (&byte as *const u8).cast(), 1) };
                    if rc < 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR)
                    {
                        continue;
                    }
                    break rc == 1;
                };
                Self::close(&mut self.release[1]);
                let deadline = Instant::now() + Duration::from_secs(5);
                while self.running()? {
                    if Instant::now() >= deadline {
                        return Err("child exit timeout".into());
                    }
                    std::thread::yield_now();
                }
                match self.status {
                    Some(status)
                        if sent && libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0 =>
                    {
                        Ok(())
                    }
                    _ => Err("child failed RELEASE protocol".into()),
                }
            }
        }
        impl Drop for ChildGuard {
            fn drop(&mut self) {
                for fd in self.ready.iter_mut().chain(self.release.iter_mut()) {
                    Self::close(fd);
                }
                if self.pid <= 0 || self.status.is_some() {
                    return;
                }
                let deadline = Instant::now() + Duration::from_secs(2);
                while matches!(self.running(), Ok(true)) && Instant::now() < deadline {
                    std::thread::yield_now();
                }
                if self.pid > 0 && self.status.is_none() {
                    unsafe { libc::kill(self.pid, libc::SIGKILL) };
                    let mut status = 0;
                    loop {
                        let rc = unsafe { libc::waitpid(self.pid, &mut status, 0) };
                        if rc == self.pid {
                            self.status = Some(status);
                            break;
                        }
                        if rc < 0
                            && std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR)
                        {
                            continue;
                        }
                        break;
                    }
                }
            }
        }
        let project = tempfile::tempdir().unwrap();
        let private = tempfile::tempdir().unwrap();
        let store = AutomationCredentialStore::open(project.path(), Some(private.path())).unwrap();
        let m = store.set(identity(), "fixture-only".into(), None).unwrap();
        let b = binding(&m);
        store
            .authorize_binding(&context(), &b, &m.identity_revision)
            .unwrap();
        store.revoke_binding(&context(), &b).unwrap();
        drop(store.clone());
        let contender = open_private(&store.0.directory, "credentials.lock", false).unwrap();
        assert_eq!(
            unsafe { libc::flock(contender.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
            -1
        );
        assert!(
            matches!(std::io::Error::last_os_error().raw_os_error(), Some(e) if e == libc::EWOULDBLOCK || e == libc::EAGAIN)
        );
        drop(contender);
        let mut child = ChildGuard {
            ready: [-1; 2],
            release: [-1; 2],
            pid: -1,
            status: None,
        };
        assert_eq!(
            unsafe { libc::pipe2(child.ready.as_mut_ptr(), libc::O_CLOEXEC) },
            0
        );
        assert_eq!(
            unsafe { libc::pipe2(child.release.as_mut_ptr(), libc::O_CLOEXEC) },
            0
        );
        child.pid = unsafe { libc::fork() };
        assert!(child.pid >= 0);
        if child.pid == 0 {
            unsafe {
                libc::close(child.ready[0]);
                libc::close(child.release[1]);
                let ready = b'R';
                loop {
                    let rc = libc::write(child.ready[1], (&ready as *const u8).cast(), 1);
                    if rc == 1 {
                        break;
                    }
                    if rc < 0 && *libc::__errno_location() == libc::EINTR {
                        continue;
                    }
                    libc::_exit(11);
                }
                let mut release = 0u8;
                loop {
                    let rc = libc::read(child.release[0], (&mut release as *mut u8).cast(), 1);
                    if rc == 1 {
                        break;
                    }
                    if rc < 0 && *libc::__errno_location() == libc::EINTR {
                        continue;
                    }
                    libc::_exit(12);
                }
                libc::_exit(if release == b'G' { 0 } else { 13 });
            }
        }
        ChildGuard::close(&mut child.ready[1]);
        ChildGuard::close(&mut child.release[0]);
        child.ready().unwrap();
        let alive_before = child.running();
        drop(store);
        let reopened = AutomationCredentialStore::open(project.path(), Some(private.path()));
        let captures = reopened.as_ref().map(|store| {
            let revoked = store.resolve(&context(), &b);
            let contender =
                open_private(&store.0.directory, "credentials.lock", false).map(|file| {
                    let rc =
                        unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
                    (rc, std::io::Error::last_os_error().raw_os_error())
                });
            (revoked, contender)
        });
        let alive_after = child.running();
        let cleanup = child.finish();
        drop(child);
        cleanup.unwrap();
        assert_eq!(alive_before, Ok(true));
        assert_eq!(alive_after, Ok(true));
        let (revoked, contender) =
            captures.expect("fork child must not retain a dropped host capability lock");
        assert_eq!(revoked.unwrap_err(), DENIED);
        let (rc, errno) = contender.unwrap();
        assert_eq!(rc, -1);
        assert!(matches!(errno, Some(e) if e == libc::EWOULDBLOCK || e == libc::EAGAIN));
    }

    #[test]
    fn private_permissions_and_unsafe_seed_rejected_without_following_links() {
        let project = tempfile::tempdir().unwrap();
        let private = tempfile::tempdir().unwrap();
        let store = AutomationCredentialStore::open(project.path(), Some(private.path())).unwrap();
        let location = path(project.path(), private.path());
        assert!(!location.starts_with(project.path()));
        assert_eq!(std::fs::metadata(&location).unwrap().mode() & 0o777, 0o600);
        assert_eq!(
            std::fs::metadata(location.parent().unwrap())
                .unwrap()
                .mode()
                & 0o777,
            0o700
        );
        drop(store);
        let external = private.path().join("external-fixture");
        std::fs::write(&external, b"unchanged-fixture").unwrap();
        std::fs::remove_file(&location).unwrap();
        symlink(&external, &location).unwrap();
        assert!(AutomationCredentialStore::open(project.path(), Some(private.path())).is_err());
        assert_eq!(std::fs::read(&external).unwrap(), b"unchanged-fixture");
        std::fs::remove_file(&location).unwrap();
        std::fs::hard_link(&external, &location).unwrap();
        std::fs::set_permissions(&external, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(AutomationCredentialStore::open(project.path(), Some(private.path())).is_err());
        std::fs::remove_file(&location).unwrap();
        std::fs::write(&location, b"{}").unwrap();
        std::fs::set_permissions(&location, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(AutomationCredentialStore::open(project.path(), Some(private.path())).is_err());
    }

    #[test]
    fn project_local_and_symlinked_private_directory_are_rejected() {
        let project = tempfile::tempdir().unwrap();
        assert!(AutomationCredentialStore::open(project.path(), Some(project.path())).is_err());
        let private = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        symlink(
            elsewhere.path(),
            private.path().join("refact-automation-runtime"),
        )
        .unwrap();
        assert!(AutomationCredentialStore::open(project.path(), Some(private.path())).is_err());
    }

    #[test]
    fn memory_only_resolution_and_disk_cas_conflict() {
        let project = tempfile::tempdir().unwrap();
        let private = tempfile::tempdir().unwrap();
        let store = AutomationCredentialStore::open(project.path(), Some(private.path())).unwrap();
        let m = store
            .set(identity(), "fixture-memory".into(), None)
            .unwrap();
        let b = binding(&m);
        store
            .authorize_binding(&context(), &b, &m.identity_revision)
            .unwrap();
        let location = path(project.path(), private.path());
        let mut state: State = serde_json::from_slice(&std::fs::read(&location).unwrap()).unwrap();
        state.revision = revision();
        std::fs::write(&location, serde_json::to_vec(&state).unwrap()).unwrap();
        assert!(store
            .rotate(&m.identity.id, &m.secret_revision, "fixture-new".into())
            .is_err());
        std::fs::remove_file(&location).unwrap();
        // Removing backing file doesn't trigger secret IO on the collector path.
        assert_eq!(store.resolve(&context(), &b).unwrap(), "fixture-memory");
    }
}

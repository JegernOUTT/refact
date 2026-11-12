use crate::{AppManifest, AppState, StateScope, validate_identifier};
use cap_std::fs::{Dir, OpenOptions};
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap},
    io::{Read, Write},
    path::{Path},
    sync::Arc,
};
use uuid::Uuid;

pub type AppResult<T> = Result<T, String>;
pub const MAX_FILE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_BUNDLE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_FILES: usize = 512;
const MANIFEST: &str = "app.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppRecord {
    pub slug: String,
    pub manifest: AppManifest,
    pub revision: String,
    pub active_revision: Option<String>,
    pub files: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppDiagnostic {
    pub slug: String,
    pub message: String,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppListing {
    pub apps: Vec<AppRecord>,
    pub diagnostics: Vec<AppDiagnostic>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublishedBundle {
    pub manifest: AppManifest,
    pub revision: String,
    pub files: BTreeMap<String, Vec<u8>>,
}
#[derive(Clone)]
pub struct AppStore {
    root: Arc<Dir>,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

pub fn validate_resource_path(path: &str) -> AppResult<()> {
    if path.is_empty()
        || path.len() > 512
        || path.contains('\\')
        || path.contains(':')
        || path
            .split('/')
            .any(|p| p.is_empty() || p.starts_with('.') || p.bytes().any(|b| b < 32))
    {
        return Err("unsafe resource path".into());
    }
    if path != MANIFEST
        && path != "README.md"
        && !["ui/", "assets/", "sources/", "scripts/"]
            .iter()
            .any(|p| path.starts_with(p))
    {
        return Err(
            "resource must be app.json, README.md, ui/*, assets/*, sources/* or scripts/*".into(),
        );
    }
    Ok(())
}
fn slug_ok(slug: &str) -> AppResult<()> {
    validate_identifier(slug)?;
    if slug.starts_with('-') || slug.bytes().any(|b| b.is_ascii_uppercase()) {
        return Err("slug must be lowercase and not start with '-'".into());
    }
    Ok(())
}
fn child(dir: &Dir, name: &str, create: bool) -> AppResult<Dir> {
    if create {
        match dir.create_dir(name) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(err(e)),
        }
    }
    dir.open_dir_nofollow(name).map_err(err)
}
fn read(dir: &Dir, path: &str, limit: usize) -> AppResult<Vec<u8>> {
    read_optional_limit(dir, path, Some(limit))
}
fn read_optional_limit(dir: &Dir, path: &str, limit: Option<usize>) -> AppResult<Vec<u8>> {
    let (parent, leaf) = parent(dir, path, false)?;
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let before = parent.symlink_metadata(leaf).map_err(err)?;
    if !before.is_file() {
        return Err("not a regular file".into());
    }
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let mut file = parent.open_with(leaf, &options).map_err(err)?;
    let metadata = file.metadata().map_err(err)?;
    if !metadata.is_file() || limit.is_some_and(|limit| metadata.len() > limit as u64) {
        return Err("not a bounded regular file".into());
    }
    #[cfg(unix)]
    {
        use cap_std::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err("hard-linked resources refused".into());
        }
    }
    let mut bytes = Vec::new();
    if let Some(limit) = limit {
        Read::by_ref(&mut file)
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(err)?;
    } else {
        file.read_to_end(&mut bytes).map_err(err)?;
    }
    if limit.is_some_and(|limit| bytes.len() > limit) {
        return Err("file exceeds limit".into());
    }
    Ok(bytes)
}
fn parent<'a>(dir: &Dir, path: &'a str, create: bool) -> AppResult<(Dir, &'a str)> {
    let mut parts = path.split('/').collect::<Vec<_>>();
    let leaf = parts.pop().ok_or("empty path")?;
    let mut current = dir.try_clone().map_err(err)?;
    for part in parts {
        current = child(&current, part, create)?;
    }
    Ok((current, leaf))
}
fn atomic(dir: &Dir, path: &str, bytes: &[u8]) -> AppResult<()> {
    let (parent, leaf) = parent(dir, path, true)?;
    let tmp = format!(".tmp-{}", Uuid::new_v4());
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No);
    let result = (|| {
        let mut file = parent.open_with(&tmp, &options).map_err(err)?;
        file.write_all(bytes).map_err(err)?;
        file.sync_all().map_err(err)?;
        drop(file);
        parent.rename(&tmp, &parent, leaf).map_err(err)?;
        // Persist the rename before reporting success on Unix.
        #[cfg(unix)]
        parent.open(".").map_err(err)?.sync_all().map_err(err)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = parent.remove_file(&tmp);
    }
    result
}
fn walk(
    dir: &Dir,
    prefix: &str,
    files: &mut BTreeMap<String, Vec<u8>>,
    total: &mut usize,
) -> AppResult<()> {
    if prefix.len() > 512 {
        return Err("resource nesting limit exceeded".into());
    }
    for entry in dir.entries().map_err(err)? {
        let entry = entry.map_err(err)?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "non-UTF8 resource")?;
        if prefix.is_empty() && name == ".local" {
            continue;
        }
        let path = format!("{prefix}{name}");
        let ty = entry.file_type().map_err(err)?;
        if ty.is_symlink() {
            return Err("symlink resource refused".into());
        }
        if ty.is_dir() {
            if path != "ui" && path != "assets" && path != "sources" && path != "scripts" {
                validate_resource_path(&format!("{path}/placeholder"))?;
            }
            walk(
                &child(dir, &name, false)?,
                &format!("{path}/"),
                files,
                total,
            )?;
        } else {
            validate_resource_path(&path)?;
            let bytes = read(dir, &name, MAX_FILE_BYTES)?;
            *total += bytes.len();
            if *total > MAX_BUNDLE_BYTES || files.len() >= MAX_FILES {
                return Err("bundle limit exceeded".into());
            }
            files.insert(path, bytes);
        }
    }
    Ok(())
}
fn digest(files: &BTreeMap<String, Vec<u8>>) -> String {
    let mut hash = Sha256::new();
    for (path, bytes) in files {
        hash.update((path.len() as u64).to_be_bytes());
        hash.update(path);
        hash.update((bytes.len() as u64).to_be_bytes());
        hash.update(bytes);
    }
    hex::encode(hash.finalize())
}
fn bundle(dir: &Dir) -> AppResult<PublishedBundle> {
    let mut files = BTreeMap::new();
    walk(dir, "", &mut files, &mut 0)?;
    let manifest: AppManifest =
        serde_json::from_slice(files.get(MANIFEST).ok_or("missing app.json")?).map_err(err)?;
    manifest.validate()?;
    if !files.contains_key(&manifest.entrypoint)
        || manifest
            .icon
            .as_ref()
            .is_some_and(|p| !p.starts_with("lucide:") && !files.contains_key(p))
    {
        return Err("missing entrypoint or icon".into());
    }
    Ok(PublishedBundle {
        revision: digest(&files),
        manifest,
        files,
    })
}
fn active(dir: &Dir) -> AppResult<Option<String>> {
    if !dir.try_exists(".local").map_err(err)? {
        return Ok(None);
    }
    let local = child(dir, ".local", false)?;
    if !local.try_exists("active.json").map_err(err)? {
        return Ok(None);
    }
    let value: String = serde_json::from_slice(&read(&local, "active.json", 1024)?).map_err(err)?;
    revision_ok(&value)?;
    Ok(Some(value))
}
fn revision_ok(revision: &str) -> AppResult<()> {
    if revision.len() != 64
        || !revision
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err("invalid revision".into());
    }
    Ok(())
}
fn record(root: &Dir, slug: &str) -> AppResult<AppRecord> {
    record_projected(root, slug, &|_| ()).map(|(record, ())| record)
}
fn record_projected<T>(
    root: &Dir,
    slug: &str,
    project: &impl Fn(&PublishedBundle) -> T,
) -> AppResult<(AppRecord, T)> {
    slug_ok(slug)?;
    let dir = child(root, slug, false)?;
    let b = bundle(&dir)?;
    if b.manifest.app_id != identity(root, slug)? {
        return Err("draft app_id differs from published identity".into());
    }
    let projection = project(&b);
    Ok((
        AppRecord {
            slug: slug.into(),
            manifest: b.manifest,
            revision: b.revision,
            active_revision: active(&dir)?,
            files: b.files.into_keys().collect(),
        },
        projection,
    ))
}
fn listing(root: &Dir) -> AppResult<AppListing> {
    listing_projected(root, &|_| ()).map(|(listing, _)| listing)
}
fn listing_projected<T>(
    root: &Dir,
    project: &impl Fn(&PublishedBundle) -> T,
) -> AppResult<(AppListing, BTreeMap<String, T>)> {
    let mut projections = BTreeMap::new();
    let mut output = AppListing::default();
    for entry in root.entries().map_err(err)? {
        let entry = entry.map_err(err)?;
        let slug = entry.file_name().to_string_lossy().into_owned();
        if slug.starts_with('.') {
            continue;
        }
        match record_projected(root, &slug, project) {
            Ok((app, projection)) => {
                projections.insert(slug.clone(), projection);
                output.apps.push(app);
            }
            Err(message) => output.diagnostics.push(AppDiagnostic {
                slug,
                message,
            }),
        }
    }
    let mut counts = BTreeMap::new();
    for entry in root.entries().map_err(err)? {
        let entry = entry.map_err(err)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        if let Ok(id) = identity(root, &name) {
            *counts.entry(id).or_insert(0) += 1;
        }
    }
    output.apps.retain(|app| {
        if counts[&app.manifest.app_id] > 1 {
            output.diagnostics.push(AppDiagnostic {
                slug: app.slug.clone(),
                message: "duplicate app_id".into(),
            });
            false
        } else {
            true
        }
    });
    output.apps.sort_by(|a, b| a.slug.cmp(&b.slug));
    output.diagnostics.sort_by(|a, b| a.slug.cmp(&b.slug));
    projections.retain(|slug, _| output.apps.iter().any(|app| &app.slug == slug));
    Ok((output, projections))
}
fn identity(root: &Dir, slug: &str) -> AppResult<Uuid> {
    slug_ok(slug)?;
    let dir = child(root, slug, false)?;
    if let Some(revision) = active(&dir)? {
        return Ok(load_revision(&dir, &revision)?.manifest.app_id);
    }
    if let Ok(bytes) = read(&dir, MANIFEST, MAX_FILE_BYTES) {
        if let Ok(manifest) = serde_json::from_slice::<AppManifest>(&bytes) {
            manifest.validate()?;
            return Ok(manifest.app_id);
        }
    }
    Err("no valid app identity".into())
}
fn published_identity(root: &Dir, slug: &str) -> AppResult<Uuid> {
    let id = identity(root, slug)?;
    for entry in root.entries().map_err(err)? {
        let entry = entry.map_err(err)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name != slug && !name.starts_with('.') && identity(root, &name).ok() == Some(id) {
            return Err("duplicate app_id".into());
        }
    }
    Ok(id)
}
fn unique(root: &Dir, slug: &str) -> AppResult<AppRecord> {
    let list = listing(root)?;
    list.apps
        .into_iter()
        .find(|a| a.slug == slug)
        .ok_or_else(|| {
            list.diagnostics
                .into_iter()
                .find(|d| d.slug == slug)
                .map_or("app not found".into(), |d| d.message)
        })
}
fn expect(actual: &str, expected: &str) -> AppResult<()> {
    if actual != expected {
        Err("revision conflict".into())
    } else {
        Ok(())
    }
}

impl AppStore {
    pub async fn open(project_root: impl AsRef<Path>) -> AppResult<Self> {
        let path = project_root.as_ref().to_path_buf();
        tokio::task::spawn_blocking(move || {
            let project = Dir::open_ambient_dir(
                std::fs::canonicalize(path).map_err(err)?,
                cap_std::ambient_authority(),
            )
            .map_err(err)?;
            let config = child(&project, ".refact", true)?;
            let root = child(&config, "apps", true)?;
            Ok(Self {
                root: Arc::new(root),
            })
        })
        .await
        .map_err(err)?
    }
    async fn transaction<T: Send + 'static>(
        &self,
        op: impl FnOnce(&Dir) -> AppResult<T> + Send + 'static,
    ) -> AppResult<T> {
        let root = self.root.clone();
        tokio::task::spawn_blocking(move || {
            let mut options = OpenOptions::new();
            options
                .read(true)
                .write(true)
                .create(true)
                .follow(FollowSymlinks::No);
            let file = root
                .open_with(".store.lock", &options)
                .map_err(err)?
                .into_std();
            let mut lock = fd_lock::RwLock::new(file);
            let _guard = lock.write().map_err(err)?;
            op(&root)
        })
        .await
        .map_err(err)?
    }
    pub async fn list(&self) -> AppResult<AppListing> {
        self.transaction(listing).await
    }
    /// Project the exact draft bundle already read for each catalog record, under the
    /// same store transaction. Callers must return only bounded inert metadata.
    pub async fn list_projected<T: Send + 'static>(
        &self,
        project: impl Fn(&PublishedBundle) -> T + Send + 'static,
    ) -> AppResult<(AppListing, BTreeMap<String, T>)> {
        self.transaction(move |root| listing_projected(root, &project))
            .await
    }
    pub async fn get(&self, slug: &str) -> AppResult<AppRecord> {
        let slug = slug.to_owned();
        self.transaction(move |r| unique(r, &slug)).await
    }
    pub async fn get_by_id(&self, app_id: Uuid) -> AppResult<AppRecord> {
        self.transaction(move |root| {
            let mut found = None;
            for entry in root.entries().map_err(err)? {
                let name = entry
                    .map_err(err)?
                    .file_name()
                    .to_string_lossy()
                    .into_owned();
                if name.starts_with('.') || identity(root, &name).ok() != Some(app_id) {
                    continue;
                }
                if found.is_some() {
                    return Err("duplicate app_id".into());
                }
                let app = match record(root, &name) {
                    Ok(app) => app,
                    Err(_) => {
                        let dir = child(root, &name, false)?;
                        let revision = active(&dir)?.ok_or("invalid unpublished draft")?;
                        let b = load_revision(&dir, &revision)?;
                        AppRecord {
                            slug: name,
                            manifest: b.manifest,
                            revision: revision.clone(),
                            active_revision: Some(revision),
                            files: b.files.into_keys().collect(),
                        }
                    }
                };
                found = Some(app);
            }
            found.ok_or("app not found".into())
        })
        .await
    }
    pub async fn create(&self, slug: &str, name: &str) -> AppResult<AppRecord> {
        let slug = slug.to_owned();
        let name = name.to_owned();
        self.transaction(move |root| {
            slug_ok(&slug)?;
            let manifest = AppManifest { schema_version: 1, app_id: Uuid::new_v4(), name, description: String::new(), icon: None, entrypoint: "ui/index.html".into(), sources: vec![], actions: vec![], permissions: vec![] };
            manifest.validate()?;
            let files = BTreeMap::from([
                (MANIFEST.into(), serde_json::to_vec_pretty(&manifest).map_err(err)?),
                ("ui/index.html".into(), b"<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><link rel=\"stylesheet\" href=\"style.css\"></head><body><main id=\"app\">New app</main><script src=\"app.js\"></script></body></html>".to_vec()),
                ("ui/style.css".into(), b"body { font-family: system-ui; margin: 1rem; }".to_vec()),
                ("ui/app.js".into(), b"'use strict';\n".to_vec()),
            ]);
            install(root, &slug, &files)?; record(root, &slug)
        }).await
    }
    pub async fn read_file(&self, slug: &str, path: &str) -> AppResult<Vec<u8>> {
        let slug = slug.to_owned();
        let path = path.to_owned();
        self.transaction(move |root| {
            unique(root, &slug)?;
            validate_resource_path(&path)?;
            read(&child(root, &slug, false)?, &path, MAX_FILE_BYTES)
        })
        .await
    }
    pub async fn write_file(
        &self,
        slug: &str,
        path: &str,
        bytes: Vec<u8>,
        expected_revision: &str,
    ) -> AppResult<AppRecord> {
        let slug = slug.to_owned();
        let path = path.to_owned();
        let expected_revision = expected_revision.to_owned();
        self.transaction(move |root| {
            let app = unique(root, &slug)?;
            expect(&app.revision, &expected_revision)?;
            validate_resource_path(&path)?;
            if bytes.len() > MAX_FILE_BYTES {
                return Err("file exceeds limit".into());
            }
            let dir = child(root, &slug, false)?;
            let mut b = bundle(&dir)?;
            b.files.insert(path.clone(), bytes.clone());
            if b.files.values().map(Vec::len).sum::<usize>() > MAX_BUNDLE_BYTES
                || b.files.len() > MAX_FILES
            {
                return Err("bundle limit exceeded".into());
            }
            let manifest: AppManifest = serde_json::from_slice(&b.files[MANIFEST]).map_err(err)?;
            manifest.validate()?;
            if manifest.app_id != app.manifest.app_id {
                return Err("app_id is immutable".into());
            }
            if !b.files.contains_key(&manifest.entrypoint)
                || manifest
                    .icon
                    .as_ref()
                    .is_some_and(|p| !p.starts_with("lucide:") && !b.files.contains_key(p))
            {
                return Err("missing entrypoint or icon".into());
            }
            atomic(&dir, &path, &bytes)?;
            record(root, &slug)
        })
        .await
    }
    pub async fn delete_file(
        &self,
        slug: &str,
        path: &str,
        expected_revision: &str,
    ) -> AppResult<AppRecord> {
        let (slug, path, expected_revision) = (
            slug.to_owned(),
            path.to_owned(),
            expected_revision.to_owned(),
        );
        self.transaction(move |root| {
            let app = unique(root, &slug)?;
            expect(&app.revision, &expected_revision)?;
            validate_resource_path(&path)?;
            if path == MANIFEST
                || path == app.manifest.entrypoint
                || app.manifest.icon.as_deref() == Some(&path)
            {
                return Err("cannot remove manifest, entrypoint or icon".into());
            }
            let dir = child(root, &slug, false)?;
            let (parent, leaf) = parent(&dir, &path, false)?;
            parent.remove_file(leaf).map_err(err)?;
            #[cfg(unix)]
            parent.open(".").map_err(err)?.sync_all().map_err(err)?;
            record(root, &slug)
        })
        .await
    }
    pub async fn rename(
        &self,
        slug: &str,
        new_slug: &str,
        expected_revision: &str,
    ) -> AppResult<AppRecord> {
        let (slug, new_slug, expected_revision) = (
            slug.to_owned(),
            new_slug.to_owned(),
            expected_revision.to_owned(),
        );
        self.transaction(move |root| {
            let app = unique(root, &slug)?;
            expect(&app.revision, &expected_revision)?;
            slug_ok(&new_slug)?;
            if root.try_exists(&new_slug).map_err(err)? {
                return Err("destination exists".into());
            }
            root.rename(&slug, root, &new_slug).map_err(err)?;
            record(root, &new_slug)
        })
        .await
    }
    pub async fn clone_app(&self, slug: &str, new_slug: &str) -> AppResult<AppRecord> {
        let (slug, new_slug) = (slug.to_owned(), new_slug.to_owned());
        self.transaction(move |root| {
            unique(root, &slug)?;
            slug_ok(&new_slug)?;
            let mut b = bundle(&child(root, &slug, false)?)?;
            b.manifest.app_id = Uuid::new_v4();
            b.files.insert(
                MANIFEST.into(),
                serde_json::to_vec_pretty(&b.manifest).map_err(err)?,
            );
            install(root, &new_slug, &b.files)?;
            record(root, &new_slug)
        })
        .await
    }
    pub async fn archive(&self, slug: &str, expected_revision: &str) -> AppResult<String> {
        let (slug, expected_revision) = (slug.to_owned(), expected_revision.to_owned());
        self.transaction(move |root| {
            let app = unique(root, &slug)?;
            expect(&app.revision, &expected_revision)?;
            let archives = child(root, ".archives", true)?;
            let id = Uuid::new_v4().to_string();
            root.rename(&slug, &archives, &id).map_err(err)?;
            Ok(id)
        })
        .await
    }
    pub async fn remove(&self, slug: &str, expected_revision: &str) -> AppResult<()> {
        // Detach atomically before recursive deletion so interrupted removal cannot leave a half-valid app.
        let id = self.archive(slug, expected_revision).await?;
        self.transaction(move |root| {
            child(root, ".archives", false)?
                .remove_dir_all(id)
                .map_err(err)
        })
        .await
    }
    pub async fn publish(&self, slug: &str, expected_revision: &str) -> AppResult<PublishedBundle> {
        let (slug, expected_revision) = (slug.to_owned(), expected_revision.to_owned());
        self.transaction(move |root| {
            let app = unique(root, &slug)?;
            expect(&app.revision, &expected_revision)?;
            let dir = child(root, &slug, false)?;
            let b = bundle(&dir)?;
            let local = child(&dir, ".local", true)?;
            let revisions = child(&local, "revisions", true)?;
            let name = format!("{}.json", b.revision);
            if revisions.try_exists(&name).map_err(err)? {
                let old = load_revision(&dir, &b.revision)?;
                if old.files != b.files {
                    return Err("immutable revision collision".into());
                }
            } else {
                atomic(&revisions, &name, &serde_json::to_vec(&b).map_err(err)?)?;
            }
            atomic(
                &local,
                "active.json",
                &serde_json::to_vec(&b.revision).map_err(err)?,
            )?;
            Ok(b)
        })
        .await
    }
    pub async fn published(&self, slug: &str) -> AppResult<PublishedBundle> {
        let slug = slug.to_owned();
        self.transaction(move |root| {
            let id = published_identity(root, &slug)?;
            let dir = child(root, &slug, false)?;
            let b = load_revision(&dir, &active(&dir)?.ok_or("app not published")?)?;
            if b.manifest.app_id != id {
                return Err("published identity mismatch".into());
            }
            Ok(b)
        })
        .await
    }
    pub async fn rollback(
        &self,
        slug: &str,
        revision: &str,
        expected_active: Option<&str>,
    ) -> AppResult<PublishedBundle> {
        let (slug, revision, expected_active) = (
            slug.to_owned(),
            revision.to_owned(),
            expected_active.map(str::to_owned),
        );
        self.transaction(move |root| {
            let id = published_identity(root, &slug)?;
            let dir = child(root, &slug, false)?;
            if active(&dir)? != expected_active {
                return Err("active revision conflict".into());
            }
            let b = load_revision(&dir, &revision)?;
            if b.manifest.app_id != id {
                return Err("revision identity mismatch".into());
            }
            atomic(
                &child(&dir, ".local", false)?,
                "active.json",
                &serde_json::to_vec(&revision).map_err(err)?,
            )?;
            Ok(b)
        })
        .await
    }
    pub async fn read_published_file(&self, slug: &str, path: &str) -> AppResult<Vec<u8>> {
        validate_resource_path(path)?;
        self.published(slug)
            .await?
            .files
            .remove(path)
            .ok_or("published resource missing".into())
    }
    pub async fn read_provenance(&self, slug: &str) -> AppResult<AppState> {
        let slug = slug.to_owned();
        self.transaction(move |root| {
            published_identity(root, &slug)?;
            let dir = child(root, &slug, false)?;
            if !dir.try_exists(".local").map_err(err)? {
                return Ok(AppState::default());
            }
            let local = child(&dir, ".local", false)?;
            if !local.try_exists("provenance.json").map_err(err)? {
                return Ok(AppState::default());
            }
            serde_json::from_slice(&read(&local, "provenance.json", MAX_FILE_BYTES)?).map_err(err)
        })
        .await
    }

    pub async fn write_provenance(
        &self,
        slug: &str,
        expected_revision: u64,
        value: serde_json::Value,
    ) -> AppResult<AppState> {
        let slug = slug.to_owned();
        self.transaction(move |root| {
            published_identity(root, &slug)?;
            let dir = child(root, &slug, false)?;
            let local = child(&dir, ".local", true)?;
            let old: AppState = if local.try_exists("provenance.json").map_err(err)? {
                serde_json::from_slice(&read(&local, "provenance.json", MAX_FILE_BYTES)?)
                    .map_err(err)?
            } else {
                AppState::default()
            };
            if old.revision != expected_revision {
                return Err("provenance revision conflict".into());
            }
            let next = AppState {
                revision: old
                    .revision
                    .checked_add(1)
                    .ok_or("provenance revision overflow")?,
                value,
            };
            let bytes = serde_json::to_vec(&next).map_err(err)?;
            if bytes.len() > MAX_FILE_BYTES {
                return Err("provenance exceeds limit".into());
            }
            atomic(&local, "provenance.json", &bytes)?;
            Ok(next)
        })
        .await
    }

    pub async fn read_state(&self, slug: &str, scope: StateScope) -> AppResult<AppState> {
        let slug = slug.to_owned();
        self.transaction(move |root| {
            published_identity(root, &slug)?;
            state(&child(root, &slug, false)?, &scope)
        })
        .await
    }
    pub async fn write_state(
        &self,
        slug: &str,
        scope: StateScope,
        expected_revision: u64,
        value: serde_json::Value,
    ) -> AppResult<AppState> {
        let slug = slug.to_owned();
        self.transaction(move |root| {
            published_identity(root, &slug)?;
            let dir = child(root, &slug, false)?;
            let old = state(&dir, &scope)?;
            if old.revision != expected_revision {
                return Err("state revision conflict".into());
            }
            let result = AppState {
                revision: old
                    .revision
                    .checked_add(1)
                    .ok_or("state revision overflow")?,
                value,
            };
            let bytes = serde_json::to_vec(&result).map_err(err)?;
            atomic(&child(&dir, ".local", true)?, &state_path(&scope), &bytes)?;
            Ok(result)
        })
        .await
    }
}
fn install(root: &Dir, slug: &str, files: &BTreeMap<String, Vec<u8>>) -> AppResult<()> {
    if root.try_exists(slug).map_err(err)? {
        return Err("destination exists".into());
    }
    let staging = format!(".staging-{}", Uuid::new_v4());
    root.create_dir(&staging).map_err(err)?;
    let result = (|| {
        let dir = child(root, &staging, false)?;
        for (path, bytes) in files {
            atomic(&dir, path, bytes)?;
        }
        root.rename(&staging, root, slug).map_err(err)
    })();
    if result.is_err() {
        let _ = root.remove_dir_all(staging);
    }
    result
}
fn load_revision(dir: &Dir, revision: &str) -> AppResult<PublishedBundle> {
    revision_ok(revision)?;
    let revisions = child(&child(dir, ".local", false)?, "revisions", false)?;
    let b: PublishedBundle = serde_json::from_slice(&read(
        &revisions,
        &format!("{revision}.json"),
        MAX_BUNDLE_BYTES * 5,
    )?)
    .map_err(err)?;
    if b.revision != revision || digest(&b.files) != revision {
        return Err("revision integrity failure".into());
    }
    b.manifest.validate()?;
    let stored: AppManifest =
        serde_json::from_slice(b.files.get(MANIFEST).ok_or("revision manifest absent")?)
            .map_err(err)?;
    if serde_json::to_value(&stored).map_err(err)?
        != serde_json::to_value(&b.manifest).map_err(err)?
    {
        return Err("revision manifest mismatch".into());
    }
    let mut size = 0;
    for (path, bytes) in &b.files {
        validate_resource_path(path)?;
        if bytes.len() > MAX_FILE_BYTES {
            return Err("revision file too large".into());
        }
        size += bytes.len();
    }
    if size > MAX_BUNDLE_BYTES
        || b.files.len() > MAX_FILES
        || !b.files.contains_key(&b.manifest.entrypoint)
    {
        return Err("invalid revision bundle".into());
    }
    Ok(b)
}
fn state_path(scope: &StateScope) -> String {
    match scope {
        StateScope::Shared => "state/shared.json".into(),
        StateScope::Standalone => "state/standalone.json".into(),
        StateScope::Binding {
            binding_id,
        } => format!("state/binding-{binding_id}.json"),
    }
}
fn state(dir: &Dir, scope: &StateScope) -> AppResult<AppState> {
    if !dir.try_exists(".local").map_err(err)? {
        return Ok(AppState::default());
    }
    let local = child(dir, ".local", false)?;
    let path = state_path(scope);
    if !local.try_exists(&path).map_err(err)? {
        return Ok(AppState::default());
    }
    serde_json::from_slice(&read_optional_limit(&local, &path, None)?).map_err(err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn state_symlink_is_refused_for_read_and_write() {
        let project = tempfile::tempdir().unwrap();
        let store = AppStore::open(project.path()).await.unwrap();
        store.create("links", "Links").await.unwrap();
        store
            .write_state(
                "links",
                StateScope::Standalone,
                0,
                serde_json::json!("safe"),
            )
            .await
            .unwrap();
        let path = project
            .path()
            .join(".refact/apps/links/.local/state/standalone.json");
        let outside = project.path().join("outside.json");
        std::fs::write(&outside, br#"{"revision":1,"value":"outside"}"#).unwrap();
        std::fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(&outside, &path).unwrap();
        assert!(store
            .read_state("links", StateScope::Standalone)
            .await
            .is_err());
        assert!(store
            .write_state("links", StateScope::Standalone, 1, serde_json::Value::Null)
            .await
            .is_err());
        assert_eq!(
            std::fs::read(&outside).unwrap(),
            br#"{"revision":1,"value":"outside"}"#
        );
    }

    #[tokio::test]
    async fn large_state_roundtrips_after_reopen_and_obeys_cas() {
        let project = tempfile::tempdir().unwrap();
        let store = AppStore::open(project.path()).await.unwrap();
        store.create("large", "Large").await.unwrap();
        let value = serde_json::json!({"data": "x".repeat(4 * 1024 * 1024 + 1)});
        let written = store
            .write_state("large", StateScope::Standalone, 0, value.clone())
            .await
            .unwrap();
        assert_eq!(written.revision, 1);
        drop(store);
        let store = AppStore::open(project.path()).await.unwrap();
        let loaded = store
            .read_state("large", StateScope::Standalone)
            .await
            .unwrap();
        assert_eq!(loaded.value, value);
        assert_eq!(loaded.revision, 1);
        assert!(store
            .write_state("large", StateScope::Standalone, 0, serde_json::Value::Null)
            .await
            .is_err());
        assert_eq!(
            store
                .read_state("large", StateScope::Standalone)
                .await
                .unwrap()
                .value,
            value
        );
    }

    #[tokio::test]
    async fn named_icons_publish_without_files_but_resource_icons_must_exist() {
        let project = tempfile::tempdir().unwrap();
        let store = AppStore::open(project.path()).await.unwrap();
        let created = store.create("icons", "Icons").await.unwrap();
        let mut manifest = created.manifest.clone();
        manifest.icon = Some("lucide:timer".into());
        let record = store
            .write_file(
                "icons",
                "app.json",
                serde_json::to_vec(&manifest).unwrap(),
                &created.revision,
            )
            .await
            .unwrap();
        let published = store.publish("icons", &record.revision).await.unwrap();
        assert_eq!(published.manifest.icon.as_deref(), Some("lucide:timer"));
        assert!(!published.files.contains_key("lucide:timer"));
        manifest.icon = Some("assets/missing.svg".into());
        let failure = store
            .write_file(
                "icons",
                "app.json",
                serde_json::to_vec(&manifest).unwrap(),
                &record.revision,
            )
            .await
            .unwrap_err();
        assert_eq!(failure, "missing entrypoint or icon");
        assert_eq!(
            store.published("icons").await.unwrap().revision,
            published.revision
        );
    }

    #[tokio::test]
    async fn transaction_holds_cross_handle_exclusive_lock() {
        let project = tempfile::tempdir().unwrap();
        let store = AppStore::open(project.path()).await.unwrap();
        store
            .transaction(|root| {
                let mut options = OpenOptions::new();
                options.read(true).write(true).follow(FollowSymlinks::No);
                let file = root
                    .open_with(".store.lock", &options)
                    .map_err(err)?
                    .into_std();
                let mut contender = fd_lock::RwLock::new(file);
                assert!(
                    contender.try_write().is_err(),
                    "independent writer acquired lock during transaction"
                );
                Ok(())
            })
            .await
            .unwrap();
    }
}

use std::{
    collections::VecDeque,
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result, bail};
use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{
    boundary,
    store::{BoundaryPolicy, SharedEventStore},
};

static FILESYSTEM_MUTATION_LOCK: Mutex<()> = Mutex::new(());

const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_SEARCH_ENTRIES: usize = 20_000;
const MAX_SEARCH_FILES: usize = 10_000;
const MAX_SEARCH_BYTES: u64 = 64 * 1024 * 1024;
const MAX_SEARCH_DEPTH: usize = 32;

#[derive(Clone)]
pub struct FilesystemHost {
    store: SharedEventStore,
    effects: Arc<Mutex<Vec<FilesystemEffect>>>,
}

#[derive(Debug)]
pub struct FilesystemEffect {
    pub event_type: String,
    pub payload: Value,
}

#[derive(Debug, Deserialize)]
pub struct PathRequest {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct WriteRequest {
    pub path: String,
    pub content: String,
    pub expected_sha256: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PatchRequest {
    pub path: String,
    pub old_text: String,
    pub new_text: String,
    pub expected_sha256: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MoveRequest {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Deserialize)]
pub struct SearchRequest {
    pub path: String,
    pub query: String,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct FileContents {
    pub path: String,
    pub content: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Serialize)]
pub struct MutationResult {
    pub path: String,
    pub old_sha256: Option<String>,
    pub new_sha256: Option<String>,
    pub bytes: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct DirectoryEntry {
    pub name: String,
    pub path: String,
    pub kind: String,
    pub bytes: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct SearchMatch {
    pub path: String,
    pub line: usize,
    pub preview: String,
}

#[derive(Debug, Serialize)]
pub struct SearchResult {
    pub matches: Vec<SearchMatch>,
    pub files_scanned: usize,
    pub bytes_scanned: u64,
    pub truncated: bool,
}

impl FilesystemHost {
    pub fn new(store: SharedEventStore) -> Self {
        Self {
            store,
            effects: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn clear_effects(&self) -> Result<()> {
        self.effects
            .lock()
            .map_err(|_| anyhow::anyhow!("filesystem effect journal lock poisoned"))?
            .clear();
        Ok(())
    }

    pub fn take_effects(&self) -> Result<Vec<FilesystemEffect>> {
        Ok(std::mem::take(&mut *self.effects.lock().map_err(|_| {
            anyhow::anyhow!("filesystem effect journal lock poisoned")
        })?))
    }

    pub fn list(&self, request: PathRequest) -> Result<Vec<DirectoryEntry>> {
        let path = self.checked_existing(&request.path)?;
        if !path.is_dir() {
            bail!("'{}' is not a directory", path.display());
        }
        let (root, relative) = self.authorized_relative(&path)?;
        let directory = if relative.as_os_str().is_empty() {
            root
        } else {
            root.open_dir(&relative)?
        };
        let mut entries = directory
            .entries()?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        let policy = self.boundaries()?;
        entries
            .into_iter()
            .filter(|entry| {
                boundary::path_allowed(
                    &path.join(entry.file_name()),
                    &policy.directory_includes,
                    &policy.directory_excludes,
                    true,
                )
            })
            .map(|entry| {
                let file_type = entry.file_type()?;
                let metadata = entry.metadata()?;
                let kind = if file_type.is_symlink() {
                    "symlink"
                } else if file_type.is_dir() {
                    "directory"
                } else if file_type.is_file() {
                    "file"
                } else {
                    "special"
                };
                let name = entry.file_name();
                Ok(DirectoryEntry {
                    name: name.to_string_lossy().into_owned(),
                    path: utf8_path(&path.join(&name))?,
                    kind: kind.into(),
                    bytes: file_type.is_file().then_some(metadata.len()),
                })
            })
            .collect()
    }

    pub fn read(&self, request: PathRequest) -> Result<FileContents> {
        let path = self.checked_regular_file(&request.path)?;
        let (root, relative) = self.authorized_relative(&path)?;
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let mut file = root.open_with(&relative, &options)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
            bail!("file must be regular UTF-8 text no larger than 2 MiB");
        }
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        Read::take(&mut file, MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            bail!("file grew beyond the 2 MiB read limit");
        }
        let content = String::from_utf8(bytes.clone()).context("file is not UTF-8 text")?;
        Ok(FileContents {
            path: utf8_path(&path)?,
            content,
            bytes: bytes.len() as u64,
            sha256: sha256(&bytes),
        })
    }

    pub fn write(&self, request: WriteRequest) -> Result<MutationResult> {
        let _mutation = FILESYSTEM_MUTATION_LOCK
            .lock()
            .map_err(|_| anyhow::anyhow!("filesystem mutation lock poisoned"))?;
        if request.content.len() as u64 > MAX_FILE_BYTES {
            bail!("content exceeds the 2 MiB write limit");
        }
        let path = self.checked_write_path(&request.path)?;
        let old_sha256 = existing_hash(&path)?;
        check_expected_hash(request.expected_sha256.as_deref(), old_sha256.as_deref())?;
        self.atomic_write(&path, request.content.as_bytes(), old_sha256.as_deref())?;
        let result = MutationResult {
            path: utf8_path(&path)?,
            old_sha256,
            new_sha256: Some(sha256(request.content.as_bytes())),
            bytes: Some(request.content.len() as u64),
        };
        self.record_effect(
            if result.old_sha256.is_some() {
                "workspace.file.written"
            } else {
                "workspace.file.created"
            },
            json!({
                "path": result.path,
                "old_sha256": result.old_sha256,
                "new_sha256": result.new_sha256,
                "bytes": result.bytes,
            }),
        )?;
        Ok(result)
    }

    pub fn patch(&self, request: PatchRequest) -> Result<MutationResult> {
        let _mutation = FILESYSTEM_MUTATION_LOCK
            .lock()
            .map_err(|_| anyhow::anyhow!("filesystem mutation lock poisoned"))?;
        if request.old_text.is_empty() {
            bail!("old_text must not be empty");
        }
        let path = self.checked_regular_file(&request.path)?;
        let bytes = fs::read(&path)?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            bail!("file exceeds the 2 MiB patch limit");
        }
        let content = String::from_utf8(bytes.clone()).context("file is not UTF-8 text")?;
        let old_sha256 = sha256(&bytes);
        check_expected_hash(request.expected_sha256.as_deref(), Some(&old_sha256))?;
        let mut matches = content.match_indices(&request.old_text);
        let Some((start, _)) = matches.next() else {
            bail!("old_text was not found");
        };
        if matches.next().is_some() {
            bail!("old_text is not unique");
        }
        let mut updated =
            String::with_capacity(content.len() - request.old_text.len() + request.new_text.len());
        updated.push_str(&content[..start]);
        updated.push_str(&request.new_text);
        updated.push_str(&content[start + request.old_text.len()..]);
        if updated.len() as u64 > MAX_FILE_BYTES {
            bail!("patched content exceeds the 2 MiB write limit");
        }
        self.atomic_write(&path, updated.as_bytes(), Some(&old_sha256))?;
        let result = MutationResult {
            path: utf8_path(&path)?,
            old_sha256: Some(old_sha256),
            new_sha256: Some(sha256(updated.as_bytes())),
            bytes: Some(updated.len() as u64),
        };
        self.record_effect(
            "workspace.file.patched",
            json!({
                "path": result.path,
                "old_sha256": result.old_sha256,
                "new_sha256": result.new_sha256,
                "bytes": result.bytes,
            }),
        )?;
        Ok(result)
    }

    pub fn create_directory(&self, request: PathRequest) -> Result<MutationResult> {
        let _mutation = FILESYSTEM_MUTATION_LOCK
            .lock()
            .map_err(|_| anyhow::anyhow!("filesystem mutation lock poisoned"))?;
        let path = self.checked_new_path(&request.path)?;
        let (root, relative) = self.authorized_relative(&path)?;
        root.create_dir(&relative)?;
        let result = MutationResult {
            path: utf8_path(&path)?,
            old_sha256: None,
            new_sha256: None,
            bytes: None,
        };
        self.record_effect(
            "workspace.directory.created",
            json!({ "path": result.path }),
        )?;
        Ok(result)
    }

    pub fn move_path(&self, request: MoveRequest) -> Result<MutationResult> {
        let _mutation = FILESYSTEM_MUTATION_LOCK
            .lock()
            .map_err(|_| anyhow::anyhow!("filesystem mutation lock poisoned"))?;
        let from = self.checked_existing(&request.from)?;
        let to = self.checked_new_path(&request.to)?;
        let root_path = Path::new("/");
        let root = Dir::open_ambient_dir(root_path, ambient_authority())?;
        let from_relative = from.strip_prefix(root_path)?;
        let to_relative = to.strip_prefix(root_path)?;
        let from_path = utf8_path(&from)?;
        rename_without_overwrite(&root, from_relative, to_relative)?;
        let result = MutationResult {
            path: utf8_path(&to)?,
            old_sha256: None,
            new_sha256: None,
            bytes: None,
        };
        self.record_effect(
            "workspace.entry.moved",
            json!({ "from": from_path, "to": result.path }),
        )?;
        Ok(result)
    }

    pub fn delete(&self, request: PathRequest) -> Result<MutationResult> {
        let _mutation = FILESYSTEM_MUTATION_LOCK
            .lock()
            .map_err(|_| anyhow::anyhow!("filesystem mutation lock poisoned"))?;
        let path = self.checked_existing(&request.path)?;
        if path == Path::new("/")
            || self
                .boundaries()?
                .directory_includes
                .iter()
                .filter(|pattern| !boundary::has_wildcards(pattern))
                .any(|root| Path::new(root) == path)
        {
            bail!("an included boundary root cannot be deleted");
        }
        let metadata = fs::metadata(&path)?;
        let old_sha256 = metadata
            .is_file()
            .then(|| existing_hash(&path))
            .transpose()?
            .flatten();
        let (root, relative) = self.authorized_relative(&path)?;
        let event_type = if metadata.is_file() {
            root.remove_file(&relative)?;
            "workspace.file.deleted"
        } else if metadata.is_dir() {
            root.remove_dir(&relative)
                .context("only empty directories can be deleted")?;
            "workspace.directory.deleted"
        } else {
            bail!("special files cannot be deleted");
        };
        let result = MutationResult {
            path: utf8_path(&path)?,
            old_sha256,
            new_sha256: None,
            bytes: None,
        };
        self.record_effect(
            event_type,
            json!({ "path": result.path, "old_sha256": result.old_sha256 }),
        )?;
        Ok(result)
    }

    pub fn search(&self, request: SearchRequest) -> Result<SearchResult> {
        if request.query.is_empty() || request.query.len() > 1024 {
            bail!("query must contain between 1 and 1024 bytes");
        }
        let root_path = self.checked_existing(&request.path)?;
        if !root_path.is_dir() {
            bail!("search path must be a directory");
        }
        let (granted_root, relative) = self.authorized_relative(&root_path)?;
        let search_root = if relative.as_os_str().is_empty() {
            granted_root
        } else {
            granted_root.open_dir(&relative)?
        };
        let policy = self.boundaries()?;
        let limit = request.limit.unwrap_or(50).clamp(1, 200);
        let needle = request.query.to_lowercase();
        let mut queue = VecDeque::from([(search_root, root_path, 0usize)]);
        let mut matches = Vec::new();
        let mut entries_visited = 0usize;
        let mut files_scanned = 0;
        let mut bytes_scanned = 0u64;
        let mut truncated = false;
        while let Some((directory, display_directory, depth)) = queue.pop_front() {
            if depth > MAX_SEARCH_DEPTH {
                truncated = true;
                continue;
            }
            let mut entries = directory
                .entries()?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries {
                entries_visited += 1;
                if entries_visited > MAX_SEARCH_ENTRIES {
                    truncated = true;
                    break;
                }
                let file_type = entry.file_type()?;
                if file_type.is_symlink() || !file_type.is_file() && !file_type.is_dir() {
                    continue;
                }
                let display_path = display_directory.join(entry.file_name());
                if !boundary::path_allowed(
                    &display_path,
                    &policy.directory_includes,
                    &policy.directory_excludes,
                    true,
                ) {
                    continue;
                }
                if file_type.is_dir() {
                    queue.push_back((entry.open_dir()?, display_path, depth + 1));
                    continue;
                }
                let mut options = OpenOptions::new();
                options.read(true).follow(FollowSymlinks::No);
                let mut file = entry.open_with(&options)?;
                let metadata = file.metadata()?;
                if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
                    continue;
                }
                if files_scanned >= MAX_SEARCH_FILES
                    || bytes_scanned.saturating_add(metadata.len()) > MAX_SEARCH_BYTES
                {
                    truncated = true;
                    break;
                }
                files_scanned += 1;
                bytes_scanned += metadata.len();
                let mut bytes = Vec::with_capacity(metadata.len() as usize);
                Read::take(&mut file, MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
                if bytes.len() as u64 > MAX_FILE_BYTES {
                    continue;
                }
                let Ok(content) = String::from_utf8(bytes) else {
                    continue;
                };
                for (index, line) in content.lines().enumerate() {
                    if line.to_lowercase().contains(&needle) {
                        matches.push(SearchMatch {
                            path: utf8_path(&display_path)?,
                            line: index + 1,
                            preview: line.chars().take(500).collect(),
                        });
                        if matches.len() >= limit {
                            truncated = true;
                            return Ok(SearchResult {
                                matches,
                                files_scanned,
                                bytes_scanned,
                                truncated,
                            });
                        }
                    }
                }
            }
            if truncated {
                break;
            }
        }
        Ok(SearchResult {
            matches,
            files_scanned,
            bytes_scanned,
            truncated,
        })
    }

    fn atomic_write(
        &self,
        path: &Path,
        contents: &[u8],
        expected_current_hash: Option<&str>,
    ) -> Result<()> {
        let (root, relative) = self.authorized_relative(path)?;
        let parent_relative = relative
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let parent = root.open_dir(parent_relative)?;
        let filename = relative.file_name().context("file path has no filename")?;
        let temporary_name = format!(".habibi-{}.tmp", uuid::Uuid::now_v7());
        let mut options = OpenOptions::new();
        options
            .write(true)
            .create_new(true)
            .follow(FollowSymlinks::No);
        let mut temporary = parent.open_with(&temporary_name, &options)?;
        let result = (|| -> Result<()> {
            temporary.write_all(contents)?;
            if let Ok(metadata) = parent.metadata(filename) {
                temporary.set_permissions(metadata.permissions())?;
            }
            temporary.sync_all()?;
            let current_hash = cap_existing_hash(&parent, Path::new(filename))?;
            if current_hash.as_deref() != expected_current_hash {
                bail!("file changed while the checked write was being prepared");
            }
            if expected_current_hash.is_none() {
                rename_without_overwrite(&parent, Path::new(&temporary_name), Path::new(filename))?;
            } else {
                parent.rename(&temporary_name, &parent, filename)?;
            }
            #[cfg(unix)]
            parent.open(".")?.sync_all()?;
            Ok(())
        })();
        if result.is_err() {
            let _ = parent.remove_file(&temporary_name);
        }
        result
    }

    fn record_effect(&self, event_type: &str, payload: Value) -> Result<()> {
        self.effects
            .lock()
            .map_err(|_| anyhow::anyhow!("filesystem effect journal lock poisoned"))?
            .push(FilesystemEffect {
                event_type: event_type.into(),
                payload,
            });
        Ok(())
    }

    fn boundaries(&self) -> Result<BoundaryPolicy> {
        let policy = self
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("event store lock poisoned"))?
            .boundary_policy()?;
        if policy.directory_includes.is_empty() {
            bail!("the global filesystem boundary includes no directories");
        }
        for pattern in policy
            .directory_includes
            .iter()
            .chain(&policy.directory_excludes)
            .filter(|pattern| !boundary::has_wildcards(pattern))
        {
            let metadata = fs::symlink_metadata(pattern)
                .with_context(|| format!("boundary directory '{pattern}' no longer exists"))?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                bail!("boundary directory '{pattern}' is no longer a canonical directory");
            }
        }
        Ok(policy)
    }

    fn authorize(&self, path: &Path) -> Result<()> {
        let policy = self.boundaries()?;
        if !boundary::path_allowed(
            path,
            &policy.directory_includes,
            &policy.directory_excludes,
            true,
        ) {
            bail!(
                "path '{}' is outside the global directory boundary",
                path.display()
            );
        }
        Ok(())
    }

    fn authorized_relative(&self, path: &Path) -> Result<(Dir, PathBuf)> {
        self.authorize(path)?;
        let root = Path::new("/");
        let directory = Dir::open_ambient_dir(root, ambient_authority())?;
        Ok((directory, path.strip_prefix(root)?.to_owned()))
    }

    fn checked_existing(&self, value: &str) -> Result<PathBuf> {
        let path = checked_absolute(value)?;
        self.authorize(&path)?;
        reject_symlinks(Path::new("/"), &path)?;
        let metadata = fs::metadata(&path)
            .with_context(|| format!("path '{}' does not exist", path.display()))?;
        if !metadata.is_file() && !metadata.is_dir() {
            bail!("special files are not supported");
        }
        Ok(path)
    }

    fn checked_regular_file(&self, value: &str) -> Result<PathBuf> {
        let path = self.checked_existing(value)?;
        if !fs::metadata(&path)?.is_file() {
            bail!("'{}' is not a regular file", path.display());
        }
        Ok(path)
    }

    fn checked_write_path(&self, value: &str) -> Result<PathBuf> {
        let path = checked_absolute(value)?;
        self.authorize(&path)?;
        let parent = path.parent().context("file path has no parent")?;
        reject_symlinks(Path::new("/"), parent)?;
        if !parent.is_dir() {
            bail!("parent directory does not exist");
        }
        if let Ok(metadata) = fs::symlink_metadata(&path)
            && (metadata.file_type().is_symlink() || !metadata.is_file())
        {
            bail!("write target must be a regular file or not exist");
        }
        Ok(path)
    }

    fn checked_new_path(&self, value: &str) -> Result<PathBuf> {
        let path = self.checked_write_path(value)?;
        if fs::symlink_metadata(&path).is_ok() {
            bail!("destination already exists");
        }
        Ok(path)
    }
}

pub fn normalize_grant_roots(values: &[String]) -> Result<Vec<String>> {
    if values.len() > 32 {
        bail!("at most 32 directory patterns may be included or excluded");
    }
    let mut patterns = values
        .iter()
        .filter(|value| !value.trim().is_empty())
        .map(|value| {
            let value = value.trim();
            boundary::validate_pattern(value)?;
            if boundary::has_wildcards(value) {
                return Ok(value.to_owned());
            }
            let path = checked_absolute(value)?;
            let canonical = fs::canonicalize(&path)
                .with_context(|| format!("filesystem root '{}' does not exist", path.display()))?;
            if !canonical.is_dir() {
                bail!(
                    "filesystem root '{}' is not a directory",
                    canonical.display()
                );
            }
            utf8_path(&canonical)
        })
        .collect::<Result<Vec<_>>>()?;
    patterns.sort();
    patterns.dedup();
    Ok(patterns)
}

fn checked_absolute(value: &str) -> Result<PathBuf> {
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        bail!("filesystem paths must be absolute");
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        bail!("filesystem paths cannot contain '.' or '..'");
    }
    Ok(path)
}

fn reject_symlinks(root: &Path, path: &Path) -> Result<()> {
    let relative = path.strip_prefix(root)?;
    let mut current = root.to_owned();
    for component in relative.components() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                bail!("symbolic links are not allowed in filesystem operations")
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn rename_without_overwrite(directory: &Dir, from: &Path, to: &Path) -> Result<()> {
    rustix::fs::renameat_with(
        directory,
        from,
        directory,
        to,
        rustix::fs::RenameFlags::NOREPLACE,
    )?;
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn rename_without_overwrite(directory: &Dir, from: &Path, to: &Path) -> Result<()> {
    directory
        .hard_link(from, directory, to)
        .context("safe no-overwrite moves are limited to regular files on this platform")?;
    directory.remove_file(from)?;
    Ok(())
}

fn cap_existing_hash(directory: &Dir, path: &Path) -> Result<Option<String>> {
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let mut file = match directory.open_with(path, &options) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
        bail!("existing target must be a regular file no larger than 2 MiB");
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    Read::take(&mut file, MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        bail!("existing target grew beyond the 2 MiB limit");
    }
    Ok(Some(sha256(&bytes)))
}

fn existing_hash(path: &Path) -> Result<Option<String>> {
    match fs::read(path) {
        Ok(bytes) => {
            if bytes.len() as u64 > MAX_FILE_BYTES {
                bail!("existing file exceeds the 2 MiB limit");
            }
            Ok(Some(sha256(&bytes)))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn check_expected_hash(expected: Option<&str>, actual: Option<&str>) -> Result<()> {
    if actual.is_some() && expected.is_none() {
        bail!("expected_sha256 is required when modifying an existing file");
    }
    if let Some(expected) = expected
        && Some(expected) != actual
    {
        bail!(
            "file changed: expected hash '{}', found '{}'",
            expected,
            actual.unwrap_or("missing")
        );
    }
    Ok(())
}

fn sha256(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn utf8_path(path: &Path) -> Result<String> {
    path.to_str()
        .map(str::to_owned)
        .with_context(|| format!("path '{}' is not valid UTF-8", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::EventStore;

    fn host(root: &Path) -> FilesystemHost {
        let store = EventStore::open(":memory:").unwrap().shared();
        store
            .lock()
            .unwrap()
            .set_boundary_policy(&crate::store::BoundaryPolicy {
                directory_includes: vec![utf8_path(root).unwrap()],
                ..Default::default()
            })
            .unwrap();
        FilesystemHost::new(store)
    }

    #[test]
    fn denies_access_without_grants() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("note.txt");
        fs::write(&file, "private").unwrap();
        let store = EventStore::open(":memory:").unwrap().shared();
        let host = FilesystemHost::new(store);
        assert!(
            host.read(PathRequest {
                path: utf8_path(&file).unwrap(),
            })
            .unwrap_err()
            .to_string()
            .contains("global filesystem boundary includes no directories")
        );
    }

    #[test]
    fn reads_writes_and_checks_hashes_inside_granted_root() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("note.txt");
        let host = host(root.path());
        let written = host
            .write(WriteRequest {
                path: utf8_path(&file).unwrap(),
                content: "sentinel-content".into(),
                expected_sha256: None,
            })
            .unwrap();
        assert_eq!(written.new_sha256, Some(sha256(b"sentinel-content")));
        let effects = host.take_effects().unwrap();
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0].event_type, "workspace.file.created");
        assert!(!effects[0].payload.to_string().contains("sentinel-content"));
        let read = host
            .read(PathRequest {
                path: utf8_path(&file).unwrap(),
            })
            .unwrap();
        assert_eq!(read.content, "sentinel-content");
        assert!(
            host.write(WriteRequest {
                path: utf8_path(&file).unwrap(),
                content: "blind overwrite".into(),
                expected_sha256: None,
            })
            .is_err()
        );
        assert!(
            host.write(WriteRequest {
                path: utf8_path(&file).unwrap(),
                content: "lost update".into(),
                expected_sha256: Some("sha256:wrong".into()),
            })
            .is_err()
        );
        assert_eq!(fs::read_to_string(file).unwrap(), "sentinel-content");
    }

    #[test]
    fn global_exclusions_override_included_directories() {
        let root = tempfile::tempdir().unwrap();
        let private = root.path().join("private");
        fs::create_dir(&private).unwrap();
        fs::write(private.join("secret"), "secret").unwrap();
        let host = host(root.path());
        host.store
            .lock()
            .unwrap()
            .set_boundary_policy(&crate::store::BoundaryPolicy {
                directory_includes: vec![utf8_path(root.path()).unwrap()],
                directory_excludes: vec![utf8_path(&private).unwrap()],
                ..Default::default()
            })
            .unwrap();
        assert!(
            host.read(PathRequest {
                path: utf8_path(&private.join("secret")).unwrap()
            })
            .is_err()
        );
        assert!(
            host.list(PathRequest {
                path: utf8_path(root.path()).unwrap()
            })
            .unwrap()
            .iter()
            .all(|entry| entry.name != "private")
        );
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinks_and_paths_outside_grants() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret"), "secret").unwrap();
        symlink(outside.path(), root.path().join("escape")).unwrap();
        let host = host(root.path());
        assert!(
            host.read(PathRequest {
                path: utf8_path(&root.path().join("escape/secret")).unwrap(),
            })
            .unwrap_err()
            .to_string()
            .contains("symbolic links")
        );
        assert!(
            host.read(PathRequest {
                path: utf8_path(&outside.path().join("secret")).unwrap(),
            })
            .is_err()
        );
    }

    #[test]
    fn move_never_overwrites_and_granted_root_cannot_be_deleted() {
        let root = tempfile::tempdir().unwrap();
        let from = root.path().join("from.txt");
        let to = root.path().join("to.txt");
        fs::write(&from, "from").unwrap();
        fs::write(&to, "to").unwrap();
        let host = host(root.path());
        assert!(
            host.move_path(MoveRequest {
                from: utf8_path(&from).unwrap(),
                to: utf8_path(&to).unwrap(),
            })
            .is_err()
        );
        assert_eq!(fs::read_to_string(&from).unwrap(), "from");
        assert_eq!(fs::read_to_string(&to).unwrap(), "to");
        assert!(
            host.delete(PathRequest {
                path: utf8_path(root.path()).unwrap(),
            })
            .is_err()
        );
    }

    #[test]
    fn exact_patch_requires_one_match() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("source.txt");
        fs::write(&file, "one two three").unwrap();
        let host = host(root.path());
        host.patch(PatchRequest {
            path: utf8_path(&file).unwrap(),
            old_text: "two".into(),
            new_text: "2".into(),
            expected_sha256: Some(sha256(b"one two three")),
        })
        .unwrap();
        assert_eq!(fs::read_to_string(file).unwrap(), "one 2 three");
    }
}

use std::{
    collections::VecDeque,
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::{Context, Result, bail};
use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::installer::{DraftValidation, ExtensionInstaller};

const MAX_DRAFT_FILE_BYTES: u64 = 1024 * 1024;
const MAX_DRAFT_FILES: usize = 200;
const MAX_DRAFT_DEPTH: usize = 8;
const ALLOWED_EXTENSIONS: &[&str] = &["toml", "lua", "html", "css", "js", "md", "json"];

#[derive(Clone)]
pub struct StudioHost {
    service: StudioService,
    action_enabled: Arc<AtomicBool>,
}

pub struct StudioActionGuard {
    enabled: Arc<AtomicBool>,
}

impl Drop for StudioActionGuard {
    fn drop(&mut self) {
        self.enabled.store(false, Ordering::Release);
    }
}

impl StudioHost {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            service: StudioService::from_env()?,
            action_enabled: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn begin_action(&self) -> Result<StudioActionGuard> {
        if self.action_enabled.swap(true, Ordering::AcqRel) {
            bail!("Extension Studio action context is already active");
        }
        Ok(StudioActionGuard {
            enabled: self.action_enabled.clone(),
        })
    }

    fn require_action(&self) -> Result<()> {
        if !self.action_enabled.load(Ordering::Acquire) {
            bail!("Extension Studio is available only during a registered tool action");
        }
        Ok(())
    }

    pub fn list_drafts(&self) -> Result<Vec<DraftSummary>> {
        self.require_action()?;
        self.service.list_drafts()
    }

    pub fn create_draft(&self, request: CreateDraftRequest) -> Result<DraftSummary> {
        self.require_action()?;
        self.service.create_draft(request)
    }

    pub fn list_files(&self, draft_id: &str) -> Result<Vec<String>> {
        self.require_action()?;
        self.service.list_files(draft_id)
    }

    pub fn read_file(&self, request: DraftFileRequest) -> Result<DraftFile> {
        self.require_action()?;
        self.service.read_file(request)
    }

    pub fn write_file(&self, request: WriteDraftFileRequest) -> Result<DraftWriteResult> {
        self.require_action()?;
        self.service.write_file(request)
    }

    pub fn create_directory(&self, request: CreateDraftDirectoryRequest) -> Result<()> {
        self.require_action()?;
        self.service.create_directory(request)
    }

    pub fn validate(&self, draft_id: &str) -> Result<DraftValidation> {
        self.require_action()?;
        self.service.validate(draft_id)
    }
}

#[derive(Clone)]
pub struct StudioService {
    root: Arc<Dir>,
    root_path: PathBuf,
    root_identity: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateDraftRequest {
    pub id: String,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Deserialize)]
pub struct DraftFileRequest {
    pub draft_id: String,
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct WriteDraftFileRequest {
    pub draft_id: String,
    pub path: String,
    pub content: String,
    pub expected_sha256: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateDraftDirectoryRequest {
    pub draft_id: String,
    pub path: String,
}

#[derive(Debug, Serialize)]
pub struct DraftSummary {
    pub id: String,
}

#[derive(Debug, Serialize)]
pub struct DraftFile {
    pub path: String,
    pub content: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Serialize)]
pub struct DraftWriteResult {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

impl StudioService {
    pub fn new(path: impl AsRef<Path>) -> Result<Self> {
        let requested = path.as_ref();
        fs::create_dir_all(requested).with_context(|| {
            format!(
                "failed to create extension drafts root {}",
                requested.display()
            )
        })?;
        let root_path = requested.canonicalize()?;
        let metadata = fs::metadata(&root_path)?;
        let root = Dir::open_ambient_dir(&root_path, ambient_authority())?;
        Ok(Self {
            root: Arc::new(root),
            root_path,
            root_identity: directory_identity(&metadata),
        })
    }

    pub fn from_env() -> Result<Self> {
        Self::new(
            std::env::var("HABIBI_EXTENSION_DRAFTS_DIR")
                .unwrap_or_else(|_| "extension-drafts".to_owned()),
        )
    }

    pub fn root_path(&self) -> &Path {
        &self.root_path
    }

    pub fn list_drafts(&self) -> Result<Vec<DraftSummary>> {
        let mut drafts = self
            .root
            .entries()?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drafts.sort_by_key(|entry| entry.file_name());
        drafts
            .into_iter()
            .filter_map(|entry| match entry.file_type() {
                Ok(file_type) if file_type.is_dir() && !file_type.is_symlink() => {
                    let id = entry.file_name().to_string_lossy().into_owned();
                    validate_draft_id(&id)
                        .is_ok()
                        .then_some(Ok(DraftSummary { id }))
                }
                Ok(_) => None,
                Err(error) => Some(Err(error.into())),
            })
            .collect()
    }

    pub fn create_draft(&self, request: CreateDraftRequest) -> Result<DraftSummary> {
        validate_draft_id(&request.id)?;
        validate_text_field("name", &request.name, 100)?;
        validate_text_field("description", &request.description, 500)?;
        self.root.create_dir(&request.id)?;
        let draft = self.root.open_dir(&request.id)?;
        let manifest = format!(
            "id = {}\nname = {}\nversion = \"0.1.0\"\ndescription = {}\napi_version = 2\n\n[capabilities]\ntools = true\n",
            toml_string(&request.id),
            toml_string(&request.name),
            toml_string(&request.description),
        );
        let lua = format!(
            "habibi.tools.register({{\n  name = {},\n  description = \"Echo a message from the generated extension.\",\n  input_schema = {{\n    type = \"object\",\n    additionalProperties = false,\n    properties = {{ message = {{ type = \"string\" }} }},\n    required = {{ \"message\" }}\n  }}\n}}, function(arguments)\n  return {{ result = {{ message = arguments.message }} }}\nend)\n",
            lua_string(&format!("{}.echo", request.id)),
        );
        if let Err(error) = write_new_file(&draft, Path::new("extension.toml"), manifest.as_bytes())
            .and_then(|_| write_new_file(&draft, Path::new("extension.lua"), lua.as_bytes()))
        {
            let _ = draft.remove_file("extension.toml");
            let _ = draft.remove_file("extension.lua");
            let _ = self.root.remove_dir(&request.id);
            return Err(error);
        }
        Ok(DraftSummary { id: request.id })
    }

    pub fn list_files(&self, draft_id: &str) -> Result<Vec<String>> {
        let draft = self.draft(draft_id)?;
        let mut queue = VecDeque::from([(draft, PathBuf::new(), 0usize)]);
        let mut files = Vec::new();
        while let Some((directory, prefix, depth)) = queue.pop_front() {
            if depth > MAX_DRAFT_DEPTH {
                bail!("extension draft exceeds maximum directory depth");
            }
            let mut entries = directory
                .entries()?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries {
                let file_type = entry.file_type()?;
                if file_type.is_symlink() || !file_type.is_file() && !file_type.is_dir() {
                    bail!("extension drafts may contain only regular files and directories");
                }
                let path = prefix.join(entry.file_name());
                if file_type.is_dir() {
                    queue.push_back((entry.open_dir()?, path, depth + 1));
                } else {
                    validate_file_path(&path)?;
                    files.push(path.to_string_lossy().into_owned());
                    if files.len() > MAX_DRAFT_FILES {
                        bail!("extension draft exceeds the 200 file limit");
                    }
                }
            }
        }
        Ok(files)
    }

    pub fn read_file(&self, request: DraftFileRequest) -> Result<DraftFile> {
        let relative = validate_file_path(Path::new(&request.path))?;
        let draft = self.draft(&request.draft_id)?;
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let mut file = draft.open_with(&relative, &options)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.len() > MAX_DRAFT_FILE_BYTES {
            bail!("draft file must be a regular file no larger than 1 MiB");
        }
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        Read::take(&mut file, MAX_DRAFT_FILE_BYTES + 1).read_to_end(&mut bytes)?;
        let content = String::from_utf8(bytes).context("draft files must be UTF-8")?;
        Ok(DraftFile {
            path: request.path,
            bytes: content.len() as u64,
            sha256: sha256(content.as_bytes()),
            content,
        })
    }

    pub fn write_file(&self, request: WriteDraftFileRequest) -> Result<DraftWriteResult> {
        if request.content.len() as u64 > MAX_DRAFT_FILE_BYTES {
            bail!("draft file exceeds the 1 MiB limit");
        }
        let relative = validate_file_path(Path::new(&request.path))?;
        let draft = self.draft(&request.draft_id)?;
        let parent_path = relative
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let parent = draft.open_dir(parent_path)?;
        let filename = relative
            .file_name()
            .context("draft file path has no filename")?;
        let current_hash = existing_hash(&parent, Path::new(filename))?;
        match (&current_hash, &request.expected_sha256) {
            (Some(_), None) => bail!("expected_sha256 is required when replacing a draft file"),
            (Some(current), Some(expected)) if current != expected => {
                bail!("draft file changed since it was read")
            }
            (None, Some(_)) => bail!("draft file does not exist"),
            _ => {}
        }
        let temporary = format!(".habibi-studio-{}", Uuid::now_v7());
        let mut options = OpenOptions::new();
        options
            .write(true)
            .create_new(true)
            .follow(FollowSymlinks::No);
        let mut file = parent.open_with(&temporary, &options)?;
        let result = (|| {
            file.write_all(request.content.as_bytes())?;
            file.sync_all()?;
            if existing_hash(&parent, Path::new(filename))? != current_hash {
                bail!("draft file changed while it was being written");
            }
            if current_hash.is_some() {
                parent.rename(&temporary, &parent, filename)?;
            } else {
                rename_without_overwrite(&parent, Path::new(&temporary), Path::new(filename))?;
            }
            #[cfg(unix)]
            parent.open(".")?.sync_all()?;
            Ok(())
        })();
        if result.is_err() {
            let _ = parent.remove_file(&temporary);
        }
        result?;
        Ok(DraftWriteResult {
            path: request.path,
            bytes: request.content.len() as u64,
            sha256: sha256(request.content.as_bytes()),
        })
    }

    pub fn create_directory(&self, request: CreateDraftDirectoryRequest) -> Result<()> {
        let relative = validate_relative_path(Path::new(&request.path))?;
        let draft = self.draft(&request.draft_id)?;
        let parent_path = relative
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let parent = draft.open_dir(parent_path)?;
        parent.create_dir(relative.file_name().context("directory path has no name")?)?;
        Ok(())
    }

    pub fn validate(&self, draft_id: &str) -> Result<DraftValidation> {
        validate_draft_id(draft_id)?;
        let draft = self.draft(draft_id)?;
        self.ensure_root_identity()?;
        let path = self.root_path.join(draft_id);
        ensure_opened_directory(&draft, &path)?;
        ExtensionInstaller::new(self.root_path.join(".validation-output")).inspect_local(&path)
    }

    pub fn draft_path(&self, draft_id: &str) -> Result<PathBuf> {
        validate_draft_id(draft_id)?;
        let draft = self.draft(draft_id)?;
        self.ensure_root_identity()?;
        let path = self.root_path.join(draft_id);
        ensure_opened_directory(&draft, &path)?;
        Ok(path)
    }

    fn ensure_root_identity(&self) -> Result<()> {
        let canonical = self.root_path.canonicalize()?;
        let metadata = fs::metadata(&canonical)?;
        if canonical != self.root_path || directory_identity(&metadata) != self.root_identity {
            bail!("extension drafts root changed identity; restart Habibi after reviewing it");
        }
        Ok(())
    }

    fn draft(&self, draft_id: &str) -> Result<Dir> {
        validate_draft_id(draft_id)?;
        self.root
            .open_dir(draft_id)
            .with_context(|| format!("extension draft '{draft_id}' does not exist"))
    }
}

#[cfg(unix)]
fn ensure_opened_directory(directory: &Dir, path: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let opened = rustix::fs::fstat(directory)?;
    let current = fs::metadata(path)?;
    if opened.st_dev != current.dev() || opened.st_ino != current.ino() {
        bail!("extension draft changed identity while it was being opened");
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_opened_directory(_directory: &Dir, path: &Path) -> Result<()> {
    if !fs::metadata(path)?.is_dir() {
        bail!("extension draft is not a directory");
    }
    Ok(())
}

#[cfg(unix)]
fn directory_identity(metadata: &fs::Metadata) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    Some(format!("{}:{}", metadata.dev(), metadata.ino()))
}

#[cfg(not(unix))]
fn directory_identity(_metadata: &fs::Metadata) -> Option<String> {
    None
}

fn validate_draft_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 64
        || !id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        })
        || !id.as_bytes()[0].is_ascii_lowercase()
    {
        bail!(
            "draft id must start with a lowercase letter and use lowercase ASCII, digits, '-' or '_'"
        );
    }
    Ok(())
}

fn validate_text_field(name: &str, value: &str, max: usize) -> Result<()> {
    if value.trim().is_empty() || value.len() > max || value.contains(['\r', '\n']) {
        bail!("{name} must contain 1-{max} bytes on one line");
    }
    Ok(())
}

fn validate_relative_path(path: &Path) -> Result<PathBuf> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        bail!("draft paths must be relative without '.', '..', or empty components");
    }
    Ok(path.to_owned())
}

fn validate_file_path(path: &Path) -> Result<PathBuf> {
    let path = validate_relative_path(path)?;
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .context("draft files require an allowlisted extension")?;
    if !ALLOWED_EXTENSIONS.contains(&extension) {
        bail!("draft file type '.{extension}' is not allowed");
    }
    Ok(path)
}

fn write_new_file(directory: &Dir, path: &Path, content: &[u8]) -> Result<()> {
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No);
    let mut file = directory.open_with(path, &options)?;
    file.write_all(content)?;
    file.sync_all()?;
    Ok(())
}

fn existing_hash(directory: &Dir, path: &Path) -> Result<Option<String>> {
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let mut file = match directory.open_with(path, &options) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_DRAFT_FILE_BYTES {
        bail!("draft target must be a regular file no larger than 1 MiB");
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    Read::take(&mut file, MAX_DRAFT_FILE_BYTES + 1).read_to_end(&mut bytes)?;
    Ok(Some(sha256(&bytes)))
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
    directory.hard_link(from, directory, to)?;
    directory.remove_file(from)?;
    Ok(())
}

fn toml_string(value: &str) -> String {
    format!("{:?}", value)
}

fn lua_string(value: &str) -> String {
    format!("{:?}", value)
}

fn sha256(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_edits_and_validates_a_draft() {
        let root = tempfile::tempdir().unwrap();
        let studio = StudioService::new(root.path()).unwrap();
        studio
            .create_draft(CreateDraftRequest {
                id: "example".into(),
                name: "Example".into(),
                description: "Generated test extension".into(),
            })
            .unwrap();
        let file = studio
            .read_file(DraftFileRequest {
                draft_id: "example".into(),
                path: "extension.lua".into(),
            })
            .unwrap();
        assert!(file.content.contains("example.echo"));
        assert!(
            studio
                .write_file(WriteDraftFileRequest {
                    draft_id: "example".into(),
                    path: "extension.lua".into(),
                    content: "-- stale".into(),
                    expected_sha256: Some("sha256:wrong".into()),
                })
                .is_err()
        );
        studio
            .write_file(WriteDraftFileRequest {
                draft_id: "example".into(),
                path: "README.md".into(),
                content: "# Example\n".into(),
                expected_sha256: None,
            })
            .unwrap();
        let validation = studio.validate("example").unwrap();
        assert!(validation.valid, "{:?}", validation.validation_error);
        assert!(validation.security_scan.passed);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinks_and_traversal() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let studio = StudioService::new(root.path()).unwrap();
        studio
            .create_draft(CreateDraftRequest {
                id: "example".into(),
                name: "Example".into(),
                description: "Generated test extension".into(),
            })
            .unwrap();
        symlink(outside.path(), root.path().join("example/web")).unwrap();
        assert!(studio.list_files("example").is_err());
        assert!(
            studio
                .read_file(DraftFileRequest {
                    draft_id: "example".into(),
                    path: "../outside.lua".into(),
                })
                .is_err()
        );
    }
}

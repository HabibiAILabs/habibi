use std::{
    fs::{self, File, OpenOptions},
    io::Read,
    path::{Component, Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail};
use chrono::Utc;
use fs2::FileExt;
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use uuid::Uuid;

use crate::{
    extension::{ExtensionCapabilities, ExtensionManifest, LoadedExtension},
    store::EventStore,
};

pub const INSTALL_METADATA_FILE: &str = ".habibi-install.json";

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExtensionSource {
    Local {
        path: String,
    },
    Git {
        url: String,
        reference: Option<String>,
        subdir: Option<String>,
        revision: String,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct InstallMetadata {
    pub id: String,
    pub name: String,
    pub version: String,
    pub source: ExtensionSource,
    pub content_hash: String,
    pub installed_at: String,
    pub capabilities: ExtensionCapabilities,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateStatus {
    pub id: String,
    pub installed_version: String,
    pub available_version: String,
    pub installed_revision: Option<String>,
    pub available_revision: Option<String>,
    pub installed_content_hash: String,
    pub available_content_hash: String,
    pub update_available: bool,
    pub installed_capabilities: ExtensionCapabilities,
    pub available_capabilities: ExtensionCapabilities,
}

#[derive(Debug, Clone, Default)]
pub struct SourceOptions {
    pub reference: Option<String>,
    pub subdir: Option<String>,
}

pub struct ExtensionInstaller {
    extensions_dir: PathBuf,
}

struct PreparedPackage {
    _checkout: Option<TempDir>,
    package_root: PathBuf,
    source: ExtensionSource,
    manifest: ExtensionManifest,
    content_hash: String,
}

impl ExtensionInstaller {
    pub fn new(extensions_dir: PathBuf) -> Self {
        Self { extensions_dir }
    }

    pub fn install(&self, source: &str, options: SourceOptions) -> Result<InstallMetadata> {
        let _operation_lock = self.operation_lock()?;
        let package = self.prepare(source, options)?;
        if self.extensions_dir.join(&package.manifest.id).exists() {
            bail!(
                "extension '{}' is already installed; use 'habibi update {}'",
                package.manifest.id,
                package.manifest.id
            );
        }
        self.commit(package)
    }

    pub fn update(&self, extension_id: &str) -> Result<InstallMetadata> {
        let _operation_lock = self.operation_lock()?;
        let installed = self.metadata(extension_id)?;
        let (source, options) = source_request(&installed.source);
        let package = self.prepare(&source, options)?;
        if package.manifest.id != extension_id {
            bail!(
                "update source now contains extension '{}' instead of '{}'",
                package.manifest.id,
                extension_id
            );
        }
        let installed_version = Version::parse(&installed.version)?;
        let available_version = Version::parse(&package.manifest.version)?;
        if available_version < installed_version {
            bail!(
                "refusing to downgrade '{}' from {} to {}",
                extension_id,
                installed_version,
                available_version
            );
        }
        if available_version == installed_version && package.content_hash != installed.content_hash
        {
            bail!(
                "extension '{}' changed without a version bump (still {}); refusing update",
                extension_id,
                installed_version
            );
        }
        if package.content_hash == installed.content_hash
            && source_revision(&package.source) == source_revision(&installed.source)
        {
            return Ok(installed);
        }
        self.commit(package)
    }

    pub fn check_update(&self, extension_id: &str) -> Result<UpdateStatus> {
        let installed = self.metadata(extension_id)?;
        let (source, options) = source_request(&installed.source);
        let package = self.prepare(&source, options)?;
        if package.manifest.id != extension_id {
            bail!("update source contains a different extension id");
        }
        let installed_revision = source_revision(&installed.source);
        let available_revision = source_revision(&package.source);
        let newer_version =
            Version::parse(&package.manifest.version)? > Version::parse(&installed.version)?;
        let changed_revision =
            available_revision.is_some() && available_revision != installed_revision;
        let changed_content = package.content_hash != installed.content_hash;
        Ok(UpdateStatus {
            id: extension_id.to_owned(),
            installed_version: installed.version,
            available_version: package.manifest.version,
            installed_revision,
            available_revision,
            installed_content_hash: installed.content_hash,
            available_content_hash: package.content_hash,
            update_available: newer_version || changed_revision || changed_content,
            installed_capabilities: installed.capabilities,
            available_capabilities: package.manifest.capabilities,
        })
    }

    fn operation_lock(&self) -> Result<File> {
        fs::create_dir_all(&self.extensions_dir)?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.extensions_dir.join(".habibi-install.lock"))?;
        file.try_lock_exclusive()
            .context("another extension installation or update is already running")?;
        Ok(file)
    }

    pub fn metadata(&self, extension_id: &str) -> Result<InstallMetadata> {
        let path = self
            .extensions_dir
            .join(extension_id)
            .join(INSTALL_METADATA_FILE);
        serde_json::from_slice(&fs::read(&path).with_context(|| {
            format!(
                "extension '{extension_id}' has no installation metadata; reinstall it with habibi install"
            )
        })?)
        .with_context(|| format!("invalid installation metadata {}", path.display()))
    }

    fn prepare(&self, source: &str, options: SourceOptions) -> Result<PreparedPackage> {
        fs::create_dir_all(&self.extensions_dir)?;
        let (checkout, source_root, resolved_source) = if looks_like_git(source) {
            let checkout = tempfile::tempdir()?;
            clone_git(source, checkout.path(), options.reference.as_deref())?;
            let revision = git_revision(checkout.path())?;
            let root = safe_subdir(checkout.path(), options.subdir.as_deref())?;
            (
                Some(checkout),
                root,
                ExtensionSource::Git {
                    url: source.to_owned(),
                    reference: options.reference,
                    subdir: options.subdir,
                    revision,
                },
            )
        } else {
            if options.reference.is_some() {
                bail!("--ref can only be used with a Git source");
            }
            let base = Path::new(source)
                .canonicalize()
                .with_context(|| format!("extension source '{source}' does not exist"))?;
            let root = safe_subdir(&base, options.subdir.as_deref())?;
            (
                None,
                root.clone(),
                ExtensionSource::Local {
                    path: root.to_string_lossy().into_owned(),
                },
            )
        };
        if !source_root.join("extension.toml").is_file()
            || !source_root.join("extension.lua").is_file()
        {
            bail!(
                "{} is not an extension package (extension.toml and extension.lua are required)",
                source_root.display()
            );
        }
        let manifest = read_manifest(&source_root)?;
        Version::parse(&manifest.version).with_context(|| {
            format!(
                "extension '{}' has an invalid semantic version",
                manifest.id
            )
        })?;
        if let Ok(extensions_root) = self.extensions_dir.canonicalize()
            && source_root.starts_with(&extensions_root)
        {
            bail!("cannot install an extension from inside the active extensions directory");
        }

        let staging = self
            .extensions_dir
            .join(format!(".habibi-stage-{}", Uuid::now_v7()));
        copy_package(&source_root, &staging)?;
        let content_hash = hash_package(&staging)?;
        let validation_store = EventStore::open(":memory:")?.shared();
        if let Err(error) = LoadedExtension::load(&staging, validation_store) {
            let _ = fs::remove_dir_all(&staging);
            return Err(error).context("extension validation failed");
        }
        Ok(PreparedPackage {
            _checkout: checkout,
            package_root: staging,
            source: resolved_source,
            manifest,
            content_hash,
        })
    }

    fn commit(&self, package: PreparedPackage) -> Result<InstallMetadata> {
        let metadata = InstallMetadata {
            id: package.manifest.id.clone(),
            name: package.manifest.name.clone(),
            version: package.manifest.version.clone(),
            source: package.source,
            content_hash: package.content_hash,
            installed_at: Utc::now().to_rfc3339(),
            capabilities: package.manifest.capabilities.clone(),
        };
        fs::write(
            package.package_root.join(INSTALL_METADATA_FILE),
            serde_json::to_vec_pretty(&metadata)?,
        )?;
        let destination = self.extensions_dir.join(&metadata.id);
        let backup = self
            .extensions_dir
            .join(format!(".habibi-rollback-{}", metadata.id));
        let had_existing = destination.exists();
        if had_existing {
            if backup.exists() {
                fs::remove_dir_all(&backup)?;
            }
            fs::rename(&destination, &backup)?;
        }
        if let Err(error) = fs::rename(&package.package_root, &destination) {
            if had_existing {
                let _ = fs::rename(&backup, &destination);
            }
            return Err(error).context("failed to atomically install extension");
        }
        Ok(metadata)
    }

    pub fn rollback(&self, extension_id: &str) -> Result<InstallMetadata> {
        let _operation_lock = self.operation_lock()?;
        let destination = self.extensions_dir.join(extension_id);
        let backup = self
            .extensions_dir
            .join(format!(".habibi-rollback-{extension_id}"));
        if !backup.is_dir() {
            bail!("extension '{extension_id}' has no rollback generation");
        }
        let failed = self
            .extensions_dir
            .join(format!(".habibi-failed-{extension_id}-{}", Uuid::now_v7()));
        fs::rename(&destination, &failed)?;
        if let Err(error) = fs::rename(&backup, &destination) {
            let _ = fs::rename(&failed, &destination);
            return Err(error).context("failed to restore extension rollback generation");
        }
        let _ = fs::remove_dir_all(failed);
        self.metadata(extension_id)
    }
}

fn source_request(source: &ExtensionSource) -> (String, SourceOptions) {
    match source {
        ExtensionSource::Local { path } => (path.clone(), SourceOptions::default()),
        ExtensionSource::Git {
            url,
            reference,
            subdir,
            ..
        } => (
            url.clone(),
            SourceOptions {
                reference: reference.clone(),
                subdir: subdir.clone(),
            },
        ),
    }
}

fn source_revision(source: &ExtensionSource) -> Option<String> {
    match source {
        ExtensionSource::Git { revision, .. } => Some(revision.clone()),
        ExtensionSource::Local { .. } => None,
    }
}

fn looks_like_git(source: &str) -> bool {
    if Path::new(source).exists() {
        return false;
    }
    source.starts_with("https://") || source.starts_with("ssh://") || source.starts_with("git@")
}

fn clone_git(source: &str, destination: &Path, reference: Option<&str>) -> Result<()> {
    if let Ok(url) = url::Url::parse(source)
        && (!url.username().is_empty() || url.password().is_some())
    {
        bail!("Git URLs containing credentials are not allowed");
    }
    let mut command = Command::new("git");
    command.args(["clone", "--depth", "1", "--no-tags"]);
    if let Some(reference) = reference {
        command.args(["--branch", reference]);
    }
    let output = command
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .arg("--")
        .arg(source)
        .arg(destination)
        .output()?;
    if !output.status.success() {
        bail!(
            "git clone failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

fn git_revision(directory: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(directory)
        .output()?;
    if !output.status.success() {
        bail!("failed to resolve Git revision");
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn safe_subdir(base: &Path, subdir: Option<&str>) -> Result<PathBuf> {
    let relative = Path::new(subdir.unwrap_or(""));
    if relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        bail!("extension subdirectory is unsafe");
    }
    let path = base.join(relative).canonicalize()?;
    let base = base.canonicalize()?;
    if !path.starts_with(&base) || !path.is_dir() {
        bail!("extension subdirectory escapes its source");
    }
    Ok(path)
}

fn read_manifest(directory: &Path) -> Result<ExtensionManifest> {
    let path = directory.join("extension.toml");
    toml::from_str(&fs::read_to_string(&path)?)
        .with_context(|| format!("invalid extension manifest {}", path.display()))
}

fn copy_package(source: &Path, destination: &Path) -> Result<()> {
    let mut file_count = 0_usize;
    let mut total_size = 0_u64;
    copy_directory(source, destination, 0, &mut file_count, &mut total_size)
}

fn copy_directory(
    source: &Path,
    destination: &Path,
    depth: usize,
    file_count: &mut usize,
    total_size: &mut u64,
) -> Result<()> {
    if depth > 32 {
        bail!("extension package exceeds the maximum directory depth");
    }
    fs::create_dir_all(destination)?;
    let mut entries = fs::read_dir(source)?.collect::<std::result::Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name();
        if name == ".git" || name == INSTALL_METADATA_FILE {
            continue;
        }
        let file_type = entry.file_type()?;
        let target = destination.join(&name);
        if file_type.is_symlink() {
            bail!("extension packages cannot contain symbolic links");
        } else if file_type.is_dir() {
            copy_directory(&entry.path(), &target, depth + 1, file_count, total_size)?;
        } else if file_type.is_file() {
            *file_count += 1;
            *total_size = total_size.saturating_add(entry.metadata()?.len());
            if *file_count > 2_000 || *total_size > 64 * 1024 * 1024 {
                bail!("extension package exceeds the 2,000 file or 64 MiB limit");
            }
            fs::copy(entry.path(), target)?;
        } else {
            bail!("extension packages may contain only regular files and directories");
        }
    }
    Ok(())
}

fn hash_package(directory: &Path) -> Result<String> {
    let mut hasher = Sha256::new();
    hash_directory(directory, directory, &mut hasher)?;
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

fn hash_directory(root: &Path, directory: &Path, hasher: &mut Sha256) -> Result<()> {
    let mut entries = fs::read_dir(directory)?.collect::<std::result::Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name();
        if name == ".git" || name == INSTALL_METADATA_FILE {
            continue;
        }
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            bail!("extension packages cannot contain symbolic links");
        }
        let path = entry.path();
        let relative = path.strip_prefix(root)?;
        let relative = relative.to_string_lossy();
        hasher.update((relative.len() as u64).to_le_bytes());
        hasher.update(relative.as_bytes());
        if file_type.is_dir() {
            hasher.update(b"directory");
            hash_directory(root, &path, hasher)?;
        } else if file_type.is_file() {
            hasher.update(b"file");
            let mut file = fs::File::open(path)?;
            let mut buffer = [0_u8; 8192];
            loop {
                let read = file.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                hasher.update(&buffer[..read]);
            }
        } else {
            bail!("extension packages may contain only regular files and directories");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(root: &Path, version: &str) {
        fs::create_dir_all(root).unwrap();
        fs::write(
            root.join("extension.toml"),
            format!(
                "id = \"example\"\nname = \"Example\"\nversion = \"{version}\"\napi_version = 1\n"
            ),
        )
        .unwrap();
        fs::write(
            root.join("extension.lua"),
            "habibi.reactions.context(function(_) return habibi.array({}) end)\n",
        )
        .unwrap();
    }

    #[test]
    fn installs_local_subdirectory_and_records_canonical_source() {
        let source = tempfile::tempdir().unwrap();
        package(&source.path().join("packages/example"), "1.0.0");
        let destination = tempfile::tempdir().unwrap();
        let installer = ExtensionInstaller::new(destination.path().join("extensions"));
        let installed = installer
            .install(
                source.path().to_str().unwrap(),
                SourceOptions {
                    reference: None,
                    subdir: Some("packages/example".into()),
                },
            )
            .unwrap();
        assert_eq!(installed.id, "example");
        let ExtensionSource::Local { path } = installed.source else {
            panic!("expected local source")
        };
        assert!(path.ends_with("packages/example"));
    }

    #[test]
    fn requires_a_version_bump_for_changed_content() {
        let source = tempfile::tempdir().unwrap();
        package(source.path(), "1.0.0");
        let destination = tempfile::tempdir().unwrap();
        let installer = ExtensionInstaller::new(destination.path().join("extensions"));
        installer
            .install(source.path().to_str().unwrap(), SourceOptions::default())
            .unwrap();
        fs::write(
            source.path().join("extension.lua"),
            "local changed = true\n",
        )
        .unwrap();
        let error = installer.update("example").unwrap_err();
        assert!(error.to_string().contains("without a version bump"));
        package(source.path(), "1.1.0");
        let updated = installer.update("example").unwrap();
        assert_eq!(updated.version, "1.1.0");
        let rolled_back = installer.rollback("example").unwrap();
        assert_eq!(rolled_back.version, "1.0.0");
    }

    #[cfg(unix)]
    #[test]
    fn rejects_package_symlinks() {
        use std::os::unix::fs::symlink;
        let source = tempfile::tempdir().unwrap();
        package(source.path(), "1.0.0");
        symlink("/etc/passwd", source.path().join("escape")).unwrap();
        let destination = tempfile::tempdir().unwrap();
        let error = ExtensionInstaller::new(destination.path().join("extensions"))
            .install(source.path().to_str().unwrap(), SourceOptions::default())
            .unwrap_err();
        assert!(error.to_string().contains("symbolic links"));
    }
}

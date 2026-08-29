use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{Read, Write},
    os::{fd::AsRawFd, unix::fs::MetadataExt, unix::process::CommandExt},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use rustix::fs::{MemfdFlags, Mode, OFlags, SealFlags, fcntl_add_seals, memfd_create, open};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::store::{ProcessExecutableGrant, SharedEventStore};

const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const MAX_TIMEOUT_MS: u64 = 120_000;
const MAX_ARGUMENTS: usize = 128;
const MAX_ARGUMENT_BYTES: usize = 64 * 1024;
const MAX_OUTPUT_BYTES: usize = 1024 * 1024;
const MAX_EXECUTABLE_BYTES: u64 = 256 * 1024 * 1024;
// chisle: one process globally; use a bounded semaphore if parallel workloads become necessary.
static PROCESS_EXECUTION_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone)]
pub struct ProcessHost {
    extension_id: String,
    store: SharedEventStore,
    action_enabled: Arc<AtomicBool>,
    effects: Arc<Mutex<Vec<ProcessEffect>>>,
}

#[derive(Debug)]
pub struct ProcessEffect {
    pub event_type: String,
    pub payload: Value,
}

#[derive(Debug, Deserialize)]
pub struct ProcessRequest {
    pub executable: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub cwd: String,
    pub timeout_ms: Option<u64>,
    pub filesystem_root: Option<String>,
    #[serde(default)]
    pub filesystem_access: FilesystemAccess,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FilesystemAccess {
    ReadOnly,
    #[default]
    ReadWrite,
}

#[derive(Debug, Serialize)]
pub struct ProcessResult {
    pub status: &'static str,
    pub success: bool,
    pub code: Option<i32>,
    pub signal: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub stdout_bytes: usize,
    pub stderr_bytes: usize,
    pub stdout_utf8: bool,
    pub stderr_utf8: bool,
    pub duration_ms: u64,
}

pub struct ProcessActionGuard {
    enabled: Arc<AtomicBool>,
}

impl Drop for ProcessActionGuard {
    fn drop(&mut self) {
        self.enabled.store(false, Ordering::Release);
    }
}

impl ProcessHost {
    pub fn new(extension_id: impl Into<String>, store: SharedEventStore) -> Self {
        Self {
            extension_id: extension_id.into(),
            store,
            action_enabled: Arc::new(AtomicBool::new(false)),
            effects: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn begin_action(&self) -> Result<ProcessActionGuard> {
        self.clear_effects()?;
        if self.action_enabled.swap(true, Ordering::AcqRel) {
            bail!("process action context is already active");
        }
        Ok(ProcessActionGuard {
            enabled: self.action_enabled.clone(),
        })
    }

    pub fn clear_effects(&self) -> Result<()> {
        self.effects
            .lock()
            .map_err(|_| anyhow::anyhow!("process effect journal lock poisoned"))?
            .clear();
        Ok(())
    }

    pub fn take_effects(&self) -> Result<Vec<ProcessEffect>> {
        Ok(std::mem::take(&mut *self.effects.lock().map_err(|_| {
            anyhow::anyhow!("process effect journal lock poisoned")
        })?))
    }

    pub fn run(&self, request: ProcessRequest) -> Result<ProcessResult> {
        if !self.action_enabled.load(Ordering::Acquire) {
            bail!("process execution is only available during a registered tool action");
        }
        validate_request(&request)?;
        let _execution = PROCESS_EXECUTION_LOCK
            .lock()
            .map_err(|_| anyhow::anyhow!("process execution lock poisoned"))?;
        let grant = self.executable_grant(&request.executable)?;
        let image = verified_executable(&grant)?;
        let (filesystem_root, root_handle, cwd) =
            self.authorized_cwd(&request.cwd, request.filesystem_root.as_deref())?;
        let timeout = Duration::from_millis(request.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS));
        let started = Instant::now();
        let execution = execute_sandboxed(
            &image,
            &filesystem_root,
            root_handle,
            &cwd,
            &request.args,
            &request.filesystem_access,
            timeout,
        );
        let duration_ms = started.elapsed().as_millis() as u64;
        let result = execution?;
        self.effects
            .lock()
            .map_err(|_| anyhow::anyhow!("process effect journal lock poisoned"))?
            .push(ProcessEffect {
                event_type: "process.execution.completed".into(),
                payload: json!({
                    "executable": grant.alias,
                    "executable_sha256": grant.sha256,
                    "cwd": request.cwd,
                    "filesystem_root": filesystem_root,
                    "filesystem_access": request.filesystem_access,
                    "status": result.status,
                    "success": result.success,
                    "code": result.code,
                    "signal": result.signal,
                    "duration_ms": duration_ms,
                    "stdout_bytes": result.stdout_bytes,
                    "stderr_bytes": result.stderr_bytes,
                }),
            });
        Ok(ProcessResult {
            duration_ms,
            ..result
        })
    }

    fn executable_grant(&self, alias: &str) -> Result<ProcessExecutableGrant> {
        let grants = self
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("event store lock poisoned"))?
            .extension_process_executables(&self.extension_id)?;
        grants
            .into_iter()
            .find(|grant| grant.alias == alias)
            .with_context(|| format!("process executable alias '{alias}' is not granted"))
    }

    fn authorized_cwd(
        &self,
        requested: &str,
        requested_root: Option<&str>,
    ) -> Result<(PathBuf, File, PathBuf)> {
        let path = strict_absolute_path(requested)?;
        let canonical = fs::canonicalize(&path)
            .with_context(|| format!("process cwd '{}' does not exist", path.display()))?;
        if canonical != path || !canonical.is_dir() {
            bail!("process cwd must be an existing canonical directory without symbolic links");
        }
        let roots = self
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("event store lock poisoned"))?
            .extension_filesystem_roots(&self.extension_id)?;
        let root = if let Some(requested_root) = requested_root {
            let requested_root = strict_absolute_path(requested_root)?;
            if !roots.iter().any(|root| Path::new(root) == requested_root) {
                bail!("requested process filesystem root is not an exact extension grant");
            }
            if !canonical.starts_with(&requested_root) {
                bail!("process cwd is outside the requested filesystem root");
            }
            requested_root
        } else {
            roots
                .into_iter()
                .map(PathBuf::from)
                .find(|root| canonical.starts_with(root))
                .context("process cwd is outside the extension's filesystem grants")?
        };
        let root_handle = File::from(open(
            &root,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?);
        let still_granted = self
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("event store lock poisoned"))?
            .extension_filesystem_roots(&self.extension_id)?
            .into_iter()
            .any(|granted| Path::new(&granted) == root);
        if !still_granted {
            bail!("process filesystem grant changed while opening the sandbox");
        }
        Ok((root, root_handle, canonical))
    }
}

#[cfg(test)]
pub fn process_backend_available() -> bool {
    Path::new("/usr/bin/bwrap").is_file() && ProcessCgroup::create().is_ok()
}

pub fn normalize_executable_grants(
    requested: &BTreeMap<String, String>,
) -> Result<Vec<ProcessExecutableGrant>> {
    if requested.len() > 32 {
        bail!("at most 32 process executables may be granted");
    }
    let mut paths = BTreeSet::new();
    let mut grants = Vec::new();
    for (alias, requested_path) in requested {
        validate_alias(alias)?;
        let path = strict_absolute_path(requested_path)?;
        let canonical = fs::canonicalize(&path)
            .with_context(|| format!("process executable '{}' does not exist", path.display()))?;
        if !paths.insert(canonical.clone()) {
            bail!("process executable paths must be unique");
        }
        let metadata = fs::metadata(&canonical)?;
        if !metadata.is_file() || metadata.mode() & 0o111 == 0 {
            bail!(
                "process executable '{}' is not an executable regular file",
                canonical.display()
            );
        }
        let image = read_bounded(&canonical, MAX_EXECUTABLE_BYTES)?;
        if !image.starts_with(b"\x7fELF") {
            bail!(
                "process executable '{}' is not a native ELF image",
                canonical.display()
            );
        }
        grants.push(ProcessExecutableGrant {
            alias: alias.clone(),
            path: canonical.to_string_lossy().into_owned(),
            identity: Some(format!("{}:{}", metadata.dev(), metadata.ino())),
            sha256: sha256(&image),
        });
    }
    Ok(grants)
}

fn validate_request(request: &ProcessRequest) -> Result<()> {
    validate_alias(&request.executable)?;
    if request.args.len() > MAX_ARGUMENTS {
        bail!("process arguments exceed the 128 argument limit");
    }
    let bytes = request.args.iter().try_fold(0usize, |total, argument| {
        if argument.contains('\0') {
            bail!("process arguments must not contain NUL bytes");
        }
        total
            .checked_add(argument.len())
            .context("process argument size overflow")
    })?;
    if bytes > MAX_ARGUMENT_BYTES {
        bail!("process arguments exceed the 64 KiB limit");
    }
    if request.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS) == 0
        || request.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS) > MAX_TIMEOUT_MS
    {
        bail!("process timeout must be between 1 and 120000 milliseconds");
    }
    Ok(())
}

fn validate_alias(alias: &str) -> Result<()> {
    if alias.is_empty()
        || alias.len() > 64
        || !alias
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        bail!("process executable aliases must use 1-64 ASCII letters, numbers, '.', '_', or '-'");
    }
    Ok(())
}

fn strict_absolute_path(value: &str) -> Result<PathBuf> {
    let path = PathBuf::from(value);
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        bail!("process paths must be absolute without '.' or '..' components");
    }
    Ok(path)
}

fn verified_executable(grant: &ProcessExecutableGrant) -> Result<Vec<u8>> {
    let path = Path::new(&grant.path);
    let metadata = fs::metadata(path)?;
    let identity = format!("{}:{}", metadata.dev(), metadata.ino());
    if grant.identity.as_deref() != Some(&identity) {
        bail!("granted process executable changed identity; grant it again");
    }
    let image = read_bounded(path, MAX_EXECUTABLE_BYTES)?;
    if sha256(&image) != grant.sha256 || !image.starts_with(b"\x7fELF") {
        bail!("granted process executable changed content; grant it again");
    }
    Ok(image)
}

fn execute_sandboxed(
    image: &[u8],
    filesystem_root: &Path,
    root: File,
    cwd: &Path,
    args: &[String],
    filesystem_access: &FilesystemAccess,
    timeout: Duration,
) -> Result<ProcessResult> {
    let memfd = memfd_create("habibi-process", MemfdFlags::ALLOW_SEALING)?;
    let mut executable = File::from(memfd);
    executable.write_all(image)?;
    executable.flush()?;
    fcntl_add_seals(
        &executable,
        SealFlags::SEAL | SealFlags::SHRINK | SealFlags::GROW | SealFlags::WRITE,
    )?;
    clear_cloexec(executable.as_raw_fd())?;
    clear_cloexec(root.as_raw_fd())?;
    let cgroup = ProcessCgroup::create()?;
    let cgroup_procs = fs::OpenOptions::new()
        .write(true)
        .open(cgroup.path.join("cgroup.procs"))?;

    let mut command = Command::new("/usr/bin/bwrap");
    command
        .env_clear()
        .env("LANG", "C")
        .env("LC_ALL", "C")
        .env("TZ", "UTC")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .args([
            "--unshare-all",
            "--unshare-user",
            "--disable-userns",
            "--die-with-parent",
            "--new-session",
            "--tmpfs",
            "/",
            "--dir",
            "/usr",
            "--ro-bind",
            "/usr",
            "/usr",
            "--dir",
            "/lib",
            "--ro-bind",
            "/lib",
            "/lib",
            "--dir",
            "/lib64",
            "--ro-bind",
            "/lib64",
            "/lib64",
            "--dir",
            "/proc",
            "--proc",
            "/proc",
            "--dir",
            "/dev",
            "--dev",
            "/dev",
            "--dir",
            "/tmp",
            "--tmpfs",
            "/tmp",
        ]);
    add_sandbox_directory(&mut command, filesystem_root);
    command
        .arg(match filesystem_access {
            FilesystemAccess::ReadOnly => "--ro-bind",
            FilesystemAccess::ReadWrite => "--bind",
        })
        .arg(format!("/proc/self/fd/{}", root.as_raw_fd()))
        .arg(filesystem_root)
        .arg("--chdir")
        .arg(cwd)
        .arg("--")
        .arg(format!("/proc/self/fd/{}", executable.as_raw_fd()))
        .args(args);
    let cgroup_fd = cgroup_procs.as_raw_fd();
    let process_limit = current_user_task_count()?.saturating_add(256) as libc::rlim_t;
    unsafe {
        command.pre_exec(move || {
            let nofile = libc::rlimit {
                rlim_cur: 256,
                rlim_max: 256,
            };
            let nproc = libc::rlimit {
                rlim_cur: process_limit,
                rlim_max: process_limit,
            };
            let core = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            if libc::setrlimit(libc::RLIMIT_NOFILE, &nofile) != 0
                || libc::setrlimit(libc::RLIMIT_NPROC, &nproc) != 0
                || libc::setrlimit(libc::RLIMIT_CORE, &core) != 0
            {
                return Err(std::io::Error::last_os_error());
            }
            let moved = libc::write(cgroup_fd, b"0".as_ptr().cast(), 1);
            if moved == 1 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        });
    }
    let mut child = command
        .spawn()
        .context("failed to start the process sandbox")?;
    drop(cgroup_procs);
    let stdout = child.stdout.take().context("process stdout pipe missing")?;
    let stderr = child.stderr.take().context("process stderr pipe missing")?;
    let output_limit = Arc::new(AtomicBool::new(false));
    let stdout_limit = output_limit.clone();
    let stderr_limit = output_limit.clone();
    let stdout_reader = thread::spawn(move || read_output(stdout, stdout_limit));
    let stderr_reader = thread::spawn(move || read_output(stderr, stderr_limit));
    let started = Instant::now();
    let (status_name, status) = loop {
        if let Some(status) = child.try_wait()? {
            break ("exited", status);
        }
        if output_limit.load(Ordering::Acquire) {
            cgroup.kill()?;
            break ("output_limit", child.wait()?);
        }
        if started.elapsed() >= timeout {
            cgroup.kill()?;
            break ("timed_out", child.wait()?);
        }
        thread::sleep(Duration::from_millis(10));
    };
    cgroup.kill()?;
    let stdout = stdout_reader
        .join()
        .map_err(|_| anyhow::anyhow!("process stdout reader panicked"))??;
    let stderr = stderr_reader
        .join()
        .map_err(|_| anyhow::anyhow!("process stderr reader panicked"))??;
    let status_name = if stdout.overflow || stderr.overflow {
        "output_limit"
    } else {
        status_name
    };
    Ok(ProcessResult {
        status: status_name,
        success: status_name == "exited" && status.success(),
        code: status.code(),
        signal: std::os::unix::process::ExitStatusExt::signal(&status),
        stdout: String::from_utf8_lossy(&stdout.bytes).into_owned(),
        stderr: String::from_utf8_lossy(&stderr.bytes).into_owned(),
        stdout_bytes: stdout.total,
        stderr_bytes: stderr.total,
        stdout_utf8: std::str::from_utf8(&stdout.bytes).is_ok(),
        stderr_utf8: std::str::from_utf8(&stderr.bytes).is_ok(),
        duration_ms: 0,
    })
}

fn add_sandbox_directory(command: &mut Command, directory: &Path) {
    let mut current = PathBuf::from("/");
    for component in directory.components().skip(1) {
        current.push(component.as_os_str());
        command.arg("--dir").arg(&current);
    }
}

struct BoundedOutput {
    bytes: Vec<u8>,
    total: usize,
    overflow: bool,
}

fn read_output(mut reader: impl Read, output_limit: Arc<AtomicBool>) -> Result<BoundedOutput> {
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 8192];
    let mut total = 0usize;
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        total = total.saturating_add(count);
        if total > MAX_OUTPUT_BYTES {
            output_limit.store(true, Ordering::Release);
        }
        let remaining = MAX_OUTPUT_BYTES.saturating_sub(bytes.len());
        bytes.extend_from_slice(&buffer[..count.min(remaining)]);
    }
    Ok(BoundedOutput {
        bytes,
        total,
        overflow: total > MAX_OUTPUT_BYTES,
    })
}

struct ProcessCgroup {
    path: PathBuf,
}

impl ProcessCgroup {
    fn create() -> Result<Self> {
        let membership = fs::read_to_string("/proc/self/cgroup")?;
        let relative = membership
            .lines()
            .find_map(|line| line.strip_prefix("0::"))
            .context("cgroup v2 membership is unavailable")?;
        let path = Path::new("/sys/fs/cgroup")
            .join(relative.trim_start_matches('/'))
            .join(format!("habibi-process-{}", Uuid::now_v7()));
        fs::create_dir(&path).context("process cgroup delegation is unavailable")?;
        if let Err(error) = fs::write(path.join("cgroup.max.descendants"), "0") {
            let _ = fs::remove_dir(&path);
            return Err(error.into());
        }
        let pids_max = path.join("pids.max");
        if pids_max.exists()
            && let Err(error) = fs::write(pids_max, "128")
        {
            let _ = fs::remove_dir(&path);
            return Err(error.into());
        }
        Ok(Self { path })
    }

    fn kill(&self) -> Result<()> {
        fs::write(self.path.join("cgroup.kill"), "1")?;
        for _ in 0..100 {
            let events = fs::read_to_string(self.path.join("cgroup.events"))?;
            if events.lines().any(|line| line == "populated 0") {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(10));
        }
        bail!("process cgroup did not become empty")
    }
}

impl Drop for ProcessCgroup {
    fn drop(&mut self) {
        let _ = fs::write(self.path.join("cgroup.kill"), "1");
        let _ = fs::remove_dir(&self.path);
    }
}

fn current_user_task_count() -> Result<usize> {
    let uid = unsafe { libc::geteuid() };
    let mut count = 0usize;
    for entry in fs::read_dir("/proc")? {
        let entry = entry?;
        if !entry
            .file_name()
            .as_encoded_bytes()
            .iter()
            .all(u8::is_ascii_digit)
        {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.uid() == uid
            && let Ok(tasks) = fs::read_dir(entry.path().join("task"))
        {
            count = count.saturating_add(tasks.count());
        }
    }
    Ok(count)
}

fn clear_cloexec(fd: i32) -> Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) } < 0 {
        bail!("failed to make process sandbox descriptor inheritable");
    }
    Ok(())
}

fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let metadata = fs::metadata(path)?;
    if metadata.len() > limit {
        bail!("process executable exceeds the 256 MiB limit");
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    File::open(path)?.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        bail!("process executable exceeds the 256 MiB limit");
    }
    Ok(bytes)
}

fn sha256(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::EventStore;

    fn backend_available() -> bool {
        process_backend_available()
    }

    fn host(root: &Path, executable: &str) -> ProcessHost {
        let store = EventStore::open(":memory:").unwrap().shared();
        let requested = BTreeMap::from([("test".to_owned(), executable.to_owned())]);
        let grants = normalize_executable_grants(&requested).unwrap();
        {
            let mut store = store.lock().unwrap();
            store
                .set_extension_filesystem_roots("process", &[root.to_string_lossy().into_owned()])
                .unwrap();
            store
                .set_extension_process_executables("process", &grants)
                .unwrap();
        }
        ProcessHost::new("process", store)
    }

    #[test]
    fn changed_executables_are_rejected() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("printf");
        fs::copy("/usr/bin/printf", &executable).unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        let host = host(root.path(), executable.to_str().unwrap());
        fs::OpenOptions::new()
            .append(true)
            .open(&executable)
            .unwrap()
            .write_all(b"changed")
            .unwrap();
        let _guard = host.begin_action().unwrap();
        assert!(
            host.run(ProcessRequest {
                executable: "test".into(),
                args: vec![],
                cwd: root.path().to_string_lossy().into_owned(),
                timeout_ms: None,
                filesystem_root: None,
                filesystem_access: FilesystemAccess::ReadWrite,
            })
            .unwrap_err()
            .to_string()
            .contains("changed content")
        );
    }

    #[test]
    fn output_overflow_kills_the_sandbox_and_stays_bounded() {
        if !backend_available() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let host = host(root.path(), "/usr/bin/yes");
        let _guard = host.begin_action().unwrap();
        let result = host
            .run(ProcessRequest {
                executable: "test".into(),
                args: vec![],
                cwd: root.path().to_string_lossy().into_owned(),
                timeout_ms: Some(5_000),
                filesystem_root: None,
                filesystem_access: FilesystemAccess::ReadWrite,
            })
            .unwrap();
        assert_eq!(result.status, "output_limit");
        assert_eq!(result.stdout.len(), MAX_OUTPUT_BYTES);
        assert!(result.stdout_bytes > MAX_OUTPUT_BYTES);
    }

    #[test]
    fn timeout_kills_descendants() {
        if !backend_available() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let host = host(root.path(), "/usr/bin/bash");
        let _guard = host.begin_action().unwrap();
        let result = host
            .run(ProcessRequest {
                executable: "test".into(),
                args: vec![
                    "-c".into(),
                    "sleep 10 & grep NSpid /proc/$!/status > child.pid; wait".into(),
                ],
                cwd: root.path().to_string_lossy().into_owned(),
                timeout_ms: Some(100),
                filesystem_root: None,
                filesystem_access: FilesystemAccess::ReadWrite,
            })
            .unwrap();
        assert_eq!(result.status, "timed_out");
        let child = fs::read_to_string(root.path().join("child.pid")).unwrap();
        assert!(child.starts_with("NSpid:"));
    }

    #[test]
    fn timeout_kills_the_sandbox() {
        if !backend_available() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let host = host(root.path(), "/usr/bin/sleep");
        let _guard = host.begin_action().unwrap();
        let result = host
            .run(ProcessRequest {
                executable: "test".into(),
                args: vec!["10".into()],
                cwd: root.path().to_string_lossy().into_owned(),
                timeout_ms: Some(30),
                filesystem_root: None,
                filesystem_access: FilesystemAccess::ReadWrite,
            })
            .unwrap();
        assert_eq!(result.status, "timed_out");
        assert!(!result.success);
    }

    #[test]
    fn exact_read_only_root_supports_git_inspection() {
        if !backend_available() {
            return;
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let host = host(root, "/usr/bin/git");
        let _guard = host.begin_action().unwrap();
        let result = host
            .run(ProcessRequest {
                executable: "test".into(),
                args: vec![
                    "--no-pager".into(),
                    "--no-optional-locks".into(),
                    "status".into(),
                    "--short".into(),
                ],
                cwd: root.to_string_lossy().into_owned(),
                timeout_ms: Some(5_000),
                filesystem_root: Some(root.to_string_lossy().into_owned()),
                filesystem_access: FilesystemAccess::ReadOnly,
            })
            .unwrap();
        assert!(result.success, "{result:?}");
    }

    #[test]
    fn exact_read_only_roots_prevent_mutation() {
        if !backend_available() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("note.txt");
        fs::write(&file, "unchanged").unwrap();
        let host = host(root.path(), "/usr/bin/touch");
        let _guard = host.begin_action().unwrap();
        let result = host
            .run(ProcessRequest {
                executable: "test".into(),
                args: vec![file.to_string_lossy().into_owned()],
                cwd: root.path().to_string_lossy().into_owned(),
                timeout_ms: Some(5_000),
                filesystem_root: Some(root.path().to_string_lossy().into_owned()),
                filesystem_access: FilesystemAccess::ReadOnly,
            })
            .unwrap();
        assert!(!result.success);
        assert_eq!(fs::read_to_string(file).unwrap(), "unchanged");
    }

    #[test]
    fn sandbox_cannot_read_ambient_user_files() {
        if !backend_available() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        fs::write(outside.path(), "ambient-secret").unwrap();
        let host = host(root.path(), "/usr/bin/cat");
        let _guard = host.begin_action().unwrap();
        let result = host
            .run(ProcessRequest {
                executable: "test".into(),
                args: vec![outside.path().to_string_lossy().into_owned()],
                cwd: root.path().to_string_lossy().into_owned(),
                timeout_ms: Some(5_000),
                filesystem_root: None,
                filesystem_access: FilesystemAccess::ReadWrite,
            })
            .unwrap();
        assert!(!result.success);
        assert!(!result.stdout.contains("ambient-secret"));
    }

    #[test]
    fn process_environment_is_fixed_and_non_secret() {
        if !backend_available() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let host = host(root.path(), "/usr/bin/env");
        let _guard = host.begin_action().unwrap();
        let result = host
            .run(ProcessRequest {
                executable: "test".into(),
                args: vec![],
                cwd: root.path().to_string_lossy().into_owned(),
                timeout_ms: Some(5_000),
                filesystem_root: None,
                filesystem_access: FilesystemAccess::ReadWrite,
            })
            .unwrap();
        assert!(result.success, "{result:?}");
        assert!(result.stdout.contains("LANG=C\n"));
        assert!(result.stdout.contains("LC_ALL=C\n"));
        assert!(result.stdout.contains("TZ=UTC\n"));
        assert!(!result.stdout.contains("HOME="));
        assert!(!result.stdout.contains("TOKEN="));
    }

    #[test]
    fn process_execution_is_action_only_and_shell_free() {
        if !backend_available() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let host = host(root.path(), "/usr/bin/printf");
        let request = || ProcessRequest {
            executable: "test".into(),
            args: vec!["%s".into(), ";$(touch escaped)".into()],
            cwd: root.path().to_string_lossy().into_owned(),
            timeout_ms: Some(5_000),
            filesystem_root: None,
            filesystem_access: FilesystemAccess::ReadWrite,
        };
        assert!(
            host.run(request())
                .unwrap_err()
                .to_string()
                .contains("tool action")
        );
        let _guard = host.begin_action().unwrap();
        let result = host.run(request()).unwrap();
        assert!(result.success, "{result:?}");
        assert_eq!(result.stdout, ";$(touch escaped)");
        assert!(!root.path().join("escaped").exists());
        let effects = host.take_effects().unwrap();
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0].event_type, "process.execution.completed");
        assert!(!effects[0].payload.to_string().contains("touch escaped"));
    }
}

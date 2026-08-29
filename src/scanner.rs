use std::{fs, path::Path};

use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::extension::ExtensionManifest;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FindingSeverity {
    Warning,
    Blocker,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ScanFinding {
    pub severity: FindingSeverity,
    pub rule: String,
    pub file: String,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ScanReport {
    pub scanner_version: u32,
    pub scanned_at: String,
    pub passed: bool,
    pub files_scanned: usize,
    pub bytes_scanned: u64,
    pub warning_count: usize,
    pub blocker_count: usize,
    pub declared_capabilities: Vec<String>,
    pub findings: Vec<ScanFinding>,
}

pub fn scan_extension(directory: &Path, manifest: &ExtensionManifest) -> Result<ScanReport> {
    let mut state = ScanState::default();
    scan_directory(directory, directory, &mut state)?;
    let blocker_count = state
        .findings
        .iter()
        .filter(|finding| finding.severity == FindingSeverity::Blocker)
        .count();
    let warning_count = state.findings.len() - blocker_count;
    let capabilities = &manifest.capabilities;
    let mut declared_capabilities = Vec::new();
    for (name, enabled) in [
        ("web", capabilities.web),
        ("kv", capabilities.kv),
        ("events", capabilities.events),
        ("tools", capabilities.tools),
        ("context", capabilities.context),
        ("filesystem", capabilities.filesystem),
        ("process", capabilities.process),
        ("studio", capabilities.studio),
        ("search", capabilities.search),
    ] {
        if enabled {
            declared_capabilities.push(name.to_owned());
        }
    }
    Ok(ScanReport {
        scanner_version: 1,
        scanned_at: Utc::now().to_rfc3339(),
        passed: blocker_count == 0,
        files_scanned: state.files_scanned,
        bytes_scanned: state.bytes_scanned,
        warning_count,
        blocker_count,
        declared_capabilities,
        findings: state.findings,
    })
}

pub fn ensure_scan_passed(report: &ScanReport) -> Result<()> {
    if report.passed {
        return Ok(());
    }
    let summary = report
        .findings
        .iter()
        .filter(|finding| finding.severity == FindingSeverity::Blocker)
        .take(5)
        .map(|finding| format!("{}: {}", finding.file, finding.message))
        .collect::<Vec<_>>()
        .join("; ");
    bail!(
        "extension security/privacy scan found {} blocking issue(s): {summary}",
        report.blocker_count
    )
}

#[derive(Default)]
struct ScanState {
    files_scanned: usize,
    bytes_scanned: u64,
    findings: Vec<ScanFinding>,
}

fn scan_directory(root: &Path, directory: &Path, state: &mut ScanState) -> Result<()> {
    let mut entries = fs::read_dir(directory)?.collect::<std::result::Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            add(
                state,
                FindingSeverity::Blocker,
                "filesystem.symlink",
                root,
                &entry.path(),
                "Symbolic links are not allowed in extension packages.",
            );
            continue;
        }
        if file_type.is_dir() {
            scan_directory(root, &entry.path(), state)?;
        } else if file_type.is_file() {
            scan_file(root, &entry.path(), state)?;
        }
    }
    Ok(())
}

fn scan_file(root: &Path, path: &Path, state: &mut ScanState) -> Result<()> {
    let bytes = fs::read(path)?;
    state.files_scanned += 1;
    state.bytes_scanned = state.bytes_scanned.saturating_add(bytes.len() as u64);
    if bytes.contains(&0) {
        return Ok(());
    }
    let text = String::from_utf8(bytes)
        .with_context(|| format!("extension text file '{}' is not UTF-8", path.display()))?;
    let lower = text.to_lowercase();
    for marker in [
        "-----begin private key-----",
        "-----begin rsa private key-----",
        "-----begin openssh private key-----",
    ] {
        if lower.contains(marker) {
            add(
                state,
                FindingSeverity::Blocker,
                "secret.private_key",
                root,
                path,
                "Embedded private key material was detected.",
            );
        }
    }
    for marker in ["ghp_", "gho_", "github_pat_", "sk-proj-"] {
        if text.contains(marker) {
            add(
                state,
                FindingSeverity::Blocker,
                "secret.token",
                root,
                path,
                "A value resembling an access token was detected.",
            );
        }
    }

    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    if matches!(extension, "js" | "html" | "htm") {
        for (rule, marker, message) in [
            (
                "browser.dynamic_code",
                "eval(",
                "Dynamic JavaScript evaluation was detected.",
            ),
            (
                "browser.dynamic_code",
                "new function(",
                "Dynamic JavaScript function construction was detected.",
            ),
            (
                "browser.remote_script",
                "<script src=\"http",
                "A remotely hosted script is loaded into the extension UI.",
            ),
            (
                "browser.remote_script",
                "<script src='http",
                "A remotely hosted script is loaded into the extension UI.",
            ),
        ] {
            if lower.contains(marker) {
                add(state, FindingSeverity::Blocker, rule, root, path, message);
            }
        }
        for (rule, marker, message) in [
            (
                "privacy.outbound_network",
                "fetch(\"http",
                "Browser code sends data to an external HTTP endpoint.",
            ),
            (
                "privacy.outbound_network",
                "fetch('http",
                "Browser code sends data to an external HTTP endpoint.",
            ),
            (
                "privacy.outbound_network",
                "websocket(",
                "Browser code opens a WebSocket connection.",
            ),
            (
                "privacy.outbound_network",
                "sendbeacon(",
                "Browser code uses a background beacon.",
            ),
            (
                "privacy.external_frame",
                "<iframe",
                "The extension UI embeds an iframe.",
            ),
            (
                "privacy.browser_storage",
                "localstorage",
                "The extension uses browser-persistent local storage.",
            ),
            (
                "runtime.management_api",
                "/api/extensions",
                "The extension references Habibi's extension-management API.",
            ),
        ] {
            if lower.contains(marker) {
                add(state, FindingSeverity::Warning, rule, root, path, message);
            }
        }
    }
    if extension == "lua" {
        for marker in [
            "os.",
            "io.",
            "require(",
            "loadfile(",
            "dofile(",
            "package.",
            "debug.",
        ] {
            if lower.match_indices(marker).any(|(index, _)| {
                index == 0
                    || !lower.as_bytes()[index - 1].is_ascii_alphanumeric()
                        && lower.as_bytes()[index - 1] != b'_'
            }) {
                add(
                    state,
                    FindingSeverity::Warning,
                    "lua.restricted_api",
                    root,
                    path,
                    "Lua source references an API that is unavailable in Habibi's sandbox.",
                );
                break;
            }
        }
    }
    Ok(())
}

fn add(
    state: &mut ScanState,
    severity: FindingSeverity,
    rule: &str,
    root: &Path,
    path: &Path,
    message: &str,
) {
    state.findings.push(ScanFinding {
        severity,
        rule: rule.to_owned(),
        file: path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/"),
        message: message.to_owned(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extension::ExtensionCapabilities;

    fn manifest() -> ExtensionManifest {
        ExtensionManifest {
            id: "scan-test".into(),
            name: "Scan Test".into(),
            version: "1.0.0".into(),
            description: None,
            api_version: 1,
            capabilities: ExtensionCapabilities {
                web: true,
                ..ExtensionCapabilities::default()
            },
            web: None,
        }
    }

    #[test]
    fn does_not_mistake_studio_for_the_restricted_io_api() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(
            directory.path().join("extension.lua"),
            "return habibi.studio.list()\n",
        )
        .unwrap();
        let report = scan_extension(directory.path(), &manifest()).unwrap();
        assert_eq!(report.warning_count, 0);
    }

    #[test]
    fn blocks_remote_scripts_and_reports_network_usage() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(
            directory.path().join("app.html"),
            "<script src=\"https://bad.example/app.js\"></script><script>fetch('https://api.example')</script>",
        )
        .unwrap();
        let report = scan_extension(directory.path(), &manifest()).unwrap();
        assert!(!report.passed);
        assert_eq!(report.blocker_count, 1);
        assert_eq!(report.warning_count, 1);
    }
}

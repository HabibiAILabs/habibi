# Security policy

## Extension trust model

Installed Habibi extensions are fully trusted local code. Lua execution is capability-scoped and sandboxed, but an extension may also provide browser JavaScript on Habibi's origin. Capabilities and scanner findings are review aids, not a security boundary for hostile packages.

Install extensions only from sources you trust. Prefer immutable Git tags or reviewed commits and verify the recorded source, revision, version, content hash, and capabilities.

## Automatic extension scan

Every install and update is copied into a private staging directory and scanned before it can replace or enter the active runtime. The scanner currently checks for:

- Symbolic links and unsupported filesystem entries
- Embedded private keys and common access-token markers
- Remotely hosted browser scripts
- Dynamic JavaScript evaluation
- Outbound browser network APIs
- Embedded frames and persistent browser storage
- References to management APIs
- Lua APIs unavailable in Habibi's sandbox

Blocking findings abort installation. Warnings are recorded in `.habibi-install.json`, printed by the CLI, and displayed in the Extensions UI.

Static scanning cannot prove that an extension is safe or private. It may miss obfuscated behavior, dynamically constructed endpoints, logic flaws, or data disclosure through allowed APIs.

## Filesystem grants

The filesystem capability defaults to no access. Users grant existing canonical directories per
extension. Host operations are capability-confined, reject symbolic links and special files, bound
all reads/searches, serialize mutations, require hashes for existing-file changes, and use atomic
replacement. Grants reduce accidental scope; they do not make a fully trusted extension hostile-code
safe. File contents and patch arguments remain present in durable action events and exact model logs.

## Process execution

Process execution is Linux-only and default-deny. Users grant exact native executable aliases and
filesystem roots independently. Habibi verifies executable identity and SHA-256, executes sealed
verified bytes without a shell, clears the environment, disables network access, bounds arguments,
time, and output, and kills the complete delegated cgroup after every run. The API is available only
during registered tool actions and fails closed without Bubblewrap or delegated cgroup v2 support.

The selected filesystem root is mounted read-write inside the sandbox; process writes do not receive
Workspace's per-file hash checks. System runtime libraries under `/usr` and `/lib*`, plus Bubblewrap
itself, are trusted platform dependencies and are not pinned by the executable grant. Explicitly
granting a native interpreter grants its normal argv authority. Arguments and returned stdout/stderr
become durable action/model history. Never pass credentials or other secrets through process tools.
The host-authored process effect omits argv, environment, stdout, and stderr.

## Extension Studio

Extension Studio is available only on loopback binds and keeps drafts in a dedicated host-owned root
separate from installed packages. Draft edits are relative, text-only, bounded, no-follow, and
hash-checked. The model may author and validate but cannot install. Installation requires an explicit
browser confirmation for a passing immutable hash and reruns the normal scanner/installer pipeline.
Scanning and isolated loading assist review; they do not make installed code untrusted or safe.

## Web search

Web Search exposes fixed Brave and SearXNG adapters rather than generic HTTP. Brave uses a host-only
header credential. SearXNG must be one configured HTTPS origin or loopback HTTP service. Redirects,
oversized responses, non-JSON responses, malformed citation URLs, and calls outside tool actions are
rejected. Provider errors omit response bodies and credentials.

Queries leave the machine and are durable in Habibi action/model history. Search snippets are
untrusted input that may contain prompt injection, falsehoods, or copyrighted text. Version 0.1 does
not fetch pages. Never search credentials, private source, or personal data unless disclosure to the
configured provider is intended.

## Local embedding model

Habibi never downloads an embedding model during ordinary startup. `habibi embeddings install` is the only model download path. It uses an immutable repository revision, bounds each file by its pinned size, verifies SHA-256 before installation, and atomically installs the complete model directory. Startup verifies every model file again before local ONNX inference. Model identity, artifact metadata, and licenses are checked into `models/`; model binaries are not stored in Git or embedded in the Habibi executable.

## Reporting vulnerabilities

Please use GitHub private vulnerability reporting for the affected repository when available. Do not include access tokens, OAuth credentials, private event data, or database contents in a public issue.

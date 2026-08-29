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

## Reporting vulnerabilities

Please use GitHub private vulnerability reporting for the affected repository when available. Do not include access tokens, OAuth credentials, private event data, or database contents in a public issue.

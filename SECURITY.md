# Security Policy

## Boundaries

Roblox Multi-Account Launcher is an external Rust launcher and Windows process/window manager.

It does not:

- read, parse, copy, print, modify, or delete Roblox cookie contents;
- store passwords, `.ROBLOSECURITY`, authentication tickets, account tokens, or private-server parameters;
- inject DLLs, hook Roblox, read/modify process memory, patch binaries, or manipulate anti-cheat systems;
- hide processes or handles, install drivers, spoof hardware, or intercept network traffic;
- automate gameplay, farming, quests, or anti-termination behavior;
- require administrator privileges;
- transmit telemetry, logs, diagnostics, or process information.

Multi-instance protection owns two native mutex objects named `ROBLOX_singletonMutex` and `ROBLOX_singletonEvent` on one Rust owner thread. It never opens or closes handles in Roblox processes. Teleport protection opens `RobloxCookies.dat` with read access and no sharing and retains only the Windows handle. Automated tests use project-contained fixture files and isolated test mutex names.

Per-instance path isolation creates only verified NTFS mount-point junctions named `Client-NNNN` under `%LOCALAPPDATA%\RobloxMultiAccountLauncher\Instances`. Each junction targets a detected Roblox version directory containing `RobloxPlayerBeta.exe`; no installation files are copied, modified, or deleted. Cleanup refuses non-junction entries and never recursively deletes through an alias. An alias associated with a running client is retained.

## Diagnostics

Diagnostics redact common cookie, ticket, token, bearer, authentication, and private-server values. The optional local-state tracer uses native directory notifications and records only relative path, timestamp, create/write/delete/rename action, and `process=unavailable`; it does not open changed files. PowerShell Assist runs invisibly with captured output, bounded timeouts, cancellation, and no critical resource ownership.

Harmless settings, sanitized logs/history, per-client junction aliases, requested metadata traces/support bundles, and updater state are application-owned runtime data under `%LOCALAPPDATA%\RobloxMultiAccountLauncher` or a sibling `data` folder when the user deliberately enables `portable.flag`. Multi-account mode, authentication material, private links, and Roblox file contents are not persisted.

## Update security

The updater is configured for this public GitHub repository. It makes unauthenticated HTTPS requests only for public GitHub release metadata and explicitly selected release assets; no diagnostics, Roblox state, or credentials are transmitted. Network failures are non-fatal and automatic checks never install updates.

Installation requires a release tag and machine-readable manifest that agree with `Cargo.toml` semantic versioning, channel, release URL, exact executable asset name, and a locally calculated SHA-256 hash. The staged executable is re-verified by the same-binary helper immediately before replacement. Replacement targets the exact portable executable that was launched and preserves a SHA-256-verified previous executable for explicit rollback. Update and rollback replacement are blocked while Multi-Account Mode or Roblox clients are active.

SHA-256 alone does not authenticate a compromised release source. Signed-manifest verification is deferred until the public release pipeline and offline private-key handling exist; configuring a public key before the verifier is implemented fails closed and refuses installation.

## Runtime dependencies

The portable release requires normal supported Windows components only. It has no .NET, Python, Java, Node.js, WebView2, Edge, Chromium, Electron, or PowerShell requirement for core functionality.

## Reporting security issues

Use [GitHub private vulnerability reporting](https://github.com/Wolfyisdabest/Roblox-Multi-Account-Launcher/security/advisories/new). Do not include credentials, cookie values, authentication tickets, private links, or personal data in a public issue.

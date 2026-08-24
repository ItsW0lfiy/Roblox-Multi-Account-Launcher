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

Harmless settings, sanitized logs, per-client junction aliases, and explicitly requested metadata traces are application-owned runtime data under `%LOCALAPPDATA%\RobloxMultiAccountLauncher`. Multi-account mode, authentication material, and Roblox file contents are not persisted.

## Runtime dependencies

The portable release requires normal supported Windows components only. It has no .NET, Python, Java, Node.js, WebView2, Edge, Chromium, Electron, or PowerShell requirement for core functionality.

## Reporting security issues

This repository is local and unpublished. If it is published later, configure a private security-reporting channel before accepting reports.

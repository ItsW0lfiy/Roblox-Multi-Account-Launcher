# Roblox Multi-Account Launcher

A portable Windows launcher and client manager implemented in Rust. The application uses native Windows APIs for all critical behavior and an `egui`/`eframe` desktop interface with no browser engine.

This is an independent community project. It is not affiliated with or endorsed by Roblox Corporation, Fishstrap, or Bloxstrap. Roblox does not officially support this multi-instance behavior, and platform behavior may change.

## Highlights

- One clear **Launch Roblox** action. In normal mode it launches Roblox normally; in multi-account mode each click prepares another client.
- Multi-account launches use native NTFS junctions under `%LOCALAPPDATA%\RobloxMultiAccountLauncher\Instances\Client-NNNN` so every client is started through a unique path without copying or modifying Roblox files.
- Multi-account mode always starts **off** and is never persisted.
- Multi-instance mechanism: one Rust owner thread holds mutex objects named `ROBLOX_singletonMutex` and `ROBLOX_singletonEvent`; teleport protection separately holds an exclusive, read-only handle to `RobloxCookies.dat`.
- Launch confirmation snapshots existing clients and requires a genuinely new `RobloxPlayerBeta.exe` PID to remain alive for a short stability window.
- Auto backend priority: Fishstrap, Bloxstrap, then stock Roblox.
- Read-only `roblox:` and `roblox-player:` protocol inspection.
- Client list with PID, uptime, RAM, window title, focus, graceful close, confirmed force close, and two-client layouts.
- Protection-aware tray lifecycle.
- Optional embedded PowerShell Assist with PowerShell 7, Windows PowerShell compatibility, or Rust-only mode.
- Optional metadata-only `Roblox\LocalStorage` change tracing for shared-login investigation; no file contents or credentials are read.
- Sanitized diagnostics, no telemetry, no credentials, no injection, and no administrator requirement.
- Portable Rust self-updater architecture for GitHub Releases: quiet checks, explicit user approval, SHA-256 verification, same-binary helper replacement, and no installer/service/admin requirement. Repository coordinates are intentionally not configured yet, so this build remains dormant.

## Architecture

Rust is authoritative for lifecycle, single-instance state, protection handles, process tracking, launching, queueing, backend detection, window management, settings, tray behavior, logging, and failures. PowerShell is an optional diagnostic assistant and never owns critical runtime state.

The GUI uses `egui`/`eframe` because it compiles into the executable, provides DPI-aware responsive rendering, requires no WebView/.NET/browser runtime, and supports a controlled high-contrast dark theme.

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

Shared-login investigation and its current experimental/manual-validation status are documented in [docs/LOGIN-STATE-INVESTIGATION.md](docs/LOGIN-STATE-INVESTIGATION.md).

## Build

Build requirements only:

- Windows 10/11 x64
- Rust 1.94 or newer with the `x86_64-pc-windows-msvc` target
- MSVC linker/build tools

```powershell
.\build-release.ps1
```

The script keeps Cargo caches and target output under `.tmp` and copies the validated executable to:

```text
dist\RobloxMultiAccountLauncher.exe
```

The release executable uses the Windows GUI subsystem and statically links the MSVC CRT. End users do not need Rust, Visual Studio, .NET, PowerShell 7, Python, Java, Node, WebView2, or a browser runtime.

## Local data

The application owns an explicit, small persistence layer under `%LOCALAPPDATA%\RobloxMultiAccountLauncher` for harmless settings, sanitized logs, per-client junction aliases, user-requested metadata-only local-state traces, and verified update staging under `Updates`. Set `RMAL_DATA_DIR` for controlled development/testing; the repository build and tests use a project-contained `.tmp` location. Multi-account state, Roblox cookies, credentials, authentication tickets, launch tokens, private-server parameters, and file contents are never persisted.

`eframe`'s generic persistence is deliberately disabled so there is one auditable application-owned settings format.

## PowerShell Assist

Auditable source scripts live in `scripts` and are embedded into the Rust executable at compile time. Engine order:

1. PowerShell 7 (`pwsh.exe`) — Full
2. Windows PowerShell 5.1 (`powershell.exe`) — Compatibility
3. No engine — Rust-only mode

Invocations use no profile, no interactive terminal, captured output/error, a hidden process, cancellation, and bounded timeouts. Diagnostic scripts are read-only. Repair inspection returns proposed actions; mutation requires a separate explicit GUI confirmation.

The known-good V1 PowerShell helper remains preserved under `legacy\powershell`.

## Portable updates

The Settings page contains update preferences and compact About/update status. The installed version comes only from `Cargo.toml`. Automatic checks default on, run after startup without blocking the GUI, and never install silently. Public releases require `RobloxMultiAccountLauncher.exe` plus `update-manifest.json`; the manifest must identify the same version/filename and contain the executable SHA-256 hash.

The updater targets the exact portable executable that is running. After explicit approval, it stages and verifies the release under `%LOCALAPPDATA%\RobloxMultiAccountLauncher\Updates`, starts a temporary copy of the same Rust executable in hidden helper mode, exits, replaces the original with rollback protection, restarts it, and removes owned staging/backup/helper files. It will not begin replacement while multi-account protection or Roblox clients are active.

No GitHub owner/repository is configured in this build. The UI therefore reports **Update status: Not configured**, performs no requests, and does not nag. See [docs/UPDATER.md](docs/UPDATER.md) for the future release contract and deferred signed-manifest requirement.

## Security and privacy

The launcher is an external process/window manager. It does not read cookie contents, inspect Roblox process memory, inject, hook, patch binaries, modify authentication data, bypass anti-cheat, automate gameplay, elevate, or transmit telemetry. See [SECURITY.md](SECURITY.md).

## Current limitations

- Roblox/Fishstrap/Bloxstrap command-line and protocol behavior can change.
- File-version display is best-effort and may show `Unknown`.
- Two-client layouts are primary; additional clients remain individually manageable.
- Graceful close depends on a Roblox window accepting `WM_CLOSE`.
- Holding both singleton names, two-client coexistence, teleporting, and bidirectional logout behavior have been manually validated three times on the validation machine. Roblox behavior remains undocumented and can change.
- Per-instance path isolation is implemented and fixture-tested, but the final in-launcher Client 1 → Client 2 test is still required. It is not claimed fixed until that succeeds.
- Login-state behavior is experimental/manual-validation-specific; no separate auth-state lock or credential handling was added and the exact causal local file remains unidentified.
- Real three-client, post-update, tray, and multi-monitor behavior still require manual validation.
- GitHub update checking and real portable self-replacement require a future public repository/release and manual validation. SHA-256 is mandatory; cryptographically signed manifests remain a required pre-public-release hardening step.

## License

MIT. See [LICENSE](LICENSE).

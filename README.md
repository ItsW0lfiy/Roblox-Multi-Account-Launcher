# Roblox Multi-Account Launcher

A portable Windows launcher and client manager implemented in Rust. The application uses native Windows APIs for all critical behavior and an `egui`/`eframe` desktop interface with no browser engine.

This is an independent community project. It is not affiliated with or endorsed by Roblox Corporation, Fishstrap, or Bloxstrap. Roblox does not officially support this multi-instance behavior, and platform behavior may change.

## Highlights

- One clear **Launch Roblox** action. In normal mode it launches Roblox normally; in multi-account mode each click prepares another client.
- In Multi-Account Mode, Client 1 uses the selected backend normally; Client 2+ use native NTFS junctions under `%LOCALAPPDATA%\RobloxMultiAccountLauncher\Instances\Client-NNNN`. This preserves backend bootstrap/update behavior for the first client and gives later clients distinct paths without copying or modifying Roblox files.
- Multi-account mode always starts **off** and is never persisted.
- Multi-instance mechanism: one Rust owner thread holds mutex objects named `ROBLOX_singletonMutex` and `ROBLOX_singletonEvent`; teleport protection separately holds an exclusive, read-only handle to `RobloxCookies.dat`.
- Launch confirmation snapshots existing clients and requires a genuinely new `RobloxPlayerBeta.exe` PID to remain alive for a short stability window.
- Auto backend priority: Fishstrap, Bloxstrap, then stock Roblox.
- Read-only `roblox:` and `roblox-player:` protocol inspection.
- Client roles, PID/uptime/RAM/window state, per-process Core Audio controls, safe resource presets, focus/close actions, role-aware layouts, optional auto-arrange, and recent launch/exit history.
- Soft client limit (default 2), optional bounded one-click target count, transient validated Roblox link input, and protection-aware tray lifecycle.
- Optional `RegisterHotKey` shortcuts are off by default. No keyboard hook is used.
- Optional embedded PowerShell Assist uses PowerShell 7 only; absence leaves the application in Rust-only mode.
- Optional metadata-only `Roblox\LocalStorage` change tracing for shared-login investigation; no file contents or credentials are read.
- Sanitized diagnostics, no telemetry, no credentials, no injection, and no administrator requirement.
- Portable Rust self-updater targeting this public repository: quiet checks, explicit user approval, exact release-manifest agreement, SHA-256 verification, same-binary helper replacement, preserved rollback executable, and no installer/service/admin requirement.
- Portable-data mode through a `portable.flag` beside the executable, settings import/export/reset, conservative recovery/instance cleanup, and user-created sanitized support bundles.

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

The application owns an explicit persistence layer under `%LOCALAPPDATA%\RobloxMultiAccountLauncher` for harmless settings, sanitized logs, launch history, per-client junction aliases, requested metadata-only traces, support bundles, and verified update/rollback state. If `portable.flag` exists beside the launched executable, these files instead use a sibling `data` directory. `RMAL_DATA_DIR` remains a development/test override. Multi-account state, Roblox cookies, credentials, authentication tickets, launch tokens, private URLs, and Roblox file contents are never persisted.

`eframe`'s generic persistence is deliberately disabled so there is one auditable application-owned settings format.

## PowerShell Assist

Auditable source scripts live in `scripts` and are embedded into the Rust executable at compile time. The optional engine is PowerShell 7 (`pwsh.exe`). Windows PowerShell 5.1 is intentionally not used. When PS7 is absent, the UI reports Rust-only mode and all core launching/protection/client-management features remain available.

Invocations use no profile, no interactive terminal, captured output/error, a hidden process, cancellation, and bounded timeouts. Diagnostic scripts are read-only. Repair inspection returns proposed actions; mutation requires a separate explicit GUI confirmation.

The known-good V1 PowerShell helper remains preserved under `legacy\powershell`.

## Portable updates

The Settings page contains update preferences and compact About/update status. The installed version comes only from `Cargo.toml`. Automatic checks default on, run after startup without blocking the GUI, and never install silently. Public releases require `RobloxMultiAccountLauncher.exe` plus `update-manifest.json`; the manifest must identify the same version/filename and contain the executable SHA-256 hash.

The updater targets the exact portable executable that is running. After explicit approval, it stages and verifies the release under the active application data root, starts a temporary copy of the same Rust executable in hidden helper mode, exits, replaces the original with rollback protection, preserves a verified previous executable in `Updates\Rollback`, restarts, and removes only named transient files. It will not begin replacement while multi-account protection or Roblox clients are active.

The updater is configured for `Wolfyisdabest/Roblox-Multi-Account-Launcher`. Until a matching GitHub Release exists it fails quietly during automatic checks and never affects launching. See [docs/UPDATER.md](docs/UPDATER.md) for the release contract and deferred signed-manifest requirement.

## Pre-release status

`3.1.0-beta.1` is experimental/pre-release software. The validated external protection core is intentionally preserved, but the in-launcher Client 1 → Client 2 path-isolation sequence and new QOL controls still require live validation. Do not describe this project as ban-proof, undetectable, officially supported, or stable 1.0 software.

## Security and privacy

The launcher is an external process/window manager. It does not read cookie contents, inspect Roblox process memory, inject, hook, patch binaries, modify authentication data, bypass anti-cheat, automate gameplay, elevate, or transmit telemetry. See [SECURITY.md](SECURITY.md).

## Current limitations

- Roblox/Fishstrap/Bloxstrap command-line and protocol behavior can change.
- A Fishstrap/Bloxstrap configuration bridge is deliberately deferred until a documented, versioned, rollback-safe settings schema can be validated. The launcher does not rewrite bootstrapper configuration today.
- File-version display is best-effort and may show `Unknown`.
- Role-aware layout automation currently arranges two clients; additional clients remain individually manageable.
- Graceful close depends on a Roblox window accepting `WM_CLOSE`.
- Holding both singleton names, two-client coexistence, teleporting, and bidirectional logout behavior have been manually validated three times on the validation machine. Roblox behavior remains undocumented and can change.
- Per-instance path isolation and the corrected first-client/second-client launch sequencing are implemented and fixture/logic-tested, but the final in-launcher Client 1 → Client 2 test is still required. It is not claimed fixed until that succeeds.
- Login-state behavior is experimental/manual-validation-specific; no separate auth-state lock or credential handling was added and the exact causal local file remains unidentified.
- Real three-client, post-update, tray, and multi-monitor behavior still require manual validation.
- Live GitHub update checking, portable replacement/rollback, per-client Core Audio mapping, resource presets, hotkeys, tray actions, auto-layout, portable mode, and recovery workflows require manual validation. SHA-256 is mandatory; cryptographically signed manifests remain deferred security hardening.

## License

MIT. See [LICENSE](LICENSE).

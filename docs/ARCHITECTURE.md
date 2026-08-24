# Architecture

## Runtime core

The application is a Rust 2024 Windows desktop program. `egui`/`eframe` owns rendering and DPI-aware layout; `windows-sys` calls native Win32 APIs.

Critical state is never delegated to PowerShell:

- `AppInstanceGuard` owns `Wolfy_RobloxMultiAccountLauncher`.
- `ProtectionController` owns a dedicated resource thread.
- That thread owns native mutex objects named `ROBLOX_singletonMutex` and `ROBLOX_singletonEvent`, plus the exclusive read-only cookie-file handle.
- The `ROBLOX_singletonEvent` name is deliberately occupied by a mutex rather than a Win32 event. Named synchronization objects share one session-local `BaseNamedObjects` namespace; this prevents Roblox from creating its event while retaining the proven mutex-ownership model.
- RAII drops only handles actually acquired, on the owner thread.
- The two singleton names are acquired all-or-nothing. Partial acquisition drops the first handle before reporting a recoverable setup failure. Cookie-lock failure retains both singleton guards as a separate teleport Warning state.
- Atomic health bits let the launch monitor verify both singleton handles before backend start and throughout bootstrap without moving ownership away from the resource thread.
- GUI code receives typed channel events; failures remain inside the running application.

## Previous C# failure investigation

The previous launcher log records successful mutex and cookie acquisition, two successful Fishstrap launch detections, and then stops without the normal shutdown-cleanup entry. No exception, stack trace, or definitive crash cause was recorded.

The C# implementation already tried to respect thread-affine mutex ownership with a dedicated thread, but its GUI used several `async void` event paths and a dispatcher timer while shutdown/resource disposal could occur. An unobserved UI-event exception or lifecycle race is plausible, but not proven. The Rust migration avoids blindly porting that lifecycle: critical handles live in one owner loop, cleanup is RAII, UI/background communication is channel-based, and every setup result becomes a recoverable UI state.

## Launching

`LaunchManager` atomically rejects overlapping bootstraps. It snapshots current Roblox PIDs, verifies both singleton guards when multi-account mode is active, and waits with a bounded timeout for a genuinely new `RobloxPlayerBeta.exe` PID. A candidate must remain alive for two seconds before success. Existing clients cannot be mistaken for the new client.

Normal mode launches the selected backend through `ShellExecuteExW`. Multi-account mode first asks `InstancePathManager` to resolve the newest valid `RobloxPlayerBeta.exe` version directory for the selected backend, creates a unique `Instances\Client-NNNN` NTFS junction to that directory, and directly starts the client through the alias path. Fishstrap/Bloxstrap remain responsible for installation, updates, and modifications; no Roblox files are copied or changed. Auto checks Fishstrap → Bloxstrap → stock and selects the first backend with both an installed launcher and a usable active version directory.

Each alias record tracks client ID, alias, target version, backend, and confirmed PID. A bound alias is removed only after its PID is gone. An unconfirmed alias is retained while any Roblox client is running. On restart, stale aliases are cleaned only when no Roblox clients exist. A backend update affects only new allocations; existing aliases and running clients are not retargeted.

Launch diagnostics distinguish backend-start failure, backend start with no new player PID, transient/replaced player PIDs, observed Roblox launcher/installer processes, existing clients that close during bootstrap, and protection loss. Fishstrap/Bloxstrap process creation alone is never treated as Roblox launch success.

Auto resolves Fishstrap → Bloxstrap → stock Roblox. Detection requires a real executable, using known locations and valid protocol ownership. Manual selections do not silently rewrite settings.

## Windows integration

Native APIs implement:

- named mutexes and exclusive file handles;
- Tool Help process enumeration and safe process metadata;
- read-only registry/protocol inspection;
- `WM_CLOSE`, bounded exit checks, confirmed `TerminateProcess`;
- window focus and positioning;
- monitor work-area enumeration and 50/50, 70/30, vertical, and swapped layouts;
- shell launch/open behavior;
- native mount-point reparse creation/verification for per-client launch paths;
- existing-window activation;
- notification-area menu integration.
- metadata-only `ReadDirectoryChangesW` tracing under `Roblox\LocalStorage`.

No injection, hooks, memory-content reads, registry writes, protocol rewrites, or elevation are used.

## Shared desktop login state

Roblox desktop clients under the same Windows user profile can observe shared local login state. The launcher does not store accounts, credentials, `.ROBLOSECURITY`, authentication tickets, or cookies, and it does not attempt to restore stale authentication state.

Bidirectional logout behavior has been manually validated three times while the existing external protections were active. The UI reports this as **Experimental / manually validated**, not as universal support. No additional auth-state file lock was introduced and no causal file is claimed. The Diagnostics page retains the explicit metadata-only trace of `%LOCALAPPDATA%\Roblox\LocalStorage` for future Roblox-version regressions.

## PowerShell Assist

Readable scripts remain in `scripts` and compile into the binary with `include_str!`. Rust detects PS7, then Windows PowerShell, then uses Rust-only mode. Child processes use `CREATE_NO_WINDOW`, redirected streams, no profile, no interaction, timeouts, cancellation, and forced child cleanup after timeout.

Scripts return JSON. Diagnostic scripts are read-only. The repair script reports proposed actions only.

## Data

Runtime data defaults to `%LOCALAPPDATA%\RobloxMultiAccountLauncher`. `RMAL_DATA_DIR` redirects development/test data to project-contained `.tmp`. The application owns this persistence explicitly; generic `eframe` persistence is disabled. Multi-account mode is absent from the settings schema and always initializes disabled.

## Portable updater

`UpdateManager` is an isolated, non-critical Rust subsystem. Compile-time GitHub owner/repository constants are optional; when absent the state is `NotConfigured`, no background thread/request starts, and core launching is unaffected. When configured, a delayed startup check uses GitHub's public Releases REST endpoint through bounded HTTPS without a token. Stable selection ignores drafts and prereleases; the Prerelease channel uses semantic-version ordering.

The explicit states are `NotConfigured`, `Idle`, `Checking`, `UpToDate`, `UpdateAvailable`, `Downloading`, `Verifying`, `ReadyToInstall`, `WaitingForSafeRestart`, `Installing`, and `Failed`. One atomic busy guard prevents overlapping checks/downloads. Cancellation is checked before requests and during bounded response streaming; partial executable files are deleted.

Installation requires an exact-version machine-readable manifest and matching SHA-256. The verified executable is staged under `%LOCALAPPDATA%\RobloxMultiAccountLauncher\Updates`. With no active protection and no Roblox clients, the running executable copies itself to a temporary helper path and starts that same binary with `--self-update-helper`. The helper waits for the original PID, re-verifies SHA-256, copies the update beside the exact launched portable executable, renames the original to a unique rollback backup, promotes the replacement, and restarts it with `--self-update-cleanup`. The restarted application waits for the helper and removes only the named helper/staged/backup files. Replacement failure restores the original where possible and records a bounded application-owned error for the next launch.

The updater never kills Roblox or releases the singleton/cookie resources to update. Protection-active updates remain notices until sessions finish and Multi-Account Mode is disabled. Automatic installation, installers, services, scheduled tasks, elevation, external runtimes, WebViews, and browser-rendered release notes are not used.

The configuration has a reserved public-verification-key slot, but signature verification is deliberately not claimed or enabled without a real release pipeline. SHA-256 is mandatory now; signed manifests remain required hardening before public release. See `docs/UPDATER.md`.

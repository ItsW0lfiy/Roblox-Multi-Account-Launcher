# Architecture

The application is a dependency-light .NET 10 Windows desktop utility.

## Lifecycle

`App` acquires `Wolfy_RobloxMultiAccountLauncher` before constructing the main window. A second process signals the primary process and exits before any Roblox or protection action.

The initial state is always `Normal`. Multi-account state is intentionally absent from persisted settings.

## Resource ownership

`MultiAccountService` uses a dedicated background resource thread. Mutex acquisition, retention, release, and `FileStream` disposal all occur on that same thread, respecting the thread-affine semantics of `System.Threading.Mutex` without blocking WPF.

Preparation and cancellation either retain a complete usable state or clean up acquired resources. A cookie-lock failure produces a warning while keeping the successfully acquired Roblox mutex.

## Launching

`LauncherDetectionService` inspects executable files and protocol registrations read-only. Auto selection prefers Fishstrap, then Bloxstrap, then stock Roblox. Explicit selections never silently fall back.

`LaunchQueue` rejects overlapping bootstrap work. `LaunchService` watches only normal process enumeration for a new `RobloxPlayerBeta` PID and uses a bounded timeout. It never reads process memory.

## Client windows

`RobloxProcessService` exposes safe process/window metadata. `WindowManager` uses standard User32 window positioning/focus calls and monitor work areas. Global focus shortcuts use `RegisterHotKey`, not keyboard hooks.

## Local data

Harmless settings and rotated launcher logs live below `%LOCALAPPDATA%\RobloxMultiAccountLauncher`. No multi-account enabled flag, credential, cookie, launch URI, or telemetry is stored.

## Deliberately rejected approaches

- DLL injection, hooks, memory access, memory scanning, process hiding, kernel drivers, anti-cheat manipulation, hardware spoofing, or binary replacement.
- Credential/cookie-based account management or account-to-process mapping.
- WebView2, Edge, Chromium, Electron, browser UI, cloud services, telemetry, and analytics.
- Registry writes or protocol reassociation during routine launching.
- Hard-coded Roblox version directories as the primary launch mechanism.
- Automatic killing of RobloxCrashHandler or unrelated/stale-looking processes without confirmation.
- Automatic restart or anti-closure loops when a Roblox client exits.

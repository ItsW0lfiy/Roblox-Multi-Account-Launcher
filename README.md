# Roblox Multi-Account Launcher

A local Windows launcher and process manager for Roblox with optional multi-account support. It provides backend detection for Fishstrap, Bloxstrap, and stock Roblox; client window controls; diagnostics; and an opt-in implementation of the known-working Roblox multi-instance and teleport-protection handles.

This is an independent community project. It is not affiliated with or endorsed by Roblox Corporation, Fishstrap, or Bloxstrap. Roblox does not officially support this multi-instance behavior, and platform behavior may change.

## Features

- Native C# / WPF dark interface with no console window.
- Multi-account mode always starts **off** and never persists as enabled.
- Auto backend priority: Fishstrap, Bloxstrap, then stock Roblox.
- Read-only inspection of `roblox:` and `roblox-player:` protocol handlers.
- Fishstrap and Bloxstrap version, path, folder, log, and protocol-owner diagnostics.
- Process-based Roblox client list with PID, uptime, memory, window title, focus, graceful close, and approved force-close.
- Two-client 50/50, 70/30, and vertical layouts; swap and preferred-monitor movement.
- Optional `Ctrl+Alt+1` / `Ctrl+Alt+2` focus hotkeys using normal Windows hotkey registration.
- Notification-area controls and protection-aware exit behavior.
- Local rotating logs, recent activity, safe recovery checks, and sanitized copied diagnostics.
- No telemetry, accounts, stored credentials, browser runtime, injection, memory access, or administrator requirement.

## Multi-account mechanism

When explicitly enabled, a dedicated resource-owner thread:

1. owns the named mutex `ROBLOX_singletonMutex`;
2. opens `%LOCALAPPDATA%\Roblox\LocalStorage\RobloxCookies.dat` with `FileMode.Open`, `FileAccess.Read`, and `FileShare.None`.

The application never reads, parses, copies, prints, modifies, or deletes cookie contents. The file handle exists only to preserve the previously validated teleport behavior.

## Build

Requirements:

- Windows 10/11 x64
- .NET 10 SDK

```powershell
dotnet restore .\RobloxMultiAccountLauncher.sln --configfile .\NuGet.Config
dotnet build .\RobloxMultiAccountLauncher.sln -c Release --no-restore
dotnet run --project .\tests\RobloxMultiAccountLauncher.Tests -c Release --no-build
dotnet publish .\src\RobloxMultiAccountLauncher\RobloxMultiAccountLauncher.csproj -c Release -r win-x64 --self-contained true -p:PublishSingleFile=true
```

The application project uses `OutputType=WinExe`; normal operation does not start PowerShell, CMD, or a console backend.

## Architecture

- `Models`: launcher, client, settings, state, and protection types.
- `Services`: resource ownership, backend detection, launching/queueing, process management, layouts, settings, logging, diagnostics, tray/hotkey support.
- `Themes`: reusable dark WPF resources.
- `legacy/powershell`: preserved known-good V1 implementation.
- `tests`: dependency-free automated validation executable using fixtures and test-only mutex names.

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) and [docs/MANUAL-TESTING.md](docs/MANUAL-TESTING.md).

## Privacy and security

Everything remains local. The launcher does not store Roblox credentials or cookies, inspect process memory, intercept network traffic, upload logs, or collect analytics. Copied diagnostics are redacted for common ticket, token, cookie, bearer, and private-server values. See [SECURITY.md](SECURITY.md).

## Current limitations

- All V2 behavior needs real-world manual validation; automated tests do not launch Roblox.
- Launcher command-line behavior can change in future Fishstrap, Bloxstrap, or Roblox versions.
- High-level log analysis is best-effort and intentionally avoids brittle exact-message parsing.
- Two-client tiling is the primary layout; more than two clients are listed and managed but not automatically gridded.
- A neutral system placeholder icon is used; original branding remains TODO.
- Graceful close depends on a Roblox window accepting the standard Windows close request.

## License

MIT. See [LICENSE](LICENSE).

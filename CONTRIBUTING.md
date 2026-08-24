# Contributing

Keep changes focused, auditable, and external-only. Discuss substantial lifecycle or protection changes before implementing them.

## Development

The primary application is Rust 2024 on Windows. Use Rust stable, preserve `Cargo.lock`, and run:

```powershell
cargo fmt --all -- --check
cargo check --locked
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

Core functionality must not depend on PowerShell. Optional assistance targets PowerShell 7 only. Do not introduce C#/.NET, Node/Electron/WebView, Java/JVM, a browser runtime, or another runtime without explicit project approval.

## Security boundaries

Contributions must not store or request Roblox credentials, read cookie contents, inject DLLs, access or modify Roblox process memory, patch Roblox, close Roblox-owned handles, manipulate anti-cheat, hide processes, automate gameplay, or add telemetry. Diagnostic output must be sanitized and local-only.

Never commit binaries, `target`, `dist`, logs, dumps, local settings, AppData captures, support bundles, keys, certificates, tokens, or signing private material.

## Pull requests

Explain the root cause, files changed, tests run, and remaining manual validation. Automated tests must not launch or close real Roblox, contact GitHub Releases, or touch the real `RobloxCookies.dat`. Roblox/Fishstrap/Bloxstrap behavior that cannot be safely automated must be described as manual and must not be claimed fixed before validation.

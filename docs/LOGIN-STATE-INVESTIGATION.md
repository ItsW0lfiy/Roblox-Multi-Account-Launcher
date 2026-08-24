# Shared Roblox login-state investigation

## Current conclusion

Concurrent Roblox clients share desktop-app login state under the same Windows user profile; full per-client logout isolation is not safely achievable with the current external-only design.

No login-state protection is enabled. The launcher does not read, store, copy, restore, or display credentials, `.ROBLOSECURITY`, cookies, authentication tickets, passwords, or private authentication data.

## Safe metadata inventory

A read-only name/size/timestamp inventory of `%LOCALAPPDATA%\Roblox\LocalStorage` on the validation machine found:

- `appStorage.json`
- `memProfStorage*.json`
- `RobloxCookies.dat`

No contents were opened. These names do not prove which file causes cross-client logout. `RobloxCookies.dat` remains the separately proven teleport-protection target; no other file is treated as an authentication-state target without event correlation and manual validation.

## Diagnostic trace

Diagnostics offers **Start Local-State Trace** / **Stop Trace**. The native Rust tracer recursively watches `%LOCALAPPDATA%\Roblox\LocalStorage` and writes only:

- timestamp;
- relative file path;
- create/write/delete/rename action;
- `process=unavailable`.

`ReadDirectoryChangesW` does not identify the process responsible for a notification, so the launcher records the actor as unavailable instead of guessing. The tracer never opens a changed file and never captures file contents.

## Protection decision gate

An experimental lock may be considered only after a manual two-client logout trace repeatedly identifies a narrow shared-state file and separate testing shows Roblox tolerates a read-only/exclusive external handle. It must remain off by default until the shared-logout test passes.

The project will not implement account vaults, credential storage, authentication-ticket generation, embedded login browsers, filesystem virtualization, injection, hooks, kernel filters, or stale-token restoration.

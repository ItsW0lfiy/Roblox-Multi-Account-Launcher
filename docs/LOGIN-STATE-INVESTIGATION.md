# Shared Roblox login-state investigation

## Current conclusion

Bidirectional independent logout behavior has been manually validated three times with two clients while the existing external protections were active. Logging out either client did not log out the other.

This result is reported as **Experimental / manually validated**, not universal support. No additional login-state file lock is enabled, and the test does not prove which existing external protection causes the behavior. The launcher does not read, store, copy, restore, or display credentials, `.ROBLOSECURITY`, cookies, authentication tickets, passwords, or private authentication data.

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

No new auth-state lock is justified by the passing manual result. If a Roblox update regresses logout behavior, an experimental lock may be considered only after a manual two-client trace repeatedly identifies a narrow shared-state file and separate testing shows Roblox tolerates a read-only/exclusive external handle.

The project will not implement account vaults, credential storage, authentication-ticket generation, embedded login browsers, filesystem virtualization, injection, hooks, kernel filters, or stale-token restoration.

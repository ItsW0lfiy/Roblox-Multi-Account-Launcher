# Portable updater release contract

The Rust updater is implemented but intentionally dormant. This project has no approved public GitHub repository or release source, so no owner/repository is compiled into the current binary and the UI reports **Not configured**.

## Future compile-time configuration

Release builds may later define:

- `RMAL_GITHUB_OWNER`
- `RMAL_GITHUB_REPOSITORY`
- `RMAL_UPDATE_PUBLIC_KEY` (reserved for the signed-manifest verifier)

Both repository coordinates must be non-empty or the updater remains dormant. No runtime token or GitHub account is required for public release checks. The application version is always `CARGO_PKG_VERSION`, sourced from `Cargo.toml`.

## Required release assets

Each considered release must contain exactly named assets:

```text
RobloxMultiAccountLauncher.exe
update-manifest.json
```

Manifest schema:

```json
{
  "version": "3.1.0",
  "filename": "RobloxMultiAccountLauncher.exe",
  "sha256": "64-lower-or-upper-case-hex-digits"
}
```

The release tag, manifest version, filename, and downloaded SHA-256 must agree. A missing asset, malformed response/manifest, timeout, network/rate-limit failure, oversized response, or hash mismatch fails closed and leaves the installed executable unchanged.

## Replacement and rollback

The user explicitly chooses download and later **Update & Restart**. Replacement is blocked while Multi-Account Mode is active or any Roblox client is running. A temporary copy of the same Rust executable waits for the current process, re-verifies the staged file, places a replacement beside the exact current executable, keeps a unique rollback backup, promotes the replacement, and restarts it. The restarted binary cleans only the exact helper, staged file, and backup paths passed by the helper.

This design needs no installer, service, scheduled task, administrator permission, Python, PowerShell, .NET, or permanently installed updater binary.

## Signing status

SHA-256 prevents accidental corruption and mismatch but, by itself, does not authenticate a compromised release source. The configuration and manifest boundary are prepared for a public verification key, but an Ed25519 (or comparably reviewed) signed-manifest verifier is deferred until the real release process exists. The private signing key must remain outside the repository and distributed application. Signed metadata is a required security improvement before public releases are enabled; the current application does not claim signed-update support.

## Validation boundary

Automated tests use fixture metadata, fixture manifests, dummy executables, and project-contained temporary paths. They do not call GitHub and never replace the real launcher. Real validation remains pending until an approved repository and release exist: live release selection, asset download, hash verification against published metadata, portable replacement from a path containing spaces/on another drive, rollback under a deliberate replacement failure, and restart/cleanup.

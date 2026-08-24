# Portable updater release contract

The Rust updater targets the public `Wolfyisdabest/Roblox-Multi-Account-Launcher` GitHub repository. Automatic checks remain quiet and non-fatal; no update installs without explicit approval.

## Compile-time configuration

Release builds may later define:

- `RMAL_GITHUB_OWNER`
- `RMAL_GITHUB_REPOSITORY`
- `RMAL_UPDATE_PUBLIC_KEY` (reserved for the signed-manifest verifier)

The real repository coordinates are defaults and may be overridden for controlled fixture builds. Empty coordinates leave the updater dormant. No runtime token or GitHub account is required for public release checks. The application version is always `CARGO_PKG_VERSION`, sourced from `Cargo.toml`.

## Required release assets

Each considered release must contain exactly named assets:

```text
RobloxMultiAccountLauncher.exe
update-manifest.json
```

Manifest schema:

```json
{
  "version": "3.1.0-beta.1",
  "filename": "RobloxMultiAccountLauncher.exe",
  "sha256": "64-lower-or-upper-case-hex-digits",
  "channel": "prerelease",
  "release_url": "https://github.com/Wolfyisdabest/Roblox-Multi-Account-Launcher/releases/tag/v3.1.0-beta.1",
  "signature": null
}
```

The release tag, manifest version, filename, stable/prerelease channel, canonical release URL, and downloaded SHA-256 must agree. A missing asset, malformed response/manifest, timeout, network/rate-limit failure, oversized response, or hash mismatch fails closed and leaves the installed executable unchanged.

## Replacement and rollback

The user explicitly chooses download and later **Update & Restart**. Replacement is blocked while Multi-Account Mode is active or any Roblox client is running. A temporary copy of the same Rust executable waits for the current process, re-verifies the staged file, places a replacement beside the exact current executable, keeps a unique temporary backup, promotes the replacement, and restarts it. Before restart, the helper preserves a verified previous executable and its hash under application-owned `Updates\Rollback`. The restarted binary cleans only exact transient helper/staged/temporary backup paths. A user may explicitly prepare the previous executable and use the same protection-aware flow to roll back.

This design needs no installer, service, scheduled task, administrator permission, Python, PowerShell, .NET, or permanently installed updater binary.

## Signing status

SHA-256 prevents accidental corruption and mismatch but, by itself, does not authenticate a compromised release source. The configuration and optional `signature` manifest field are prepared for a public verification key, but an Ed25519 (or comparably reviewed) signed-manifest verifier is deferred until offline key management exists. The future private key must remain outside the repository and application. If signature enforcement is compiled before a verifier exists, installation fails closed. The application does not claim signed-update support.

## Validation boundary

Automated tests use fixture metadata, fixture manifests, dummy executables, and project-contained temporary paths. They do not call GitHub and never replace the real launcher. Real validation remains pending until a matching GitHub prerelease exists: live release selection, asset download, hash verification, portable replacement from a path containing spaces/on another drive, explicit rollback, and restart/cleanup.

[CmdletBinding()]
param(
    [string]$ReleaseUrl
)

$ErrorActionPreference = 'Stop'
$projectRoot = $PSScriptRoot
$env:CARGO_HOME = Join-Path $projectRoot '.tmp\cargo-home'
$env:CARGO_TARGET_DIR = Join-Path $projectRoot '.tmp\target'
$env:CARGO_TERM_COLOR = 'never'

Push-Location -LiteralPath $projectRoot
try {
    cargo test --locked --target x86_64-pc-windows-msvc
    if ($LASTEXITCODE -ne 0) { throw "cargo test failed with exit code $LASTEXITCODE." }

    cargo build --locked --release --target x86_64-pc-windows-msvc
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed with exit code $LASTEXITCODE." }

    $dist = Join-Path $projectRoot 'dist'
    New-Item -ItemType Directory -Path $dist -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path $env:CARGO_TARGET_DIR 'x86_64-pc-windows-msvc\release\roblox-multi-account-launcher.exe') -Destination (Join-Path $dist 'RobloxMultiAccountLauncher.exe') -Force
    $cargo = Get-Content -LiteralPath (Join-Path $projectRoot 'Cargo.toml') -Raw
    $versionMatch = [regex]::Match($cargo, '(?m)^version\s*=\s*"([^"]+)"')
    if (-not $versionMatch.Success) { throw 'Could not read the package version from Cargo.toml.' }
    $version = $versionMatch.Groups[1].Value
    if ([string]::IsNullOrWhiteSpace($ReleaseUrl)) {
        $ReleaseUrl = "https://github.com/Wolfyisdabest/Roblox-Multi-Account-Launcher/releases/tag/v$version"
    }
    $executable = Join-Path $dist 'RobloxMultiAccountLauncher.exe'
    $manifest = [ordered]@{
        version = $version
        filename = 'RobloxMultiAccountLauncher.exe'
        sha256 = (Get-FileHash -LiteralPath $executable -Algorithm SHA256).Hash.ToLowerInvariant()
        channel = if ($version.Contains('-')) { 'prerelease' } else { 'stable' }
        release_url = $ReleaseUrl
        signature = $null
    }
    $manifest | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $dist 'update-manifest.json') -Encoding utf8NoBOM
    Write-Host "Built: $dist\RobloxMultiAccountLauncher.exe"
    Write-Host "Manifest: $dist\update-manifest.json"
}
finally {
    Pop-Location
}

[CmdletBinding()]
param()

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
    Write-Host "Built: $dist\RobloxMultiAccountLauncher.exe"
}
finally {
    Pop-Location
}

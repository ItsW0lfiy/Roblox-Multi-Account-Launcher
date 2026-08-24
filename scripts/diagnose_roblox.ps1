$ErrorActionPreference = 'Stop'
$clients = @(Get-Process -Name RobloxPlayerBeta -ErrorAction SilentlyContinue | ForEach-Object {
    [ordered]@{ pid = $_.Id; started = $(try { $_.StartTime.ToString('o') } catch { $null }); window = $_.MainWindowTitle }
})
[ordered]@{ action = 'Inspect Roblox State'; installed = Test-Path (Join-Path $env:LOCALAPPDATA 'Roblox'); clients = $clients } | ConvertTo-Json -Depth 4 -Compress

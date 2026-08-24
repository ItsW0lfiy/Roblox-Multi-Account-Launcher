$ErrorActionPreference = 'Stop'
$names = @('RobloxPlayerBeta', 'RobloxCrashHandler', 'Fishstrap', 'Bloxstrap')
$items = @(Get-Process -Name $names -ErrorAction SilentlyContinue | ForEach-Object { [ordered]@{ name = $_.ProcessName; pid = $_.Id } })
[ordered]@{ action = 'Inspect Processes'; processes = $items } | ConvertTo-Json -Depth 4 -Compress

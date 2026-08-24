$ErrorActionPreference = 'Stop'
$stale = @(Get-Process -Name RobloxCrashHandler -ErrorAction SilentlyContinue | ForEach-Object {
    [ordered]@{ name = $_.ProcessName; pid = $_.Id; proposedAction = 'Close process after user confirmation' }
})
[ordered]@{ action = 'Repair Launch State'; mutationPerformed = $false; proposedActions = $stale } | ConvertTo-Json -Depth 4 -Compress

$ErrorActionPreference = 'Stop'
$exe = Join-Path $env:LOCALAPPDATA 'Fishstrap\Fishstrap.exe'
[ordered]@{ action = 'Inspect Fishstrap'; detected = Test-Path -LiteralPath $exe; executable = $exe } | ConvertTo-Json -Compress

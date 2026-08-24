$ErrorActionPreference = 'Stop'
$exe = Join-Path $env:LOCALAPPDATA 'Bloxstrap\Bloxstrap.exe'
[ordered]@{ action = 'Inspect Bloxstrap'; detected = Test-Path -LiteralPath $exe; executable = $exe } | ConvertTo-Json -Compress

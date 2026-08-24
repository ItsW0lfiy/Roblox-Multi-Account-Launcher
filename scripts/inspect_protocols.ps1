$ErrorActionPreference = 'Stop'
$result = foreach ($scheme in @('roblox', 'roblox-player')) {
    $path = "Registry::HKEY_CURRENT_USER\Software\Classes\$scheme\shell\open\command"
    $command = if (Test-Path $path) { (Get-Item -LiteralPath $path).GetValue('') } else { $null }
    [ordered]@{ scheme = $scheme; command = $command }
}
[ordered]@{ action = 'Check Roblox Protocols'; protocols = @($result) } | ConvertTo-Json -Depth 4 -Compress

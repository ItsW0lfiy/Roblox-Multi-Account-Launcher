[CmdletBinding()]
param(
    [switch]$NoKill,
    [switch]$Quiet,
    [switch]$ValidateOnly
)

Set-StrictMode -Version 3.0
$ErrorActionPreference = "Stop"

Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing

try {
    [System.Windows.Forms.Application]::SetHighDpiMode(
        [System.Windows.Forms.HighDpiMode]::SystemAware
    ) | Out-Null
}
catch {
    # SetHighDpiMode is not available on every Windows Forms runtime.
}

[System.Windows.Forms.Application]::EnableVisualStyles()
[System.Windows.Forms.Application]::SetCompatibleTextRenderingDefault($false)

if ($ValidateOnly) {
    $validationForm = [System.Windows.Forms.Form]::new()
    $validationTimer = [System.Windows.Forms.Timer]::new()
    $validationTimer.Dispose()
    $validationForm.Dispose()
    Write-Output "Windows Forms validation passed."
    return
}

$script:HelperMutexName = "Wolfy_RobloxMultiAccountHelper"
$script:RobloxMutexName = "ROBLOX_singletonMutex"
$script:CookiePath = Join-Path $env:LOCALAPPDATA "Roblox\LocalStorage\RobloxCookies.dat"

$script:HelperMutex = $null
$script:HelperMutexOwned = $false
$script:RobloxMutex = $null
$script:RobloxMutexOwned = $false
$script:CookieLock = $null
$script:TeleportFailure = $null
$script:State = "Idle"
$script:IsBusy = $false
$script:CleanupComplete = $false
$script:AllowFormClose = $false
$script:LastProcessCount = -1
$script:LastProcessIds = @()
$script:ProcessTimer = $null
$script:Form = $null
$script:StateValueLabel = $null
$script:MultiValueLabel = $null
$script:TeleportValueLabel = $null
$script:ProcessValueLabel = $null
$script:StartButton = $null
$script:StatusButton = $null
$script:CloseRobloxButton = $null
$script:StopButton = $null
$script:LogBox = $null

function Get-RobloxProcesses {
    return @(Get-Process -Name "RobloxPlayerBeta" -ErrorAction SilentlyContinue | Sort-Object Id)
}

function Write-UiLog {
    param(
        [Parameter(Mandatory)]
        [string]$Message,

        [switch]$Important
    )

    if ($Quiet -and -not $Important) {
        return
    }

    if ($null -eq $script:LogBox -or $script:LogBox.IsDisposed) {
        return
    }

    $entry = "{0:HH:mm:ss}  {1}" -f [DateTime]::Now, $Message
    $script:LogBox.AppendText($entry + [Environment]::NewLine)

    if ($script:LogBox.Lines.Count -gt 250) {
        $script:LogBox.Lines = @($script:LogBox.Lines | Select-Object -Last 200)
        $script:LogBox.SelectionStart = $script:LogBox.TextLength
        $script:LogBox.ScrollToCaret()
    }
}

function Set-StateLabel {
    param(
        [Parameter(Mandatory)]
        [System.Windows.Forms.Label]$Label,

        [Parameter(Mandatory)]
        [string]$Text,

        [Parameter(Mandatory)]
        [System.Drawing.Color]$Color
    )

    $Label.Text = [string][char]0x25CF + " " + $Text
    $Label.ForeColor = $Color
}

function Update-ResourceIndicators {
    if ($null -eq $script:MultiValueLabel -or
        $null -eq $script:TeleportValueLabel -or
        $script:MultiValueLabel.IsDisposed -or
        $script:TeleportValueLabel.IsDisposed) {
        return
    }

    if ($script:RobloxMutexOwned) {
        Set-StateLabel -Label $script:MultiValueLabel -Text "Enabled" -Color ([System.Drawing.Color]::ForestGreen)
    }
    else {
        Set-StateLabel -Label $script:MultiValueLabel -Text "Disabled" -Color ([System.Drawing.Color]::DimGray)
    }

    if ($null -ne $script:CookieLock) {
        Set-StateLabel -Label $script:TeleportValueLabel -Text "Enabled" -Color ([System.Drawing.Color]::ForestGreen)
    }
    elseif ($script:RobloxMutexOwned -and $script:TeleportFailure) {
        Set-StateLabel -Label $script:TeleportValueLabel -Text "Warning" -Color ([System.Drawing.Color]::DarkOrange)
    }
    else {
        Set-StateLabel -Label $script:TeleportValueLabel -Text "Disabled" -Color ([System.Drawing.Color]::DimGray)
    }
}

function Update-ButtonStates {
    if ($null -eq $script:StartButton) {
        return
    }

    $script:StartButton.Enabled = -not $script:IsBusy -and -not $script:RobloxMutexOwned
    $script:StatusButton.Enabled = -not $script:IsBusy
    $script:CloseRobloxButton.Enabled = -not $script:IsBusy -and $script:LastProcessCount -gt 0
    $script:StopButton.Enabled = $script:State -ne "Stopping"
}

function Set-HelperState {
    param(
        [Parameter(Mandatory)]
        [ValidateSet("Idle", "Closing Roblox", "Acquiring multi-instance", "Enabling teleport protection", "Ready", "Warning", "Error", "Stopping")]
        [string]$State,

        [string]$Message
    )

    $script:State = $State

    $color = switch ($State) {
        "Ready" { [System.Drawing.Color]::ForestGreen }
        "Warning" { [System.Drawing.Color]::DarkOrange }
        "Error" { [System.Drawing.Color]::Firebrick }
        "Stopping" { [System.Drawing.Color]::Firebrick }
        default { [System.Drawing.Color]::SteelBlue }
    }

    Set-StateLabel -Label $script:StateValueLabel -Text $State -Color $color
    Update-ResourceIndicators
    Update-ButtonStates

    if ($Message) {
        Write-UiLog -Message $Message -Important:($State -in @("Ready", "Warning", "Error"))
    }
}

function Update-ProcessCount {
    $processes = @(Get-RobloxProcesses)
    $count = $processes.Count
    $ids = @($processes | ForEach-Object { $_.Id })

    $suffix = if ($count -eq 1) { "client" } else { "clients" }
    $script:ProcessValueLabel.Text = "$count $suffix running"

    $changed = $count -ne $script:LastProcessCount -or
        (@($script:LastProcessIds) -join ",") -ne (@($ids) -join ",")

    if ($changed -and $script:LastProcessCount -ge 0) {
        if ($count -gt 0) {
            Write-UiLog -Message ("{0} Roblox {1} running (PID: {2})." -f $count, $suffix, ($ids -join ", "))
        }
        else {
            Write-UiLog -Message "No Roblox clients running."
        }
    }

    $script:LastProcessCount = $count
    $script:LastProcessIds = $ids
    Update-ButtonStates
    return $processes
}

function Acquire-HelperMutex {
    $createdNew = $false
    $mutex = [System.Threading.Mutex]::new($true, $script:HelperMutexName, [ref]$createdNew)

    if (-not $createdNew) {
        $mutex.Dispose()
        return $false
    }

    $script:HelperMutex = $mutex
    $script:HelperMutexOwned = $true
    return $true
}

function Stop-RobloxProcesses {
    param(
        [int]$TimeoutSeconds = 10
    )

    $processes = @(Get-RobloxProcesses)
    if ($processes.Count -eq 0) {
        Write-UiLog -Message "No existing Roblox client found."
        Update-ProcessCount | Out-Null
        return
    }

    $ids = @($processes | ForEach-Object { $_.Id })
    Write-UiLog -Message ("Found {0} Roblox client(s) (PID: {1})." -f $processes.Count, ($ids -join ", "))
    Write-UiLog -Message "Closing Roblox..."

    $stopErrors = [System.Collections.Generic.List[string]]::new()
    foreach ($process in $processes) {
        try {
            if (-not $process.HasExited) {
                Stop-Process -Id $process.Id -Force -ErrorAction Stop
            }
        }
        catch [System.InvalidOperationException] {
            # The process ended between discovery and Stop-Process.
        }
        catch {
            $stopErrors.Add("PID $($process.Id): $($_.Exception.Message)")
        }
    }

    $stopwatch = [System.Diagnostics.Stopwatch]::StartNew()
    do {
        [System.Windows.Forms.Application]::DoEvents()
        $remaining = @(Get-RobloxProcesses)
        if ($remaining.Count -eq 0) {
            $stopwatch.Stop()
            Update-ProcessCount | Out-Null
            Write-UiLog -Message "Roblox closed."
            return
        }

        Start-Sleep -Milliseconds 200
    }
    while ($stopwatch.Elapsed.TotalSeconds -lt $TimeoutSeconds)

    $stopwatch.Stop()
    $remaining = @(Get-RobloxProcesses)
    $remainingIds = @($remaining | ForEach-Object { $_.Id }) -join ", "
    $details = if ($stopErrors.Count -gt 0) { " " + ($stopErrors -join " ") } else { "" }
    throw "Roblox did not close within $TimeoutSeconds seconds. Remaining PID(s): $remainingIds.$details"
}

function Acquire-RobloxMutex {
    param(
        [int]$TimeoutSeconds = 6,
        [int]$RetryDelayMilliseconds = 250
    )

    $mutex = [System.Threading.Mutex]::new($false, $script:RobloxMutexName)
    $stopwatch = [System.Diagnostics.Stopwatch]::StartNew()
    $lastProgressSecond = -1

    try {
        do {
            $acquired = $false
            try {
                $acquired = $mutex.WaitOne(0)
            }
            catch [System.Threading.AbandonedMutexException] {
                $acquired = $true
            }

            if ($acquired) {
                $stopwatch.Stop()
                $script:RobloxMutex = $mutex
                $script:RobloxMutexOwned = $true
                Update-ResourceIndicators
                Write-UiLog -Message "Multi-instance enabled." -Important
                return $true
            }

            $elapsedSecond = [Math]::Floor($stopwatch.Elapsed.TotalSeconds)
            if ($elapsedSecond -ne $lastProgressSecond) {
                Write-UiLog -Message "Waiting for ROBLOX_singletonMutex..."
                $lastProgressSecond = $elapsedSecond
            }

            [System.Windows.Forms.Application]::DoEvents()
            Start-Sleep -Milliseconds $RetryDelayMilliseconds
        }
        while ($stopwatch.Elapsed.TotalSeconds -lt $TimeoutSeconds)
    }
    catch {
        $mutex.Dispose()
        throw
    }

    $stopwatch.Stop()
    $mutex.Dispose()
    return $false
}

function Lock-RobloxCookies {
    $script:TeleportFailure = $null

    if (-not [System.IO.File]::Exists($script:CookiePath)) {
        $script:TeleportFailure = "RobloxCookies.dat was not found. Start Roblox normally once so it can create the file."
        return $false
    }

    try {
        # Known-good teleport protection: do not read, parse, copy, or modify this file.
        $script:CookieLock = [System.IO.File]::Open(
            $script:CookiePath,
            [System.IO.FileMode]::Open,
            [System.IO.FileAccess]::Read,
            [System.IO.FileShare]::None
        )

        Write-UiLog -Message "Teleport protection enabled." -Important
        return $true
    }
    catch [System.UnauthorizedAccessException] {
        $script:TeleportFailure = "Access to RobloxCookies.dat was denied. The helper does not need administrator rights; check the file permissions and security software."
    }
    catch [System.IO.FileNotFoundException] {
        $script:TeleportFailure = "RobloxCookies.dat disappeared before it could be locked."
    }
    catch [System.IO.IOException] {
        $script:TeleportFailure = "RobloxCookies.dat could not be locked exclusively. Another process may already have it open: $($_.Exception.Message)"
    }
    catch {
        $script:TeleportFailure = "RobloxCookies.dat could not be locked: $($_.Exception.Message)"
    }

    return $false
}

function Release-ProtectionResources {
    if ($null -ne $script:CookieLock) {
        try {
            $script:CookieLock.Dispose()
            Write-UiLog -Message "Teleport protection released."
        }
        catch {
            Write-UiLog -Message "Teleport protection cleanup reported an error: $($_.Exception.Message)" -Important
        }
        finally {
            $script:CookieLock = $null
        }
    }

    if ($null -ne $script:RobloxMutex) {
        if ($script:RobloxMutexOwned) {
            try {
                $script:RobloxMutex.ReleaseMutex()
                Write-UiLog -Message "Multi-instance mutex released."
            }
            catch {
                Write-UiLog -Message "Multi-instance mutex cleanup reported an error: $($_.Exception.Message)" -Important
            }
            finally {
                $script:RobloxMutexOwned = $false
            }
        }

        try {
            $script:RobloxMutex.Dispose()
        }
        catch {
        }
        finally {
            $script:RobloxMutex = $null
        }
    }

    Update-ResourceIndicators
}

function Release-AllResources {
    if ($script:CleanupComplete) {
        return
    }

    $script:CleanupComplete = $true

    if ($null -ne $script:ProcessTimer) {
        try {
            $script:ProcessTimer.Stop()
            $script:ProcessTimer.Dispose()
        }
        catch {
        }
        finally {
            $script:ProcessTimer = $null
        }
    }

    Release-ProtectionResources

    if ($null -ne $script:HelperMutex) {
        if ($script:HelperMutexOwned) {
            try {
                $script:HelperMutex.ReleaseMutex()
            }
            catch {
            }
            finally {
                $script:HelperMutexOwned = $false
            }
        }

        try {
            $script:HelperMutex.Dispose()
        }
        catch {
        }
        finally {
            $script:HelperMutex = $null
        }
    }
}

function Start-Preparation {
    if ($script:IsBusy -or $script:RobloxMutexOwned) {
        return
    }

    if (-not $script:HelperMutexOwned) {
        Set-HelperState -State "Error" -Message "The helper single-instance guard is not owned. Setup was not started."
        return
    }

    $script:IsBusy = $true
    $script:TeleportFailure = $null
    Update-ButtonStates

    try {
        if ($NoKill) {
            $existing = @(Update-ProcessCount)
            if ($existing.Count -gt 0) {
                Write-UiLog -Message ("-NoKill is active; leaving {0} existing Roblox client(s) running." -f $existing.Count) -Important
            }
        }
        else {
            Set-HelperState -State "Closing Roblox" -Message "Preparing Roblox for multi-account use."
            Stop-RobloxProcesses -TimeoutSeconds 10
        }

        Set-HelperState -State "Acquiring multi-instance" -Message "Acquiring ROBLOX_singletonMutex..."
        if (-not (Acquire-RobloxMutex -TimeoutSeconds 6 -RetryDelayMilliseconds 250)) {
            $message = "ROBLOX_singletonMutex could not be acquired within 6 seconds."
            if ($NoKill -and @(Get-RobloxProcesses).Count -gt 0) {
                $message += " An existing Roblox client probably owns it; close Roblox or run again without -NoKill."
            }
            else {
                $message += " Another Roblox instance or multi-instance tool may still own it."
            }
            throw $message
        }

        Set-HelperState -State "Enabling teleport protection" -Message "Locking RobloxCookies.dat without reading it..."
        if (Lock-RobloxCookies) {
            Set-HelperState -State "Ready" -Message "Ready. You can launch both Roblox accounts."
        }
        else {
            Set-HelperState -State "Warning" -Message ("Teleport protection is disabled. Multi-instance remains enabled, but multi-client teleports may fail. {0}" -f $script:TeleportFailure)
            [System.Windows.Forms.MessageBox]::Show(
                $script:Form,
                "Teleport protection could not be enabled.`r`n`r`n$($script:TeleportFailure)`r`n`r`nMulti-instance remains enabled, but multi-client teleports may fail.",
                "Teleport protection warning",
                [System.Windows.Forms.MessageBoxButtons]::OK,
                [System.Windows.Forms.MessageBoxIcon]::Warning
            ) | Out-Null
        }
    }
    catch {
        Release-ProtectionResources
        Set-HelperState -State "Error" -Message $_.Exception.Message
        [System.Windows.Forms.MessageBox]::Show(
            $script:Form,
            $_.Exception.Message,
            "Roblox Multi-Account Helper",
            [System.Windows.Forms.MessageBoxButtons]::OK,
            [System.Windows.Forms.MessageBoxIcon]::Error
        ) | Out-Null
    }
    finally {
        $script:IsBusy = $false
        Update-ProcessCount | Out-Null
        Update-ButtonStates
    }
}

function Show-CurrentStatus {
    $processes = @(Update-ProcessCount)
    $ids = @($processes | ForEach-Object { $_.Id })
    $pidText = if ($ids.Count -gt 0) { $ids -join ", " } else { "None" }
    $teleportText = if ($null -ne $script:CookieLock) {
        "Active"
    }
    elseif ($script:TeleportFailure) {
        "Disabled - $($script:TeleportFailure)"
    }
    else {
        "Disabled"
    }

    $status = @(
        "Helper mutex owned: $(if ($script:HelperMutexOwned) { 'Yes' } else { 'No' })"
        "ROBLOX_singletonMutex owned: $(if ($script:RobloxMutexOwned) { 'Yes' } else { 'No' })"
        "Teleport protection: $teleportText"
        "Roblox clients: $($processes.Count)"
        "Roblox PIDs: $pidText"
        "Helper state: $($script:State)"
    ) -join [Environment]::NewLine

    Write-UiLog -Message ("Status checked: helper={0}, multi-instance={1}, teleport={2}, clients={3}." -f $script:HelperMutexOwned, $script:RobloxMutexOwned, ($null -ne $script:CookieLock), $processes.Count)
    [System.Windows.Forms.MessageBox]::Show(
        $script:Form,
        $status,
        "Helper Status",
        [System.Windows.Forms.MessageBoxButtons]::OK,
        [System.Windows.Forms.MessageBoxIcon]::Information
    ) | Out-Null
}

function Close-RobloxFromUi {
    if ($script:IsBusy) {
        return
    }

    $processes = @(Update-ProcessCount)
    if ($processes.Count -eq 0) {
        Write-UiLog -Message "No Roblox clients are running."
        return
    }

    if ($processes.Count -gt 1) {
        $answer = [System.Windows.Forms.MessageBox]::Show(
            $script:Form,
            "Close all $($processes.Count) running Roblox clients?",
            "Close Roblox",
            [System.Windows.Forms.MessageBoxButtons]::YesNo,
            [System.Windows.Forms.MessageBoxIcon]::Warning
        )

        if ($answer -ne [System.Windows.Forms.DialogResult]::Yes) {
            return
        }
    }

    $script:IsBusy = $true
    Set-HelperState -State "Closing Roblox" -Message "Closing Roblox by user request..."

    try {
        Stop-RobloxProcesses -TimeoutSeconds 10
        if ($script:RobloxMutexOwned -and $null -ne $script:CookieLock) {
            Set-HelperState -State "Ready" -Message "Roblox closed. The helper remains ready."
        }
        elseif ($script:RobloxMutexOwned) {
            Set-HelperState -State "Warning" -Message "Roblox closed. Multi-instance remains enabled; teleport protection is disabled."
        }
        else {
            Set-HelperState -State "Idle" -Message "Roblox closed."
        }
    }
    catch {
        Set-HelperState -State "Error" -Message $_.Exception.Message
        [System.Windows.Forms.MessageBox]::Show(
            $script:Form,
            $_.Exception.Message,
            "Could not close Roblox",
            [System.Windows.Forms.MessageBoxButtons]::OK,
            [System.Windows.Forms.MessageBoxIcon]::Error
        ) | Out-Null
    }
    finally {
        $script:IsBusy = $false
        Update-ProcessCount | Out-Null
        Update-ButtonStates
    }
}

function Confirm-StopWithRobloxRunning {
    param(
        [int]$ProcessCount
    )

    $dialog = [System.Windows.Forms.Form]::new()
    $dialog.Text = "Stop Helper"
    $dialog.ClientSize = [System.Drawing.Size]::new(430, 155)
    $dialog.FormBorderStyle = [System.Windows.Forms.FormBorderStyle]::FixedDialog
    $dialog.StartPosition = [System.Windows.Forms.FormStartPosition]::CenterParent
    $dialog.MaximizeBox = $false
    $dialog.MinimizeBox = $false
    $dialog.ShowInTaskbar = $false
    $dialog.AutoScaleMode = [System.Windows.Forms.AutoScaleMode]::Dpi

    $message = [System.Windows.Forms.Label]::new()
    $message.Location = [System.Drawing.Point]::new(18, 16)
    $message.Size = [System.Drawing.Size]::new(395, 75)
    $message.Text = "Roblox clients are still running. Stopping the helper will disable multi-instance and teleport protection.`r`n`r`n$ProcessCount Roblox client(s) detected."
    $dialog.Controls.Add($message)

    $cancelButton = [System.Windows.Forms.Button]::new()
    $cancelButton.Text = "Cancel"
    $cancelButton.Location = [System.Drawing.Point]::new(175, 108)
    $cancelButton.Size = [System.Drawing.Size]::new(95, 30)
    $cancelButton.DialogResult = [System.Windows.Forms.DialogResult]::Cancel
    $dialog.Controls.Add($cancelButton)

    $stopAnywayButton = [System.Windows.Forms.Button]::new()
    $stopAnywayButton.Text = "Stop Helper Anyway"
    $stopAnywayButton.Location = [System.Drawing.Point]::new(278, 108)
    $stopAnywayButton.Size = [System.Drawing.Size]::new(135, 30)
    $stopAnywayButton.DialogResult = [System.Windows.Forms.DialogResult]::OK
    $dialog.Controls.Add($stopAnywayButton)

    $dialog.CancelButton = $cancelButton
    $dialog.AcceptButton = $stopAnywayButton

    try {
        return $dialog.ShowDialog($script:Form) -eq [System.Windows.Forms.DialogResult]::OK
    }
    finally {
        $dialog.Dispose()
    }
}

function Add-FieldLabel {
    param(
        [string]$Text,
        [int]$Top
    )

    $label = [System.Windows.Forms.Label]::new()
    $label.Text = $Text
    $label.Location = [System.Drawing.Point]::new(24, $Top)
    $label.Size = [System.Drawing.Size]::new(175, 23)
    $label.TextAlign = [System.Drawing.ContentAlignment]::MiddleLeft
    $script:Form.Controls.Add($label)
    return $label
}

function Add-ValueLabel {
    param(
        [int]$Top
    )

    $label = [System.Windows.Forms.Label]::new()
    $label.Location = [System.Drawing.Point]::new(210, $Top)
    $label.Size = [System.Drawing.Size]::new(275, 23)
    $label.TextAlign = [System.Drawing.ContentAlignment]::MiddleLeft
    $label.Font = [System.Drawing.Font]::new($script:Form.Font, [System.Drawing.FontStyle]::Bold)
    $script:Form.Controls.Add($label)
    return $label
}

try {
    if (-not (Acquire-HelperMutex)) {
        [System.Windows.Forms.MessageBox]::Show(
            "Roblox Multi-Account Helper is already running.`r`n`r`nThe existing helper and Roblox clients were not disturbed.",
            "Helper already running",
            [System.Windows.Forms.MessageBoxButtons]::OK,
            [System.Windows.Forms.MessageBoxIcon]::Information
        ) | Out-Null
        return
    }

    $script:Form = [System.Windows.Forms.Form]::new()
    $script:Form.Text = "Roblox Multi-Account Helper"
    $script:Form.ClientSize = [System.Drawing.Size]::new(510, 465)
    $script:Form.FormBorderStyle = [System.Windows.Forms.FormBorderStyle]::FixedSingle
    $script:Form.MaximizeBox = $false
    $script:Form.StartPosition = [System.Windows.Forms.FormStartPosition]::CenterScreen
    $script:Form.AutoScaleMode = [System.Windows.Forms.AutoScaleMode]::Dpi
    $script:Form.Font = [System.Drawing.Font]::new("Segoe UI", 9)

    $heading = [System.Windows.Forms.Label]::new()
    $heading.Text = "Roblox Multi-Account Helper"
    $heading.Location = [System.Drawing.Point]::new(20, 15)
    $heading.Size = [System.Drawing.Size]::new(465, 30)
    $heading.Font = [System.Drawing.Font]::new("Segoe UI Semibold", 14)
    $script:Form.Controls.Add($heading)

    Add-FieldLabel -Text "Multi-instance" -Top 57 | Out-Null
    $script:MultiValueLabel = Add-ValueLabel -Top 57
    Add-FieldLabel -Text "Teleport protection" -Top 84 | Out-Null
    $script:TeleportValueLabel = Add-ValueLabel -Top 84
    Add-FieldLabel -Text "Roblox clients" -Top 111 | Out-Null
    $script:ProcessValueLabel = Add-ValueLabel -Top 111
    Add-FieldLabel -Text "Helper state" -Top 138 | Out-Null
    $script:StateValueLabel = Add-ValueLabel -Top 138

    $script:StartButton = [System.Windows.Forms.Button]::new()
    $script:StartButton.Text = "Start / Prepare"
    $script:StartButton.Location = [System.Drawing.Point]::new(24, 180)
    $script:StartButton.Size = [System.Drawing.Size]::new(220, 34)
    $script:Form.Controls.Add($script:StartButton)

    $script:StatusButton = [System.Windows.Forms.Button]::new()
    $script:StatusButton.Text = "Show Status"
    $script:StatusButton.Location = [System.Drawing.Point]::new(266, 180)
    $script:StatusButton.Size = [System.Drawing.Size]::new(220, 34)
    $script:Form.Controls.Add($script:StatusButton)

    $script:CloseRobloxButton = [System.Windows.Forms.Button]::new()
    $script:CloseRobloxButton.Text = "Close Roblox"
    $script:CloseRobloxButton.Location = [System.Drawing.Point]::new(24, 224)
    $script:CloseRobloxButton.Size = [System.Drawing.Size]::new(220, 34)
    $script:Form.Controls.Add($script:CloseRobloxButton)

    $script:StopButton = [System.Windows.Forms.Button]::new()
    $script:StopButton.Text = "Stop Helper"
    $script:StopButton.Location = [System.Drawing.Point]::new(266, 224)
    $script:StopButton.Size = [System.Drawing.Size]::new(220, 34)
    $script:Form.Controls.Add($script:StopButton)

    $statusHeading = [System.Windows.Forms.Label]::new()
    $statusHeading.Text = "Status"
    $statusHeading.Location = [System.Drawing.Point]::new(24, 276)
    $statusHeading.Size = [System.Drawing.Size]::new(100, 22)
    $statusHeading.Font = [System.Drawing.Font]::new($script:Form.Font, [System.Drawing.FontStyle]::Bold)
    $script:Form.Controls.Add($statusHeading)

    $script:LogBox = [System.Windows.Forms.TextBox]::new()
    $script:LogBox.Location = [System.Drawing.Point]::new(24, 300)
    $script:LogBox.Size = [System.Drawing.Size]::new(462, 142)
    $script:LogBox.Multiline = $true
    $script:LogBox.ReadOnly = $true
    $script:LogBox.ScrollBars = [System.Windows.Forms.ScrollBars]::Vertical
    $script:LogBox.WordWrap = $true
    $script:LogBox.BackColor = [System.Drawing.SystemColors]::Window
    $script:Form.Controls.Add($script:LogBox)

    $script:StartButton.Add_Click({ Start-Preparation })
    $script:StatusButton.Add_Click({ Show-CurrentStatus })
    $script:CloseRobloxButton.Add_Click({ Close-RobloxFromUi })
    $script:StopButton.Add_Click({ $script:Form.Close() })

    $script:ProcessTimer = [System.Windows.Forms.Timer]::new()
    $script:ProcessTimer.Interval = 1000
    $script:ProcessTimer.Add_Tick({
        try {
            Update-ProcessCount | Out-Null
        }
        catch {
            Write-UiLog -Message "Could not update Roblox process count: $($_.Exception.Message)" -Important
        }
    })

    $script:Form.Add_FormClosing({
        param($sender, $eventArgs)

        if ($script:AllowFormClose) {
            return
        }

        if ($script:IsBusy) {
            [System.Windows.Forms.MessageBox]::Show(
                $script:Form,
                "Setup is currently running. Wait for it to finish before stopping the helper.",
                "Helper is busy",
                [System.Windows.Forms.MessageBoxButtons]::OK,
                [System.Windows.Forms.MessageBoxIcon]::Information
            ) | Out-Null
            $eventArgs.Cancel = $true
            return
        }

        $processes = @(Get-RobloxProcesses)
        if ($processes.Count -gt 0 -and -not (Confirm-StopWithRobloxRunning -ProcessCount $processes.Count)) {
            $eventArgs.Cancel = $true
            return
        }

        Set-HelperState -State "Stopping" -Message "Stopping helper and releasing owned resources..."
        Release-AllResources
        $script:AllowFormClose = $true
    })

    $script:Form.Add_Shown({
        Write-UiLog -Message "Helper started." -Important
        Update-ProcessCount | Out-Null
        $script:ProcessTimer.Start()
        Start-Preparation
    })

    Set-HelperState -State "Idle" -Message "Helper loaded."
    Update-ProcessCount | Out-Null
    [System.Windows.Forms.Application]::Run($script:Form)
}
catch {
    [System.Windows.Forms.MessageBox]::Show(
        "The helper could not start.`r`n`r`n$($_.Exception.Message)",
        "Roblox Multi-Account Helper",
        [System.Windows.Forms.MessageBoxButtons]::OK,
        [System.Windows.Forms.MessageBoxIcon]::Error
    ) | Out-Null
}
finally {
    Release-AllResources

    if ($null -ne $script:Form) {
        try {
            $script:Form.Dispose()
        }
        catch {
        }
    }
}

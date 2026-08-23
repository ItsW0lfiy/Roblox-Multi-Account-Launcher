@echo off
setlocal
cd /d "%~dp0"

where pwsh.exe >nul 2>&1
if errorlevel 1 (
    echo Roblox Multi-Account Helper requires PowerShell 7 ^(pwsh.exe^).
    echo Install PowerShell 7, then run this launcher again.
    echo.
    pause
    exit /b 1
)

start "" pwsh.exe -NoLogo -NoProfile -STA -WindowStyle Hidden -ExecutionPolicy Bypass -File "%~dp0roblox-multi-account.ps1" %*
if errorlevel 1 (
    echo Failed to start Roblox Multi-Account Helper.
    pause
    exit /b 1
)

exit /b 0

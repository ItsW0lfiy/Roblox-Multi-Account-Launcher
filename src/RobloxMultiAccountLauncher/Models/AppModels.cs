using System.Diagnostics;

namespace RobloxMultiAccountLauncher.Models;

public enum LaunchBackend
{
    Auto,
    Fishstrap,
    Bloxstrap,
    DefaultRoblox
}

public enum LauncherState
{
    Normal,
    Preparing,
    ClosingRoblox,
    WaitingForRobloxExit,
    AcquiringMultiInstance,
    EnablingTeleportProtection,
    MultiAccountReady,
    MultiAccountWarning,
    LaunchingRoblox,
    ProtectionLost,
    Disabling,
    Error
}

public enum ProtectionHealth
{
    Disabled,
    Enabled,
    Warning,
    Lost
}

public enum WindowLayoutMode
{
    FiftyFifty,
    PrimarySecondary,
    Vertical
}

public sealed record ProtocolRegistration(string Scheme, string? Command, string Owner, string? ExecutablePath, bool IsHealthy);

public sealed record LauncherInstallation(
    LaunchBackend Backend,
    bool IsInstalled,
    string? ExecutablePath,
    string? Version,
    string? BaseDirectory,
    string? LogsDirectory);

public sealed record LauncherDetection(
    LauncherInstallation Fishstrap,
    LauncherInstallation Bloxstrap,
    LauncherInstallation DefaultRoblox,
    ProtocolRegistration RobloxProtocol,
    ProtocolRegistration RobloxPlayerProtocol)
{
    public LaunchBackend Resolve(LaunchBackend selected) => selected switch
    {
        LaunchBackend.Auto when Fishstrap.IsInstalled => LaunchBackend.Fishstrap,
        LaunchBackend.Auto when Bloxstrap.IsInstalled => LaunchBackend.Bloxstrap,
        LaunchBackend.Auto => LaunchBackend.DefaultRoblox,
        _ => selected
    };

    public LauncherInstallation Get(LaunchBackend backend) => backend switch
    {
        LaunchBackend.Fishstrap => Fishstrap,
        LaunchBackend.Bloxstrap => Bloxstrap,
        _ => DefaultRoblox
    };
}

public sealed record RobloxClientInfo(
    int Number,
    int ProcessId,
    DateTime? StartedAt,
    TimeSpan Uptime,
    long WorkingSetBytes,
    nint MainWindowHandle,
    string WindowTitle,
    bool HasVisibleWindow)
{
    public string MemoryText => $"{WorkingSetBytes / 1024d / 1024d:N0} MB";
    public string UptimeText => Uptime.TotalHours >= 1
        ? Uptime.ToString(@"h\:mm\:ss")
        : Uptime.ToString(@"m\:ss");
}

public sealed class AppSettings
{
    public LaunchBackend LaunchBackend { get; set; } = LaunchBackend.Auto;
    public WindowLayoutMode WindowLayout { get; set; } = WindowLayoutMode.FiftyFifty;
    public string? PreferredMonitorDeviceName { get; set; }
    public bool MinimizeToTray { get; set; } = true;
    public bool GlobalHotkeysEnabled { get; set; }
    public bool AutoDisableMultiAccount { get; set; }
    public int LaunchTimeoutSeconds { get; set; } = 45;
    public int GracefulCloseTimeoutSeconds { get; set; } = 8;
    public double WindowWidth { get; set; } = 1040;
    public double WindowHeight { get; set; } = 720;
    public double? WindowLeft { get; set; }
    public double? WindowTop { get; set; }

    public void Normalize()
    {
        LaunchTimeoutSeconds = Math.Clamp(LaunchTimeoutSeconds, 10, 180);
        GracefulCloseTimeoutSeconds = Math.Clamp(GracefulCloseTimeoutSeconds, 2, 30);
        WindowWidth = Math.Clamp(WindowWidth, 920, 1800);
        WindowHeight = Math.Clamp(WindowHeight, 620, 1200);
    }
}

public sealed record RecentActivity(DateTime Timestamp, string Message, bool IsError = false)
{
    public string Display => $"{Timestamp:HH:mm}  {Message}";
}

public sealed record MultiAccountResult(bool MultiInstanceEnabled, bool TeleportProtectionEnabled, string Message)
{
    public ProtectionHealth Health => !MultiInstanceEnabled
        ? ProtectionHealth.Disabled
        : TeleportProtectionEnabled ? ProtectionHealth.Enabled : ProtectionHealth.Warning;
}

public sealed record LaunchResult(bool Success, string Message, int? ProcessId = null);

public static class ProcessExtensions
{
    public static bool HasExitedSafe(this Process process)
    {
        try { return process.HasExited; }
        catch { return true; }
    }
}

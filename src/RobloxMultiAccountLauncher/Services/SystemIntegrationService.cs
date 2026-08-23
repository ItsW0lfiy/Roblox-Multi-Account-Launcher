using System.Diagnostics;
using RobloxMultiAccountLauncher.Models;

namespace RobloxMultiAccountLauncher.Services;

public static class SystemIntegrationService
{
    public static bool OpenPath(string? path)
    {
        if (string.IsNullOrWhiteSpace(path) || (!File.Exists(path) && !Directory.Exists(path))) return false;
        try
        {
            Process.Start(new ProcessStartInfo(path) { UseShellExecute = true });
            return true;
        }
        catch { return false; }
    }

    public static bool OpenLauncher(LauncherInstallation installation) => OpenPath(installation.ExecutablePath);
    public static bool OpenFolder(LauncherInstallation installation) => OpenPath(installation.BaseDirectory);
    public static bool OpenLogs(LauncherInstallation installation) => OpenPath(installation.LogsDirectory);
}

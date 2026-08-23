using System.Text;
using System.Text.RegularExpressions;
using RobloxMultiAccountLauncher.Models;

namespace RobloxMultiAccountLauncher.Services;

public static partial class DiagnosticService
{
    public static string Redact(string text)
    {
        if (string.IsNullOrEmpty(text)) return text;
        var redacted = SecurityCookieRegex().Replace(text, "$1[REDACTED]");
        redacted = SensitiveQueryRegex().Replace(redacted, "$1=[REDACTED]");
        redacted = BearerRegex().Replace(redacted, "$1[REDACTED]");
        return redacted;
    }

    public static string BuildReport(LauncherDetection detection, LaunchBackend selected, IReadOnlyList<RobloxClientInfo> clients, MultiAccountService multiAccount, string? recentError)
    {
        var resolved = detection.Resolve(selected);
        var builder = new StringBuilder();
        builder.AppendLine("Roblox Multi-Account Launcher:");
        builder.AppendLine($"Version: {typeof(DiagnosticService).Assembly.GetName().Version}");
        builder.AppendLine();
        builder.AppendLine("Windows:");
        builder.AppendLine($"Version: {Environment.OSVersion.VersionString}");
        builder.AppendLine($".NET: {Environment.Version}");
        builder.AppendLine();
        builder.AppendLine("Launch backend:");
        builder.AppendLine($"Selected: {selected}");
        builder.AppendLine($"Resolved: {resolved}");
        builder.AppendLine();
        AppendLauncher(builder, "Fishstrap", detection.Fishstrap, detection);
        AppendLauncher(builder, "Bloxstrap", detection.Bloxstrap, detection);
        AppendLauncher(builder, "Default Roblox", detection.DefaultRoblox, detection);
        builder.AppendLine("Roblox:");
        builder.AppendLine($"Installed: {detection.DefaultRoblox.IsInstalled}");
        builder.AppendLine($"Processes: {clients.Count}");
        builder.AppendLine();
        builder.AppendLine("Multi-account:");
        builder.AppendLine($"Enabled: {multiAccount.MultiInstanceEnabled}");
        builder.AppendLine($"Singleton mutex: {(multiAccount.MultiInstanceEnabled ? "Owned" : "Not owned")}");
        builder.AppendLine($"Teleport protection: {multiAccount.Health}");
        builder.AppendLine();
        builder.AppendLine($"Recent error: {recentError ?? "None"}");
        return Redact(builder.ToString());
    }

    private static void AppendLauncher(StringBuilder builder, string name, LauncherInstallation installation, LauncherDetection detection)
    {
        builder.AppendLine($"{name}:");
        builder.AppendLine($"Detected: {installation.IsInstalled}");
        builder.AppendLine($"Version: {installation.Version ?? "Unknown"}");
        builder.AppendLine($"Protocol owner: {detection.RobloxProtocol.Owner}");
        builder.AppendLine();
    }

    public static string AnalyzeRecentRobloxLog(string? logsDirectory)
    {
        if (string.IsNullOrWhiteSpace(logsDirectory) || !Directory.Exists(logsDirectory)) return "Roblox logs directory was not found.";
        try
        {
            var file = new DirectoryInfo(logsDirectory).GetFiles("*.log").OrderByDescending(item => item.LastWriteTimeUtc).FirstOrDefault();
            if (file is null) return "No Roblox logs were found.";
            var text = File.ReadAllText(file.FullName);
            var findings = new List<string>();
            if (text.Contains("773", StringComparison.OrdinalIgnoreCase) || text.Contains("unauthorized teleport", StringComparison.OrdinalIgnoreCase)) findings.Add("Possible teleport authorization failure (including error 773).");
            if (text.Contains("crash", StringComparison.OrdinalIgnoreCase)) findings.Add("Crash-related entries detected.");
            if (text.Contains("update", StringComparison.OrdinalIgnoreCase) && text.Contains("fail", StringComparison.OrdinalIgnoreCase)) findings.Add("Possible update failure detected.");
            if (text.Contains("network", StringComparison.OrdinalIgnoreCase) && text.Contains("error", StringComparison.OrdinalIgnoreCase)) findings.Add("Possible network failure detected.");
            return findings.Count == 0 ? "No recognized high-level failure pattern was found in the newest log." : string.Join(Environment.NewLine, findings);
        }
        catch (Exception ex) { return $"Log analysis failed: {ex.Message}"; }
    }

    [GeneratedRegex("(?i)(\\.ROBLOSECURITY\\s*[:=]\\s*)[^\\s;]+")]
    private static partial Regex SecurityCookieRegex();
    [GeneratedRegex("(?i)(ticket|token|code|auth|privateServerLinkCode)=([^&\\s]+)")]
    private static partial Regex SensitiveQueryRegex();
    [GeneratedRegex("(?i)(Bearer\\s+)[A-Za-z0-9._~-]+")]
    private static partial Regex BearerRegex();
}

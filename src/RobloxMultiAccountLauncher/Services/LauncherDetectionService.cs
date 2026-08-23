using System.Diagnostics;
using System.Text.RegularExpressions;
using Microsoft.Win32;
using RobloxMultiAccountLauncher.Models;

namespace RobloxMultiAccountLauncher.Services;

public sealed partial class LauncherDetectionService
{
    private readonly string _localAppData;

    public LauncherDetectionService(string? localAppData = null)
    {
        _localAppData = localAppData ?? Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData);
    }

    public LauncherDetection Detect()
    {
        var fishstrap = DetectLauncher(
            LaunchBackend.Fishstrap,
            new[] { Path.Combine(_localAppData, "Fishstrap", "Fishstrap.exe") },
            Path.Combine(_localAppData, "Fishstrap"));

        var bloxstrap = DetectLauncher(
            LaunchBackend.Bloxstrap,
            new[]
            {
                Path.Combine(_localAppData, "Bloxstrap", "Bloxstrap.exe"),
                Path.Combine(_localAppData, "Bloxstrap", "Bloxstrap-v2.0.0.exe")
            },
            Path.Combine(_localAppData, "Bloxstrap"));

        var stock = DetectStockRoblox();
        return new LauncherDetection(fishstrap, bloxstrap, stock, ReadProtocol("roblox"), ReadProtocol("roblox-player"));
    }

    private static LauncherInstallation DetectLauncher(LaunchBackend backend, IEnumerable<string> candidates, string baseDirectory)
    {
        var executable = candidates.FirstOrDefault(File.Exists);
        if (executable is null && Directory.Exists(baseDirectory))
        {
            executable = SafeEnumerateFiles(baseDirectory, $"{backend}*.exe", SearchOption.TopDirectoryOnly).FirstOrDefault();
        }

        var logs = new[] { Path.Combine(baseDirectory, "Logs"), Path.Combine(baseDirectory, "logs") }.FirstOrDefault(Directory.Exists);
        return new LauncherInstallation(backend, executable is not null, executable, GetVersion(executable), Directory.Exists(baseDirectory) ? baseDirectory : null, logs);
    }

    private LauncherInstallation DetectStockRoblox()
    {
        var versions = Path.Combine(_localAppData, "Roblox", "Versions");
        var executable = ReadMachineProtocolExecutable("roblox");
        if (!File.Exists(executable)) executable = null;
        if (Directory.Exists(versions))
        {
            executable ??= SafeEnumerateFiles(versions, "RobloxPlayerBeta.exe", SearchOption.AllDirectories)
                .Select(path => new FileInfo(path))
                .OrderByDescending(file => file.LastWriteTimeUtc)
                .Select(file => file.FullName)
                .FirstOrDefault();
        }

        return new LauncherInstallation(
            LaunchBackend.DefaultRoblox,
            executable is not null,
            executable,
            GetVersion(executable),
            Directory.Exists(versions) ? versions : null,
            Directory.Exists(Path.Combine(_localAppData, "Roblox", "logs")) ? Path.Combine(_localAppData, "Roblox", "logs") : null);
    }

    public static ProtocolRegistration ReadProtocol(string scheme)
    {
        string? command = null;
        try
        {
            using var key = Registry.CurrentUser.OpenSubKey($@"Software\Classes\{scheme}\shell\open\command", false)
                ?? Registry.ClassesRoot.OpenSubKey($@"{scheme}\shell\open\command", false);
            command = key?.GetValue(null) as string;
        }
        catch { }

        var executable = ExtractExecutable(command);
        var owner = executable is null ? "Unregistered"
            : Path.GetFileName(executable).Contains("Fishstrap", StringComparison.OrdinalIgnoreCase) ? "Fishstrap"
            : Path.GetFileName(executable).Contains("Bloxstrap", StringComparison.OrdinalIgnoreCase) ? "Bloxstrap"
            : Path.GetFileName(executable).Contains("Roblox", StringComparison.OrdinalIgnoreCase) ? "Default Roblox"
            : "Unknown";

        return new ProtocolRegistration(scheme, command, owner, executable, executable is not null && File.Exists(executable));
    }

    private static string? ReadMachineProtocolExecutable(string scheme)
    {
        try
        {
            using var key = Registry.LocalMachine.OpenSubKey($@"Software\Classes\{scheme}\shell\open\command", false);
            return ExtractExecutable(key?.GetValue(null) as string);
        }
        catch { return null; }
    }

    public static string? ExtractExecutable(string? command)
    {
        if (string.IsNullOrWhiteSpace(command)) return null;
        var match = CommandExecutableRegex().Match(command.Trim());
        if (!match.Success) return null;
        var value = match.Groups[1].Success ? match.Groups[1].Value : match.Groups[2].Value;
        return Environment.ExpandEnvironmentVariables(value);
    }

    private static string? GetVersion(string? executable)
    {
        try { return executable is null ? null : FileVersionInfo.GetVersionInfo(executable).FileVersion; }
        catch { return null; }
    }

    private static IEnumerable<string> SafeEnumerateFiles(string directory, string pattern, SearchOption option)
    {
        try { return Directory.EnumerateFiles(directory, pattern, option).ToArray(); }
        catch (UnauthorizedAccessException) { return Array.Empty<string>(); }
        catch (IOException) { return Array.Empty<string>(); }
    }

    [GeneratedRegex("^(?:\\\"([^\\\"]+\\.exe)\\\"|([^\\s]+\\.exe))", RegexOptions.IgnoreCase)]
    private static partial Regex CommandExecutableRegex();
}

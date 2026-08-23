using System.Diagnostics;
using RobloxMultiAccountLauncher.Models;

namespace RobloxMultiAccountLauncher.Services;

public sealed class LaunchService
{
    private readonly LaunchQueue _queue = new();
    private readonly RobloxProcessService _processes;
    private readonly AppLogger _logger;
    public bool IsLaunching => _queue.IsBusy;

    public LaunchService(RobloxProcessService processes, AppLogger logger)
    {
        _processes = processes;
        _logger = logger;
    }

    public async Task<LaunchResult> LaunchAsync(LauncherDetection detection, LaunchBackend selected, TimeSpan timeout, CancellationToken cancellationToken)
    {
        var queued = await _queue.TryRunAsync(
            token => LaunchCoreAsync(detection, selected, timeout, token),
            cancellationToken).ConfigureAwait(false);
        return queued.Accepted
            ? queued.Result!
            : new LaunchResult(false, "A launcher bootstrap operation is already active. Wait for it to finish.");
    }

    private async Task<LaunchResult> LaunchCoreAsync(LauncherDetection detection, LaunchBackend selected, TimeSpan timeout, CancellationToken cancellationToken)
    {
        try
        {
            var resolved = detection.Resolve(selected);
            var installation = detection.Get(resolved);
            if (!installation.IsInstalled || string.IsNullOrWhiteSpace(installation.ExecutablePath) || !File.Exists(installation.ExecutablePath))
                return new LaunchResult(false, $"{BackendName(resolved)} is no longer available. Retry detection or choose another backend.");

            var before = _processes.GetClients().Select(client => client.ProcessId).ToHashSet();
            var startInfo = CreateStartInfo(resolved, installation, detection);
            _logger.Info($"Starting Roblox through {BackendName(resolved)}.");
            using var bootstrapper = Process.Start(startInfo);
            if (bootstrapper is null) return new LaunchResult(false, $"Windows could not start {BackendName(resolved)}.");

            var deadline = DateTime.UtcNow + timeout;
            while (DateTime.UtcNow < deadline)
            {
                cancellationToken.ThrowIfCancellationRequested();
                var created = _processes.GetClients().FirstOrDefault(client => !before.Contains(client.ProcessId));
                if (created is not null)
                {
                    _logger.Info($"Roblox client process {created.ProcessId} detected after {BackendName(resolved)} launch.");
                    return new LaunchResult(true, $"Roblox client detected through {BackendName(resolved)}.", created.ProcessId);
                }
                await Task.Delay(350, cancellationToken).ConfigureAwait(false);
            }

            var diagnostic = AnalyzeBootstrapFailure(installation.LogsDirectory);
            _logger.Warning($"{BackendName(resolved)} launch timed out. {diagnostic}");
            return new LaunchResult(false, $"No new Roblox process appeared within {timeout.TotalSeconds:N0} seconds. {diagnostic}");
        }
        catch (OperationCanceledException)
        {
            return new LaunchResult(false, "Launch was cancelled.");
        }
        catch (Exception ex)
        {
            _logger.Error($"Launch failed: {ex.Message}");
            return new LaunchResult(false, ex.Message);
        }
    }

    public static ProcessStartInfo CreateStartInfo(LaunchBackend backend, LauncherInstallation installation, LauncherDetection detection)
    {
        if (backend == LaunchBackend.DefaultRoblox && detection.RobloxProtocol.IsHealthy && detection.RobloxProtocol.Owner == "Default Roblox")
            return new ProcessStartInfo("roblox:") { UseShellExecute = true };

        if (backend is LaunchBackend.Fishstrap or LaunchBackend.Bloxstrap)
        {
            var bootstrapper = new ProcessStartInfo(installation.ExecutablePath!) { UseShellExecute = true };
            bootstrapper.ArgumentList.Add("-player");
            bootstrapper.ArgumentList.Add("roblox:");
            return bootstrapper;
        }

        var stock = new ProcessStartInfo(installation.ExecutablePath!) { UseShellExecute = true };
        stock.ArgumentList.Add("roblox:");
        return stock;
    }

    public static string AnalyzeBootstrapFailure(string? logsDirectory)
    {
        if (string.IsNullOrWhiteSpace(logsDirectory) || !Directory.Exists(logsDirectory)) return "No launcher logs were available for local analysis.";
        try
        {
            var newest = new DirectoryInfo(logsDirectory).GetFiles("*.log").OrderByDescending(file => file.LastWriteTimeUtc).FirstOrDefault();
            if (newest is null) return "No launcher logs were available for local analysis.";
            var text = File.ReadAllText(newest.FullName);
            if (text.Contains("access denied", StringComparison.OrdinalIgnoreCase)) return "The newest local launcher log indicates an access-denied failure.";
            if (text.Contains("network", StringComparison.OrdinalIgnoreCase) || text.Contains("download", StringComparison.OrdinalIgnoreCase)) return "The newest local launcher log suggests a network or update-download failure.";
            if (text.Contains("crashhandler", StringComparison.OrdinalIgnoreCase)) return "A Roblox Crash Handler or update conflict may be blocking startup.";
            if (text.Contains("locked", StringComparison.OrdinalIgnoreCase)) return "The newest local launcher log suggests a locked Roblox file or stale process.";
            if (text.Contains("error", StringComparison.OrdinalIgnoreCase) || text.Contains("fail", StringComparison.OrdinalIgnoreCase)) return "The newest local launcher log contains a bootstrap error; open the logs for details.";
            return "The newest local launcher log did not contain a recognized high-level failure pattern.";
        }
        catch (Exception ex) { return $"Local log analysis failed: {ex.Message}"; }
    }

    public static string BackendName(LaunchBackend backend) => backend switch
    {
        LaunchBackend.Fishstrap => "Fishstrap",
        LaunchBackend.Bloxstrap => "Bloxstrap",
        LaunchBackend.DefaultRoblox => "Default Roblox",
        _ => "Auto"
    };
}

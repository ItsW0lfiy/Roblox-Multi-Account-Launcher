using System.Diagnostics;
using RobloxMultiAccountLauncher.Models;

namespace RobloxMultiAccountLauncher.Services;

public sealed class RobloxProcessService
{
    public IReadOnlyList<RobloxClientInfo> GetClients()
    {
        var now = DateTime.Now;
        var clients = new List<RobloxClientInfo>();
        foreach (var process in Process.GetProcessesByName("RobloxPlayerBeta").OrderBy(GetStartTimeSafe))
        {
            try
            {
                var started = GetStartTimeSafe(process);
                var handle = process.MainWindowHandle;
                clients.Add(new RobloxClientInfo(
                    clients.Count + 1,
                    process.Id,
                    started,
                    started is null ? TimeSpan.Zero : now - started.Value,
                    process.WorkingSet64,
                    handle,
                    string.IsNullOrWhiteSpace(process.MainWindowTitle) ? "Roblox" : process.MainWindowTitle,
                    handle != nint.Zero));
            }
            catch { }
            finally { process.Dispose(); }
        }
        return clients;
    }

    public IReadOnlyList<int> GetCrashHandlerPids() => Process.GetProcessesByName("RobloxCrashHandler")
        .Select(process => { try { return process.Id; } finally { process.Dispose(); } })
        .OrderBy(id => id)
        .ToArray();

    public async Task<IReadOnlyList<int>> CloseGracefullyAsync(IEnumerable<int> pids, TimeSpan timeout, CancellationToken cancellationToken)
    {
        var targets = pids.Distinct().Select(TryGetProcess).Where(p => p is not null).Cast<Process>().ToList();
        try
        {
            foreach (var process in targets)
            {
                try { if (!process.HasExited) process.CloseMainWindow(); } catch { }
            }

            var deadline = DateTime.UtcNow + timeout;
            while (DateTime.UtcNow < deadline)
            {
                cancellationToken.ThrowIfCancellationRequested();
                if (targets.All(p => p.HasExitedSafe())) return Array.Empty<int>();
                await Task.Delay(200, cancellationToken).ConfigureAwait(false);
            }
            return targets.Where(p => !p.HasExitedSafe()).Select(p => p.Id).ToArray();
        }
        finally
        {
            foreach (var process in targets) process.Dispose();
        }
    }

    public IReadOnlyList<int> ForceClose(IEnumerable<int> pids)
    {
        var failures = new List<int>();
        foreach (var pid in pids.Distinct())
        {
            using var process = TryGetProcess(pid);
            if (process is null) continue;
            try { if (!process.HasExited) process.Kill(true); }
            catch { failures.Add(pid); }
        }
        return failures;
    }

    private static Process? TryGetProcess(int pid)
    {
        try { return Process.GetProcessById(pid); }
        catch { return null; }
    }

    private static DateTime? GetStartTimeSafe(Process process)
    {
        try { return process.StartTime; }
        catch { return null; }
    }
}

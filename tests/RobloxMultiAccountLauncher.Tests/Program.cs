using System.Drawing;
using RobloxMultiAccountLauncher;
using RobloxMultiAccountLauncher.Models;
using RobloxMultiAccountLauncher.Services;

internal static class Program
{
    private static int _passed;
    private static int _failed;

    [STAThread]
    private static async Task<int> Main()
    {
        await Run("helper single-instance protection and cleanup", TestSingleInstance);
        await Run("isolated mutex and exclusive file lifecycle", TestMultiAccountLifecycle);
        await Run("multi-account cancellation cleanup", TestMultiAccountCancellation);
        await Run("launcher backend detection", TestDetection);
        await Run("backend fallback resolution", TestBackendResolution);
        await Run("registry protocol inspection is read-only", TestProtocolReadOnly);
        await Run("process enumeration", TestProcessEnumeration);
        await Run("settings persistence", TestSettings);
        await Run("diagnostics redaction", TestRedaction);
        await Run("window layout calculations", TestLayouts);
        await Run("monitor detection", TestMonitors);
        await Run("launch queue exclusion and cancellation", TestLaunchQueue);
        await Run("WPF main window construction and cleanup", TestWindowLoad);

        Console.WriteLine($"RESULT: {_passed} passed, {_failed} failed");
        return _failed == 0 ? 0 : 1;
    }

    private static async Task Run(string name, Func<Task> test)
    {
        try { await test(); _passed++; Console.WriteLine($"PASS  {name}"); }
        catch (Exception ex) { _failed++; Console.WriteLine($"FAIL  {name}: {ex.Message}"); }
    }

    private static async Task TestSingleInstance()
    {
        var name = $"Wolfy_RobloxMultiAccountLauncher_Test_{Guid.NewGuid():N}";
        using (var primary = new SingleInstanceService(name))
        {
            Assert(primary.IsPrimary, "first instance did not acquire its mutex");
            var secondWasPrimary = await Task.Run(() => { using var second = new SingleInstanceService(name); return second.IsPrimary; });
            Assert(!secondWasPrimary, "second instance acquired an owned mutex");
        }
        var reacquired = await Task.Run(() => { using var next = new SingleInstanceService(name); return next.IsPrimary; });
        Assert(reacquired, "mutex could not be reacquired after cleanup");
    }

    private static async Task TestMultiAccountLifecycle()
    {
        var root = CreateTempRoot();
        try
        {
            var fixture = Path.Combine(root, "RobloxCookies.dat");
            await File.WriteAllTextAsync(fixture, "fixture only");
            var mutexName = $"ROBLOX_singletonMutex_Test_{Guid.NewGuid():N}";
            using var service = new MultiAccountService(null, mutexName, fixture);
            var enabled = await service.EnableAsync(TimeSpan.FromSeconds(2));
            Assert(enabled.MultiInstanceEnabled && enabled.TeleportProtectionEnabled, "test protection did not enable");
            var exclusive = false;
            try { using var blocked = File.Open(fixture, FileMode.Open, FileAccess.Read, FileShare.ReadWrite); }
            catch (IOException) { exclusive = true; }
            Assert(exclusive, "fixture lock was not exclusive");
            await service.DisableAsync();
            using (File.Open(fixture, FileMode.Open, FileAccess.Read, FileShare.ReadWrite)) { }
            var reacquired = await service.EnableAsync(TimeSpan.FromSeconds(2));
            Assert(reacquired.TeleportProtectionEnabled, "resources could not be reacquired after cleanup");
            await service.DisableAsync();
        }
        finally { DeleteTempRoot(root); }
    }

    private static async Task TestMultiAccountCancellation()
    {
        var root = CreateTempRoot();
        try
        {
            var fixture = Path.Combine(root, "RobloxCookies.dat");
            await File.WriteAllTextAsync(fixture, "fixture only");
            using var service = new MultiAccountService(null, $"ROBLOX_singletonMutex_Test_{Guid.NewGuid():N}", fixture);
            using var cancellation = new CancellationTokenSource();
            cancellation.Cancel();
            var cancelled = false;
            try { await service.EnableAsync(TimeSpan.FromSeconds(2), cancellation.Token); }
            catch (OperationCanceledException) { cancelled = true; }
            Assert(cancelled && !service.MultiInstanceEnabled, "cancelled preparation left protection active");
        }
        finally { DeleteTempRoot(root); }
    }

    private static Task TestDetection()
    {
        var detection = new LauncherDetectionService().Detect();
        Assert(detection.RobloxProtocol.Scheme == "roblox", "roblox protocol was not inspected");
        Assert(detection.RobloxPlayerProtocol.Scheme == "roblox-player", "roblox-player protocol was not inspected");
        Assert(detection.Resolve(LaunchBackend.Auto) is LaunchBackend.Fishstrap or LaunchBackend.Bloxstrap or LaunchBackend.DefaultRoblox, "Auto did not resolve");
        return Task.CompletedTask;
    }

    private static Task TestBackendResolution()
    {
        var yesFish = Install(LaunchBackend.Fishstrap, true);
        var yesBlox = Install(LaunchBackend.Bloxstrap, true);
        var noFish = Install(LaunchBackend.Fishstrap, false);
        var noBlox = Install(LaunchBackend.Bloxstrap, false);
        var stock = Install(LaunchBackend.DefaultRoblox, true);
        var protocol = new ProtocolRegistration("roblox", null, "Unregistered", null, false);
        Assert(new LauncherDetection(yesFish, yesBlox, stock, protocol, protocol).Resolve(LaunchBackend.Auto) == LaunchBackend.Fishstrap, "Fishstrap was not first priority");
        Assert(new LauncherDetection(noFish, yesBlox, stock, protocol, protocol).Resolve(LaunchBackend.Auto) == LaunchBackend.Bloxstrap, "Bloxstrap fallback failed");
        Assert(new LauncherDetection(noFish, noBlox, stock, protocol, protocol).Resolve(LaunchBackend.Auto) == LaunchBackend.DefaultRoblox, "stock fallback failed");
        Assert(new LauncherDetection(noFish, yesBlox, stock, protocol, protocol).Resolve(LaunchBackend.Fishstrap) == LaunchBackend.Fishstrap, "manual backend was silently changed");
        return Task.CompletedTask;
    }

    private static Task TestProtocolReadOnly()
    {
        var before = LauncherDetectionService.ReadProtocol("roblox");
        _ = new LauncherDetectionService().Detect();
        var after = LauncherDetectionService.ReadProtocol("roblox");
        Assert(before.Command == after.Command, "protocol registration changed during detection");
        return Task.CompletedTask;
    }

    private static Task TestProcessEnumeration()
    {
        var service = new RobloxProcessService();
        var clients = service.GetClients();
        Assert(clients.All(client => client.ProcessId > 0), "invalid process entry returned");
        _ = service.GetCrashHandlerPids();
        return Task.CompletedTask;
    }

    private static Task TestSettings()
    {
        var root = CreateTempRoot();
        try
        {
            var path = Path.Combine(root, "settings.json");
            var service = new SettingsService(path);
            var settings = new AppSettings { LaunchBackend = LaunchBackend.Bloxstrap, WindowLayout = WindowLayoutMode.PrimarySecondary, AutoDisableMultiAccount = true, LaunchTimeoutSeconds = 60 };
            service.Save(settings);
            var loaded = service.Load();
            Assert(loaded.LaunchBackend == LaunchBackend.Bloxstrap && loaded.WindowLayout == WindowLayoutMode.PrimarySecondary && loaded.AutoDisableMultiAccount && loaded.LaunchTimeoutSeconds == 60, "settings round-trip failed");
            Assert(!File.ReadAllText(path).Contains("MultiAccountEnabled", StringComparison.OrdinalIgnoreCase), "enabled protection state was persisted");
        }
        finally { DeleteTempRoot(root); }
        return Task.CompletedTask;
    }

    private static Task TestRedaction()
    {
        const string input = ".ROBLOSECURITY=secret ticket=abc123 token=qwerty Bearer eyJ.secret privateServerLinkCode=pvt987";
        var output = DiagnosticService.Redact(input);
        foreach (var secret in new[] { "secret", "abc123", "qwerty", "eyJ.secret", "pvt987" }) Assert(!output.Contains(secret, StringComparison.Ordinal), $"diagnostics retained {secret}");
        Assert(output.Contains("[REDACTED]"), "redaction marker missing");
        return Task.CompletedTask;
    }

    private static Task TestLayouts()
    {
        foreach (var scale in new[] { 1.0, 1.25, 1.5 })
        {
            var area = new Rectangle(100, 50, (int)(1920 * scale), (int)(1040 * scale));
            foreach (var mode in Enum.GetValues<WindowLayoutMode>())
            {
                var layout = WindowManager.CalculateLayout(area, mode);
                Assert(area.Contains(layout.First) && area.Contains(layout.Second), $"{mode} exceeded work area at {scale:P0}");
                Assert(layout.First.Width > 0 && layout.Second.Width > 0 && layout.First.Height > 0 && layout.Second.Height > 0, $"{mode} produced an empty window");
            }
        }
        return Task.CompletedTask;
    }

    private static Task TestMonitors()
    {
        var monitors = WindowManager.GetMonitors();
        Assert(monitors.Count > 0 && monitors.All(screen => screen.WorkingArea.Width > 0 && screen.WorkingArea.Height > 0), "monitor work areas were unavailable");
        return Task.CompletedTask;
    }

    private static async Task TestLaunchQueue()
    {
        var queue = new LaunchQueue();
        var release = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var first = queue.TryRunAsync(async token => { await release.Task.WaitAsync(token); return 1; }, CancellationToken.None);
        while (!queue.IsBusy) await Task.Yield();
        var second = await queue.TryRunAsync(_ => Task.FromResult(2), CancellationToken.None);
        Assert(!second.Accepted, "overlapping bootstrap operation was accepted");
        release.SetResult();
        var firstResult = await first;
        Assert(firstResult.Accepted && firstResult.Result == 1 && !queue.IsBusy, "queue did not clean up after completion");
        using var cancelled = new CancellationTokenSource();
        cancelled.Cancel();
        var cancellationObserved = false;
        try { await queue.TryRunAsync(_ => Task.FromResult(3), cancelled.Token); }
        catch (OperationCanceledException) { cancellationObserved = true; }
        Assert(cancellationObserved, "queue cancellation was not observed");
    }

    private static Task TestWindowLoad()
    {
        return RunOnStaThread(() =>
        {
            var app = System.Windows.Application.Current as App ?? new App();
            app.InitializeComponent();
            var window = new MainWindow(true);
            window.Measure(new System.Windows.Size(1040, 720));
            window.Arrange(new System.Windows.Rect(0, 0, 1040, 720));
            Assert(window.MinWidth >= 920 && window.MinHeight >= 620 && window.Content is System.Windows.Controls.Grid && window.Title == "Roblox Multi-Account Launcher", "main window resources or constraints failed to load");
            window.AllowImmediateCloseForTest();
            window.Close();
        });
    }

    private static Task RunOnStaThread(Action action)
    {
        var completion = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var thread = new Thread(() =>
        {
            try { action(); completion.SetResult(); }
            catch (Exception ex) { completion.SetException(ex); }
        });
        thread.SetApartmentState(ApartmentState.STA);
        thread.Start();
        return completion.Task;
    }

    private static LauncherInstallation Install(LaunchBackend backend, bool installed) => new(backend, installed, installed ? "C:\\fixture.exe" : null, "1.0", null, null);
    private static string CreateTempRoot() { var path = Path.Combine(Path.GetTempPath(), "RMAL-Tests", Guid.NewGuid().ToString("N")); Directory.CreateDirectory(path); return path; }
    private static void DeleteTempRoot(string path) { var full = Path.GetFullPath(path); var root = Path.GetFullPath(Path.Combine(Path.GetTempPath(), "RMAL-Tests")); if (full.StartsWith(root, StringComparison.OrdinalIgnoreCase) && Directory.Exists(full)) Directory.Delete(full, true); }
    private static void Assert(bool condition, string message) { if (!condition) throw new InvalidOperationException(message); }
}

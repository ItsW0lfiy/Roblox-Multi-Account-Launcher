using System.Collections.ObjectModel;
using System.ComponentModel;
using System.Diagnostics;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Threading;
using RobloxMultiAccountLauncher.Models;
using RobloxMultiAccountLauncher.Services;
using RobloxMultiAccountLauncher.Utilities;

namespace RobloxMultiAccountLauncher;

public partial class MainWindow : Window
{
    private readonly bool _testMode;
    private readonly string? _testRoot;
    private readonly AppLogger _logger;
    private readonly SettingsService _settingsService;
    private readonly RobloxProcessService _processService = new();
    private readonly MultiAccountService _multiAccount;
    private readonly LaunchService _launchService;
    private readonly LauncherDetectionService _detectionService = new();
    private readonly HotkeyService _hotkeys = new();
    private readonly DispatcherTimer _refreshTimer;
    private readonly ObservableCollection<RobloxClientInfo> _clients = new();
    private readonly ObservableCollection<RecentActivity> _activity = new();
    private readonly CancellationTokenSource _lifetime = new();
    private LauncherDetection _detection = null!;
    private AppSettings _settings;
    private LauncherState _state = LauncherState.Normal;
    private System.Windows.Forms.NotifyIcon? _trayIcon;
    private HashSet<int> _previousPids = new();
    private Dictionary<int, RobloxClientInfo> _previousClients = new();
    private DateTime? _zeroClientsSince;
    private bool _updatingControls;
    private bool _operationActive;
    private bool _swapped;
    private bool _allowClose;
    private bool _exitRequested;
    private bool _disposed;
    private string? _recentError;

    public MainWindow(bool testMode = false)
    {
        _testMode = testMode;
        InitializeComponent();

        _testRoot = testMode ? Path.Combine(Path.GetTempPath(), "RobloxMultiAccountLauncher-Smoke", Environment.ProcessId.ToString()) : null;
        _logger = new AppLogger(_testRoot is null ? null : Path.Combine(_testRoot, "Logs"));
        _settingsService = new SettingsService(_testRoot is null ? null : Path.Combine(_testRoot, "settings.json"));
        _settings = testMode ? new AppSettings() : _settingsService.Load();
        _multiAccount = new MultiAccountService(_logger);
        _launchService = new LaunchService(_processService, _logger);

        ClientsList.ItemsSource = _clients;
        ActivityList.ItemsSource = _activity;
        BackendCombo.ItemsSource = Enum.GetValues<LaunchBackend>();
        LayoutCombo.ItemsSource = Enum.GetValues<WindowLayoutMode>();
        MonitorCombo.ItemsSource = WindowManager.GetMonitors().Select(screen => screen.DeviceName).ToArray();
        ApplySettingsToControls();

        _detection = _detectionService.Detect();
        MultiToggle.Checked += MultiToggle_Checked;
        MultiToggle.Unchecked += MultiToggle_Unchecked;
        Closing += Window_Closing;
        Closed += (_, _) => System.Windows.Application.Current.Shutdown();
        HelperStateChanged += (_, _) => UpdateVisualState();

        _hotkeys.FocusClientRequested += (_, number) => Dispatcher.BeginInvoke(() => FocusClient(number));
        Loaded += (_, _) => { if (_settings.GlobalHotkeysEnabled) _hotkeys.Enable(this); };

        _refreshTimer = new DispatcherTimer(TimeSpan.FromSeconds(1), DispatcherPriority.Background, (_, _) => RefreshStatus(), Dispatcher);
        if (!testMode) _refreshTimer.Start();
        InitializeTray();
        RestoreWindowPlacement();
        AddActivity("Launcher started in normal mode.");
        _logger.Info("Application startup. Multi-account mode is OFF.");
        RefreshStatus();
    }

    private event EventHandler? HelperStateChanged;

    public void ActivateFromSecondInstance()
    {
        Show();
        WindowState = WindowState.Normal;
        Activate();
        Topmost = true;
        Topmost = false;
        Focus();
    }

    public void AllowImmediateCloseForTest()
    {
        _allowClose = true;
        _exitRequested = true;
        DisposeResources();
    }

    private void SetState(LauncherState state, string message)
    {
        _state = state;
        StateText.Text = message;
        HelperStateChanged?.Invoke(this, EventArgs.Empty);
    }

    private void RefreshStatus()
    {
        var current = _processService.GetClients();
        var currentPids = current.Select(client => client.ProcessId).ToHashSet();
        foreach (var exited in _previousPids.Except(currentPids))
        {
            if (_previousClients.TryGetValue(exited, out var client))
            {
                AddActivity($"Client {client.Number} exited after {client.UptimeText}.");
                _logger.Info($"Roblox process {exited} exited.");
            }
        }
        foreach (var created in current.Where(client => !_previousPids.Contains(client.ProcessId)))
        {
            AddActivity($"Client {created.Number} detected (PID {created.ProcessId}).");
            _logger.Info($"Roblox process {created.ProcessId} detected.");
        }

        _clients.Clear();
        foreach (var client in current) _clients.Add(client);
        _previousPids = currentPids;
        _previousClients = current.ToDictionary(client => client.ProcessId);

        ClientCountText.Text = $"{current.Count} running";
        ClientsList.Items.Refresh();
        HandleAutoDisable(current.Count);
        UpdateVisualState();
        UpdateDiagnostics();
    }

    private void HandleAutoDisable(int clientCount)
    {
        if (!_settings.AutoDisableMultiAccount || !_multiAccount.MultiInstanceEnabled || _operationActive)
        {
            _zeroClientsSince = null;
            return;
        }
        if (clientCount > 0) { _zeroClientsSince = null; return; }
        _zeroClientsSince ??= DateTime.UtcNow;
        if (DateTime.UtcNow - _zeroClientsSince >= TimeSpan.FromSeconds(30))
        {
            _zeroClientsSince = null;
            _ = DisableMultiAccountAsync(false);
        }
    }

    private async void MultiToggle_Checked(object sender, RoutedEventArgs e)
    {
        if (_updatingControls || _multiAccount.MultiInstanceEnabled) return;
        await EnableMultiAccountAsync();
    }

    private async void MultiToggle_Unchecked(object sender, RoutedEventArgs e)
    {
        if (_updatingControls || !_multiAccount.MultiInstanceEnabled) return;
        await DisableMultiAccountAsync(true);
    }

    private async Task EnableMultiAccountAsync()
    {
        if (_operationActive) return;
        _operationActive = true;
        SetBusy(true);
        try
        {
            var clients = _processService.GetClients();
            if (clients.Count > 0)
            {
                var choice = ChoiceDialog.Show(this, "Enable Multi-Account Mode?",
                    $"{clients.Count} Roblox client(s) are currently running. Enabling multi-account mode requires closing all active Roblox clients. Any game sessions in progress will be disconnected.",
                    "Cancel", "Close Roblox and Enable");
                if (choice != 1) { SetToggle(false); return; }
                if (!await CloseClientsAsync(clients.Select(client => client.ProcessId), true)) { SetToggle(false); return; }
            }

            SetState(LauncherState.AcquiringMultiInstance, "Acquiring multi-instance protection…");
            var result = await _multiAccount.EnableAsync(TimeSpan.FromSeconds(6), _lifetime.Token);
            if (!result.MultiInstanceEnabled)
            {
                _recentError = result.Message;
                SetState(LauncherState.Error, result.Message);
                AddActivity(result.Message, true);
                SetToggle(false);
                return;
            }

            SetToggle(true);
            if (result.TeleportProtectionEnabled)
            {
                SetState(LauncherState.MultiAccountReady, "Multi-account protection is ready.");
                AddActivity("Multi-account mode enabled.");
            }
            else
            {
                _recentError = result.Message;
                SetState(LauncherState.MultiAccountWarning, result.Message);
                AddActivity("Multi-instance enabled; teleport protection warning.", true);
                MessageBox.Show(result.Message, "Teleport protection warning", MessageBoxButton.OK, MessageBoxImage.Warning);
            }
        }
        catch (OperationCanceledException) { SetToggle(false); SetState(LauncherState.Normal, "Preparation cancelled."); }
        catch (Exception ex)
        {
            await _multiAccount.DisableAsync();
            _recentError = ex.Message;
            SetToggle(false);
            SetState(LauncherState.Error, ex.Message);
            AddActivity(ex.Message, true);
        }
        finally { _operationActive = false; SetBusy(false); RefreshStatus(); }
    }

    private async Task DisableMultiAccountAsync(bool confirm)
    {
        if (_operationActive) return;
        _operationActive = true;
        SetBusy(true);
        try
        {
            var clients = _processService.GetClients();
            if (confirm && clients.Count > 0)
            {
                var choice = ChoiceDialog.Show(this, "Disable Multi-Account Mode?",
                    $"{clients.Count} Roblox client(s) are still running. Disabling will release multi-instance and teleport protection.",
                    "Cancel", "Disable Anyway", "Close Roblox and Disable");
                if (choice is < 1 or > 2) { SetToggle(true); return; }
                if (choice == 2 && !await CloseClientsAsync(clients.Select(client => client.ProcessId), false)) { SetToggle(true); return; }
            }

            SetState(LauncherState.Disabling, "Releasing multi-account protection…");
            await _multiAccount.DisableAsync();
            SetToggle(false);
            SetState(LauncherState.Normal, "Normal mode. Multi-account protection is off.");
            AddActivity("Multi-account mode disabled.");
        }
        finally { _operationActive = false; SetBusy(false); RefreshStatus(); }
    }

    private async Task<bool> CloseClientsAsync(IEnumerable<int> pids, bool enablingMultiAccount)
    {
        var ids = pids.ToArray();
        if (ids.Length == 0) return true;
        SetState(LauncherState.ClosingRoblox, "Requesting graceful Roblox shutdown…");
        var remaining = await _processService.CloseGracefullyAsync(ids, TimeSpan.FromSeconds(_settings.GracefulCloseTimeoutSeconds), _lifetime.Token);
        if (remaining.Count == 0) { AddActivity("Roblox closed gracefully."); return true; }

        var choice = ChoiceDialog.Show(this, "Roblox is still running",
            $"Roblox did not close within {_settings.GracefulCloseTimeoutSeconds} seconds. Remaining PID(s): {string.Join(", ", remaining)}.",
            "Cancel", "Force Close");
        if (choice != 1) return false;
        var failures = _processService.ForceClose(remaining);
        if (failures.Count > 0)
        {
            _recentError = $"Could not force-close PID(s): {string.Join(", ", failures)}.";
            MessageBox.Show(_recentError, "Close Roblox", MessageBoxButton.OK, MessageBoxImage.Error);
            return false;
        }
        AddActivity("Roblox force-closed after graceful shutdown timed out.");
        await Task.Delay(300);
        return _processService.GetClients().Count == 0;
    }

    private async void LaunchButton_Click(object sender, RoutedEventArgs e)
    {
        if (_operationActive || _launchService.IsLaunching) return;
        if (ReferenceEquals(sender, LaunchAnotherButton) && (!_multiAccount.MultiInstanceEnabled || !_multiAccount.TeleportProtectionEnabled))
        {
            MessageBox.Show("Multi-account protection is no longer healthy. Re-prepare multi-account mode first.", "Protection unavailable", MessageBoxButton.OK, MessageBoxImage.Warning);
            return;
        }

        var selected = _settings.LaunchBackend;
        var resolved = _detection.Resolve(selected);
        if (selected != LaunchBackend.Auto && !_detection.Get(resolved).IsInstalled)
        {
            var options = new List<string> { "Retry Detection" };
            if (_detection.Bloxstrap.IsInstalled && selected != LaunchBackend.Bloxstrap) options.Add("Use Bloxstrap");
            options.Add("Use Default Roblox");
            var choice = ChoiceDialog.Show(this, $"{LaunchService.BackendName(selected)} is no longer available.", "The manually selected backend is missing. The selection will not be changed silently.", options.ToArray());
            if (choice == 0) { RefreshDetection(); return; }
            if (choice >= 1) selected = options[choice].Contains("Bloxstrap") ? LaunchBackend.Bloxstrap : LaunchBackend.DefaultRoblox;
            else return;
        }

        _operationActive = true;
        SetBusy(true);
        SetState(LauncherState.LaunchingRoblox, $"Starting {LaunchService.BackendName(_detection.Resolve(selected))}…");
        LaunchProgressText.Text = "Preparing launch → starting bootstrapper → waiting for Roblox process";
        try
        {
            var result = await _launchService.LaunchAsync(_detection, selected, TimeSpan.FromSeconds(_settings.LaunchTimeoutSeconds), _lifetime.Token);
            LaunchProgressText.Text = result.Message;
            AddActivity(result.Success ? $"Roblox launched successfully (PID {result.ProcessId})." : result.Message, !result.Success);
            if (!result.Success) _recentError = result.Message;
            SetState(_multiAccount.Health switch
            {
                ProtectionHealth.Enabled => LauncherState.MultiAccountReady,
                ProtectionHealth.Warning => LauncherState.MultiAccountWarning,
                _ => result.Success ? LauncherState.Normal : LauncherState.Error
            }, result.Message);
        }
        finally { _operationActive = false; SetBusy(false); RefreshStatus(); }
    }

    private void RefreshDetection()
    {
        _detection = _detectionService.Detect();
        AddActivity("Launcher and protocol detection refreshed.");
        _logger.Info($"Detection: Fishstrap={_detection.Fishstrap.IsInstalled}, Bloxstrap={_detection.Bloxstrap.IsInstalled}, Stock={_detection.DefaultRoblox.IsInstalled}, protocol={_detection.RobloxProtocol.Owner}.");
        UpdateVisualState();
        UpdateDiagnostics();
    }

    private void RefreshButton_Click(object sender, RoutedEventArgs e) { RefreshDetection(); RefreshStatus(); }
    private void OpenClients_Click(object sender, RoutedEventArgs e) => MainTabs.SelectedIndex = 1;

    private async void CloseAll_Click(object sender, RoutedEventArgs e)
    {
        if (_operationActive) return;
        var clients = _processService.GetClients();
        if (clients.Count == 0) { MessageBox.Show("No Roblox clients are running.", "Close Roblox", MessageBoxButton.OK, MessageBoxImage.Information); return; }
        var choice = ChoiceDialog.Show(this, "Close Roblox", $"Close all {clients.Count} Roblox client(s)?", "Cancel", "Close All");
        if (choice == 1) await CloseClientsAsync(clients.Select(client => client.ProcessId), false);
        RefreshStatus();
    }

    private void FocusClient_Click(object sender, RoutedEventArgs e) { if (!_operationActive && (sender as Button)?.Tag is RobloxClientInfo client) WindowManager.Focus(client); }
    private async void CloseClient_Click(object sender, RoutedEventArgs e) { if (_operationActive) return; if ((sender as Button)?.Tag is RobloxClientInfo client) await CloseClientsAsync(new[] { client.ProcessId }, false); RefreshStatus(); }
    private void ForceClient_Click(object sender, RoutedEventArgs e)
    {
        if (_operationActive) return;
        if ((sender as Button)?.Tag is not RobloxClientInfo client) return;
        if (ChoiceDialog.Show(this, "Force Close Roblox", $"Force-close Client {client.Number} (PID {client.ProcessId})? Unsaved game progress may be lost.", "Cancel", "Force Close") == 1)
            _processService.ForceClose(new[] { client.ProcessId });
        RefreshStatus();
    }

    private void TileClients_Click(object sender, RoutedEventArgs e)
    {
        if (_operationActive) return;
        var clients = _processService.GetClients();
        if (!WindowManager.Tile(clients, _settings.WindowLayout, _settings.PreferredMonitorDeviceName, _swapped))
            MessageBox.Show(clients.Count == 2 ? "Both Roblox clients must have visible windows before tiling." : "The selected two-client layout requires exactly two Roblox clients.", "Tile Clients", MessageBoxButton.OK, MessageBoxImage.Information);
        else { AddActivity($"Clients tiled using {_settings.WindowLayout}."); _logger.Info($"Tiled clients using {_settings.WindowLayout}."); }
    }

    private void SwapClients_Click(object sender, RoutedEventArgs e) { _swapped = !_swapped; TileClients_Click(sender, e); }
    private void MoveClient1_Click(object sender, RoutedEventArgs e) => MoveClient(1);
    private void MoveClient2_Click(object sender, RoutedEventArgs e) => MoveClient(2);
    private void MoveClient(int number)
    {
        var client = _processService.GetClients().ElementAtOrDefault(number - 1);
        if (client is null || !WindowManager.MoveToMonitor(client, _settings.PreferredMonitorDeviceName)) MessageBox.Show($"Client {number} is unavailable or has no visible window.", "Move Client", MessageBoxButton.OK, MessageBoxImage.Information);
        else AddActivity($"Client {number} moved to the preferred monitor.");
    }

    private void FocusClient(int number)
    {
        var client = _processService.GetClients().ElementAtOrDefault(number - 1);
        if (client is not null) WindowManager.Focus(client);
    }

    private void CopyDiagnostics_Click(object sender, RoutedEventArgs e)
    {
        Clipboard.SetText(DiagnosticService.BuildReport(_detection, _settings.LaunchBackend, _processService.GetClients(), _multiAccount, _recentError));
        AddActivity("Sanitized diagnostics copied to the clipboard.");
    }
    private void OpenRobloxLogs_Click(object sender, RoutedEventArgs e) => OpenOrExplain(_detection.DefaultRoblox.LogsDirectory, "Roblox logs directory was not found.");
    private void AnalyzeRobloxLogs_Click(object sender, RoutedEventArgs e) => MessageBox.Show(DiagnosticService.AnalyzeRecentRobloxLog(_detection.DefaultRoblox.LogsDirectory), "Recent Roblox Log", MessageBoxButton.OK, MessageBoxImage.Information);
    private void OpenFishstrap_Click(object sender, RoutedEventArgs e) => OpenOrExplain(_detection.Fishstrap.ExecutablePath, "Fishstrap was not found.");
    private void OpenFishstrapFolder_Click(object sender, RoutedEventArgs e) => OpenOrExplain(_detection.Fishstrap.BaseDirectory, "Fishstrap folder was not found.");
    private void OpenFishstrapLogs_Click(object sender, RoutedEventArgs e) => OpenOrExplain(_detection.Fishstrap.LogsDirectory, "Fishstrap logs were not found.");
    private void OpenBloxstrap_Click(object sender, RoutedEventArgs e) => OpenOrExplain(_detection.Bloxstrap.ExecutablePath, "Bloxstrap was not found.");

    private void OpenOrExplain(string? path, string error)
    {
        if (!SystemIntegrationService.OpenPath(path)) MessageBox.Show(error, "Open", MessageBoxButton.OK, MessageBoxImage.Information);
    }

    private void RepairButton_Click(object sender, RoutedEventArgs e)
    {
        RefreshDetection();
        var crashHandlers = _processService.GetCrashHandlerPids();
        if (crashHandlers.Count == 0)
        {
            MessageBox.Show("Detection, protocols, processes, and multi-account preflight were refreshed. No Roblox Crash Handler processes were found.", "Repair Launch State", MessageBoxButton.OK, MessageBoxImage.Information);
            return;
        }
        var choice = ChoiceDialog.Show(this, "Roblox Crash Handler detected", $"Roblox Crash Handler is still running (PID(s): {string.Join(", ", crashHandlers)}) and may be preventing Roblox from updating or launching.", "Ignore", "Open Diagnostics", "Close Crash Handler");
        if (choice == 1) MainTabs.SelectedIndex = 2;
        if (choice == 2) _processService.ForceClose(crashHandlers);
    }

    private void SettingsChanged(object sender, RoutedEventArgs e)
    {
        if (_updatingControls) return;
        ReadSettingsFromControls();
        SaveSettings();
        if (_settings.GlobalHotkeysEnabled)
        {
            if (!_hotkeys.Enable(this)) { _settings.GlobalHotkeysEnabled = false; HotkeysCheck.IsChecked = false; MessageBox.Show("Ctrl+Alt+1 or Ctrl+Alt+2 is already registered by another application.", "Global hotkeys", MessageBoxButton.OK, MessageBoxImage.Warning); }
        }
        else _hotkeys.Disable();
        UpdateVisualState();
    }

    private void ApplySettingsToControls()
    {
        _updatingControls = true;
        BackendCombo.SelectedItem = _settings.LaunchBackend;
        LayoutCombo.SelectedItem = _settings.WindowLayout;
        MonitorCombo.SelectedItem = _settings.PreferredMonitorDeviceName ?? WindowManager.GetMonitors().FirstOrDefault()?.DeviceName;
        MinimizeTrayCheck.IsChecked = _settings.MinimizeToTray;
        HotkeysCheck.IsChecked = _settings.GlobalHotkeysEnabled;
        AutoDisableCheck.IsChecked = _settings.AutoDisableMultiAccount;
        LaunchTimeoutText.Text = _settings.LaunchTimeoutSeconds.ToString();
        CloseTimeoutText.Text = _settings.GracefulCloseTimeoutSeconds.ToString();
        _updatingControls = false;
    }

    private void ReadSettingsFromControls()
    {
        if (BackendCombo.SelectedItem is LaunchBackend backend) _settings.LaunchBackend = backend;
        if (LayoutCombo.SelectedItem is WindowLayoutMode layout) _settings.WindowLayout = layout;
        _settings.PreferredMonitorDeviceName = MonitorCombo.SelectedItem as string;
        _settings.MinimizeToTray = MinimizeTrayCheck.IsChecked == true;
        _settings.GlobalHotkeysEnabled = HotkeysCheck.IsChecked == true;
        _settings.AutoDisableMultiAccount = AutoDisableCheck.IsChecked == true;
        if (int.TryParse(LaunchTimeoutText.Text, out var launchTimeout)) _settings.LaunchTimeoutSeconds = launchTimeout;
        if (int.TryParse(CloseTimeoutText.Text, out var closeTimeout)) _settings.GracefulCloseTimeoutSeconds = closeTimeout;
        _settings.Normalize();
    }

    private void SaveSettings()
    {
        if (_testMode) return;
        _settings.WindowWidth = ActualWidth;
        _settings.WindowHeight = ActualHeight;
        if (WindowState == WindowState.Normal) { _settings.WindowLeft = Left; _settings.WindowTop = Top; }
        try { _settingsService.Save(_settings); } catch (Exception ex) { _logger.Warning($"Could not save settings: {ex.Message}"); }
    }

    private void RestoreWindowPlacement()
    {
        Width = _settings.WindowWidth; Height = _settings.WindowHeight;
        if (_settings.WindowLeft is { } left && _settings.WindowTop is { } top && WindowManager.GetMonitors().Any(screen => screen.WorkingArea.IntersectsWith(new System.Drawing.Rectangle((int)left, (int)top, 100, 100))))
        { Left = left; Top = top; WindowStartupLocation = WindowStartupLocation.Manual; }
    }

    private void UpdateVisualState()
    {
        var multi = _multiAccount.MultiInstanceEnabled;
        ModeBadge.Text = multi ? (_multiAccount.TeleportProtectionEnabled ? "MULTI-ACCOUNT ACTIVE" : "MULTI-ACCOUNT WARNING") : "NORMAL MODE";
        ModeBadge.Foreground = ResourceBrush(multi ? (_multiAccount.TeleportProtectionEnabled ? "SuccessBrush" : "WarningBrush") : "SecondaryTextBrush");
        MutexStateText.Text = multi ? "Enabled" : "Disabled";
        MutexStateText.Foreground = ResourceBrush(multi ? "SuccessBrush" : "SecondaryTextBrush");
        CookieStateText.Text = _multiAccount.TeleportProtectionEnabled ? "Enabled" : multi ? "Warning" : "Disabled";
        CookieStateText.Foreground = ResourceBrush(_multiAccount.TeleportProtectionEnabled ? "SuccessBrush" : multi ? "WarningBrush" : "SecondaryTextBrush");
        var resolved = _detection.Resolve(_settings.LaunchBackend);
        BackendSummary.Text = $"{_settings.LaunchBackend} → {LaunchService.BackendName(resolved)}  •  Protocol owner: {_detection.RobloxProtocol.Owner}";
        LaunchAnotherButton.IsEnabled = !_operationActive && _multiAccount.MultiInstanceEnabled && _multiAccount.TeleportProtectionEnabled;
        LaunchButton.IsEnabled = !_operationActive;
        MultiToggle.IsEnabled = !_operationActive;
        UpdateTray();
    }

    private void UpdateDiagnostics()
    {
        if (_detection is null) return;
        var resolved = _detection.Resolve(_settings.LaunchBackend);
        var crashHandlers = _processService.GetCrashHandlerPids();
        DiagnosticsText.Text = $"""
            ROBLOX
              Installed:              {_detection.DefaultRoblox.IsInstalled}
              Executable:             {_detection.DefaultRoblox.ExecutablePath ?? "Not found"}
              Active clients:         {_clients.Count}
              Crash handlers:         {(crashHandlers.Count == 0 ? "None" : string.Join(", ", crashHandlers))}
              roblox: owner:          {_detection.RobloxProtocol.Owner} ({Health(_detection.RobloxProtocol.IsHealthy)})
              roblox-player: owner:   {_detection.RobloxPlayerProtocol.Owner} ({Health(_detection.RobloxPlayerProtocol.IsHealthy)})

            FISHSTRAP
              Installed:              {_detection.Fishstrap.IsInstalled}
              Version:                {_detection.Fishstrap.Version ?? "Unknown"}
              Executable:             {_detection.Fishstrap.ExecutablePath ?? "Not found"}
              Protocol owner:         {_detection.RobloxProtocol.Owner == "Fishstrap"}

            BLOXSTRAP
              Installed:              {_detection.Bloxstrap.IsInstalled}
              Version:                {_detection.Bloxstrap.Version ?? "Unknown"}
              Executable:             {_detection.Bloxstrap.ExecutablePath ?? "Not found"}
              Protocol owner:         {_detection.RobloxProtocol.Owner == "Bloxstrap"}

            LAUNCH BACKEND
              Selected:               {_settings.LaunchBackend}
              Resolved:               {LaunchService.BackendName(resolved)}
              Bootstrap active:       {_launchService.IsLaunching}

            MULTI-ACCOUNT
              Application mutex:      Owned
              ROBLOX singleton mutex: {(_multiAccount.MultiInstanceEnabled ? "Owned" : "Not owned")}
              Cookie file:            {(File.Exists(_multiAccount.CookiePath) ? "Present" : "Missing")}
              Cookie lock:            {(_multiAccount.TeleportProtectionEnabled ? "Owned" : "Not owned")}
              Protection:             {_multiAccount.Health}

            WINDOWS
              Application:            {typeof(MainWindow).Assembly.GetName().Version}
              OS:                     {Environment.OSVersion.VersionString}
              .NET:                   {Environment.Version}
              Monitors:               {WindowManager.GetMonitors().Count}
            """;
    }

    private static string Health(bool healthy) => healthy ? "Healthy" : "Broken";
    private System.Windows.Media.Brush ResourceBrush(string key) => (System.Windows.Media.Brush)FindResource(key);

    private void SetBusy(bool busy)
    {
        LaunchButton.IsEnabled = !busy;
        LaunchAnotherButton.IsEnabled = !busy && _multiAccount.TeleportProtectionEnabled;
        MultiToggle.IsEnabled = !busy;
    }

    private void SetToggle(bool enabled)
    {
        _updatingControls = true;
        MultiToggle.IsChecked = enabled;
        _updatingControls = false;
    }

    private void AddActivity(string message, bool error = false)
    {
        _activity.Insert(0, new RecentActivity(DateTime.Now, message, error));
        while (_activity.Count > 12) _activity.RemoveAt(_activity.Count - 1);
        if (error) _logger.Warning(message); else _logger.Info(message);
    }

    private void InitializeTray()
    {
        if (_testMode) return;
        _trayIcon = new System.Windows.Forms.NotifyIcon
        {
            Icon = System.Drawing.SystemIcons.Application,
            Text = "Roblox Multi-Account Launcher - Normal",
            Visible = true
        };
        var menu = new System.Windows.Forms.ContextMenuStrip();
        menu.Items.Add("Open Launcher", null, (_, _) => Dispatcher.BeginInvoke(ActivateFromSecondInstance));
        menu.Items.Add("Launch Roblox", null, (_, _) => Dispatcher.BeginInvoke(() => LaunchButton_Click(LaunchButton, new RoutedEventArgs())));
        menu.Items.Add("Launch Another Client", null, (_, _) => Dispatcher.BeginInvoke(() => LaunchButton_Click(LaunchAnotherButton, new RoutedEventArgs())));
        menu.Items.Add("Tile Clients", null, (_, _) => Dispatcher.BeginInvoke(() => TileClients_Click(this, new RoutedEventArgs())));
        menu.Items.Add("Focus Client 1", null, (_, _) => Dispatcher.BeginInvoke(() => FocusClient(1)));
        menu.Items.Add("Focus Client 2", null, (_, _) => Dispatcher.BeginInvoke(() => FocusClient(2)));
        menu.Items.Add("Close Roblox", null, (_, _) => Dispatcher.BeginInvoke(() => CloseAll_Click(this, new RoutedEventArgs())));
        menu.Items.Add("Disable Multi-Account Mode", null, (_, _) => Dispatcher.BeginInvoke(() => _ = DisableMultiAccountAsync(true)));
        menu.Items.Add(new System.Windows.Forms.ToolStripSeparator());
        menu.Items.Add("Exit", null, (_, _) => Dispatcher.BeginInvoke(() => { _exitRequested = true; Show(); Close(); }));
        _trayIcon.ContextMenuStrip = menu;
        _trayIcon.DoubleClick += (_, _) => Dispatcher.BeginInvoke(ActivateFromSecondInstance);
    }

    private void UpdateTray()
    {
        if (_trayIcon is null) return;
        _trayIcon.Text = _multiAccount.Health switch
        {
            ProtectionHealth.Enabled => "Roblox Multi-Account Launcher - Active",
            ProtectionHealth.Warning => "Roblox Multi-Account Launcher - Warning",
            _ => "Roblox Multi-Account Launcher - Normal"
        };
    }

    private async void Window_Closing(object? sender, CancelEventArgs e)
    {
        if (_allowClose) { DisposeResources(); return; }

        if (_multiAccount.MultiInstanceEnabled)
        {
            var clients = _processService.GetClients();
            if (clients.Count > 0)
            {
                var choice = ChoiceDialog.Show(this, "Exit Launcher?", $"Multi-account protection is active and {clients.Count} Roblox client(s) are running.", "Cancel", "Minimize to Tray", "Exit Anyway", "Close Roblox and Exit");
                if (choice == 1) { e.Cancel = true; Hide(); return; }
                if (choice is not 2 and not 3) { e.Cancel = true; return; }
                e.Cancel = true;
                if (choice == 3 && !await CloseClientsAsync(clients.Select(client => client.ProcessId), false)) return;
                await _multiAccount.DisableAsync();
                _allowClose = true;
                _exitRequested = true;
                Close();
                return;
            }
            else
            {
                e.Cancel = true;
                await _multiAccount.DisableAsync();
                _allowClose = true;
                _exitRequested = true;
                Close();
                return;
            }
        }

        if (!_exitRequested && _settings.MinimizeToTray)
        {
            e.Cancel = true;
            Hide();
            return;
        }

        _allowClose = true;
        DisposeResources();
    }

    private void DisposeResources()
    {
        if (_disposed) return;
        _disposed = true;
        SaveSettings();
        _refreshTimer.Stop();
        _lifetime.Cancel();
        _hotkeys.Dispose();
        if (_trayIcon is not null) { _trayIcon.Visible = false; _trayIcon.Dispose(); _trayIcon = null; }
        _multiAccount.Dispose();
        _lifetime.Dispose();
        _logger.Info("Application shutdown cleanup completed.");
        if (_testRoot is not null)
        {
            var safeRoot = Path.GetFullPath(Path.Combine(Path.GetTempPath(), "RobloxMultiAccountLauncher-Smoke"));
            var fullTestRoot = Path.GetFullPath(_testRoot);
            if (fullTestRoot.StartsWith(safeRoot, StringComparison.OrdinalIgnoreCase) && Directory.Exists(fullTestRoot)) Directory.Delete(fullTestRoot, true);
        }
    }
}

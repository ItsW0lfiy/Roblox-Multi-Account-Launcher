using System.Windows;
using RobloxMultiAccountLauncher.Services;

namespace RobloxMultiAccountLauncher;

public partial class App : System.Windows.Application
{
    private SingleInstanceService? _singleInstance;
    private MainWindow? _mainWindow;

    protected override void OnStartup(StartupEventArgs e)
    {
        base.OnStartup(e);
        ShutdownMode = ShutdownMode.OnExplicitShutdown;
        _singleInstance = new SingleInstanceService("Wolfy_RobloxMultiAccountLauncher");
        if (!_singleInstance.IsPrimary)
        {
            _singleInstance.SignalPrimary();
            MessageBox.Show("Roblox Multi-Account Launcher is already running. The existing window has been requested to open.", "Launcher already running", MessageBoxButton.OK, MessageBoxImage.Information);
            Shutdown();
            return;
        }

        var smokeTest = e.Args.Contains("--smoke-test", StringComparer.OrdinalIgnoreCase);
        _mainWindow = new MainWindow(smokeTest);
        MainWindow = _mainWindow;
        _singleInstance.ActivationRequested += (_, _) => Dispatcher.BeginInvoke(_mainWindow.ActivateFromSecondInstance);
        _singleInstance.StartListening();
        _mainWindow.Show();

        if (smokeTest)
        {
            Dispatcher.BeginInvoke(async () =>
            {
                await Task.Delay(500);
                _mainWindow.AllowImmediateCloseForTest();
                _mainWindow.Close();
                Shutdown(0);
            });
        }
    }

    protected override void OnExit(ExitEventArgs e)
    {
        _singleInstance?.Dispose();
        base.OnExit(e);
    }
}

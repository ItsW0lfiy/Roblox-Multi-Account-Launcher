using System.Text;

namespace RobloxMultiAccountLauncher.Services;

public sealed class AppLogger
{
    private readonly object _sync = new();
    public string LogDirectory { get; }
    public string CurrentLogPath { get; }

    public AppLogger(string? root = null)
    {
        root ??= Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "RobloxMultiAccountLauncher", "Logs");
        LogDirectory = root;
        Directory.CreateDirectory(LogDirectory);
        RotateLogs();
        CurrentLogPath = Path.Combine(LogDirectory, $"launcher-{DateTime.Now:yyyyMMdd}.log");
    }

    public void Info(string message) => Write("INFO", message);
    public void Warning(string message) => Write("WARN", message);
    public void Error(string message) => Write("ERROR", message);

    private void Write(string level, string message)
    {
        var safe = DiagnosticService.Redact(message.ReplaceLineEndings(" "));
        lock (_sync)
        {
            File.AppendAllText(CurrentLogPath, $"{DateTimeOffset.Now:O} [{level}] {safe}{Environment.NewLine}", Encoding.UTF8);
        }
    }

    private void RotateLogs()
    {
        foreach (var file in new DirectoryInfo(LogDirectory).GetFiles("launcher-*.log").OrderByDescending(f => f.LastWriteTimeUtc).Skip(10))
        {
            try { file.Delete(); } catch { }
        }
    }
}

using System.Collections.Concurrent;
using System.IO;
using RobloxMultiAccountLauncher.Models;

namespace RobloxMultiAccountLauncher.Services;

public sealed class MultiAccountService : IDisposable
{
    private readonly BlockingCollection<IWorkItem> _queue = new();
    private readonly Thread _resourceThread;
    private readonly string _mutexName;
    private readonly string _cookiePath;
    private readonly AppLogger? _logger;
    private Mutex? _robloxMutex;
    private FileStream? _cookieLock;
    private volatile bool _mutexOwned;
    private volatile bool _cookieOwned;
    private volatile bool _disposed;

    public MultiAccountService(AppLogger? logger = null, string mutexName = "ROBLOX_singletonMutex", string? cookiePath = null)
    {
        _logger = logger;
        _mutexName = mutexName;
        _cookiePath = cookiePath ?? Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
            "Roblox", "LocalStorage", "RobloxCookies.dat");
        _resourceThread = new Thread(ResourceLoop)
        {
            Name = "Multi-account resource owner",
            IsBackground = true
        };
        _resourceThread.Start();
    }

    public bool MultiInstanceEnabled => _mutexOwned;
    public bool TeleportProtectionEnabled => _cookieOwned;
    public ProtectionHealth Health => !_mutexOwned ? ProtectionHealth.Disabled : _cookieOwned ? ProtectionHealth.Enabled : ProtectionHealth.Warning;
    public string CookiePath => _cookiePath;

    public Task<MultiAccountResult> EnableAsync(TimeSpan timeout, CancellationToken cancellationToken = default) =>
        InvokeAsync(() => EnableCore(timeout, cancellationToken));

    public Task DisableAsync() => InvokeAsync(() =>
    {
        ReleaseCore();
        return true;
    });

    private MultiAccountResult EnableCore(TimeSpan timeout, CancellationToken cancellationToken)
    {
        if (_mutexOwned)
        {
            return new MultiAccountResult(true, _cookieOwned, _cookieOwned ? "Multi-account protection is already enabled." : "Multi-instance is enabled, but teleport protection is unavailable.");
        }

        var candidate = new Mutex(false, _mutexName);
        var deadline = DateTime.UtcNow + timeout;
        var acquired = false;
        try
        {
            do
            {
                cancellationToken.ThrowIfCancellationRequested();
                try { acquired = candidate.WaitOne(0); }
                catch (AbandonedMutexException) { acquired = true; }
                if (acquired) break;
                Thread.Sleep(200);
            }
            while (DateTime.UtcNow < deadline);

            if (!acquired)
            {
                candidate.Dispose();
                return new MultiAccountResult(false, false, "ROBLOX_singletonMutex is already owned. Another Roblox client or multi-instance utility may be active.");
            }

            _robloxMutex = candidate;
            _mutexOwned = true;
            _logger?.Info("Acquired ROBLOX_singletonMutex.");

            if (!File.Exists(_cookiePath))
            {
                return new MultiAccountResult(true, false, "RobloxCookies.dat was not found. Multiple clients can launch, but multi-client teleporting may fail.");
            }

            try
            {
                // Known-good behavior: hold the file read-only and exclusively. Never read its contents.
                _cookieLock = File.Open(_cookiePath, FileMode.Open, FileAccess.Read, FileShare.None);
                _cookieOwned = true;
                _logger?.Info("Acquired exclusive RobloxCookies.dat teleport-protection lock.");
                return new MultiAccountResult(true, true, "Multi-account mode is ready.");
            }
            catch (UnauthorizedAccessException ex)
            {
                return new MultiAccountResult(true, false, $"Access to RobloxCookies.dat was denied: {ex.Message}");
            }
            catch (IOException ex)
            {
                return new MultiAccountResult(true, false, $"RobloxCookies.dat could not be locked exclusively: {ex.Message}");
            }
        }
        catch
        {
            if (acquired) ReleaseCore();
            else candidate.Dispose();
            throw;
        }
    }

    private void ReleaseCore()
    {
        if (_cookieLock is not null)
        {
            try { _cookieLock.Dispose(); _logger?.Info("Released RobloxCookies.dat lock."); }
            finally { _cookieLock = null; _cookieOwned = false; }
        }

        if (_robloxMutex is not null)
        {
            if (_mutexOwned)
            {
                try { _robloxMutex.ReleaseMutex(); _logger?.Info("Released ROBLOX_singletonMutex."); }
                finally { _mutexOwned = false; }
            }
            _robloxMutex.Dispose();
            _robloxMutex = null;
        }
    }

    private Task<T> InvokeAsync<T>(Func<T> action)
    {
        if (_disposed) return Task.FromException<T>(new ObjectDisposedException(nameof(MultiAccountService)));
        var item = new WorkItem<T>(action);
        _queue.Add(item);
        return item.Task;
    }

    private void ResourceLoop()
    {
        foreach (var work in _queue.GetConsumingEnumerable()) work.Execute();
    }

    public void Dispose()
    {
        if (_disposed) return;
        try { DisableAsync().GetAwaiter().GetResult(); } catch { }
        _disposed = true;
        _queue.CompleteAdding();
        _resourceThread.Join(2000);
        _queue.Dispose();
    }

    private interface IWorkItem { void Execute(); }

    private sealed class WorkItem<T>(Func<T> action) : IWorkItem
    {
        private readonly TaskCompletionSource<T> _source = new(TaskCreationOptions.RunContinuationsAsynchronously);
        public Task<T> Task => _source.Task;
        public void Execute()
        {
            try { _source.SetResult(action()); }
            catch (OperationCanceledException ex) { _source.SetCanceled(ex.CancellationToken); }
            catch (Exception ex) { _source.SetException(ex); }
        }
    }
}

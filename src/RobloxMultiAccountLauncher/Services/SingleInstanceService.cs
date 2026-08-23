namespace RobloxMultiAccountLauncher.Services;

public sealed class SingleInstanceService : IDisposable
{
    private readonly Mutex _mutex;
    private readonly EventWaitHandle _activationEvent;
    private readonly CancellationTokenSource _cancellation = new();
    private Task? _listener;
    private bool _ownsMutex;

    public SingleInstanceService(string name)
    {
        _mutex = new Mutex(false, name);
        _activationEvent = new EventWaitHandle(false, EventResetMode.AutoReset, $"{name}_Activate");
        try
        {
            _ownsMutex = _mutex.WaitOne(0);
        }
        catch (AbandonedMutexException)
        {
            _ownsMutex = true;
        }
    }

    public bool IsPrimary => _ownsMutex;
    public event EventHandler? ActivationRequested;

    public void SignalPrimary() => _activationEvent.Set();

    public void StartListening()
    {
        if (!IsPrimary || _listener is not null) return;
        _listener = Task.Run(() =>
        {
            var waits = new WaitHandle[] { _activationEvent, _cancellation.Token.WaitHandle };
            while (WaitHandle.WaitAny(waits) == 0)
            {
                ActivationRequested?.Invoke(this, EventArgs.Empty);
            }
        });
    }

    public void Dispose()
    {
        _cancellation.Cancel();
        _activationEvent.Set();
        try { _listener?.Wait(1000); } catch { }
        if (_ownsMutex)
        {
            try { _mutex.ReleaseMutex(); } catch { }
            _ownsMutex = false;
        }
        _activationEvent.Dispose();
        _mutex.Dispose();
        _cancellation.Dispose();
    }
}

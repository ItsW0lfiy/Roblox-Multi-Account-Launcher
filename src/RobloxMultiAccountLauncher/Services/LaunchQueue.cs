namespace RobloxMultiAccountLauncher.Services;

public sealed class LaunchQueue
{
    private readonly SemaphoreSlim _gate = new(1, 1);
    public bool IsBusy { get; private set; }

    public async Task<(bool Accepted, T? Result)> TryRunAsync<T>(Func<CancellationToken, Task<T>> operation, CancellationToken cancellationToken)
    {
        if (!await _gate.WaitAsync(0, cancellationToken).ConfigureAwait(false)) return (false, default);
        IsBusy = true;
        try { return (true, await operation(cancellationToken).ConfigureAwait(false)); }
        finally { IsBusy = false; _gate.Release(); }
    }
}

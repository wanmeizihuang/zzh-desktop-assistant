namespace ZZH.SensorService;

public sealed class SensorSnapshotCache : IAsyncDisposable
{
    private readonly HardwareSampler _sampler;
    private readonly CancellationTokenSource _cancellation = new();
    private readonly Task _worker;
    private SensorServiceSnapshot _current = SensorServiceSnapshot.Failure("warming_up");

    public SensorSnapshotCache(HardwareSampler sampler)
    {
        _sampler = sampler;
        _worker = Task.Run(UpdateLoopAsync);
    }

    public SensorServiceSnapshot Current => Volatile.Read(ref _current);

    public async ValueTask DisposeAsync()
    {
        await _cancellation.CancelAsync().ConfigureAwait(false);
        try
        {
            await _worker.ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
        }

        _cancellation.Dispose();
        _sampler.Dispose();
    }

    private async Task UpdateLoopAsync()
    {
        using var timer = new PeriodicTimer(TimeSpan.FromSeconds(2));
        while (!_cancellation.IsCancellationRequested)
        {
            SensorServiceSnapshot snapshot;
            try
            {
                snapshot = _sampler.Sample();
            }
            catch
            {
                snapshot = SensorServiceSnapshot.Failure("sensor_update_failed");
            }
            Volatile.Write(ref _current, snapshot);

            if (!await timer.WaitForNextTickAsync(_cancellation.Token).ConfigureAwait(false))
            {
                break;
            }
        }
    }
}

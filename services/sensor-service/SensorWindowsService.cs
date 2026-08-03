using System.ServiceProcess;

namespace ZZH.SensorService;

public sealed class SensorWindowsService : ServiceBase
{
    public const string InstalledServiceName = "ZZHSensorService";

    private CancellationTokenSource? _cancellation;
    private Task? _worker;

    public SensorWindowsService()
    {
        ServiceName = InstalledServiceName;
        CanStop = true;
        AutoLog = true;
    }

    protected override void OnStart(string[] args)
    {
        _cancellation = new CancellationTokenSource();
        _worker = Task.Run(() => RunAsync(_cancellation.Token));
    }

    protected override void OnStop()
    {
        _cancellation?.Cancel();
        try
        {
            _worker?.GetAwaiter().GetResult();
        }
        catch (OperationCanceledException)
        {
        }
        finally
        {
            _cancellation?.Dispose();
            _cancellation = null;
            _worker = null;
        }
    }

    private async Task RunAsync(CancellationToken cancellationToken)
    {
        try
        {
            await using var cache = new SensorSnapshotCache(new HardwareSampler());
            var server = new SnapshotPipeServer(SensorProtocol.PipeName, () => cache.Current);
            await server.RunAsync(RequestStop, cancellationToken).ConfigureAwait(false);
        }
        catch when (!cancellationToken.IsCancellationRequested)
        {
            RequestStop();
        }
    }

    private void RequestStop()
    {
        ThreadPool.QueueUserWorkItem(static service => ((SensorWindowsService)service!).Stop(), this);
    }
}

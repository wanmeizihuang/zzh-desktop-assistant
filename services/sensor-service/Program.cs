using System.ServiceProcess;
using System.Text.Json;

namespace ZZH.SensorService;

internal static class Program
{
    private static async Task<int> Main(string[] args)
    {
        if (args.Contains("--once", StringComparer.Ordinal))
        {
            try
            {
                using var sampler = new HardwareSampler();
                Console.WriteLine(JsonSerializer.Serialize(sampler.Sample(), SensorProtocol.JsonOptions));
                return 0;
            }
            catch
            {
                Console.WriteLine(JsonSerializer.Serialize(
                    SensorServiceSnapshot.Failure("sensor_initialization_failed"),
                    SensorProtocol.JsonOptions));
                return 2;
            }
        }

        if (args.Contains("--console", StringComparer.Ordinal))
        {
            using var cancellation = new CancellationTokenSource();
            Console.CancelKeyPress += (_, eventArgs) =>
            {
                eventArgs.Cancel = true;
                cancellation.Cancel();
            };

            await using var cache = new SensorSnapshotCache(new HardwareSampler());
            var server = new SnapshotPipeServer(SensorProtocol.PipeName, () => cache.Current);
            await server.RunAsync(cancellation.Cancel, cancellation.Token).ConfigureAwait(false);
            return 0;
        }

        ServiceBase.Run(new SensorWindowsService());
        return 0;
    }
}

using System.Buffers.Binary;
using System.IO.Pipes;
using System.Diagnostics;
using System.Text.Json;
using ZZH.SensorService;
using Xunit;

namespace ZZH.SensorService.Tests;

public sealed class PipeProtocolTests
{
    [Fact]
    public async Task PipeSendsOneBoundedVersionedSnapshotWithoutClientPayload()
    {
        string pipeName = $"ZZHSensorService.Test.{Guid.NewGuid():N}";
        var snapshot = new SensorServiceSnapshot(
            SensorProtocol.Version,
            123,
            new TemperatureReading(42, "CPU Package", "LibreHardwareMonitor"),
            Array.Empty<GpuTemperatureReading>(),
            null,
            null);
        using var cancellation = new CancellationTokenSource(TimeSpan.FromSeconds(5));
        var server = new SnapshotPipeServer(pipeName, () => snapshot, TimeSpan.FromSeconds(5));
        Task serverTask = server.RunAsync(cancellation.Cancel, cancellation.Token);

        await using var client = new NamedPipeClientStream(
            ".",
            pipeName,
            PipeDirection.In,
            PipeOptions.Asynchronous);
        await client.ConnectAsync(2_000, cancellation.Token);
        byte[] length = new byte[sizeof(int)];
        await client.ReadExactlyAsync(length, cancellation.Token);
        int payloadLength = BinaryPrimitives.ReadInt32LittleEndian(length);
        Assert.InRange(payloadLength, 1, SensorProtocol.MaximumSnapshotBytes);
        byte[] payload = new byte[payloadLength];
        await client.ReadExactlyAsync(payload, cancellation.Token);

        SensorServiceSnapshot? decoded = JsonSerializer.Deserialize<SensorServiceSnapshot>(
            payload,
            SensorProtocol.JsonOptions);
        Assert.NotNull(decoded);
        Assert.Equal(SensorProtocol.Version, decoded.Version);
        Assert.Equal(42, decoded.Cpu!.Celsius);

        cancellation.Cancel();
        await Assert.ThrowsAnyAsync<OperationCanceledException>(() => serverTask);
    }

    [Fact]
    public async Task PipeStopsAfterConfiguredIdleTimeout()
    {
        string pipeName = $"ZZHSensorService.Test.{Guid.NewGuid():N}";
        var idle = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var server = new SnapshotPipeServer(
            pipeName,
            () => SensorServiceSnapshot.Failure("test"),
            TimeSpan.FromMilliseconds(50));

        var elapsed = Stopwatch.StartNew();
        await server.RunAsync(() => idle.SetResult(), CancellationToken.None);

        Assert.True(idle.Task.IsCompletedSuccessfully);
        Assert.InRange(elapsed.Elapsed, TimeSpan.FromMilliseconds(25), TimeSpan.FromMilliseconds(750));
    }

    [Theory]
    [InlineData(PawnIoDependency.NotInstalledError)]
    [InlineData(PawnIoDependency.DeviceUnavailableError)]
    public void DependencyFailuresRemainMachineReadable(string error)
    {
        string json = JsonSerializer.Serialize(
            SensorServiceSnapshot.Failure(error),
            SensorProtocol.JsonOptions);

        using JsonDocument document = JsonDocument.Parse(json);
        Assert.Equal(error, document.RootElement.GetProperty("service_error").GetString());
        Assert.False(document.RootElement.TryGetProperty("cpu", out _));
    }
}

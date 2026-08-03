using System.Text.Json;
using System.Text.Json.Serialization;

namespace ZZH.SensorService;

public static class SensorProtocol
{
    public const int Version = 1;
    public const int MaximumSnapshotBytes = 16 * 1024;
    public const int MaximumGpuEntries = 8;
    public const string PipeName = "ZZHDesktopAssistant.Sensor.v1";

    public static readonly JsonSerializerOptions JsonOptions = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower,
        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
    };
}

public sealed record TemperatureReading(
    float Celsius,
    string Sensor,
    string Source);

public sealed record GpuTemperatureReading(
    uint VendorId,
    float Celsius,
    string Sensor,
    string Source);

public sealed record SensorServiceSnapshot(
    int Version,
    long GeneratedAtUnixMs,
    TemperatureReading? Cpu,
    IReadOnlyList<GpuTemperatureReading> Gpus,
    string? CpuError,
    string? ServiceError)
{
    public static SensorServiceSnapshot Failure(string error) => new(
        SensorProtocol.Version,
        DateTimeOffset.UtcNow.ToUnixTimeMilliseconds(),
        null,
        Array.Empty<GpuTemperatureReading>(),
        null,
        error);
}

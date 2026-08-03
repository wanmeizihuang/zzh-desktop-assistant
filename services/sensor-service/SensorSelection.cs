namespace ZZH.SensorService;

public enum SensorHardwareKind
{
    Cpu,
    GpuNvidia,
    GpuAmd,
    GpuIntel,
    Other,
}

public sealed record TemperatureCandidate(
    SensorHardwareKind HardwareKind,
    string Name,
    float? Celsius);

public static class SensorSelection
{
    public const uint IntelVendorId = 0x8086;
    public const uint NvidiaVendorId = 0x10de;
    public const uint AmdVendorId = 0x1002;

    public static TemperatureReading? SelectCpu(IEnumerable<TemperatureCandidate> candidates)
    {
        return candidates
            .Where(candidate => candidate.HardwareKind == SensorHardwareKind.Cpu)
            .Select(candidate => (Candidate: candidate, Priority: CpuPriority(candidate.Name)))
            .Where(entry => entry.Priority is not null && IsValid(entry.Candidate.Celsius))
            .OrderBy(entry => entry.Priority)
            .ThenByDescending(entry => entry.Candidate.Celsius)
            .Select(entry => new TemperatureReading(
                entry.Candidate.Celsius!.Value,
                NormalizeCpuSensor(entry.Candidate.Name),
                "LibreHardwareMonitor"))
            .FirstOrDefault();
    }

    public static IReadOnlyList<GpuTemperatureReading> SelectGpus(
        IEnumerable<TemperatureCandidate> candidates)
    {
        return candidates
            .Where(candidate => VendorId(candidate.HardwareKind) is not null)
            .Select(candidate => (Candidate: candidate, Priority: GpuPriority(candidate.Name)))
            .Where(entry => entry.Priority is not null && IsValid(entry.Candidate.Celsius))
            .GroupBy(entry => VendorId(entry.Candidate.HardwareKind)!.Value)
            .Select(group => group
                .OrderBy(entry => entry.Priority)
                .ThenByDescending(entry => entry.Candidate.Celsius)
                .First())
            .OrderBy(entry => entry.Candidate.HardwareKind)
            .Take(SensorProtocol.MaximumGpuEntries)
            .Select(entry => new GpuTemperatureReading(
                VendorId(entry.Candidate.HardwareKind)!.Value,
                entry.Candidate.Celsius!.Value,
                NormalizeGpuSensor(entry.Candidate.Name),
                "LibreHardwareMonitor"))
            .ToArray();
    }

    private static int? CpuPriority(string name)
    {
        string normalized = Normalize(name);
        return normalized switch
        {
            "cpu package" => 0,
            "core (tctl/tdie)" => 1,
            "cpu (tctl/tdie)" => 1,
            "tctl/tdie" => 1,
            _ => null,
        };
    }

    private static int? GpuPriority(string name)
    {
        string normalized = Normalize(name);
        if (normalized.Contains("hot spot", StringComparison.Ordinal)
            || normalized.Contains("hotspot", StringComparison.Ordinal)
            || normalized.Contains("junction", StringComparison.Ordinal)
            || normalized.Contains("memory", StringComparison.Ordinal)
            || normalized.Contains("vram", StringComparison.Ordinal))
        {
            return null;
        }

        return normalized switch
        {
            "gpu core" => 0,
            "gpu edge" => 1,
            _ => null,
        };
    }

    private static uint? VendorId(SensorHardwareKind kind) => kind switch
    {
        SensorHardwareKind.GpuIntel => IntelVendorId,
        SensorHardwareKind.GpuNvidia => NvidiaVendorId,
        SensorHardwareKind.GpuAmd => AmdVendorId,
        _ => null,
    };

    private static bool IsValid(float? value) =>
        value is >= 0 and <= 150 && float.IsFinite(value.Value);

    private static string Normalize(string value) =>
        string.Join(' ', value.Trim().ToLowerInvariant().Split(
            (char[]?)null,
            StringSplitOptions.RemoveEmptyEntries));

    private static string NormalizeCpuSensor(string name) =>
        Normalize(name).Contains("tctl/tdie", StringComparison.Ordinal)
            ? "Tctl/Tdie"
            : "CPU Package";

    private static string NormalizeGpuSensor(string name) =>
        Normalize(name) == "gpu edge" ? "GPU Edge" : "GPU Core";
}

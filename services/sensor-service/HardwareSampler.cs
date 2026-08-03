using LibreHardwareMonitor.Hardware;

namespace ZZH.SensorService;

public sealed class HardwareSampler : IDisposable
{
    private readonly Computer? _computer;
    private readonly string? _initializationError;
    private readonly object _gate = new();

    public HardwareSampler()
    {
        _initializationError = PawnIoDependency.Probe();
        if (_initializationError is not null)
        {
            return;
        }

        var computer = new Computer
        {
            IsCpuEnabled = true,
            IsGpuEnabled = true,
        };
        try
        {
            computer.Open();
            _computer = computer;
        }
        catch
        {
            computer.Close();
            _initializationError = "sensor_initialization_failed";
        }
    }

    public SensorServiceSnapshot Sample()
    {
        lock (_gate)
        {
            if (_initializationError is not null || _computer is null)
            {
                return SensorServiceSnapshot.Failure(
                    _initializationError ?? "sensor_initialization_failed");
            }

            var candidates = new List<TemperatureCandidate>();
            foreach (IHardware hardware in _computer.Hardware)
            {
                Collect(hardware, candidates);
            }

            TemperatureReading? cpu = SensorSelection.SelectCpu(candidates);
            IReadOnlyList<GpuTemperatureReading> gpus = SensorSelection.SelectGpus(candidates);
            return new SensorServiceSnapshot(
                SensorProtocol.Version,
                DateTimeOffset.UtcNow.ToUnixTimeMilliseconds(),
                cpu,
                gpus,
                cpu is null ? "cpu_package_unavailable" : null,
                null);
        }
    }

    public void Dispose()
    {
        lock (_gate)
        {
            _computer?.Close();
        }
    }

    private static void Collect(IHardware hardware, ICollection<TemperatureCandidate> output)
    {
        hardware.Update();
        SensorHardwareKind kind = HardwareKind(hardware.HardwareType);
        foreach (ISensor sensor in hardware.Sensors)
        {
            if (sensor.SensorType == SensorType.Temperature)
            {
                output.Add(new TemperatureCandidate(kind, sensor.Name, sensor.Value));
            }
        }

        foreach (IHardware subHardware in hardware.SubHardware)
        {
            Collect(subHardware, output);
        }
    }

    private static SensorHardwareKind HardwareKind(HardwareType type) => type switch
    {
        HardwareType.Cpu => SensorHardwareKind.Cpu,
        HardwareType.GpuNvidia => SensorHardwareKind.GpuNvidia,
        HardwareType.GpuAmd => SensorHardwareKind.GpuAmd,
        HardwareType.GpuIntel => SensorHardwareKind.GpuIntel,
        _ => SensorHardwareKind.Other,
    };
}

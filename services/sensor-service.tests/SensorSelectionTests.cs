using ZZH.SensorService;
using Xunit;

namespace ZZH.SensorService.Tests;

public sealed class SensorSelectionTests
{
    [Fact]
    public void CpuPrefersPackageAndRejectsPerCoreAndBoardTemperatures()
    {
        TemperatureReading? reading = SensorSelection.SelectCpu(new[]
        {
            new TemperatureCandidate(SensorHardwareKind.Cpu, "CPU Core #1", 91),
            new TemperatureCandidate(SensorHardwareKind.Other, "CPU Package", 88),
            new TemperatureCandidate(SensorHardwareKind.Cpu, "CPU Package", 62.5f),
            new TemperatureCandidate(SensorHardwareKind.Cpu, "Core (Tctl/Tdie)", 70),
        });

        Assert.NotNull(reading);
        Assert.Equal(62.5f, reading.Celsius);
        Assert.Equal("CPU Package", reading.Sensor);
    }

    [Fact]
    public void CpuAcceptsAmdTctlTdieWhenPackageIsAbsent()
    {
        TemperatureReading? reading = SensorSelection.SelectCpu(new[]
        {
            new TemperatureCandidate(SensorHardwareKind.Cpu, "Core (Tctl/Tdie)", 71.25f),
        });

        Assert.NotNull(reading);
        Assert.Equal("Tctl/Tdie", reading.Sensor);
    }

    [Fact]
    public void GpuAcceptsOnlyCoreOrEdgeAndNeverHotSpot()
    {
        IReadOnlyList<GpuTemperatureReading> readings = SensorSelection.SelectGpus(new[]
        {
            new TemperatureCandidate(SensorHardwareKind.GpuAmd, "GPU Hot Spot", 96),
            new TemperatureCandidate(SensorHardwareKind.GpuAmd, "GPU Edge", 64),
            new TemperatureCandidate(SensorHardwareKind.GpuNvidia, "GPU Core", 58),
            new TemperatureCandidate(SensorHardwareKind.GpuIntel, "GPU Memory Junction", 82),
        });

        Assert.Collection(
            readings,
            nvidia =>
            {
                Assert.Equal(SensorSelection.NvidiaVendorId, nvidia.VendorId);
                Assert.Equal(58, nvidia.Celsius);
                Assert.Equal("GPU Core", nvidia.Sensor);
            },
            amd =>
            {
                Assert.Equal(SensorSelection.AmdVendorId, amd.VendorId);
                Assert.Equal(64, amd.Celsius);
                Assert.Equal("GPU Edge", amd.Sensor);
            });
    }

    [Fact]
    public void InvalidTemperaturesAreRejected()
    {
        Assert.Null(SensorSelection.SelectCpu(new[]
        {
            new TemperatureCandidate(SensorHardwareKind.Cpu, "CPU Package", float.NaN),
            new TemperatureCandidate(SensorHardwareKind.Cpu, "CPU Package", 151),
        }));
    }

    [Fact]
    public void GenericTemperatureAliasesAreRejected()
    {
        Assert.Null(SensorSelection.SelectCpu(new[]
        {
            new TemperatureCandidate(SensorHardwareKind.Cpu, "Package", 55),
        }));

        Assert.Empty(SensorSelection.SelectGpus(new[]
        {
            new TemperatureCandidate(SensorHardwareKind.GpuNvidia, "GPU Temperature", 60),
        }));
    }
}

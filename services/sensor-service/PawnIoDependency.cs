using Microsoft.Win32;
using Microsoft.Win32.SafeHandles;
using System.Runtime.InteropServices;

namespace ZZH.SensorService;

public static class PawnIoDependency
{
    public const string NotInstalledError = "pawnio_not_installed";
    public const string DeviceUnavailableError = "pawnio_device_unavailable";

    private const string ServiceRegistryPath = @"SYSTEM\CurrentControlSet\Services\PawnIO";
    private const string DevicePath = @"\\?\GLOBALROOT\Device\PawnIO";
    private const uint GenericReadWrite = 0xC0000000;

    public static string? Probe()
    {
        using RegistryKey? service = Registry.LocalMachine.OpenSubKey(ServiceRegistryPath);
        if (service is null)
        {
            return NotInstalledError;
        }

        using SafeFileHandle device = CreateFile(
            DevicePath,
            GenericReadWrite,
            FileShare.ReadWrite,
            IntPtr.Zero,
            FileMode.Open,
            FileAttributes.Normal,
            IntPtr.Zero);
        return device.IsInvalid ? DeviceUnavailableError : null;
    }

    [DllImport("kernel32.dll", EntryPoint = "CreateFileW", SetLastError = true,
        CharSet = CharSet.Unicode)]
    private static extern SafeFileHandle CreateFile(
        string fileName,
        uint desiredAccess,
        FileShare shareMode,
        IntPtr securityAttributes,
        FileMode creationDisposition,
        FileAttributes flagsAndAttributes,
        IntPtr templateFile);
}

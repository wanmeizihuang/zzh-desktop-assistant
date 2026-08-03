# Accurate Temperature Engine - Final Report

Date: 2026-08-02

## Outcome

- Removed the Windows PDH ACPI thermal-zone path and Kelvin conversion entirely.
- CPU accepts only `CPU Package` or AMD `Tctl/Tdie` from LibreHardwareMonitor 0.9.6.
- GPU accepts only `GPU Core` or `GPU Edge`; Hot Spot, Junction, memory and global sensors are rejected.
- A dedicated GPU is selected when present; an integrated GPU is selected only when no supported dedicated GPU exists.
- Intel Level Zero and NVIDIA NVML are preferred native GPU Core/Edge providers. LHM is a same-vendor fallback; AMD uses LHM.
- UI source text now identifies the sensor and provider, such as `CPU Package · LHM` or `GPU Core · Level Zero`.

## Service And IPC

- `ZZHSensorService` is a .NET 8 x64 self-contained single-file Windows service.
- MSI installs the service as `LocalSystem` with manual start.
- Built-in Users receive only service query-status (`0x04`) and start (`0x10`) rights.
- The desktop client starts the service through SCM only when the pipe is unavailable.
- The pipe is outbound-only and sends one little-endian length-prefixed JSON snapshot.
- Snapshot size is limited to 16 KiB, GPU entries to eight, protocol version to 1 and temperatures to finite `0..=150` Celsius.
- Pipe ACL grants SYSTEM full control and built-in Users read access. The service accepts no client payload or command.
- The service stops after 30 seconds without a successful client connection.

## Verification

- `cargo fmt --all -- --check`: passed.
- `cargo clippy --workspace --all-targets --offline -- -D warnings`: passed.
- `cargo test --workspace --offline`: 161 passed, 2 ignored system-state tests, 0 failed.
- `dotnet test services/sensor-service.tests/ZZHSensorService.Tests.csproj --configuration Release --no-restore`: 7 passed, 0 failed.
- WiX 4.0.6 compile/link: 0 warnings, 0 errors.
- MSI decompile confirmed `ServiceInstall`, `ServiceControl` and `Wix4SecureObject` permission `0x14`.
- `git diff --check`: passed; line-ending notices only.
- Local non-UI probe reported `sensor_service_status: NotInstalled` and no CPU/GPU temperature, proving there is no ACPI fallback before installation. Other metrics remained available.

Standard MSI ICE validation could not run because Windows Installer service access is unavailable in the build environment. The WiX project suppresses that post-link validation; installation and uninstall behavior therefore require the manual checks below.

## Deliverables

- `desktop-assistant.exe`
  - Size: 18,997,248 bytes
  - SHA-256: `AB691F2743DF89F557A2C64C773C7E0FCCC51E45091EA814E6C8E1718B3A3084`
- `ZZHDesktopAssistant-Setup.msi`
  - Size: 43,057,152 bytes
  - SHA-256: `7AA4F438A54C33329D2703DF6C20F7267C617C8AFD01C0D46595F64768BE927A`
- Published `ZZHSensorService.exe` used in the MSI
  - SHA-256: `DE1CCCAF1370B81DBAD409E0B40C4C412C70857A43F0CF7FDD95A2627D10ABBF`

## Manual Verification

1. Run `ZZHDesktopAssistant-Setup.msi` and approve the single UAC prompt.
2. Confirm `ZZHSensorService` exists and its startup type is Manual in `services.msc`.
3. Run `C:\Program Files\ZZH Desktop Assistant\desktop-assistant.exe` and expand the monitor panel.
4. Confirm CPU temperature shows `CPU Package · LHM` or `Tctl/Tdie · LHM`; it must never show ACPI.
5. Confirm GPU temperature shows only `GPU Core` or `GPU Edge`. On the current integrated-Intel machine it may show `GPU Core · Level Zero` or same-vendor LHM fallback.
6. Compare the same sensor name against HWiNFO or Libre Hardware Monitor under idle and load. Compare at the same instant; short sampling offsets are expected.
7. Exit the assistant, wait at least 35 seconds, and confirm `ZZHSensorService` is Stopped.
8. Uninstall ZZH Desktop Assistant, then confirm the service and `C:\Program Files\ZZH Desktop Assistant` are removed.

The UI and installer checks above were intentionally not automated, per the project verification policy.

# Accurate Temperature Engine Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace ACPI temperature data with an install-once sensor service that reports real CPU Package and target-GPU Core/Edge temperatures.

**Architecture:** A self-contained .NET 8 Windows service uses LibreHardwareMonitorLib 0.9.6 and exposes bounded read-only snapshots over a local named pipe. The Rust monitor starts and consumes the service without elevation, validates all data, retains native GPU providers where available, and never falls back to ACPI or Hot Spot sensors.

**Tech Stack:** Rust 1.97, windows-rs, serde, .NET 8/C#, LibreHardwareMonitorLib 0.9.6, named pipes, WiX Toolset 4.0.6, MSI.

---

### Task 1: Freeze protocol and sensor-selection contract

**Files:**
- Create: `services/sensor-service/Protocol.cs`
- Create: `services/sensor-service/SensorSelection.cs`
- Create: `services/sensor-service.tests/SensorSelectionTests.cs`
- Modify: `crates/system-monitor/src/lib.rs`

1. Write C# tests for CPU Package/Tctl-Tdie allowlisting and rejection of per-core/board sensors.
2. Write C# tests accepting GPU Core/Edge while rejecting Hot Spot, Junction and memory temperatures.
3. Add the version-1 snapshot DTO with bounded CPU and GPU entries.
4. Add Rust tests for protocol version, 16 KiB size, finite 0..=150 Celsius values and known vendor IDs.
5. Run `dotnet test services/sensor-service.tests` and `cargo test -p system-monitor sensor_service --offline`.

### Task 2: Implement the LibreHardwareMonitor sampler

**Files:**
- Create: `services/sensor-service/ZZHSensorService.csproj`
- Create: `services/sensor-service/HardwareSampler.cs`
- Create: `services/sensor-service/Program.cs`
- Create: `services/sensor-service.tests/ZZHSensorService.Tests.csproj`

1. Reference exact `LibreHardwareMonitorLib` version `0.9.6`.
2. Enable only CPU and GPU hardware; update hardware/subhardware every two seconds.
3. Select CPU Package/Tctl-Tdie and GPU Core/Edge using the tested policy.
4. Return explicit unavailable reasons without substituting ACPI or Hot Spot.
5. Add `--console --once` mode for non-service integration tests and diagnostics.
6. Run service unit tests and a local one-shot probe.

### Task 3: Implement the read-only named-pipe service

**Files:**
- Create: `services/sensor-service/SensorWindowsService.cs`
- Create: `services/sensor-service/SnapshotPipeServer.cs`
- Create: `services/sensor-service.tests/PipeProtocolTests.cs`

1. Create an outbound-only pipe with SYSTEM/full and built-in-users/read ACLs.
2. Send one length-prefixed UTF-8 JSON snapshot per connection and accept no client payload.
3. Bound JSON to 16 KiB and GPU entries to eight.
4. Stop the service after 30 seconds without a successful client connection.
5. Test connection, malformed internal snapshots, cancellation and idle shutdown with an in-process test pipe.

### Task 4: Replace ACPI with the Rust service client

**Files:**
- Modify: `Cargo.toml`
- Modify: `crates/system-monitor/Cargo.toml`
- Modify: `crates/system-monitor/src/lib.rs`
- Modify: `apps/desktop/src/main.rs`
- Modify: `apps/desktop/ui/app-window.slint`

1. Remove `PdhCpuTemperature`, Kelvin conversion and all ACPI UI strings/tests.
2. Add a bounded named-pipe client with 250 ms connect/read limits.
3. Start the installed service through SCM when required; distinguish not-installed, starting and temporarily unavailable states.
4. Match GPU service readings to the already selected target vendor and class.
5. Carry a fixed temperature source enum into UI formatting.
6. Run focused Rust tests and verify all non-temperature metrics still sample independently.

### Task 5: Add native GPU provider coverage

**Files:**
- Modify: `crates/system-monitor/src/lib.rs`

1. Retain and test Intel Level Zero Core/Board ordering without accepting unrelated global sensors when a core sensor is absent.
2. Add dynamic NVIDIA NVML loading from the trusted system/driver path and read `GPU Core` only.
3. Use the service's LibreHardwareMonitor GPU Core/Edge value as the AMD path and as same-vendor fallback when a native provider is unavailable.
4. Never accept a different vendor or integrated GPU when a discrete target exists.
5. Run provider-selection and malformed-driver-result tests.

### Task 6: Build the MSI and service lifecycle

**Files:**
- Create: `installer/ZZHDesktopAssistant.Installer.wixproj`
- Create: `installer/Package.wxs`
- Create: `installer/ThirdPartyNotices.txt`
- Create: `installer/build-installer.ps1`
- Modify: `README.md`

1. Publish the service as a self-contained x64 single file without trimming.
2. Build the Rust release EXE and stage both binaries plus MPL-2.0 notices.
3. Install `ZZHSensorService` as manual start under LocalSystem.
4. Grant built-in users only service query/start rights and pipe read rights.
5. Stop/delete the service and remove installed files on uninstall/upgrade.
6. Build `ZZHDesktopAssistant-Setup.msi` with pinned WiX 4.0.6.

### Task 7: Security and quality gate

**Files:**
- Create: `docs/reports/2026-08-02-accurate-temperature-engine.md`
- Replace: `desktop-assistant.exe`
- Create: `ZZHDesktopAssistant-Setup.msi`

1. Run dependency/license review and ensure no third-party secret or telemetry behavior is enabled.
2. Fuzz/limit protocol inputs and verify the service receives no client-controlled command.
3. Run `dotnet test`, `dotnet publish`, Cargo formatting/tests/strict Clippy and `git diff --check`.
4. Measure the service idle CPU and working set during the local probe.
5. Hash the root EXE, service publish artifact and MSI.
6. Provide manual install, UAC, idle/load comparison, service-stop and uninstall verification steps.

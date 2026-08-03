use serde::Deserialize;

use crate::{TemperatureReading, TemperatureSensor, TemperatureSource};

pub(crate) const PROTOCOL_VERSION: u32 = 1;
pub(crate) const MAXIMUM_SNAPSHOT_BYTES: usize = 16 * 1024;
const MAXIMUM_GPU_ENTRIES: usize = 8;
const KNOWN_GPU_VENDOR_IDS: [u32; 3] = [0x8086, 0x10de, 0x1002];
const PIPE_READ_ATTEMPTS: usize = 3;

#[derive(Debug)]
pub(crate) struct ServiceSnapshot {
    pub generated_at_unix_ms: i64,
    pub cpu: Option<TemperatureReading>,
    pub gpus: Vec<(u32, TemperatureReading)>,
    pub cpu_error: Option<String>,
    pub service_error: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ClientResultKind {
    Snapshot,
    NotInstalled,
    Starting,
    TemporarilyUnavailable,
}

#[derive(Debug)]
pub(crate) struct ClientResult {
    pub kind: ClientResultKind,
    pub snapshot: Option<ServiceSnapshot>,
}

impl ClientResult {
    fn snapshot(snapshot: ServiceSnapshot) -> Self {
        Self {
            kind: ClientResultKind::Snapshot,
            snapshot: Some(snapshot),
        }
    }

    fn state(kind: ClientResultKind) -> Self {
        Self {
            kind,
            snapshot: None,
        }
    }
}

#[derive(Deserialize)]
struct WireSnapshot {
    version: u32,
    generated_at_unix_ms: i64,
    cpu: Option<WireTemperature>,
    gpus: Vec<WireGpuTemperature>,
    cpu_error: Option<String>,
    service_error: Option<String>,
}

#[derive(Deserialize)]
struct WireTemperature {
    celsius: f32,
    sensor: String,
    source: String,
}

#[derive(Deserialize)]
struct WireGpuTemperature {
    vendor_id: u32,
    celsius: f32,
    sensor: String,
    source: String,
}

pub(crate) fn decode_snapshot(payload: &[u8]) -> Result<ServiceSnapshot, ()> {
    if payload.is_empty() || payload.len() > MAXIMUM_SNAPSHOT_BYTES {
        return Err(());
    }

    let wire: WireSnapshot = serde_json::from_slice(payload).map_err(|_| ())?;
    if wire.version != PROTOCOL_VERSION
        || wire.generated_at_unix_ms <= 0
        || wire.gpus.len() > MAXIMUM_GPU_ENTRIES
    {
        return Err(());
    }

    let cpu = wire.cpu.map(convert_cpu).transpose()?;
    let gpus = wire
        .gpus
        .into_iter()
        .map(|reading| {
            if !KNOWN_GPU_VENDOR_IDS.contains(&reading.vendor_id) {
                return Err(());
            }
            Ok((reading.vendor_id, convert_gpu(reading)?))
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(ServiceSnapshot {
        generated_at_unix_ms: wire.generated_at_unix_ms,
        cpu,
        gpus,
        cpu_error: wire.cpu_error,
        service_error: wire.service_error,
    })
}

fn retry_transient<T>(
    attempts: usize,
    mut operation: impl FnMut() -> Result<T, ()>,
    mut pause: impl FnMut(),
) -> Result<T, ()> {
    debug_assert!(attempts > 0);
    for attempt in 0..attempts {
        match operation() {
            Ok(value) => return Ok(value),
            Err(()) if attempt + 1 < attempts => pause(),
            Err(()) => return Err(()),
        }
    }
    Err(())
}

fn convert_cpu(reading: WireTemperature) -> Result<TemperatureReading, ()> {
    let sensor = match reading.sensor.as_str() {
        "CPU Package" => TemperatureSensor::CpuPackage,
        "Tctl/Tdie" => TemperatureSensor::TctlTdie,
        _ => return Err(()),
    };
    convert_reading(reading.celsius, sensor, &reading.source)
}

fn convert_gpu(reading: WireGpuTemperature) -> Result<TemperatureReading, ()> {
    let sensor = match reading.sensor.as_str() {
        "GPU Core" => TemperatureSensor::GpuCore,
        "GPU Edge" => TemperatureSensor::GpuEdge,
        _ => return Err(()),
    };
    convert_reading(reading.celsius, sensor, &reading.source)
}

fn convert_reading(
    celsius: f32,
    sensor: TemperatureSensor,
    source: &str,
) -> Result<TemperatureReading, ()> {
    if !celsius.is_finite() || !(0.0..=150.0).contains(&celsius) {
        return Err(());
    }
    let source = match source {
        "LibreHardwareMonitor" => TemperatureSource::LibreHardwareMonitor,
        _ => return Err(()),
    };
    Ok(TemperatureReading {
        celsius,
        sensor,
        source,
    })
}

#[cfg(target_os = "windows")]
mod client {
    use std::{
        mem::size_of,
        slice, thread,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use windows::{
        Win32::{
            Foundation::{
                CloseHandle, ERROR_IO_PENDING, ERROR_SERVICE_ALREADY_RUNNING,
                ERROR_SERVICE_DOES_NOT_EXIST, GENERIC_READ, HANDLE,
            },
            Storage::FileSystem::{
                CreateFileW, FILE_FLAG_OVERLAPPED, FILE_SHARE_MODE, OPEN_EXISTING, ReadFile,
            },
            System::{
                IO::{CancelIoEx, GetOverlappedResult, GetOverlappedResultEx, OVERLAPPED},
                Pipes::WaitNamedPipeW,
                Services::{
                    CloseServiceHandle, OpenSCManagerW, OpenServiceW, QueryServiceStatusEx,
                    SC_HANDLE, SC_MANAGER_CONNECT, SC_STATUS_PROCESS_INFO, SERVICE_QUERY_STATUS,
                    SERVICE_RUNNING, SERVICE_START, SERVICE_START_PENDING, SERVICE_STATUS_PROCESS,
                    StartServiceW,
                },
                Threading::CreateEventW,
            },
        },
        core::{HRESULT, PCWSTR, w},
    };

    use super::{
        ClientResult, ClientResultKind, MAXIMUM_SNAPSHOT_BYTES, PIPE_READ_ATTEMPTS,
        ServiceSnapshot, decode_snapshot, retry_transient,
    };

    const PIPE_TIMEOUT_MS: u32 = 250;
    const PIPE_RETRY_DELAY: Duration = Duration::from_millis(40);
    const MAXIMUM_SNAPSHOT_AGE: Duration = Duration::from_secs(15);

    pub(crate) struct SensorServiceClient;

    impl SensorServiceClient {
        pub(crate) fn new() -> Self {
            Self
        }

        pub(crate) fn read(&mut self) -> ClientResult {
            if let Ok(snapshot) = read_pipe_snapshot_with_retry() {
                return ClientResult::snapshot(snapshot);
            }

            match ensure_service_running() {
                ServiceStartOutcome::NotInstalled => {
                    ClientResult::state(ClientResultKind::NotInstalled)
                }
                ServiceStartOutcome::Starting => {
                    if let Ok(snapshot) = read_pipe_snapshot_with_retry() {
                        ClientResult::snapshot(snapshot)
                    } else {
                        ClientResult::state(ClientResultKind::Starting)
                    }
                }
                ServiceStartOutcome::Running => {
                    ClientResult::state(ClientResultKind::TemporarilyUnavailable)
                }
                ServiceStartOutcome::Failed => {
                    ClientResult::state(ClientResultKind::TemporarilyUnavailable)
                }
            }
        }
    }

    fn read_pipe_snapshot_with_retry() -> Result<ServiceSnapshot, ()> {
        retry_transient(PIPE_READ_ATTEMPTS, read_pipe_snapshot, || {
            thread::sleep(PIPE_RETRY_DELAY)
        })
    }

    enum ServiceStartOutcome {
        NotInstalled,
        Starting,
        Running,
        Failed,
    }

    fn read_pipe_snapshot() -> Result<ServiceSnapshot, ()> {
        if !unsafe {
            WaitNamedPipeW(
                w!(r"\\.\pipe\ZZHDesktopAssistant.Sensor.v1"),
                PIPE_TIMEOUT_MS,
            )
        }
        .as_bool()
        {
            return Err(());
        }

        let handle = unsafe {
            CreateFileW(
                w!(r"\\.\pipe\ZZHDesktopAssistant.Sensor.v1"),
                GENERIC_READ.0,
                FILE_SHARE_MODE(0),
                None,
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                None,
            )
        }
        .map_err(|_| ())?;
        let handle = HandleGuard(handle);

        let mut length = [0_u8; size_of::<u32>()];
        read_exact_with_timeout(handle.0, &mut length)?;
        let payload_length = u32::from_le_bytes(length) as usize;
        if payload_length == 0 || payload_length > MAXIMUM_SNAPSHOT_BYTES {
            return Err(());
        }

        let mut payload = vec![0_u8; payload_length];
        read_exact_with_timeout(handle.0, &mut payload)?;
        let snapshot = decode_snapshot(&payload)?;
        if snapshot_is_stale(snapshot.generated_at_unix_ms) {
            return Err(());
        }
        Ok(snapshot)
    }

    fn read_exact_with_timeout(handle: HANDLE, mut output: &mut [u8]) -> Result<(), ()> {
        while !output.is_empty() {
            let event =
                unsafe { CreateEventW(None, true, false, PCWSTR::null()) }.map_err(|_| ())?;
            let event = HandleGuard(event);
            let mut overlapped = OVERLAPPED {
                hEvent: event.0,
                ..Default::default()
            };

            let read_result =
                unsafe { ReadFile(handle, Some(output), None, Some(&mut overlapped)) };
            if let Err(error) = &read_result
                && error.code() != HRESULT::from_win32(ERROR_IO_PENDING.0)
            {
                return Err(());
            }

            let mut transferred = 0_u32;
            if unsafe {
                GetOverlappedResultEx(
                    handle,
                    &overlapped,
                    &mut transferred,
                    PIPE_TIMEOUT_MS,
                    false,
                )
            }
            .is_err()
            {
                let _ = unsafe { CancelIoEx(handle, Some(&overlapped)) };
                let _ = unsafe { GetOverlappedResult(handle, &overlapped, &mut transferred, true) };
                return Err(());
            }
            if transferred == 0 || transferred as usize > output.len() {
                return Err(());
            }
            output = &mut output[transferred as usize..];
        }
        Ok(())
    }

    fn snapshot_is_stale(generated_at_unix_ms: i64) -> bool {
        let Ok(generated_at_ms) = u64::try_from(generated_at_unix_ms) else {
            return true;
        };
        let Ok(now) = SystemTime::now().duration_since(UNIX_EPOCH) else {
            return true;
        };
        let generated_at = Duration::from_millis(generated_at_ms);
        generated_at > now + Duration::from_secs(5)
            || now.saturating_sub(generated_at) > MAXIMUM_SNAPSHOT_AGE
    }

    fn ensure_service_running() -> ServiceStartOutcome {
        let manager =
            match unsafe { OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT) } {
                Ok(handle) => ServiceHandleGuard(handle),
                Err(_) => return ServiceStartOutcome::Failed,
            };
        let service = match unsafe {
            OpenServiceW(
                manager.0,
                w!("ZZHSensorService"),
                SERVICE_QUERY_STATUS | SERVICE_START,
            )
        } {
            Ok(handle) => ServiceHandleGuard(handle),
            Err(error) if error.code() == HRESULT::from_win32(ERROR_SERVICE_DOES_NOT_EXIST.0) => {
                return ServiceStartOutcome::NotInstalled;
            }
            Err(_) => return ServiceStartOutcome::Failed,
        };

        let mut status = SERVICE_STATUS_PROCESS::default();
        let status_bytes = unsafe {
            slice::from_raw_parts_mut(
                (&mut status as *mut SERVICE_STATUS_PROCESS).cast::<u8>(),
                size_of::<SERVICE_STATUS_PROCESS>(),
            )
        };
        let mut required = 0_u32;
        if unsafe {
            QueryServiceStatusEx(
                service.0,
                SC_STATUS_PROCESS_INFO,
                Some(status_bytes),
                &mut required,
            )
        }
        .is_err()
        {
            return ServiceStartOutcome::Failed;
        }

        if status.dwCurrentState == SERVICE_RUNNING {
            return ServiceStartOutcome::Running;
        }
        if status.dwCurrentState == SERVICE_START_PENDING {
            return ServiceStartOutcome::Starting;
        }

        match unsafe { StartServiceW(service.0, None) } {
            Ok(()) => ServiceStartOutcome::Starting,
            Err(error) if error.code() == HRESULT::from_win32(ERROR_SERVICE_ALREADY_RUNNING.0) => {
                ServiceStartOutcome::Running
            }
            Err(_) => ServiceStartOutcome::Failed,
        }
    }

    struct HandleGuard(HANDLE);

    impl Drop for HandleGuard {
        fn drop(&mut self) {
            let _ = unsafe { CloseHandle(self.0) };
        }
    }

    struct ServiceHandleGuard(SC_HANDLE);

    impl Drop for ServiceHandleGuard {
        fn drop(&mut self) {
            let _ = unsafe { CloseServiceHandle(self.0) };
        }
    }
}

#[cfg(target_os = "windows")]
pub(crate) use client::SensorServiceClient;

#[cfg(not(target_os = "windows"))]
pub(crate) struct SensorServiceClient;

#[cfg(not(target_os = "windows"))]
impl SensorServiceClient {
    pub(crate) fn new() -> Self {
        Self
    }

    pub(crate) fn read(&mut self) -> ClientResult {
        ClientResult::state(ClientResultKind::NotInstalled)
    }
}

#[cfg(test)]
mod tests {
    use super::{MAXIMUM_SNAPSHOT_BYTES, decode_snapshot, retry_transient};
    use crate::{TemperatureSensor, TemperatureSource};

    #[test]
    fn accepts_only_the_versioned_allowlisted_temperature_contract() {
        let snapshot = decode_snapshot(br#"{
            "version":1,
            "generated_at_unix_ms":1785676059768,
            "cpu":{"celsius":42.5,"sensor":"CPU Package","source":"LibreHardwareMonitor"},
            "gpus":[{"vendor_id":4318,"celsius":55.0,"sensor":"GPU Core","source":"LibreHardwareMonitor"}]
        }"#).expect("valid snapshot");

        let cpu = snapshot.cpu.expect("CPU reading");
        assert_eq!(cpu.sensor, TemperatureSensor::CpuPackage);
        assert_eq!(cpu.source, TemperatureSource::LibreHardwareMonitor);
        assert_eq!(snapshot.gpus[0].0, 0x10de);
    }

    #[test]
    fn rejects_wrong_versions_unknown_sensors_and_oversized_payloads() {
        assert!(decode_snapshot(br#"{"version":2,"generated_at_unix_ms":1,"gpus":[]}"#).is_err());
        assert!(decode_snapshot(br#"{
            "version":1,
            "generated_at_unix_ms":1,
            "gpus":[{"vendor_id":4318,"celsius":90,"sensor":"GPU Hot Spot","source":"LibreHardwareMonitor"}]
        }"#).is_err());
        assert!(decode_snapshot(&vec![b' '; MAXIMUM_SNAPSHOT_BYTES + 1]).is_err());
    }

    #[test]
    fn rejects_non_finite_out_of_range_and_unknown_vendor_values() {
        assert!(
            decode_snapshot(
                br#"{
            "version":1,
            "generated_at_unix_ms":1,
            "cpu":{"celsius":151,"sensor":"CPU Package","source":"LibreHardwareMonitor"},
            "gpus":[]
        }"#
            )
            .is_err()
        );
        assert!(decode_snapshot(br#"{
            "version":1,
            "generated_at_unix_ms":1,
            "gpus":[{"vendor_id":4660,"celsius":50,"sensor":"GPU Core","source":"LibreHardwareMonitor"}]
        }"#).is_err());
    }

    #[test]
    fn transient_pipe_failures_are_retried_within_one_sample() {
        let mut attempts = 0;
        let mut pauses = 0;

        let result = retry_transient(
            3,
            || {
                attempts += 1;
                (attempts == 3).then_some(42).ok_or(())
            },
            || pauses += 1,
        );

        assert_eq!(result, Ok(42));
        assert_eq!(attempts, 3);
        assert_eq!(pauses, 2);
    }
}

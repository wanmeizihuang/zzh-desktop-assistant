use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant},
};

mod sensor_service;

pub const DEFAULT_HISTORY_RETENTION: Duration = Duration::from_secs(5 * 60);
pub const DEFAULT_HISTORY_MAX_SAMPLES: usize = 301;
const MAX_COUNTER_SAMPLE_GAP: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceStatus {
    Available,
    WarmingUp,
    Unavailable,
    TemporarilyUnavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotFreshness {
    Fresh,
    Stale,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MetricSourceError {
    Unsupported,
    TemporarilyUnavailable,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MetricValue<T> {
    pub value: Option<T>,
    pub status: SourceStatus,
}

impl<T> MetricValue<T> {
    fn available(value: T) -> Self {
        Self {
            value: Some(value),
            status: SourceStatus::Available,
        }
    }

    fn warming_up() -> Self {
        Self {
            value: None,
            status: SourceStatus::WarmingUp,
        }
    }

    fn from_source_error(error: MetricSourceError) -> Self {
        Self {
            value: None,
            status: match error {
                MetricSourceError::Unsupported => SourceStatus::Unavailable,
                MetricSourceError::TemporarilyUnavailable => SourceStatus::TemporarilyUnavailable,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryUsage {
    pub used_bytes: u64,
    pub total_bytes: u64,
}

impl MemoryUsage {
    pub fn used_percent(self) -> f32 {
        if self.total_bytes == 0 {
            return 0.0;
        }

        (self.used_bytes as f64 * 100.0 / self.total_bytes as f64) as f32
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NetworkThroughput {
    pub received_bytes_per_second: f64,
    pub transmitted_bytes_per_second: f64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VideoMemoryUsage {
    pub used_bytes: u64,
    pub total_bytes: u64,
}

impl VideoMemoryUsage {
    pub fn used_percent(self) -> f32 {
        if self.total_bytes == 0 {
            return 0.0;
        }

        (self.used_bytes as f64 * 100.0 / self.total_bytes as f64) as f32
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MetricSnapshot {
    pub sampled_at: Instant,
    pub cpu_total_percent: MetricValue<f32>,
    pub cpu_temperature_celsius: MetricValue<f32>,
    pub cpu_temperature_info: Option<TemperatureInfo>,
    pub memory: MetricValue<MemoryUsage>,
    pub network: MetricValue<NetworkThroughput>,
    pub gpu_total_percent: MetricValue<f32>,
    pub video_memory: MetricValue<VideoMemoryUsage>,
    pub temperature_celsius: MetricValue<f32>,
    pub temperature_info: Option<TemperatureInfo>,
    pub temperature_target: Option<GpuTemperatureTarget>,
    pub sensor_service_status: SensorServiceStatus,
}

impl MetricSnapshot {
    pub fn freshness_at(self, now: Instant, maximum_age: Duration) -> SnapshotFreshness {
        match now.checked_duration_since(self.sampled_at) {
            Some(age) if age > maximum_age => SnapshotFreshness::Stale,
            _ => SnapshotFreshness::Fresh,
        }
    }
}

#[derive(Debug)]
pub struct MetricHistory {
    retention: Duration,
    max_samples: usize,
    snapshots: VecDeque<MetricSnapshot>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrendPoint<T> {
    pub sampled_at: Instant,
    pub value: Option<T>,
}

impl MetricHistory {
    pub fn new(retention: Duration) -> Self {
        Self::with_limits(retention, DEFAULT_HISTORY_MAX_SAMPLES)
    }

    pub fn with_limits(retention: Duration, max_samples: usize) -> Self {
        Self {
            retention,
            max_samples: max_samples.max(1),
            snapshots: VecDeque::new(),
        }
    }

    pub fn push(&mut self, snapshot: MetricSnapshot) -> bool {
        if self
            .snapshots
            .back()
            .is_some_and(|latest| snapshot.sampled_at < latest.sampled_at)
        {
            return false;
        }

        self.snapshots.push_back(snapshot);
        let Some(cutoff) = snapshot.sampled_at.checked_sub(self.retention) else {
            return true;
        };
        while self
            .snapshots
            .front()
            .is_some_and(|oldest| oldest.sampled_at < cutoff)
        {
            self.snapshots.pop_front();
        }
        while self.snapshots.len() > self.max_samples {
            self.snapshots.pop_front();
        }
        true
    }

    pub fn len(&self) -> usize {
        self.snapshots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.snapshots.is_empty()
    }

    pub fn oldest(&self) -> Option<&MetricSnapshot> {
        self.snapshots.front()
    }

    pub fn latest(&self) -> Option<&MetricSnapshot> {
        self.snapshots.back()
    }

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &MetricSnapshot> {
        self.snapshots.iter()
    }

    pub fn trend_points<T>(
        &self,
        maximum_points: usize,
        mut value_of: impl FnMut(&MetricSnapshot) -> Option<T>,
    ) -> Vec<TrendPoint<T>> {
        let output_len = maximum_points.min(self.snapshots.len());
        if output_len == 0 {
            return Vec::new();
        }

        if output_len == 1 {
            let snapshot = self.snapshots.back().expect("history is not empty");
            return vec![TrendPoint {
                sampled_at: snapshot.sampled_at,
                value: value_of(snapshot),
            }];
        }

        let last_index = self.snapshots.len() - 1;
        (0..output_len)
            .map(|point_index| {
                let snapshot_index = point_index * last_index / (output_len - 1);
                let snapshot = &self.snapshots[snapshot_index];
                TrendPoint {
                    sampled_at: snapshot.sampled_at,
                    value: value_of(snapshot),
                }
            })
            .collect()
    }
}

impl Default for MetricHistory {
    fn default() -> Self {
        Self::new(DEFAULT_HISTORY_RETENTION)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CpuTimes {
    idle: u64,
    kernel: u64,
    user: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct NetworkCounters {
    received_bytes: u64,
    transmitted_bytes: u64,
    interface_signature: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct GpuEngineSample<'a> {
    instance: &'a str,
    utilization_percent: f64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct VideoAdapterCandidate {
    index: usize,
    is_software: bool,
    current_usage_bytes: u64,
    dedicated_bytes: u64,
    budget_bytes: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GpuTemperatureTarget {
    Integrated,
    Dedicated,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TemperatureSensor {
    CpuPackage,
    TctlTdie,
    GpuCore,
    GpuEdge,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TemperatureSource {
    LibreHardwareMonitor,
    IntelLevelZero,
    NvidiaNvml,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TemperatureInfo {
    pub sensor: TemperatureSensor,
    pub source: TemperatureSource,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SensorServiceStatus {
    Ready,
    NotInstalled,
    PawnIoNotInstalled,
    PawnIoDeviceUnavailable,
    InitializationFailed,
    Starting,
    TemporarilyUnavailable,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct TemperatureReading {
    celsius: f32,
    sensor: TemperatureSensor,
    source: TemperatureSource,
}

impl TemperatureReading {
    const fn info(self) -> TemperatureInfo {
        TemperatureInfo {
            sensor: self.sensor,
            source: self.source,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct TemperatureSample {
    cpu: Result<TemperatureReading, MetricSourceError>,
    gpu: Result<TemperatureReading, MetricSourceError>,
    service_status: SensorServiceStatus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct GpuAdapterIdentityCandidate {
    index: usize,
    is_software: bool,
    is_integrated: bool,
    vendor_id: u32,
    dedicated_bytes: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NativeGpuTemperatureProvider {
    IntelLevelZero,
    NvidiaNvml,
}

pub struct SystemSampler {
    previous_cpu: Option<(CpuTimes, Instant)>,
    previous_network: Option<(NetworkCounters, Instant)>,
    gpu: platform::GpuSampler,
    temperature: platform::TemperatureSampler,
}

/// # Safety
///
/// Call this once during process startup, before any worker threads are created.
/// Legacy Intel Level Zero loaders inspect this process environment switch.
pub unsafe fn prepare_gpu_temperature_sources() {
    unsafe { platform::prepare_gpu_temperature_sources() };
}

impl SystemSampler {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            previous_cpu: platform::read_cpu_times().ok().map(|times| (times, now)),
            previous_network: platform::read_network_counters()
                .ok()
                .map(|counters| (counters, now)),
            gpu: platform::GpuSampler::new(),
            temperature: platform::TemperatureSampler::new(),
        }
    }

    pub fn sample(&mut self) -> MetricSnapshot {
        let cpu_total_percent = self.sample_cpu();
        let temperatures = self.temperature.sample();
        let cpu_temperature_celsius = temperatures
            .cpu
            .map(|reading| MetricValue::available(reading.celsius))
            .unwrap_or_else(MetricValue::from_source_error);
        let cpu_temperature_info = temperatures.cpu.ok().map(TemperatureReading::info);
        let memory = self.sample_memory();
        let network = self.sample_network();
        let gpu_total_percent = self.sample_gpu();
        let video_memory = self.sample_video_memory();
        let temperature_celsius = temperatures
            .gpu
            .map(|reading| MetricValue::available(reading.celsius))
            .unwrap_or_else(MetricValue::from_source_error);
        let temperature_info = temperatures.gpu.ok().map(TemperatureReading::info);
        let temperature_target = self.temperature.target();

        MetricSnapshot {
            sampled_at: Instant::now(),
            cpu_total_percent,
            cpu_temperature_celsius,
            cpu_temperature_info,
            memory,
            network,
            gpu_total_percent,
            video_memory,
            temperature_celsius,
            temperature_info,
            temperature_target,
            sensor_service_status: temperatures.service_status,
        }
    }

    fn sample_cpu(&mut self) -> MetricValue<f32> {
        let Ok(current) = platform::read_cpu_times() else {
            self.previous_cpu = None;
            return MetricValue::from_source_error(MetricSourceError::TemporarilyUnavailable);
        };
        let now = Instant::now();

        let value = self.previous_cpu.and_then(|(previous, sampled_at)| {
            now.checked_duration_since(sampled_at)
                .filter(|elapsed| sample_gap_is_usable(*elapsed))
                .and_then(|_| calculate_cpu_percent(previous, current))
        });
        self.previous_cpu = Some((current, now));

        value.map_or_else(MetricValue::warming_up, MetricValue::available)
    }

    fn sample_memory(&self) -> MetricValue<MemoryUsage> {
        platform::read_memory_usage()
            .map(MetricValue::available)
            .unwrap_or_else(|_| {
                MetricValue::from_source_error(MetricSourceError::TemporarilyUnavailable)
            })
    }

    fn sample_network(&mut self) -> MetricValue<NetworkThroughput> {
        let Ok(current) = platform::read_network_counters() else {
            self.previous_network = None;
            return MetricValue::from_source_error(MetricSourceError::TemporarilyUnavailable);
        };
        let now = Instant::now();
        let value = self.previous_network.and_then(|(previous, sampled_at)| {
            now.checked_duration_since(sampled_at)
                .and_then(|elapsed| calculate_network_throughput(previous, current, elapsed))
        });
        self.previous_network = Some((current, now));

        value.map_or_else(MetricValue::warming_up, MetricValue::available)
    }

    fn sample_gpu(&mut self) -> MetricValue<f32> {
        match self.gpu.read_usage_percent() {
            Ok(Some(value)) => MetricValue::available(value),
            Ok(None) => MetricValue::warming_up(),
            Err(error) => MetricValue::from_source_error(error),
        }
    }

    fn sample_video_memory(&mut self) -> MetricValue<VideoMemoryUsage> {
        self.gpu
            .read_video_memory()
            .map(MetricValue::available)
            .unwrap_or_else(MetricValue::from_source_error)
    }
}

impl Default for SystemSampler {
    fn default() -> Self {
        Self::new()
    }
}

fn calculate_cpu_percent(previous: CpuTimes, current: CpuTimes) -> Option<f32> {
    let idle_delta = current.idle.saturating_sub(previous.idle);
    let kernel_delta = current.kernel.saturating_sub(previous.kernel);
    let user_delta = current.user.saturating_sub(previous.user);
    let total_delta = kernel_delta.saturating_add(user_delta);
    if total_delta == 0 {
        return None;
    }

    let busy_delta = total_delta.saturating_sub(idle_delta);
    Some((busy_delta as f64 * 100.0 / total_delta as f64).clamp(0.0, 100.0) as f32)
}

fn calculate_network_throughput(
    previous: NetworkCounters,
    current: NetworkCounters,
    elapsed: Duration,
) -> Option<NetworkThroughput> {
    if !sample_gap_is_usable(elapsed)
        || previous.interface_signature != current.interface_signature
        || current.received_bytes < previous.received_bytes
        || current.transmitted_bytes < previous.transmitted_bytes
    {
        return None;
    }

    let seconds = elapsed.as_secs_f64();
    if seconds <= f64::EPSILON {
        return None;
    }

    Some(NetworkThroughput {
        received_bytes_per_second: current
            .received_bytes
            .saturating_sub(previous.received_bytes) as f64
            / seconds,
        transmitted_bytes_per_second: current
            .transmitted_bytes
            .saturating_sub(previous.transmitted_bytes)
            as f64
            / seconds,
    })
}

fn sample_gap_is_usable(elapsed: Duration) -> bool {
    !elapsed.is_zero() && elapsed <= MAX_COUNTER_SAMPLE_GAP
}

fn include_network_interface(operational: bool, loopback: bool, hardware: bool) -> bool {
    operational && !loopback && hardware
}

fn aggregate_gpu_percent(samples: &[GpuEngineSample<'_>]) -> Option<f32> {
    let mut engine_totals = HashMap::<&str, f64>::new();

    for sample in samples {
        if !sample.utilization_percent.is_finite() {
            continue;
        }

        let engine_key = sample
            .instance
            .find("_luid_")
            .map_or(sample.instance, |position| &sample.instance[position + 1..]);
        let total = engine_totals.entry(engine_key).or_default();
        *total = (*total + sample.utilization_percent.clamp(0.0, 100.0)).min(100.0);
    }

    engine_totals
        .into_values()
        .reduce(f64::max)
        .map(|value| value as f32)
}

fn select_video_adapter(candidates: &[VideoAdapterCandidate]) -> Option<usize> {
    candidates
        .iter()
        .filter(|candidate| !candidate.is_software)
        .max_by_key(|candidate| {
            (
                candidate.current_usage_bytes,
                candidate.dedicated_bytes > 0,
                if candidate.dedicated_bytes > 0 {
                    candidate.dedicated_bytes
                } else {
                    candidate.budget_bytes
                },
                std::cmp::Reverse(candidate.index),
            )
        })
        .map(|candidate| candidate.index)
}

fn select_gpu_temperature_target(
    candidates: &[GpuAdapterIdentityCandidate],
) -> Option<(usize, u32, GpuTemperatureTarget)> {
    const SUPPORTED_GPU_VENDORS: [u32; 3] = [0x8086, 0x10de, 0x1002];

    candidates
        .iter()
        .filter(|candidate| {
            !candidate.is_software && SUPPORTED_GPU_VENDORS.contains(&candidate.vendor_id)
        })
        .max_by_key(|candidate| {
            (
                !candidate.is_integrated,
                candidate.dedicated_bytes,
                std::cmp::Reverse(candidate.index),
            )
        })
        .map(|candidate| {
            (
                candidate.index,
                candidate.vendor_id,
                if candidate.is_integrated {
                    GpuTemperatureTarget::Integrated
                } else {
                    GpuTemperatureTarget::Dedicated
                },
            )
        })
}

fn intel_level_zero_device_matches_target(
    vendor_id: u32,
    flags: u32,
    target: GpuTemperatureTarget,
) -> bool {
    const INTEL_VENDOR_ID: u32 = 0x8086;
    const INTEGRATED_FLAG: u32 = 1;

    vendor_id == INTEL_VENDOR_ID
        && match target {
            GpuTemperatureTarget::Integrated => flags & INTEGRATED_FLAG != 0,
            GpuTemperatureTarget::Dedicated => flags & INTEGRATED_FLAG == 0,
        }
}

fn native_gpu_temperature_provider(
    vendor_id: u32,
    target: GpuTemperatureTarget,
) -> Option<NativeGpuTemperatureProvider> {
    match (vendor_id, target) {
        (0x8086, _) => Some(NativeGpuTemperatureProvider::IntelLevelZero),
        (0x10de, GpuTemperatureTarget::Dedicated) => Some(NativeGpuTemperatureProvider::NvidiaNvml),
        _ => None,
    }
}

fn level_zero_sensor_priority(sensor_type: u32) -> Option<u8> {
    match sensor_type {
        1 => Some(0), // GPU
        6 => Some(1), // GPU board
        _ => None,
    }
}

fn valid_gpu_temperature_celsius(value: f64) -> Option<f32> {
    (value.is_finite() && (0.0..=150.0).contains(&value)).then_some(value as f32)
}

fn adapter_luid_key(instance: &str) -> Option<String> {
    let normalized = instance.to_ascii_lowercase();
    if !normalized.starts_with("luid_") {
        return None;
    }

    let end = normalized.find("_phys_").unwrap_or(normalized.len());
    Some(normalized[..end].to_owned())
}

#[cfg(target_os = "windows")]
mod platform {
    use std::{collections::HashMap, ffi::c_void, mem::size_of, ptr, slice};

    use windows::Win32::{
        Foundation::{ERROR_SUCCESS, FILETIME, FreeLibrary, HMODULE},
        Graphics::{
            DXCore::{
                DXCORE_ADAPTER_ATTRIBUTE_D3D12_GRAPHICS, DXCoreAdapterProperty,
                DXCoreCreateAdapterFactory, DXCoreHardwareID, DXCoreHardwareIDParts,
                DedicatedAdapterMemory, HardwareID, HardwareIDParts, IDXCoreAdapter,
                IDXCoreAdapterFactory, IDXCoreAdapterList, IsHardware, IsIntegrated,
            },
            Dxgi::{
                CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, DXGI_MEMORY_SEGMENT_GROUP_LOCAL,
                DXGI_MEMORY_SEGMENT_GROUP_NON_LOCAL, DXGI_QUERY_VIDEO_MEMORY_INFO, IDXGIAdapter3,
                IDXGIFactory1,
            },
        },
        NetworkManagement::{
            IpHelper::{
                FreeMibTable, GetIfTable2, IF_TYPE_SOFTWARE_LOOPBACK, MIB_IF_ROW2, MIB_IF_TABLE2,
            },
            Ndis::IfOperStatusUp,
        },
        System::{
            LibraryLoader::{GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW},
            Performance::{
                PDH_CSTATUS_NEW_DATA, PDH_CSTATUS_VALID_DATA, PDH_FMT_COUNTERVALUE_ITEM_W,
                PDH_FMT_DOUBLE, PDH_FMT_LARGE, PDH_HCOUNTER, PDH_HQUERY, PDH_MORE_DATA,
                PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData,
                PdhGetFormattedCounterArrayW, PdhOpenQueryW,
            },
            SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX},
            Threading::GetSystemTimes,
        },
    };
    use windows::core::{Interface, PCSTR, PCWSTR, s, w};

    use super::{
        CpuTimes, GpuAdapterIdentityCandidate, GpuEngineSample, GpuTemperatureTarget, MemoryUsage,
        MetricSourceError, NativeGpuTemperatureProvider, NetworkCounters, SensorServiceStatus,
        TemperatureReading, TemperatureSample, TemperatureSensor, TemperatureSource,
        VideoAdapterCandidate, VideoMemoryUsage, adapter_luid_key, aggregate_gpu_percent,
        include_network_interface, intel_level_zero_device_matches_target,
        level_zero_sensor_priority, native_gpu_temperature_provider, select_gpu_temperature_target,
        select_video_adapter, valid_gpu_temperature_celsius,
    };
    use crate::sensor_service::{ClientResultKind, SensorServiceClient};

    pub unsafe fn prepare_gpu_temperature_sources() {
        // SAFETY: The desktop calls this before starting any worker threads.
        unsafe { std::env::set_var("ZES_ENABLE_SYSMAN", "1") };
    }

    pub struct GpuSampler {
        usage: Option<PdhGpuUsage>,
        video_memory: Option<PdhVideoMemory>,
    }

    pub struct TemperatureSampler {
        target: Option<GpuTemperatureTarget>,
        vendor_id: Option<u32>,
        level_zero: Option<IntelLevelZeroTemperature>,
        nvml: Option<NvidiaNvmlTemperature>,
        service: SensorServiceClient,
    }

    impl TemperatureSampler {
        pub fn new() -> Self {
            let selected = enumerate_gpu_temperature_target().ok().flatten();
            let target = selected.map(|(_, _, target)| target);
            let vendor_id = selected.map(|(_, vendor_id, _)| vendor_id);
            let provider = selected.and_then(|(_, vendor_id, target)| {
                native_gpu_temperature_provider(vendor_id, target)
            });
            let level_zero = match (provider, target) {
                (Some(NativeGpuTemperatureProvider::IntelLevelZero), Some(target)) => {
                    IntelLevelZeroTemperature::new(target).ok()
                }
                _ => None,
            };
            let nvml = matches!(provider, Some(NativeGpuTemperatureProvider::NvidiaNvml))
                .then(NvidiaNvmlTemperature::new)
                .and_then(Result::ok);
            Self {
                target,
                vendor_id,
                level_zero,
                nvml,
                service: SensorServiceClient::new(),
            }
        }

        pub fn sample(&mut self) -> TemperatureSample {
            let service_result = self.service.read();
            let service_status = match service_result.kind {
                ClientResultKind::Snapshot => match service_result
                    .snapshot
                    .as_ref()
                    .and_then(|snapshot| snapshot.service_error.as_deref())
                {
                    None => SensorServiceStatus::Ready,
                    Some("pawnio_not_installed") => SensorServiceStatus::PawnIoNotInstalled,
                    Some("pawnio_device_unavailable") => {
                        SensorServiceStatus::PawnIoDeviceUnavailable
                    }
                    Some("sensor_initialization_failed") => {
                        SensorServiceStatus::InitializationFailed
                    }
                    Some(_) => SensorServiceStatus::TemporarilyUnavailable,
                },
                ClientResultKind::NotInstalled => SensorServiceStatus::NotInstalled,
                ClientResultKind::Starting => SensorServiceStatus::Starting,
                ClientResultKind::TemporarilyUnavailable => {
                    SensorServiceStatus::TemporarilyUnavailable
                }
            };

            let service_error = match service_result.kind {
                ClientResultKind::NotInstalled => MetricSourceError::Unsupported,
                _ => MetricSourceError::TemporarilyUnavailable,
            };
            let service_cpu =
                service_result
                    .snapshot
                    .as_ref()
                    .map_or(Err(service_error), |snapshot| {
                        if snapshot.service_error.is_some() {
                            Err(MetricSourceError::TemporarilyUnavailable)
                        } else if let Some(reading) = snapshot.cpu {
                            Ok(reading)
                        } else if snapshot.cpu_error.as_deref() == Some("cpu_package_unavailable") {
                            Err(MetricSourceError::Unsupported)
                        } else {
                            Err(MetricSourceError::TemporarilyUnavailable)
                        }
                    });

            let service_gpu = match (self.vendor_id, service_result.snapshot.as_ref()) {
                (None, _) => Err(MetricSourceError::Unsupported),
                (Some(_), Some(snapshot)) if snapshot.service_error.is_some() => {
                    Err(MetricSourceError::TemporarilyUnavailable)
                }
                (Some(vendor_id), Some(snapshot)) => snapshot
                    .gpus
                    .iter()
                    .find_map(|(candidate_vendor_id, reading)| {
                        (*candidate_vendor_id == vendor_id).then_some(*reading)
                    })
                    .ok_or(MetricSourceError::Unsupported),
                (Some(_), None) => Err(service_error),
            };

            let native_gpu = self
                .level_zero
                .as_mut()
                .and_then(|reader| reader.sample().ok())
                .or_else(|| self.nvml.as_mut().and_then(|reader| reader.sample().ok()));
            TemperatureSample {
                cpu: service_cpu,
                gpu: native_gpu.map_or(service_gpu, Ok),
                service_status,
            }
        }

        pub const fn target(&self) -> Option<GpuTemperatureTarget> {
            self.target
        }
    }

    impl GpuSampler {
        pub fn new() -> Self {
            Self {
                usage: PdhGpuUsage::new().ok(),
                video_memory: PdhVideoMemory::new().ok(),
            }
        }

        pub fn read_usage_percent(&mut self) -> Result<Option<f32>, MetricSourceError> {
            self.usage
                .as_mut()
                .ok_or(MetricSourceError::Unsupported)?
                .sample()
                .map_err(|_| MetricSourceError::TemporarilyUnavailable)
        }

        pub fn read_video_memory(&mut self) -> Result<VideoMemoryUsage, MetricSourceError> {
            self.video_memory
                .as_mut()
                .ok_or(MetricSourceError::Unsupported)?
                .sample()
                .map_err(|_| MetricSourceError::TemporarilyUnavailable)
        }
    }

    pub fn read_cpu_times() -> Result<CpuTimes, ()> {
        let mut idle = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)) }
            .map_err(|_| ())?;

        Ok(CpuTimes {
            idle: filetime_to_u64(idle),
            kernel: filetime_to_u64(kernel),
            user: filetime_to_u64(user),
        })
    }

    pub fn read_memory_usage() -> Result<MemoryUsage, ()> {
        let mut status = MEMORYSTATUSEX {
            dwLength: size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };
        unsafe { GlobalMemoryStatusEx(&mut status) }.map_err(|_| ())?;

        Ok(MemoryUsage {
            used_bytes: status.ullTotalPhys.saturating_sub(status.ullAvailPhys),
            total_bytes: status.ullTotalPhys,
        })
    }

    pub fn read_network_counters() -> Result<NetworkCounters, ()> {
        let mut table = ptr::null_mut();
        let result = unsafe { GetIfTable2(&mut table) };
        if result != ERROR_SUCCESS || table.is_null() {
            return Err(());
        }

        let table = MibTable(table);
        let mut counters = NetworkCounters {
            received_bytes: 0,
            transmitted_bytes: 0,
            interface_signature: 0,
        };
        let mut active_interfaces = 0u64;
        for row in table.rows() {
            let hardware = row.InterfaceAndOperStatusFlags._bitfield & 1 != 0;
            if !include_network_interface(
                row.OperStatus == IfOperStatusUp,
                row.Type == IF_TYPE_SOFTWARE_LOOPBACK,
                hardware,
            ) {
                continue;
            }

            counters.received_bytes = counters.received_bytes.saturating_add(row.InOctets);
            counters.transmitted_bytes = counters.transmitted_bytes.saturating_add(row.OutOctets);
            let interface_hash = u64::from(row.InterfaceIndex)
                .wrapping_mul(0x9e37_79b9_7f4a_7c15)
                .rotate_left(row.InterfaceIndex % 64);
            counters.interface_signature ^= interface_hash;
            active_interfaces = active_interfaces.saturating_add(1);
        }
        counters.interface_signature ^= active_interfaces.wrapping_mul(0xd6e8_feb8_6659_fd93);

        Ok(counters)
    }

    fn filetime_to_u64(value: FILETIME) -> u64 {
        (u64::from(value.dwHighDateTime) << 32) | u64::from(value.dwLowDateTime)
    }

    type LevelZeroHandle = *mut c_void;
    type LevelZeroInit = unsafe extern "system" fn(u32) -> u32;
    type LevelZeroDriverGet = unsafe extern "system" fn(*mut u32, *mut LevelZeroHandle) -> u32;
    type LevelZeroDeviceGet =
        unsafe extern "system" fn(LevelZeroHandle, *mut u32, *mut LevelZeroHandle) -> u32;
    type LevelZeroDeviceGetProperties =
        unsafe extern "system" fn(LevelZeroHandle, *mut ZesDeviceProperties) -> u32;
    type LevelZeroTemperatureEnum =
        unsafe extern "system" fn(LevelZeroHandle, *mut u32, *mut LevelZeroHandle) -> u32;
    type LevelZeroTemperatureGetProperties =
        unsafe extern "system" fn(LevelZeroHandle, *mut ZesTemperatureProperties) -> u32;
    type LevelZeroTemperatureGetState = unsafe extern "system" fn(LevelZeroHandle, *mut f64) -> u32;
    type NvmlHandle = *mut c_void;
    type NvmlInit = unsafe extern "C" fn() -> i32;
    type NvmlShutdown = unsafe extern "C" fn() -> i32;
    type NvmlDeviceGetCount = unsafe extern "C" fn(*mut u32) -> i32;
    type NvmlDeviceGetHandleByIndex = unsafe extern "C" fn(u32, *mut NvmlHandle) -> i32;
    type NvmlDeviceGetTemperature = unsafe extern "C" fn(NvmlHandle, u32, *mut u32) -> i32;

    #[repr(C)]
    struct ZeDeviceProperties {
        stype: u32,
        p_next: *mut c_void,
        device_type: u32,
        vendor_id: u32,
        device_id: u32,
        flags: u32,
        subdevice_id: u32,
        core_clock_rate: u32,
        max_mem_alloc_size: u64,
        max_hardware_contexts: u32,
        max_command_queue_priority: u32,
        num_threads_per_eu: u32,
        physical_eu_simd_width: u32,
        num_eus_per_subslice: u32,
        num_subslices_per_slice: u32,
        num_slices: u32,
        timer_resolution: u64,
        timestamp_valid_bits: u32,
        kernel_timestamp_valid_bits: u32,
        uuid: [u8; 16],
        name: [i8; 256],
    }

    #[repr(C)]
    struct ZesDeviceProperties {
        stype: u32,
        p_next: *mut c_void,
        core: ZeDeviceProperties,
        num_subdevices: u32,
        serial_number: [i8; 64],
        board_number: [i8; 64],
        brand_name: [i8; 64],
        model_name: [i8; 64],
        vendor_name: [i8; 64],
        driver_version: [i8; 64],
    }

    impl ZesDeviceProperties {
        fn new() -> Self {
            // All-zero byte patterns are valid for this C POD structure.
            let mut properties = unsafe { std::mem::zeroed::<Self>() };
            properties.stype = 0x1;
            properties.core.stype = 0x3;
            properties
        }
    }

    #[repr(C)]
    struct ZesTemperatureProperties {
        stype: u32,
        p_next: *mut c_void,
        sensor_type: u32,
        on_subdevice: u32,
        subdevice_id: u32,
        max_temperature: f64,
        is_critical_supported: u32,
        is_threshold_1_supported: u32,
        is_threshold_2_supported: u32,
    }

    impl ZesTemperatureProperties {
        fn new() -> Self {
            Self {
                stype: 0x14,
                p_next: ptr::null_mut(),
                sensor_type: 0,
                on_subdevice: 0,
                subdevice_id: 0,
                max_temperature: 0.0,
                is_critical_supported: 0,
                is_threshold_1_supported: 0,
                is_threshold_2_supported: 0,
            }
        }
    }

    struct LevelZeroModule(HMODULE);

    impl LevelZeroModule {
        fn load() -> Result<Self, ()> {
            unsafe {
                LoadLibraryExW(w!("ze_loader.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32)
                    .map(Self)
                    .map_err(|_| ())
            }
        }

        fn procedure(&self, name: PCSTR) -> Option<unsafe extern "system" fn() -> isize> {
            unsafe { GetProcAddress(self.0, name) }
        }
    }

    impl Drop for LevelZeroModule {
        fn drop(&mut self) {
            let _ = unsafe { FreeLibrary(self.0) };
        }
    }

    struct IntelLevelZeroTemperature {
        sensors: Vec<LevelZeroHandle>,
        sensor: TemperatureSensor,
        get_state: LevelZeroTemperatureGetState,
        _module: LevelZeroModule,
    }

    impl IntelLevelZeroTemperature {
        fn new(target: GpuTemperatureTarget) -> Result<Self, ()> {
            let module = LevelZeroModule::load()?;

            macro_rules! required_procedure {
                ($name:expr, $function_type:ty) => {{
                    let procedure = module.procedure($name).ok_or(())?;
                    unsafe {
                        std::mem::transmute::<unsafe extern "system" fn() -> isize, $function_type>(
                            procedure,
                        )
                    }
                }};
            }
            macro_rules! optional_procedure {
                ($name:expr, $function_type:ty) => {{
                    module.procedure($name).map(|procedure| unsafe {
                        std::mem::transmute::<unsafe extern "system" fn() -> isize, $function_type>(
                            procedure,
                        )
                    })
                }};
            }

            let modern_init = optional_procedure!(s!("zesInit"), LevelZeroInit);
            let modern_driver_get = optional_procedure!(s!("zesDriverGet"), LevelZeroDriverGet);
            let modern_device_get = optional_procedure!(s!("zesDeviceGet"), LevelZeroDeviceGet);
            let (driver_get, device_get) = if let (Some(init), Some(driver_get), Some(device_get)) =
                (modern_init, modern_driver_get, modern_device_get)
            {
                if unsafe { init(0) } != 0 {
                    return Err(());
                }
                (driver_get, device_get)
            } else {
                let init = required_procedure!(s!("zeInit"), LevelZeroInit);
                if unsafe { init(1) } != 0 {
                    return Err(());
                }
                (
                    required_procedure!(s!("zeDriverGet"), LevelZeroDriverGet),
                    required_procedure!(s!("zeDeviceGet"), LevelZeroDeviceGet),
                )
            };
            let device_get_properties =
                required_procedure!(s!("zesDeviceGetProperties"), LevelZeroDeviceGetProperties);
            let temperature_enum = required_procedure!(
                s!("zesDeviceEnumTemperatureSensors"),
                LevelZeroTemperatureEnum
            );
            let temperature_get_properties = required_procedure!(
                s!("zesTemperatureGetProperties"),
                LevelZeroTemperatureGetProperties
            );
            let get_state =
                required_procedure!(s!("zesTemperatureGetState"), LevelZeroTemperatureGetState);

            let mut driver_count = 0;
            if unsafe { driver_get(&mut driver_count, ptr::null_mut()) } != 0 || driver_count == 0 {
                return Err(());
            }
            let mut drivers = vec![ptr::null_mut(); driver_count as usize];
            if unsafe { driver_get(&mut driver_count, drivers.as_mut_ptr()) } != 0 {
                return Err(());
            }
            drivers.truncate(driver_count as usize);

            let mut ranked_sensors = Vec::<(u8, LevelZeroHandle)>::new();
            for driver in drivers {
                let mut device_count = 0;
                if unsafe { device_get(driver, &mut device_count, ptr::null_mut()) } != 0
                    || device_count == 0
                {
                    continue;
                }
                let mut devices = vec![ptr::null_mut(); device_count as usize];
                if unsafe { device_get(driver, &mut device_count, devices.as_mut_ptr()) } != 0 {
                    continue;
                }
                devices.truncate(device_count as usize);

                for device in devices {
                    let mut device_properties = ZesDeviceProperties::new();
                    if unsafe { device_get_properties(device, &mut device_properties) } != 0
                        || !intel_level_zero_device_matches_target(
                            device_properties.core.vendor_id,
                            device_properties.core.flags,
                            target,
                        )
                    {
                        continue;
                    }

                    let mut sensor_count = 0;
                    if unsafe { temperature_enum(device, &mut sensor_count, ptr::null_mut()) } != 0
                        || sensor_count == 0
                    {
                        continue;
                    }
                    let mut sensors = vec![ptr::null_mut(); sensor_count as usize];
                    if unsafe { temperature_enum(device, &mut sensor_count, sensors.as_mut_ptr()) }
                        != 0
                    {
                        continue;
                    }
                    sensors.truncate(sensor_count as usize);

                    for sensor in sensors {
                        let mut properties = ZesTemperatureProperties::new();
                        if unsafe { temperature_get_properties(sensor, &mut properties) } != 0 {
                            continue;
                        }
                        if let Some(priority) = level_zero_sensor_priority(properties.sensor_type) {
                            ranked_sensors.push((priority, sensor));
                        }
                    }
                }
            }

            let best_priority = ranked_sensors
                .iter()
                .map(|(priority, _)| *priority)
                .min()
                .ok_or(())?;
            let sensors = ranked_sensors
                .into_iter()
                .filter_map(|(priority, sensor)| (priority == best_priority).then_some(sensor))
                .collect();
            let sensor = match best_priority {
                0 => TemperatureSensor::GpuCore,
                1 => TemperatureSensor::GpuEdge,
                _ => return Err(()),
            };

            Ok(Self {
                sensors,
                sensor,
                get_state,
                _module: module,
            })
        }

        fn sample(&mut self) -> Result<TemperatureReading, ()> {
            let celsius = self
                .sensors
                .iter()
                .filter_map(|sensor| {
                    let mut temperature = 0.0;
                    (unsafe { (self.get_state)(*sensor, &mut temperature) } == 0)
                        .then(|| valid_gpu_temperature_celsius(temperature))
                        .flatten()
                })
                .max_by(f32::total_cmp)
                .ok_or(())?;
            Ok(TemperatureReading {
                celsius,
                sensor: self.sensor,
                source: TemperatureSource::IntelLevelZero,
            })
        }
    }

    struct NvidiaNvmlModule(HMODULE);

    impl NvidiaNvmlModule {
        fn load() -> Result<Self, ()> {
            unsafe {
                LoadLibraryExW(w!("nvml.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32)
                    .map(Self)
                    .map_err(|_| ())
            }
        }

        fn procedure(&self, name: PCSTR) -> Option<unsafe extern "system" fn() -> isize> {
            unsafe { GetProcAddress(self.0, name) }
        }
    }

    impl Drop for NvidiaNvmlModule {
        fn drop(&mut self) {
            let _ = unsafe { FreeLibrary(self.0) };
        }
    }

    struct NvidiaNvmlTemperature {
        devices: Vec<NvmlHandle>,
        get_temperature: NvmlDeviceGetTemperature,
        shutdown: NvmlShutdown,
        _module: NvidiaNvmlModule,
    }

    impl NvidiaNvmlTemperature {
        fn new() -> Result<Self, ()> {
            let module = NvidiaNvmlModule::load()?;

            macro_rules! procedure {
                ($name:expr, $function_type:ty) => {{
                    let procedure = module.procedure($name).ok_or(())?;
                    unsafe {
                        std::mem::transmute::<unsafe extern "system" fn() -> isize, $function_type>(
                            procedure,
                        )
                    }
                }};
            }

            let init = procedure!(s!("nvmlInit_v2"), NvmlInit);
            let shutdown = procedure!(s!("nvmlShutdown"), NvmlShutdown);
            let get_count = procedure!(s!("nvmlDeviceGetCount_v2"), NvmlDeviceGetCount);
            let get_handle = procedure!(
                s!("nvmlDeviceGetHandleByIndex_v2"),
                NvmlDeviceGetHandleByIndex
            );
            let get_temperature =
                procedure!(s!("nvmlDeviceGetTemperature"), NvmlDeviceGetTemperature);

            if unsafe { init() } != 0 {
                return Err(());
            }
            let devices = (|| {
                let mut count = 0_u32;
                if unsafe { get_count(&mut count) } != 0 || count == 0 || count > 32 {
                    return Err(());
                }
                let mut devices = Vec::with_capacity(count as usize);
                for index in 0..count {
                    let mut device = ptr::null_mut();
                    if unsafe { get_handle(index, &mut device) } == 0 && !device.is_null() {
                        devices.push(device);
                    }
                }
                (!devices.is_empty()).then_some(devices).ok_or(())
            })();
            let devices = match devices {
                Ok(devices) => devices,
                Err(()) => {
                    let _ = unsafe { shutdown() };
                    return Err(());
                }
            };

            Ok(Self {
                devices,
                get_temperature,
                shutdown,
                _module: module,
            })
        }

        fn sample(&mut self) -> Result<TemperatureReading, ()> {
            const NVML_TEMPERATURE_GPU: u32 = 0;
            let celsius = self
                .devices
                .iter()
                .filter_map(|device| {
                    let mut temperature = 0_u32;
                    (unsafe {
                        (self.get_temperature)(*device, NVML_TEMPERATURE_GPU, &mut temperature)
                    } == 0)
                        .then(|| valid_gpu_temperature_celsius(f64::from(temperature)))
                        .flatten()
                })
                .max_by(f32::total_cmp)
                .ok_or(())?;
            Ok(TemperatureReading {
                celsius,
                sensor: TemperatureSensor::GpuCore,
                source: TemperatureSource::NvidiaNvml,
            })
        }
    }

    impl Drop for NvidiaNvmlTemperature {
        fn drop(&mut self) {
            let _ = unsafe { (self.shutdown)() };
        }
    }

    struct PdhGpuUsage {
        query: PDH_HQUERY,
        counter: PDH_HCOUNTER,
    }

    impl PdhGpuUsage {
        fn new() -> Result<Self, ()> {
            let mut query = PDH_HQUERY::default();
            if unsafe { PdhOpenQueryW(PCWSTR::null(), 0, &mut query) } != 0 {
                return Err(());
            }

            let mut reader = Self {
                query,
                counter: PDH_HCOUNTER::default(),
            };
            if unsafe {
                PdhAddEnglishCounterW(
                    reader.query,
                    w!(r"\GPU Engine(*)\Utilization Percentage"),
                    0,
                    &mut reader.counter,
                )
            } != 0
            {
                return Err(());
            }

            if unsafe { PdhCollectQueryData(reader.query) } != 0 {
                return Err(());
            }

            Ok(reader)
        }

        fn sample(&mut self) -> Result<Option<f32>, ()> {
            if unsafe { PdhCollectQueryData(self.query) } != 0 {
                return Err(());
            }

            let readings = read_formatted_samples(self.counter, PDH_FMT_DOUBLE)?;
            let samples = readings
                .iter()
                .map(|(instance, utilization_percent)| GpuEngineSample {
                    instance,
                    utilization_percent: *utilization_percent,
                })
                .collect::<Vec<_>>();
            Ok(aggregate_gpu_percent(&samples))
        }
    }

    impl Drop for PdhGpuUsage {
        fn drop(&mut self) {
            if !self.query.is_invalid() {
                unsafe { PdhCloseQuery(self.query) };
            }
        }
    }

    struct PdhVideoMemory {
        query: PDH_HQUERY,
        dedicated_counter: PDH_HCOUNTER,
        shared_counter: PDH_HCOUNTER,
        adapters: Vec<DxgiVideoAdapter>,
    }

    struct DxgiVideoAdapter {
        index: usize,
        luid: String,
        dedicated_bytes: u64,
        budget_bytes: u64,
        total_bytes: u64,
    }

    impl PdhVideoMemory {
        fn new() -> Result<Self, ()> {
            let adapters = enumerate_dxgi_adapters()?;
            let mut query = PDH_HQUERY::default();
            if unsafe { PdhOpenQueryW(PCWSTR::null(), 0, &mut query) } != 0 {
                return Err(());
            }

            let mut reader = Self {
                query,
                dedicated_counter: PDH_HCOUNTER::default(),
                shared_counter: PDH_HCOUNTER::default(),
                adapters,
            };
            if unsafe {
                PdhAddEnglishCounterW(
                    reader.query,
                    w!(r"\GPU Adapter Memory(*)\Dedicated Usage"),
                    0,
                    &mut reader.dedicated_counter,
                )
            } != 0
                || unsafe {
                    PdhAddEnglishCounterW(
                        reader.query,
                        w!(r"\GPU Adapter Memory(*)\Shared Usage"),
                        0,
                        &mut reader.shared_counter,
                    )
                } != 0
                || unsafe { PdhCollectQueryData(reader.query) } != 0
            {
                return Err(());
            }

            Ok(reader)
        }

        fn sample(&mut self) -> Result<VideoMemoryUsage, ()> {
            if unsafe { PdhCollectQueryData(self.query) } != 0 {
                return Err(());
            }

            let dedicated = read_formatted_samples(self.dedicated_counter, PDH_FMT_LARGE)?;
            let shared = read_formatted_samples(self.shared_counter, PDH_FMT_LARGE)?;
            let mut usage_by_luid = HashMap::<String, u64>::new();
            for (instance, value) in dedicated.into_iter().chain(shared) {
                let Some(luid) = adapter_luid_key(&instance) else {
                    continue;
                };
                if !value.is_finite() || value <= 0.0 {
                    continue;
                }
                let usage = usage_by_luid.entry(luid).or_default();
                *usage = usage.saturating_add(value as u64);
            }

            let readings = self
                .adapters
                .iter()
                .map(|adapter| {
                    let used_bytes = usage_by_luid.get(&adapter.luid).copied().unwrap_or(0);
                    (
                        VideoAdapterCandidate {
                            index: adapter.index,
                            is_software: false,
                            current_usage_bytes: used_bytes,
                            dedicated_bytes: adapter.dedicated_bytes,
                            budget_bytes: adapter.budget_bytes,
                        },
                        VideoMemoryUsage {
                            used_bytes: used_bytes.min(adapter.total_bytes),
                            total_bytes: adapter.total_bytes,
                        },
                    )
                })
                .collect::<Vec<_>>();
            let candidates = readings
                .iter()
                .map(|(candidate, _)| *candidate)
                .collect::<Vec<_>>();
            let selected_index = select_video_adapter(&candidates).ok_or(())?;
            readings
                .into_iter()
                .find(|(candidate, _)| candidate.index == selected_index)
                .map(|(_, usage)| usage)
                .ok_or(())
        }
    }

    impl Drop for PdhVideoMemory {
        fn drop(&mut self) {
            if !self.query.is_invalid() {
                unsafe { PdhCloseQuery(self.query) };
            }
        }
    }

    fn enumerate_gpu_temperature_target() -> Result<Option<(usize, u32, GpuTemperatureTarget)>, ()>
    {
        let factory =
            unsafe { DXCoreCreateAdapterFactory::<IDXCoreAdapterFactory>() }.map_err(|_| ())?;
        let adapters = unsafe {
            factory
                .CreateAdapterList::<IDXCoreAdapterList>(&[DXCORE_ADAPTER_ATTRIBUTE_D3D12_GRAPHICS])
        }
        .map_err(|_| ())?;
        let mut candidates = Vec::new();

        for adapter_index in 0..unsafe { adapters.GetAdapterCount() } {
            let adapter =
                unsafe { adapters.GetAdapter::<IDXCoreAdapter>(adapter_index) }.map_err(|_| ())?;
            let is_hardware = read_dxcore_property::<bool>(&adapter, IsHardware)?;
            let is_integrated = read_dxcore_property::<bool>(&adapter, IsIntegrated)?;
            let dedicated_bytes =
                read_dxcore_property::<u64>(&adapter, DedicatedAdapterMemory).unwrap_or(0);
            let vendor_id =
                read_dxcore_property::<DXCoreHardwareIDParts>(&adapter, HardwareIDParts)
                    .map(|hardware| hardware.vendorID)
                    .or_else(|_| {
                        read_dxcore_property::<DXCoreHardwareID>(&adapter, HardwareID)
                            .map(|hardware| hardware.vendorID)
                    })?;
            candidates.push(GpuAdapterIdentityCandidate {
                index: adapter_index as usize,
                is_software: !is_hardware,
                is_integrated,
                vendor_id,
                dedicated_bytes,
            });
        }

        Ok(select_gpu_temperature_target(&candidates))
    }

    fn read_dxcore_property<T: Copy + Default>(
        adapter: &IDXCoreAdapter,
        property: DXCoreAdapterProperty,
    ) -> Result<T, ()> {
        if !unsafe { adapter.IsPropertySupported(property) } {
            return Err(());
        }
        let mut value = T::default();
        unsafe {
            adapter.GetProperty(
                property,
                size_of::<T>(),
                (&mut value as *mut T).cast::<c_void>(),
            )
        }
        .map_err(|_| ())?;
        Ok(value)
    }

    fn enumerate_dxgi_adapters() -> Result<Vec<DxgiVideoAdapter>, ()> {
        let factory = unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }.map_err(|_| ())?;
        let mut adapters = Vec::new();

        for adapter_index in 0u32.. {
            let Ok(adapter) = (unsafe { factory.EnumAdapters1(adapter_index) }) else {
                break;
            };
            let desc = unsafe { adapter.GetDesc1() }.map_err(|_| ())?;
            let Ok(adapter) = adapter.cast::<IDXGIAdapter3>() else {
                continue;
            };
            let mut info = DXGI_QUERY_VIDEO_MEMORY_INFO::default();
            if unsafe {
                adapter.QueryVideoMemoryInfo(0, DXGI_MEMORY_SEGMENT_GROUP_LOCAL, &mut info)
            }
            .is_err()
            {
                continue;
            }
            let mut non_local_info = DXGI_QUERY_VIDEO_MEMORY_INFO::default();
            if unsafe {
                adapter.QueryVideoMemoryInfo(
                    0,
                    DXGI_MEMORY_SEGMENT_GROUP_NON_LOCAL,
                    &mut non_local_info,
                )
            }
            .is_err()
            {
                continue;
            }

            let candidate = VideoAdapterCandidate {
                index: adapter_index as usize,
                is_software: desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0,
                current_usage_bytes: info
                    .CurrentUsage
                    .saturating_add(non_local_info.CurrentUsage),
                dedicated_bytes: desc.DedicatedVideoMemory as u64,
                budget_bytes: info.Budget.saturating_add(non_local_info.Budget),
            };
            if candidate.is_software {
                continue;
            }
            let total_bytes =
                (desc.DedicatedVideoMemory as u64).saturating_add(desc.SharedSystemMemory as u64);
            adapters.push(DxgiVideoAdapter {
                index: candidate.index,
                luid: format!(
                    "luid_0x{:08x}_0x{:08x}",
                    desc.AdapterLuid.HighPart as u32, desc.AdapterLuid.LowPart
                ),
                dedicated_bytes: candidate.dedicated_bytes,
                budget_bytes: candidate.budget_bytes,
                total_bytes: if total_bytes > 0 {
                    total_bytes
                } else {
                    candidate.budget_bytes
                },
            });
        }

        (!adapters.is_empty()).then_some(adapters).ok_or(())
    }

    fn read_formatted_samples(
        counter: PDH_HCOUNTER,
        format: windows::Win32::System::Performance::PDH_FMT,
    ) -> Result<Vec<(String, f64)>, ()> {
        let mut buffer_size = 0u32;
        let mut item_count = 0u32;
        let result = unsafe {
            PdhGetFormattedCounterArrayW(counter, format, &mut buffer_size, &mut item_count, None)
        };
        if result != PDH_MORE_DATA || buffer_size == 0 {
            return Err(());
        }

        for _ in 0..2 {
            let words = (buffer_size as usize).div_ceil(size_of::<usize>());
            let mut buffer = vec![0usize; words];
            let items = buffer.as_mut_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>();
            let result = unsafe {
                PdhGetFormattedCounterArrayW(
                    counter,
                    format,
                    &mut buffer_size,
                    &mut item_count,
                    Some(items),
                )
            };
            if result == PDH_MORE_DATA {
                continue;
            }
            if result != 0 {
                return Err(());
            }

            let items = unsafe { slice::from_raw_parts(items, item_count as usize) };
            let mut samples = Vec::with_capacity(items.len());
            for item in items {
                if !matches!(
                    item.FmtValue.CStatus,
                    PDH_CSTATUS_VALID_DATA | PDH_CSTATUS_NEW_DATA
                ) || item.szName.0.is_null()
                {
                    continue;
                }

                let instance = unsafe { item.szName.to_string() }.map_err(|_| ())?;
                let value = if format == PDH_FMT_DOUBLE {
                    unsafe { item.FmtValue.Anonymous.doubleValue }
                } else {
                    unsafe { item.FmtValue.Anonymous.largeValue as f64 }
                };
                samples.push((instance, value));
            }
            return Ok(samples);
        }

        Err(())
    }

    struct MibTable(*mut MIB_IF_TABLE2);

    impl MibTable {
        fn rows(&self) -> &[MIB_IF_ROW2] {
            let count = unsafe { (*self.0).NumEntries as usize };
            let first = unsafe { (*self.0).Table.as_ptr() };
            // GetIfTable2 allocates one contiguous table with NumEntries rows.
            unsafe { slice::from_raw_parts(first, count) }
        }
    }

    impl Drop for MibTable {
        fn drop(&mut self) {
            unsafe { FreeMibTable(self.0.cast::<c_void>()) };
        }
    }
}

#[cfg(not(target_os = "windows"))]
mod platform {
    use super::{
        CpuTimes, GpuTemperatureTarget, MemoryUsage, MetricSourceError, NetworkCounters,
        SensorServiceStatus, TemperatureSample, VideoMemoryUsage,
    };

    pub unsafe fn prepare_gpu_temperature_sources() {}

    pub struct GpuSampler;

    pub struct TemperatureSampler;

    impl TemperatureSampler {
        pub fn new() -> Self {
            Self
        }

        pub fn sample(&mut self) -> TemperatureSample {
            TemperatureSample {
                cpu: Err(MetricSourceError::Unsupported),
                gpu: Err(MetricSourceError::Unsupported),
                service_status: SensorServiceStatus::NotInstalled,
            }
        }

        pub fn target(&self) -> Option<GpuTemperatureTarget> {
            None
        }
    }

    impl GpuSampler {
        pub fn new() -> Self {
            Self
        }

        pub fn read_usage_percent(&mut self) -> Result<Option<f32>, MetricSourceError> {
            Err(MetricSourceError::Unsupported)
        }

        pub fn read_video_memory(&mut self) -> Result<VideoMemoryUsage, MetricSourceError> {
            Err(MetricSourceError::Unsupported)
        }
    }

    pub fn read_cpu_times() -> Result<CpuTimes, ()> {
        Err(())
    }

    pub fn read_memory_usage() -> Result<MemoryUsage, ()> {
        Err(())
    }

    pub fn read_network_counters() -> Result<NetworkCounters, ()> {
        Err(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{
        CpuTimes, GpuAdapterIdentityCandidate, GpuEngineSample, GpuTemperatureTarget,
        MAX_COUNTER_SAMPLE_GAP, MemoryUsage, MetricHistory, MetricSnapshot, MetricSourceError,
        MetricValue, NativeGpuTemperatureProvider, NetworkCounters, NetworkThroughput,
        SensorServiceStatus, SnapshotFreshness, SourceStatus, VideoAdapterCandidate,
        VideoMemoryUsage, adapter_luid_key, aggregate_gpu_percent, calculate_cpu_percent,
        calculate_network_throughput, include_network_interface,
        intel_level_zero_device_matches_target, level_zero_sensor_priority,
        native_gpu_temperature_provider, sample_gap_is_usable, select_gpu_temperature_target,
        select_video_adapter, valid_gpu_temperature_celsius,
    };

    fn snapshot_at(sampled_at: Instant) -> MetricSnapshot {
        MetricSnapshot {
            sampled_at,
            cpu_total_percent: MetricValue {
                value: Some(25.0),
                status: SourceStatus::Available,
            },
            cpu_temperature_celsius: MetricValue {
                value: Some(42.0),
                status: SourceStatus::Available,
            },
            cpu_temperature_info: None,
            memory: MetricValue {
                value: Some(MemoryUsage {
                    used_bytes: 4,
                    total_bytes: 8,
                }),
                status: SourceStatus::Available,
            },
            network: MetricValue {
                value: Some(NetworkThroughput {
                    received_bytes_per_second: 1.0,
                    transmitted_bytes_per_second: 2.0,
                }),
                status: SourceStatus::Available,
            },
            gpu_total_percent: MetricValue {
                value: None,
                status: SourceStatus::Unavailable,
            },
            video_memory: MetricValue {
                value: None,
                status: SourceStatus::Unavailable,
            },
            temperature_celsius: MetricValue {
                value: None,
                status: SourceStatus::TemporarilyUnavailable,
            },
            temperature_info: None,
            temperature_target: None,
            sensor_service_status: SensorServiceStatus::TemporarilyUnavailable,
        }
    }

    #[test]
    fn snapshot_becomes_stale_only_after_the_maximum_age() {
        let sampled_at = Instant::now();
        let snapshot = snapshot_at(sampled_at);
        let maximum_age = Duration::from_secs(5);

        assert_eq!(
            snapshot.freshness_at(sampled_at + maximum_age, maximum_age),
            SnapshotFreshness::Fresh
        );
        assert_eq!(
            snapshot.freshness_at(
                sampled_at + maximum_age + Duration::from_nanos(1),
                maximum_age
            ),
            SnapshotFreshness::Stale
        );
    }

    #[test]
    fn future_timestamp_is_treated_as_fresh_after_clock_discontinuity() {
        let now = Instant::now();
        let snapshot = snapshot_at(now + Duration::from_secs(1));

        assert_eq!(
            snapshot.freshness_at(now, Duration::from_secs(5)),
            SnapshotFreshness::Fresh
        );
    }

    #[test]
    fn metric_history_keeps_only_the_configured_time_window() {
        let start = Instant::now();
        let mut history = MetricHistory::new(Duration::from_secs(5));

        assert!(history.push(snapshot_at(start)));
        assert!(history.push(snapshot_at(start + Duration::from_secs(5))));
        assert_eq!(history.len(), 2, "the exact retention boundary is kept");

        assert!(history.push(snapshot_at(
            start + Duration::from_secs(5) + Duration::from_nanos(1)
        )));
        assert_eq!(history.len(), 2);
        assert_eq!(
            history
                .oldest()
                .expect("history should retain the boundary sample")
                .sampled_at,
            start + Duration::from_secs(5)
        );
    }

    #[test]
    fn metric_history_starts_empty() {
        let history = MetricHistory::default();

        assert!(history.is_empty());
        assert_eq!(history.len(), 0);
        assert_eq!(history.oldest(), None);
        assert_eq!(history.latest(), None);
        assert_eq!(history.iter().count(), 0);
    }

    #[test]
    fn metric_history_never_exceeds_the_sample_limit() {
        let sampled_at = Instant::now();
        let mut history = MetricHistory::with_limits(Duration::from_secs(300), 3);

        for _ in 0..4 {
            assert!(history.push(snapshot_at(sampled_at)));
        }

        assert_eq!(history.len(), 3);
    }

    #[test]
    fn metric_history_rejects_out_of_order_samples() {
        let start = Instant::now();
        let mut history = MetricHistory::new(Duration::from_secs(300));

        assert!(history.push(snapshot_at(start + Duration::from_secs(2))));
        assert!(!history.push(snapshot_at(start + Duration::from_secs(1))));
        assert_eq!(history.len(), 1);
        assert_eq!(
            history
                .latest()
                .expect("latest sample should remain")
                .sampled_at,
            start + Duration::from_secs(2)
        );
    }

    #[test]
    fn trend_points_are_empty_without_history_or_a_point_budget() {
        let start = Instant::now();
        let mut history = MetricHistory::default();

        assert!(
            history
                .trend_points(8, |snapshot| snapshot.cpu_total_percent.value)
                .is_empty()
        );
        assert!(history.push(snapshot_at(start)));
        assert!(
            history
                .trend_points(0, |snapshot| snapshot.cpu_total_percent.value)
                .is_empty()
        );
    }

    #[test]
    fn trend_points_preserve_endpoints_and_missing_samples() {
        let start = Instant::now();
        let mut history = MetricHistory::default();
        for second in 0..5 {
            let mut snapshot = snapshot_at(start + Duration::from_secs(second));
            snapshot.cpu_total_percent.value = (second != 2).then_some(second as f32 * 10.0);
            snapshot.cpu_total_percent.status = if second == 2 {
                SourceStatus::TemporarilyUnavailable
            } else {
                SourceStatus::Available
            };
            assert!(history.push(snapshot));
        }

        let points = history.trend_points(3, |snapshot| {
            (snapshot.cpu_total_percent.status == SourceStatus::Available)
                .then_some(snapshot.cpu_total_percent.value)
                .flatten()
        });

        assert_eq!(points.len(), 3);
        assert_eq!(points[0].sampled_at, start);
        assert_eq!(points[0].value, Some(0.0));
        assert_eq!(points[1].sampled_at, start + Duration::from_secs(2));
        assert_eq!(points[1].value, None);
        assert_eq!(points[2].sampled_at, start + Duration::from_secs(4));
        assert_eq!(points[2].value, Some(40.0));
    }

    #[test]
    fn one_trend_point_uses_the_latest_sample() {
        let start = Instant::now();
        let mut history = MetricHistory::default();
        assert!(history.push(snapshot_at(start)));
        assert!(history.push(snapshot_at(start + Duration::from_secs(2))));

        let points = history.trend_points(1, |snapshot| snapshot.cpu_total_percent.value);

        assert_eq!(points.len(), 1);
        assert_eq!(points[0].sampled_at, start + Duration::from_secs(2));
    }

    #[test]
    fn cpu_percent_uses_kernel_and_user_deltas_minus_idle() {
        let result = calculate_cpu_percent(
            CpuTimes {
                idle: 100,
                kernel: 300,
                user: 200,
            },
            CpuTimes {
                idle: 130,
                kernel: 380,
                user: 240,
            },
        );

        assert_eq!(result, Some(75.0));
    }

    #[test]
    fn cpu_percent_waits_when_no_time_has_elapsed() {
        let times = CpuTimes {
            idle: 100,
            kernel: 300,
            user: 200,
        };

        assert_eq!(calculate_cpu_percent(times, times), None);
    }

    #[test]
    fn network_throughput_uses_counter_delta_and_elapsed_time() {
        let result = calculate_network_throughput(
            NetworkCounters {
                received_bytes: 1_000,
                transmitted_bytes: 500,
                interface_signature: 7,
            },
            NetworkCounters {
                received_bytes: 2_024,
                transmitted_bytes: 1_524,
                interface_signature: 7,
            },
            Duration::from_millis(500),
        )
        .expect("stable counters should produce throughput");

        assert_eq!(result.received_bytes_per_second, 2_048.0);
        assert_eq!(result.transmitted_bytes_per_second, 2_048.0);
    }

    #[test]
    fn network_counter_reset_does_not_create_a_spike() {
        let result = calculate_network_throughput(
            NetworkCounters {
                received_bytes: 10_000,
                transmitted_bytes: 20_000,
                interface_signature: 7,
            },
            NetworkCounters {
                received_bytes: 100,
                transmitted_bytes: 200,
                interface_signature: 7,
            },
            Duration::from_secs(1),
        );

        assert_eq!(result, None);
    }

    #[test]
    fn network_topology_change_restarts_the_rate_baseline() {
        let result = calculate_network_throughput(
            NetworkCounters {
                received_bytes: 1_000,
                transmitted_bytes: 500,
                interface_signature: 7,
            },
            NetworkCounters {
                received_bytes: 50_000,
                transmitted_bytes: 25_000,
                interface_signature: 9,
            },
            Duration::from_secs(1),
        );

        assert_eq!(result, None);
    }

    #[test]
    fn network_defaults_to_active_physical_non_loopback_interfaces() {
        assert!(include_network_interface(true, false, true));
        assert!(!include_network_interface(false, false, true));
        assert!(!include_network_interface(true, true, true));
        assert!(!include_network_interface(true, false, false));
    }

    #[test]
    fn long_counter_gap_requires_a_fresh_baseline() {
        assert!(sample_gap_is_usable(MAX_COUNTER_SAMPLE_GAP));
        assert!(!sample_gap_is_usable(
            MAX_COUNTER_SAMPLE_GAP + Duration::from_nanos(1)
        ));
    }

    #[test]
    fn source_errors_distinguish_unsupported_from_temporary_failure() {
        let unsupported = MetricValue::<f32>::from_source_error(MetricSourceError::Unsupported);
        let temporary =
            MetricValue::<f32>::from_source_error(MetricSourceError::TemporarilyUnavailable);

        assert_eq!(unsupported.status, SourceStatus::Unavailable);
        assert_eq!(temporary.status, SourceStatus::TemporarilyUnavailable);
        assert_eq!(unsupported.value, None);
        assert_eq!(temporary.value, None);
    }

    #[test]
    fn memory_percent_is_derived_from_used_and_total_bytes() {
        let memory = MemoryUsage {
            used_bytes: 6,
            total_bytes: 16,
        };

        assert_eq!(memory.used_percent(), 37.5);
    }

    #[test]
    fn gpu_percent_sums_processes_per_engine_then_uses_busiest_engine() {
        let result = aggregate_gpu_percent(&[
            GpuEngineSample {
                instance: "pid_10_luid_0x0_0x1_phys_0_eng_0_engtype_3D",
                utilization_percent: 22.0,
            },
            GpuEngineSample {
                instance: "pid_20_luid_0x0_0x1_phys_0_eng_0_engtype_3D",
                utilization_percent: 18.0,
            },
            GpuEngineSample {
                instance: "pid_10_luid_0x0_0x1_phys_0_eng_1_engtype_Copy",
                utilization_percent: 35.0,
            },
        ]);

        assert_eq!(result, Some(40.0));
    }

    #[test]
    fn gpu_percent_clamps_engine_totals_and_ignores_invalid_values() {
        let result = aggregate_gpu_percent(&[
            GpuEngineSample {
                instance: "pid_10_luid_0x0_0x1_phys_0_eng_0_engtype_3D",
                utilization_percent: 70.0,
            },
            GpuEngineSample {
                instance: "pid_20_luid_0x0_0x1_phys_0_eng_0_engtype_3D",
                utilization_percent: 60.0,
            },
            GpuEngineSample {
                instance: "invalid",
                utilization_percent: f64::NAN,
            },
        ]);

        assert_eq!(result, Some(100.0));
    }

    #[test]
    fn video_adapter_selection_prefers_largest_dedicated_hardware_adapter() {
        let candidates = [
            VideoAdapterCandidate {
                index: 0,
                is_software: false,
                current_usage_bytes: 0,
                dedicated_bytes: 0,
                budget_bytes: 8_000,
            },
            VideoAdapterCandidate {
                index: 1,
                is_software: true,
                current_usage_bytes: 0,
                dedicated_bytes: 16_000,
                budget_bytes: 16_000,
            },
            VideoAdapterCandidate {
                index: 2,
                is_software: false,
                current_usage_bytes: 0,
                dedicated_bytes: 4_000,
                budget_bytes: 3_000,
            },
        ];

        assert_eq!(select_video_adapter(&candidates), Some(2));
    }

    #[test]
    fn video_adapter_selection_falls_back_to_largest_local_budget() {
        let candidates = [
            VideoAdapterCandidate {
                index: 0,
                is_software: false,
                current_usage_bytes: 0,
                dedicated_bytes: 0,
                budget_bytes: 2_000,
            },
            VideoAdapterCandidate {
                index: 1,
                is_software: false,
                current_usage_bytes: 0,
                dedicated_bytes: 0,
                budget_bytes: 6_000,
            },
        ];

        assert_eq!(select_video_adapter(&candidates), Some(1));
    }

    #[test]
    fn video_adapter_selection_follows_the_adapter_with_current_usage() {
        let candidates = [
            VideoAdapterCandidate {
                index: 0,
                is_software: false,
                current_usage_bytes: 1_500,
                dedicated_bytes: 128,
                budget_bytes: 8_000,
            },
            VideoAdapterCandidate {
                index: 1,
                is_software: false,
                current_usage_bytes: 0,
                dedicated_bytes: 4_000,
                budget_bytes: 12_000,
            },
        ];

        assert_eq!(select_video_adapter(&candidates), Some(0));
    }

    #[test]
    fn gpu_temperature_target_prefers_a_dedicated_adapter_over_integrated_activity() {
        let candidates = [
            GpuAdapterIdentityCandidate {
                index: 0,
                is_software: false,
                is_integrated: true,
                vendor_id: 0x8086,
                dedicated_bytes: 128,
            },
            GpuAdapterIdentityCandidate {
                index: 1,
                is_software: false,
                is_integrated: false,
                vendor_id: 0x10de,
                dedicated_bytes: 8_000,
            },
        ];

        assert_eq!(
            select_gpu_temperature_target(&candidates),
            Some((1, 0x10de, GpuTemperatureTarget::Dedicated))
        );
    }

    #[test]
    fn gpu_temperature_target_uses_integrated_only_without_a_dedicated_adapter() {
        let candidates = [GpuAdapterIdentityCandidate {
            index: 2,
            is_software: false,
            is_integrated: true,
            vendor_id: 0x8086,
            dedicated_bytes: 128,
        }];

        assert_eq!(
            select_gpu_temperature_target(&candidates),
            Some((2, 0x8086, GpuTemperatureTarget::Integrated))
        );
    }

    #[test]
    fn gpu_temperature_target_ignores_virtual_and_unknown_adapters() {
        let candidates = [
            GpuAdapterIdentityCandidate {
                index: 0,
                is_software: true,
                is_integrated: false,
                vendor_id: 0x10de,
                dedicated_bytes: 8_000,
            },
            GpuAdapterIdentityCandidate {
                index: 1,
                is_software: false,
                is_integrated: false,
                vendor_id: 0x1234,
                dedicated_bytes: 16_000,
            },
        ];

        assert_eq!(select_gpu_temperature_target(&candidates), None);
    }

    #[test]
    fn intel_level_zero_device_must_match_the_selected_gpu_class() {
        assert!(intel_level_zero_device_matches_target(
            0x8086,
            1,
            GpuTemperatureTarget::Integrated
        ));
        assert!(intel_level_zero_device_matches_target(
            0x8086,
            0,
            GpuTemperatureTarget::Dedicated
        ));
        assert!(!intel_level_zero_device_matches_target(
            0x8086,
            1,
            GpuTemperatureTarget::Dedicated
        ));
        assert!(!intel_level_zero_device_matches_target(
            0x10de,
            0,
            GpuTemperatureTarget::Dedicated
        ));
    }

    #[test]
    fn level_zero_accepts_only_gpu_core_or_board_edge_temperature() {
        assert_eq!(level_zero_sensor_priority(1), Some(0));
        assert_eq!(level_zero_sensor_priority(6), Some(1));
        assert_eq!(level_zero_sensor_priority(0), None);
        assert_eq!(level_zero_sensor_priority(2), None);
        assert_eq!(level_zero_sensor_priority(8), None);
    }

    #[test]
    fn native_gpu_provider_is_limited_to_matching_intel_and_nvidia_targets() {
        assert_eq!(
            native_gpu_temperature_provider(0x8086, GpuTemperatureTarget::Integrated),
            Some(NativeGpuTemperatureProvider::IntelLevelZero)
        );
        assert_eq!(
            native_gpu_temperature_provider(0x10de, GpuTemperatureTarget::Dedicated),
            Some(NativeGpuTemperatureProvider::NvidiaNvml)
        );
        assert_eq!(
            native_gpu_temperature_provider(0x10de, GpuTemperatureTarget::Integrated),
            None
        );
        assert_eq!(
            native_gpu_temperature_provider(0x1002, GpuTemperatureTarget::Dedicated),
            None
        );
    }

    #[test]
    fn gpu_temperature_rejects_invalid_or_implausible_values() {
        assert_eq!(valid_gpu_temperature_celsius(42.5), Some(42.5));
        assert_eq!(valid_gpu_temperature_celsius(-1.0), None);
        assert_eq!(valid_gpu_temperature_celsius(151.0), None);
        assert_eq!(valid_gpu_temperature_celsius(f64::NAN), None);
    }

    #[test]
    fn video_memory_percent_is_derived_from_used_and_total_bytes() {
        let memory = VideoMemoryUsage {
            used_bytes: 3,
            total_bytes: 12,
        };

        assert_eq!(memory.used_percent(), 25.0);
    }

    #[test]
    fn gpu_adapter_instance_is_normalized_to_its_luid() {
        assert_eq!(
            adapter_luid_key("LUID_0x00000000_0x0000F98E_phys_0"),
            Some("luid_0x00000000_0x0000f98e".into())
        );
        assert_eq!(adapter_luid_key("not-a-gpu-adapter"), None);
    }
}

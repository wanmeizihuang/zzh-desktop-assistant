use std::{collections::HashMap, time::Instant};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceStatus {
    Available,
    WarmingUp,
    Unavailable,
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

    fn unavailable() -> Self {
        Self {
            value: None,
            status: SourceStatus::Unavailable,
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
    pub cpu_total_percent: MetricValue<f32>,
    pub memory: MetricValue<MemoryUsage>,
    pub network: MetricValue<NetworkThroughput>,
    pub gpu_total_percent: MetricValue<f32>,
    pub video_memory: MetricValue<VideoMemoryUsage>,
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

pub struct SystemSampler {
    previous_cpu: Option<CpuTimes>,
    previous_network: Option<(NetworkCounters, Instant)>,
    gpu: platform::GpuSampler,
}

impl SystemSampler {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            previous_cpu: platform::read_cpu_times().ok(),
            previous_network: platform::read_network_counters()
                .ok()
                .map(|counters| (counters, now)),
            gpu: platform::GpuSampler::new(),
        }
    }

    pub fn sample(&mut self) -> MetricSnapshot {
        MetricSnapshot {
            cpu_total_percent: self.sample_cpu(),
            memory: self.sample_memory(),
            network: self.sample_network(),
            gpu_total_percent: self.sample_gpu(),
            video_memory: self.sample_video_memory(),
        }
    }

    fn sample_cpu(&mut self) -> MetricValue<f32> {
        let Ok(current) = platform::read_cpu_times() else {
            return MetricValue::unavailable();
        };

        let value = self
            .previous_cpu
            .and_then(|previous| calculate_cpu_percent(previous, current));
        self.previous_cpu = Some(current);

        value.map_or_else(MetricValue::warming_up, MetricValue::available)
    }

    fn sample_memory(&self) -> MetricValue<MemoryUsage> {
        platform::read_memory_usage()
            .map(MetricValue::available)
            .unwrap_or_else(|_| MetricValue::unavailable())
    }

    fn sample_network(&mut self) -> MetricValue<NetworkThroughput> {
        let Ok(current) = platform::read_network_counters() else {
            return MetricValue::unavailable();
        };
        let now = Instant::now();
        let value = self.previous_network.map(|(previous, sampled_at)| {
            calculate_network_throughput(previous, current, now.duration_since(sampled_at))
        });
        self.previous_network = Some((current, now));

        value.map_or_else(MetricValue::warming_up, MetricValue::available)
    }

    fn sample_gpu(&mut self) -> MetricValue<f32> {
        match self.gpu.read_usage_percent() {
            Ok(Some(value)) => MetricValue::available(value),
            Ok(None) => MetricValue::warming_up(),
            Err(()) => MetricValue::unavailable(),
        }
    }

    fn sample_video_memory(&mut self) -> MetricValue<VideoMemoryUsage> {
        self.gpu
            .read_video_memory()
            .map(MetricValue::available)
            .unwrap_or_else(|_| MetricValue::unavailable())
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
    elapsed: std::time::Duration,
) -> NetworkThroughput {
    let seconds = elapsed.as_secs_f64();
    if seconds <= f64::EPSILON {
        return NetworkThroughput {
            received_bytes_per_second: 0.0,
            transmitted_bytes_per_second: 0.0,
        };
    }

    NetworkThroughput {
        received_bytes_per_second: current
            .received_bytes
            .saturating_sub(previous.received_bytes) as f64
            / seconds,
        transmitted_bytes_per_second: current
            .transmitted_bytes
            .saturating_sub(previous.transmitted_bytes)
            as f64
            / seconds,
    }
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
        Foundation::{ERROR_SUCCESS, FILETIME},
        Graphics::Dxgi::{
            CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, DXGI_MEMORY_SEGMENT_GROUP_LOCAL,
            DXGI_MEMORY_SEGMENT_GROUP_NON_LOCAL, DXGI_QUERY_VIDEO_MEMORY_INFO, IDXGIAdapter3,
            IDXGIFactory1,
        },
        NetworkManagement::{
            IpHelper::{
                FreeMibTable, GetIfTable2, IF_TYPE_SOFTWARE_LOOPBACK, MIB_IF_ROW2, MIB_IF_TABLE2,
            },
            Ndis::IfOperStatusUp,
        },
        System::{
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
    use windows::core::{Interface, PCWSTR, w};

    use super::{
        CpuTimes, GpuEngineSample, MemoryUsage, NetworkCounters, VideoAdapterCandidate,
        VideoMemoryUsage, adapter_luid_key, aggregate_gpu_percent, select_video_adapter,
    };

    pub struct GpuSampler {
        usage: Option<PdhGpuUsage>,
        video_memory: Option<PdhVideoMemory>,
    }

    impl GpuSampler {
        pub fn new() -> Self {
            Self {
                usage: PdhGpuUsage::new().ok(),
                video_memory: PdhVideoMemory::new().ok(),
            }
        }

        pub fn read_usage_percent(&mut self) -> Result<Option<f32>, ()> {
            self.usage.as_mut().ok_or(())?.sample()
        }

        pub fn read_video_memory(&mut self) -> Result<VideoMemoryUsage, ()> {
            self.video_memory.as_mut().ok_or(())?.sample()
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
        };
        for row in table.rows() {
            if row.OperStatus != IfOperStatusUp || row.Type == IF_TYPE_SOFTWARE_LOOPBACK {
                continue;
            }

            counters.received_bytes = counters.received_bytes.saturating_add(row.InOctets);
            counters.transmitted_bytes = counters.transmitted_bytes.saturating_add(row.OutOctets);
        }

        Ok(counters)
    }

    fn filetime_to_u64(value: FILETIME) -> u64 {
        (u64::from(value.dwHighDateTime) << 32) | u64::from(value.dwLowDateTime)
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
    use super::{CpuTimes, MemoryUsage, NetworkCounters, VideoMemoryUsage};

    pub struct GpuSampler;

    impl GpuSampler {
        pub fn new() -> Self {
            Self
        }

        pub fn read_usage_percent(&mut self) -> Result<Option<f32>, ()> {
            Err(())
        }

        pub fn read_video_memory(&mut self) -> Result<VideoMemoryUsage, ()> {
            Err(())
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
    use std::time::Duration;

    use super::{
        CpuTimes, GpuEngineSample, MemoryUsage, NetworkCounters, VideoAdapterCandidate,
        VideoMemoryUsage, adapter_luid_key, aggregate_gpu_percent, calculate_cpu_percent,
        calculate_network_throughput, select_video_adapter,
    };

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
            },
            NetworkCounters {
                received_bytes: 2_024,
                transmitted_bytes: 1_524,
            },
            Duration::from_millis(500),
        );

        assert_eq!(result.received_bytes_per_second, 2_048.0);
        assert_eq!(result.transmitted_bytes_per_second, 2_048.0);
    }

    #[test]
    fn network_counter_reset_does_not_create_a_spike() {
        let result = calculate_network_throughput(
            NetworkCounters {
                received_bytes: 10_000,
                transmitted_bytes: 20_000,
            },
            NetworkCounters {
                received_bytes: 100,
                transmitted_bytes: 200,
            },
            Duration::from_secs(1),
        );

        assert_eq!(result.received_bytes_per_second, 0.0);
        assert_eq!(result.transmitted_bytes_per_second, 0.0);
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

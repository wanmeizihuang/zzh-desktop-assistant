use std::time::Instant;

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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MetricSnapshot {
    pub cpu_total_percent: MetricValue<f32>,
    pub memory: MetricValue<MemoryUsage>,
    pub network: MetricValue<NetworkThroughput>,
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

pub struct SystemSampler {
    previous_cpu: Option<CpuTimes>,
    previous_network: Option<(NetworkCounters, Instant)>,
}

impl SystemSampler {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            previous_cpu: platform::read_cpu_times().ok(),
            previous_network: platform::read_network_counters()
                .ok()
                .map(|counters| (counters, now)),
        }
    }

    pub fn sample(&mut self) -> MetricSnapshot {
        MetricSnapshot {
            cpu_total_percent: self.sample_cpu(),
            memory: self.sample_memory(),
            network: self.sample_network(),
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

#[cfg(target_os = "windows")]
mod platform {
    use std::{ffi::c_void, mem::size_of, ptr, slice};

    use windows::Win32::{
        Foundation::{ERROR_SUCCESS, FILETIME},
        NetworkManagement::{
            IpHelper::{
                FreeMibTable, GetIfTable2, IF_TYPE_SOFTWARE_LOOPBACK, MIB_IF_ROW2, MIB_IF_TABLE2,
            },
            Ndis::IfOperStatusUp,
        },
        System::{
            SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX},
            Threading::GetSystemTimes,
        },
    };

    use super::{CpuTimes, MemoryUsage, NetworkCounters};

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
    use super::{CpuTimes, MemoryUsage, NetworkCounters};

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
        CpuTimes, MemoryUsage, NetworkCounters, calculate_cpu_percent, calculate_network_throughput,
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
}

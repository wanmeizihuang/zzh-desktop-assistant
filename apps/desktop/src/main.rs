#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{
    cell::RefCell,
    error::Error,
    rc::Rc,
    sync::mpsc::{self, RecvTimeoutError, Sender},
    thread::{self, JoinHandle},
    time::Duration,
};

use app_core::{
    DragDecision, DragGesture, PhysicalPosition as CorePhysicalPosition,
    PhysicalSize as CorePhysicalSize, PointerPosition, ScreenBounds, WindowState,
    clamp_window_position,
};
use slint::ComponentHandle;
use slint::winit_030::{EventResult, WinitWindowAccessor, winit};
use system_monitor::{MetricSnapshot, MetricValue, SourceStatus, SystemSampler};

slint::include_modules!();

fn main() -> Result<(), Box<dyn Error>> {
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("software".into())
        .select()?;

    let ui = AppWindow::new()?;
    let state = Rc::new(RefCell::new(WindowState::default()));
    let gesture = Rc::new(RefCell::new(DragGesture::default()));
    let monitor_worker = MonitorWorker::start(ui.as_weak())?;

    register_window_bounds(&ui);
    register_drag_callbacks(&ui, Rc::clone(&gesture));

    let ui_weak = ui.as_weak();
    let gesture_for_click = Rc::clone(&gesture);
    let state_for_click = Rc::clone(&state);

    ui.on_mascot_clicked(move || {
        if !gesture_for_click.borrow_mut().take_click() {
            return;
        }

        let Some(ui) = ui_weak.upgrade() else {
            return;
        };

        toggle_window(&ui, &state_for_click);
    });

    let ui_weak = ui.as_weak();

    ui.on_toggle_requested(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };

        toggle_window(&ui, &state);
    });

    let result = ui.run();
    drop(monitor_worker);
    result.map_err(Into::into)
}

struct MonitorWorker {
    stop_sender: Sender<()>,
    thread: Option<JoinHandle<()>>,
}

impl MonitorWorker {
    fn start(ui_weak: slint::Weak<AppWindow>) -> std::io::Result<Self> {
        let (stop_sender, stop_receiver) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("system-monitor".into())
            .spawn(move || {
                let mut sampler = SystemSampler::new();
                let mut delay = Duration::from_millis(250);

                loop {
                    match stop_receiver.recv_timeout(delay) {
                        Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
                        Err(RecvTimeoutError::Timeout) => {}
                    }

                    let snapshot = sampler.sample();
                    if ui_weak
                        .upgrade_in_event_loop(move |ui| apply_snapshot(&ui, snapshot))
                        .is_err()
                    {
                        break;
                    }
                    delay = Duration::from_secs(2);
                }
            })?;

        Ok(Self {
            stop_sender,
            thread: Some(thread),
        })
    }
}

impl Drop for MonitorWorker {
    fn drop(&mut self) {
        let _ = self.stop_sender.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn apply_snapshot(ui: &AppWindow, snapshot: MetricSnapshot) {
    let (cpu_value, cpu_detail) = format_cpu(snapshot.cpu_total_percent);
    ui.set_cpu_value(cpu_value.into());
    ui.set_cpu_detail(cpu_detail.into());

    let (memory_value, memory_detail) = format_memory(snapshot.memory);
    ui.set_memory_value(memory_value.into());
    ui.set_memory_detail(memory_detail.into());

    let (network_value, network_detail) = format_network(snapshot.network);
    ui.set_network_value(network_value.into());
    ui.set_network_detail(network_detail.into());

    let (gpu_value, gpu_detail) = format_gpu(snapshot.gpu_total_percent);
    ui.set_gpu_value(gpu_value.into());
    ui.set_gpu_detail(gpu_detail.into());

    let (video_memory_value, video_memory_detail) = format_video_memory(snapshot.video_memory);
    ui.set_video_memory_value(video_memory_value.into());
    ui.set_video_memory_detail(video_memory_detail.into());
}

fn format_cpu(metric: MetricValue<f32>) -> (String, String) {
    match (metric.status, metric.value) {
        (SourceStatus::Available, Some(value)) => (format!("{value:.0}%"), "系统总占用".into()),
        (SourceStatus::WarmingUp, _) => warming_up_text(),
        _ => unavailable_text(),
    }
}

fn format_memory(metric: MetricValue<system_monitor::MemoryUsage>) -> (String, String) {
    match (metric.status, metric.value) {
        (SourceStatus::Available, Some(memory)) => (
            format!("{:.0}%", memory.used_percent()),
            format!(
                "{:.1} / {:.1} GB",
                bytes_to_gib(memory.used_bytes),
                bytes_to_gib(memory.total_bytes)
            ),
        ),
        (SourceStatus::WarmingUp, _) => warming_up_text(),
        _ => unavailable_text(),
    }
}

fn format_network(metric: MetricValue<system_monitor::NetworkThroughput>) -> (String, String) {
    match (metric.status, metric.value) {
        (SourceStatus::Available, Some(network)) => (
            format!("↓ {}", format_rate(network.received_bytes_per_second)),
            format!("↑ {}", format_rate(network.transmitted_bytes_per_second)),
        ),
        (SourceStatus::WarmingUp, _) => warming_up_text(),
        _ => unavailable_text(),
    }
}

fn format_gpu(metric: MetricValue<f32>) -> (String, String) {
    match (metric.status, metric.value) {
        (SourceStatus::Available, Some(value)) => (format!("{value:.0}%"), "最忙引擎".into()),
        (SourceStatus::WarmingUp, _) => warming_up_text(),
        _ => unavailable_text(),
    }
}

fn format_video_memory(metric: MetricValue<system_monitor::VideoMemoryUsage>) -> (String, String) {
    match (metric.status, metric.value) {
        (SourceStatus::Available, Some(memory)) => (
            format!("{:.0}%", memory.used_percent()),
            format!(
                "{:.1} / {:.1} GB",
                bytes_to_gib(memory.used_bytes),
                bytes_to_gib(memory.total_bytes)
            ),
        ),
        (SourceStatus::WarmingUp, _) => warming_up_text(),
        _ => unavailable_text(),
    }
}

fn warming_up_text() -> (String, String) {
    ("采样中".into(), "正在建立差分基线".into())
}

fn unavailable_text() -> (String, String) {
    ("不可用".into(), "Windows 数据源失败".into())
}

fn bytes_to_gib(bytes: u64) -> f64 {
    bytes as f64 / 1024.0_f64.powi(3)
}

fn format_rate(bytes_per_second: f64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;

    if bytes_per_second >= GIB {
        format!("{:.1} GB/s", bytes_per_second / GIB)
    } else if bytes_per_second >= MIB {
        format!("{:.1} MB/s", bytes_per_second / MIB)
    } else if bytes_per_second >= KIB {
        format!("{:.1} KB/s", bytes_per_second / KIB)
    } else {
        format!("{bytes_per_second:.0} B/s")
    }
}

fn register_drag_callbacks(ui: &AppWindow, gesture: Rc<RefCell<DragGesture>>) {
    let gesture_for_press = Rc::clone(&gesture);
    ui.on_drag_pointer_pressed(move |x, y| {
        gesture_for_press
            .borrow_mut()
            .press(PointerPosition { x, y });
    });

    let ui_weak = ui.as_weak();
    ui.on_drag_pointer_moved(move |x, y| {
        let decision = gesture.borrow_mut().move_to(PointerPosition { x, y });
        if decision != DragDecision::BeginDrag {
            return;
        }

        let Some(ui) = ui_weak.upgrade() else {
            return;
        };

        ui.window().with_winit_window(|window| {
            if let Err(error) = window.drag_window() {
                eprintln!("failed to start native window drag: {error}");
            }
        });
    });
}

fn register_window_bounds(ui: &AppWindow) {
    ui.window().on_winit_window_event(|slint_window, event| {
        let winit::event::WindowEvent::Moved(position) = event else {
            return EventResult::Propagate;
        };

        slint_window.with_winit_window(|window| {
            let Some(monitor) = window
                .current_monitor()
                .or_else(|| window.primary_monitor())
            else {
                return;
            };

            let monitor_position = monitor.position();
            let monitor_size = monitor.size();
            let window_size = window.outer_size();
            let clamped = clamp_window_position(
                CorePhysicalPosition {
                    x: position.x,
                    y: position.y,
                },
                CorePhysicalSize {
                    width: window_size.width,
                    height: window_size.height,
                },
                ScreenBounds {
                    position: CorePhysicalPosition {
                        x: monitor_position.x,
                        y: monitor_position.y,
                    },
                    size: CorePhysicalSize {
                        width: monitor_size.width,
                        height: monitor_size.height,
                    },
                },
                32,
            );

            if clamped.x != position.x || clamped.y != position.y {
                window.set_outer_position(winit::dpi::PhysicalPosition::new(clamped.x, clamped.y));
            }
        });

        EventResult::Propagate
    });
}

fn toggle_window(ui: &AppWindow, state: &RefCell<WindowState>) {
    let mut state = state.borrow_mut();
    state.toggle();
    ui.set_expanded(state.mode().is_expanded());
}

#[cfg(test)]
mod tests {
    use system_monitor::{MemoryUsage, MetricValue, SourceStatus, VideoMemoryUsage};

    use super::{format_gpu, format_memory, format_rate, format_video_memory};

    #[test]
    fn network_rate_uses_compact_binary_units() {
        assert_eq!(format_rate(512.0), "512 B/s");
        assert_eq!(format_rate(2.0 * 1024.0), "2.0 KB/s");
        assert_eq!(format_rate(3.5 * 1024.0 * 1024.0), "3.5 MB/s");
    }

    #[test]
    fn memory_text_contains_percent_and_capacity() {
        let (value, detail) = format_memory(MetricValue {
            value: Some(MemoryUsage {
                used_bytes: 8 * 1024 * 1024 * 1024,
                total_bytes: 16 * 1024 * 1024 * 1024,
            }),
            status: SourceStatus::Available,
        });

        assert_eq!(value, "50%");
        assert_eq!(detail, "8.0 / 16.0 GB");
    }

    #[test]
    fn unavailable_memory_has_an_honest_status() {
        let (value, detail) = format_memory(MetricValue {
            value: None,
            status: SourceStatus::Unavailable,
        });

        assert_eq!(value, "不可用");
        assert_eq!(detail, "Windows 数据源失败");
    }

    #[test]
    fn gpu_text_uses_busiest_engine_semantics() {
        let (value, detail) = format_gpu(MetricValue {
            value: Some(42.4),
            status: SourceStatus::Available,
        });

        assert_eq!(value, "42%");
        assert_eq!(detail, "最忙引擎");
    }

    #[test]
    fn video_memory_text_contains_percent_and_capacity() {
        let (value, detail) = format_video_memory(MetricValue {
            value: Some(VideoMemoryUsage {
                used_bytes: 2 * 1024 * 1024 * 1024,
                total_bytes: 8 * 1024 * 1024 * 1024,
            }),
            status: SourceStatus::Available,
        });

        assert_eq!(value, "25%");
        assert_eq!(detail, "2.0 / 8.0 GB");
    }
}

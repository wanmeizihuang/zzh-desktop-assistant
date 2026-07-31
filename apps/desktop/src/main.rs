#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod display;
mod single_instance;
mod startup;

use std::{
    cell::{Cell, RefCell},
    env,
    error::Error,
    io,
    path::PathBuf,
    rc::Rc,
    sync::mpsc::{self, RecvTimeoutError, Sender},
    thread::{self, JoinHandle},
    time::Duration,
};

use app_core::{
    ApplicationPhase, DragDecision, DragGesture, PhysicalPosition as CorePhysicalPosition,
    PhysicalSize as CorePhysicalSize, PointerPosition, WindowMode, WindowState,
    clamp_window_for_mode,
    config::{AppConfig, ConfigLoadStatus, ConfigStore},
};
use slint::ComponentHandle;
use slint::winit_030::winit::platform::windows::WindowExtWindows;
use slint::winit_030::{EventResult, WinitWindowAccessor, winit};
use system_monitor::{MetricSnapshot, MetricValue, SourceStatus, SystemSampler};

slint::include_modules!();

fn main() -> Result<(), Box<dyn Error>> {
    let primary_instance = match single_instance::acquire()? {
        single_instance::AcquireResult::Primary(instance) => instance,
        single_instance::AcquireResult::SecondaryNotified => return Ok(()),
    };

    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("software".into())
        .select()?;

    let config_store = ConfigStore::new(local_config_path()?);
    let loaded_config = load_config_or_default(&config_store);
    let config = Rc::new(RefCell::new(loaded_config));
    let state = Rc::new(RefCell::new(WindowState::with_behavior(
        config.borrow().always_on_top,
        config.borrow().position_locked,
    )));
    let config_worker = ConfigWorker::start(config_store)?;
    let ui = AppWindow::new()?;
    let tray = AssistantTray::new()?;
    let gesture = Rc::new(RefCell::new(DragGesture::default()));
    let saved_window_position = config.borrow().window_position;
    let startup_enabled = Rc::new(Cell::new(startup::is_enabled().unwrap_or_else(|error| {
        eprintln!("failed to read startup state: {error}");
        false
    })));
    let monitor_worker = MonitorWorker::start(ui.as_weak())?;

    state.borrow_mut().start();
    ui.set_always_on_top_enabled(state.borrow().always_on_top());
    ui.set_position_locked(state.borrow().position_locked());
    ui.set_startup_enabled(startup_enabled.get());
    sync_tray_state(&tray, &state.borrow());
    tray.set_startup_enabled(startup_enabled.get());

    register_window_events(
        &ui,
        &tray,
        Rc::clone(&state),
        Rc::clone(&config),
        config_worker.handle(),
    );
    register_drag_callbacks(&ui, Rc::clone(&gesture), Rc::clone(&state));
    register_behavior_callbacks(
        &ui,
        &tray,
        Rc::clone(&state),
        Rc::clone(&gesture),
        Rc::clone(&config),
        Rc::clone(&startup_enabled),
        config_worker.handle(),
    );
    register_tray_callbacks(
        &ui,
        &tray,
        Rc::clone(&state),
        Rc::clone(&gesture),
        Rc::clone(&config),
        Rc::clone(&startup_enabled),
        config_worker.handle(),
    );

    let ui_weak = ui.as_weak();
    let gesture_for_click = Rc::clone(&gesture);
    let state_for_click = Rc::clone(&state);
    let config_for_click = Rc::clone(&config);
    let saver_for_click = config_worker.handle();
    let tray_weak = tray.as_weak();

    ui.on_mascot_clicked(move || {
        if !gesture_for_click.borrow_mut().take_click() {
            return;
        }

        let Some(ui) = ui_weak.upgrade() else {
            return;
        };

        let Some(tray) = tray_weak.upgrade() else {
            return;
        };
        toggle_window(
            &ui,
            &tray,
            &state_for_click,
            &config_for_click,
            &saver_for_click,
        );
    });

    let ui_weak = ui.as_weak();

    let tray_weak = tray.as_weak();
    let state_for_toggle = Rc::clone(&state);
    let config_for_toggle = Rc::clone(&config);
    let saver_for_toggle = config_worker.handle();
    ui.on_toggle_requested(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let Some(tray) = tray_weak.upgrade() else {
            return;
        };

        toggle_window(
            &ui,
            &tray,
            &state_for_toggle,
            &config_for_toggle,
            &saver_for_toggle,
        );
    });

    let ui_weak = ui.as_weak();
    let tray_weak = tray.as_weak();
    let state_for_wake = Rc::clone(&state);
    ui.on_wake_requested(move || {
        let (Some(ui), Some(tray)) = (ui_weak.upgrade(), tray_weak.upgrade()) else {
            return;
        };
        restore_window(&ui, &tray, &state_for_wake);
    });

    let ui_weak = ui.as_weak();
    let single_instance_runtime = primary_instance.start_wake_listener(move || {
        if ui_weak
            .upgrade_in_event_loop(|ui| ui.invoke_wake_requested())
            .is_err()
        {
            eprintln!("failed to deliver a single-instance wake request");
        }
    })?;

    ui.show()?;
    show_native_window(&ui, false);
    restore_saved_window_position_later(
        &ui,
        saved_window_position,
        Rc::clone(&config),
        config_worker.handle(),
    );
    tray.show()?;
    let result = slint::run_event_loop();
    drop(single_instance_runtime);
    drop(monitor_worker);
    drop(config_worker);
    result.map_err(Into::into)
}

fn local_config_path() -> io::Result<PathBuf> {
    let local_app_data = env::var_os("LOCALAPPDATA").filter(|value| !value.is_empty());
    let Some(local_app_data) = local_app_data else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "LOCALAPPDATA is not available",
        ));
    };

    Ok(PathBuf::from(local_app_data)
        .join("Xiaoxi Desktop Assistant")
        .join("settings.json"))
}

fn load_config_or_default(store: &ConfigStore) -> AppConfig {
    match store.load() {
        Ok(loaded) => {
            if let ConfigLoadStatus::RecoveredCorrupt { backup_path } = loaded.status {
                eprintln!(
                    "recovered corrupt configuration; backup saved at {}",
                    backup_path.display()
                );
            }
            loaded.config
        }
        Err(error) => {
            eprintln!(
                "failed to load configuration from {}: {error}",
                store.path().display()
            );
            AppConfig::default()
        }
    }
}

enum ConfigCommand {
    Save(AppConfig),
    Stop,
}

#[derive(Clone)]
struct ConfigSaveHandle {
    sender: Sender<ConfigCommand>,
}

impl ConfigSaveHandle {
    fn request(&self, config: AppConfig) {
        if self.sender.send(ConfigCommand::Save(config)).is_err() {
            eprintln!("configuration worker is no longer available");
        }
    }
}

struct ConfigWorker {
    handle: ConfigSaveHandle,
    thread: Option<JoinHandle<()>>,
}

impl ConfigWorker {
    const SAVE_DELAY: Duration = Duration::from_millis(300);

    fn start(store: ConfigStore) -> io::Result<Self> {
        let (sender, receiver) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("config-writer".into())
            .spawn(move || Self::run(receiver, store))?;
        Ok(Self {
            handle: ConfigSaveHandle { sender },
            thread: Some(thread),
        })
    }

    fn handle(&self) -> ConfigSaveHandle {
        self.handle.clone()
    }

    fn run(receiver: mpsc::Receiver<ConfigCommand>, store: ConfigStore) {
        while let Ok(command) = receiver.recv() {
            let ConfigCommand::Save(mut pending) = command else {
                return;
            };

            loop {
                match receiver.recv_timeout(Self::SAVE_DELAY) {
                    Ok(ConfigCommand::Save(config)) => pending = config,
                    Ok(ConfigCommand::Stop) | Err(RecvTimeoutError::Disconnected) => {
                        save_config(&store, &pending);
                        return;
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        save_config(&store, &pending);
                        break;
                    }
                }
            }
        }
    }
}

impl Drop for ConfigWorker {
    fn drop(&mut self) {
        let _ = self.handle.sender.send(ConfigCommand::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn save_config(store: &ConfigStore, config: &AppConfig) {
    if let Err(error) = store.save(config) {
        eprintln!(
            "failed to save configuration to {}: {error}",
            store.path().display()
        );
    }
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

    let (temperature_value, temperature_detail) = format_temperature(snapshot.temperature_celsius);
    ui.set_temperature_value(temperature_value.into());
    ui.set_temperature_detail(temperature_detail.into());
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

fn format_temperature(metric: MetricValue<f32>) -> (String, String) {
    match (metric.status, metric.value) {
        (SourceStatus::Available, Some(value)) => (format!("{value:.0}°C"), "ACPI 热区".into()),
        (SourceStatus::WarmingUp, _) => ("采样中".into(), "正在读取 ACPI 热区".into()),
        _ => ("不可用".into(), "未检测到可用温度源".into()),
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

fn register_drag_callbacks(
    ui: &AppWindow,
    gesture: Rc<RefCell<DragGesture>>,
    state: Rc<RefCell<WindowState>>,
) {
    let gesture_for_press = Rc::clone(&gesture);
    ui.on_drag_pointer_pressed(move |x, y| {
        gesture_for_press
            .borrow_mut()
            .press(PointerPosition { x, y });
    });

    let ui_weak = ui.as_weak();
    ui.on_drag_pointer_moved(move |x, y| {
        if !state.borrow().can_drag() {
            return;
        }

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

fn register_behavior_callbacks(
    ui: &AppWindow,
    tray: &AssistantTray,
    state: Rc<RefCell<WindowState>>,
    gesture: Rc<RefCell<DragGesture>>,
    config: Rc<RefCell<AppConfig>>,
    startup_enabled: Rc<Cell<bool>>,
    config_saver: ConfigSaveHandle,
) {
    let ui_weak = ui.as_weak();
    let tray_weak = tray.as_weak();
    let state_for_topmost = Rc::clone(&state);
    let config_for_topmost = Rc::clone(&config);
    let saver_for_topmost = config_saver.clone();
    ui.on_always_on_top_toggle_requested(move || {
        let (Some(ui), Some(tray)) = (ui_weak.upgrade(), tray_weak.upgrade()) else {
            return;
        };

        toggle_always_on_top(
            &ui,
            &tray,
            &state_for_topmost,
            &config_for_topmost,
            &saver_for_topmost,
        );
    });

    let ui_weak = ui.as_weak();
    let tray_weak = tray.as_weak();
    ui.on_position_lock_toggle_requested(move || {
        let (Some(ui), Some(tray)) = (ui_weak.upgrade(), tray_weak.upgrade()) else {
            return;
        };

        toggle_position_lock(&ui, &tray, &state, &gesture, &config, &config_saver);
    });

    let ui_weak = ui.as_weak();
    let tray_weak = tray.as_weak();
    ui.on_startup_toggle_requested(move || {
        let (Some(ui), Some(tray)) = (ui_weak.upgrade(), tray_weak.upgrade()) else {
            return;
        };

        toggle_startup(&ui, &tray, &startup_enabled);
    });
}

fn register_tray_callbacks(
    ui: &AppWindow,
    tray: &AssistantTray,
    state: Rc<RefCell<WindowState>>,
    gesture: Rc<RefCell<DragGesture>>,
    config: Rc<RefCell<AppConfig>>,
    startup_enabled: Rc<Cell<bool>>,
    config_saver: ConfigSaveHandle,
) {
    let ui_weak = ui.as_weak();
    let tray_weak = tray.as_weak();
    let state_for_click = Rc::clone(&state);
    tray.on_tray_clicked(move || {
        let (Some(ui), Some(tray)) = (ui_weak.upgrade(), tray_weak.upgrade()) else {
            return;
        };
        restore_window(&ui, &tray, &state_for_click);
    });

    let ui_weak = ui.as_weak();
    let tray_weak = tray.as_weak();
    let state_for_visibility = Rc::clone(&state);
    tray.on_show_hide_requested(move || {
        let (Some(ui), Some(tray)) = (ui_weak.upgrade(), tray_weak.upgrade()) else {
            return;
        };
        if state_for_visibility.borrow().phase() == ApplicationPhase::Hidden {
            restore_window(&ui, &tray, &state_for_visibility);
        } else {
            hide_window(&ui, &tray, &state_for_visibility);
        }
    });

    let ui_weak = ui.as_weak();
    let tray_weak = tray.as_weak();
    let state_for_toggle = Rc::clone(&state);
    let config_for_toggle = Rc::clone(&config);
    let saver_for_toggle = config_saver.clone();
    tray.on_toggle_requested(move || {
        let (Some(ui), Some(tray)) = (ui_weak.upgrade(), tray_weak.upgrade()) else {
            return;
        };
        toggle_window(
            &ui,
            &tray,
            &state_for_toggle,
            &config_for_toggle,
            &saver_for_toggle,
        );
    });

    let ui_weak = ui.as_weak();
    let tray_weak = tray.as_weak();
    let state_for_topmost = Rc::clone(&state);
    let config_for_topmost = Rc::clone(&config);
    let saver_for_topmost = config_saver.clone();
    tray.on_always_on_top_toggle_requested(move || {
        let (Some(ui), Some(tray)) = (ui_weak.upgrade(), tray_weak.upgrade()) else {
            return;
        };
        toggle_always_on_top(
            &ui,
            &tray,
            &state_for_topmost,
            &config_for_topmost,
            &saver_for_topmost,
        );
    });

    let ui_weak = ui.as_weak();
    let tray_weak = tray.as_weak();
    let state_for_lock = Rc::clone(&state);
    let gesture_for_lock = Rc::clone(&gesture);
    let config_for_lock = Rc::clone(&config);
    let saver_for_lock = config_saver.clone();
    tray.on_position_lock_toggle_requested(move || {
        let (Some(ui), Some(tray)) = (ui_weak.upgrade(), tray_weak.upgrade()) else {
            return;
        };
        toggle_position_lock(
            &ui,
            &tray,
            &state_for_lock,
            &gesture_for_lock,
            &config_for_lock,
            &saver_for_lock,
        );
    });

    let ui_weak = ui.as_weak();
    let tray_weak = tray.as_weak();
    tray.on_startup_toggle_requested(move || {
        let (Some(ui), Some(tray)) = (ui_weak.upgrade(), tray_weak.upgrade()) else {
            return;
        };

        toggle_startup(&ui, &tray, &startup_enabled);
    });

    let ui_weak = ui.as_weak();
    let tray_weak = tray.as_weak();
    let state_for_settings = Rc::clone(&state);
    tray.on_settings_requested(move || {
        let (Some(ui), Some(tray)) = (ui_weak.upgrade(), tray_weak.upgrade()) else {
            return;
        };
        open_settings(&ui, &tray, &state_for_settings);
    });

    let ui_weak = ui.as_weak();
    let tray_weak = tray.as_weak();
    tray.on_exit_requested(move || {
        let (Some(ui), Some(tray)) = (ui_weak.upgrade(), tray_weak.upgrade()) else {
            return;
        };
        if !state.borrow_mut().exit() {
            return;
        }
        let _ = ui.hide();
        let _ = tray.hide();
        if let Err(error) = slint::quit_event_loop() {
            eprintln!("failed to quit the event loop: {error}");
        }
    });
}

fn register_window_events(
    ui: &AppWindow,
    tray: &AssistantTray,
    state: Rc<RefCell<WindowState>>,
    config: Rc<RefCell<AppConfig>>,
    config_saver: ConfigSaveHandle,
) {
    let ui_weak = ui.as_weak();
    let tray_weak = tray.as_weak();
    ui.window()
        .on_winit_window_event(move |slint_window, event| {
            if let winit::event::WindowEvent::KeyboardInput { event, .. } = event
                && event.state == winit::event::ElementState::Pressed
                && !event.repeat
                && event.logical_key
                    == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
            {
                let (Some(ui), Some(tray)) = (ui_weak.upgrade(), tray_weak.upgrade()) else {
                    return EventResult::Propagate;
                };

                if collapse_window(&ui, &tray, &state, &config, &config_saver) {
                    return EventResult::PreventDefault;
                }
                return EventResult::Propagate;
            }

            let event_position = match event {
                winit::event::WindowEvent::Moved(position) => Some(CorePhysicalPosition {
                    x: position.x,
                    y: position.y,
                }),
                winit::event::WindowEvent::Resized(_)
                | winit::event::WindowEvent::ScaleFactorChanged { .. } => None,
                _ => return EventResult::Propagate,
            };

            slint_window.with_winit_window(|window| {
                let requested_position = event_position.or_else(|| {
                    window
                        .outer_position()
                        .ok()
                        .map(|position| CorePhysicalPosition {
                            x: position.x,
                            y: position.y,
                        })
                });
                let Some(requested_position) = requested_position else {
                    return;
                };
                recover_and_store_window_position(
                    window,
                    requested_position,
                    &config,
                    &config_saver,
                    false,
                    state.borrow().mode(),
                );
            });

            EventResult::Propagate
        });
}

fn restore_saved_window_position_later(
    ui: &AppWindow,
    saved_position: Option<CorePhysicalPosition>,
    config: Rc<RefCell<AppConfig>>,
    config_saver: ConfigSaveHandle,
) {
    let Some(saved_position) = saved_position else {
        return;
    };
    for delay_ms in [150, 400] {
        let ui_weak = ui.as_weak();
        let config = Rc::clone(&config);
        let config_saver = config_saver.clone();
        slint::Timer::single_shot(Duration::from_millis(delay_ms), move || {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            ui.window().with_winit_window(|window| {
                recover_and_store_window_position(
                    window,
                    saved_position,
                    &config,
                    &config_saver,
                    true,
                    WindowMode::Collapsed,
                );
            });
        });
    }
}

fn remember_collapsed_position(ui: &AppWindow, state: &RefCell<WindowState>) {
    ui.window().with_winit_window(|window| {
        let Ok(position) = window.outer_position() else {
            return;
        };
        state
            .borrow_mut()
            .remember_collapsed_position(CorePhysicalPosition {
                x: position.x,
                y: position.y,
            });
    });
}

fn restore_collapsed_position_later(
    ui: &AppWindow,
    position: CorePhysicalPosition,
    config: Rc<RefCell<AppConfig>>,
    config_saver: ConfigSaveHandle,
) {
    for delay_ms in [50, 200] {
        let ui_weak = ui.as_weak();
        let config = Rc::clone(&config);
        let config_saver = config_saver.clone();
        slint::Timer::single_shot(Duration::from_millis(delay_ms), move || {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            ui.window().with_winit_window(|window| {
                recover_and_store_window_position(
                    window,
                    position,
                    &config,
                    &config_saver,
                    true,
                    WindowMode::Collapsed,
                );
            });
        });
    }
}

fn show_native_window(ui: &AppWindow, focus: bool) {
    apply_native_window_state(ui, focus);
    for delay_ms in [50, 250] {
        let ui_weak = ui.as_weak();
        slint::Timer::single_shot(Duration::from_millis(delay_ms), move || {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            apply_native_window_state(&ui, false);
        });
    }
}

fn apply_native_window_state(ui: &AppWindow, focus: bool) {
    ui.window().with_winit_window(|window| {
        window.set_visible(true);
        window.set_skip_taskbar(true);
        if let Err(error) = display::enforce_tool_window(window) {
            eprintln!("failed to hide the assistant taskbar entry: {error}");
        }
        if focus {
            window.focus_window();
        }
    });
}

fn recover_and_store_window_position(
    window: &winit::window::Window,
    requested_position: CorePhysicalPosition,
    config: &RefCell<AppConfig>,
    config_saver: &ConfigSaveHandle,
    force_position: bool,
    mode: WindowMode,
) -> CorePhysicalPosition {
    let window_size = window.outer_size();
    let position = match display::work_areas() {
        Ok(work_areas) => clamp_window_for_mode(
            requested_position,
            CorePhysicalSize {
                width: window_size.width,
                height: window_size.height,
            },
            &work_areas,
            mode,
        )
        .unwrap_or(requested_position),
        Err(error) => {
            eprintln!("failed to enumerate monitor work areas: {error}");
            requested_position
        }
    };

    if force_position
        || position != requested_position
        || window
            .outer_position()
            .ok()
            .is_none_or(|current| current.x != position.x || current.y != position.y)
    {
        match display::set_window_position(window, position) {
            Ok(()) => {}
            Err(error) => eprintln!("failed to set native window position: {error}"),
        }
    }

    let snapshot = {
        let mut config = config.borrow_mut();
        if config.window_position == Some(position) {
            None
        } else {
            config.window_position = Some(position);
            Some(config.clone())
        }
    };
    if let Some(snapshot) = snapshot {
        config_saver.request(snapshot);
    }

    position
}

fn toggle_window(
    ui: &AppWindow,
    tray: &AssistantTray,
    state: &RefCell<WindowState>,
    config: &Rc<RefCell<AppConfig>>,
    config_saver: &ConfigSaveHandle,
) {
    if state.borrow().phase() == ApplicationPhase::Expanded {
        let _ = collapse_window(ui, tray, state, config, config_saver);
        return;
    }
    if state.borrow().phase() != ApplicationPhase::Collapsed {
        return;
    }

    remember_collapsed_position(ui, state);
    let mut state = state.borrow_mut();
    state.toggle();
    ui.set_expanded(true);
    sync_tray_state(tray, &state);
}

fn collapse_window(
    ui: &AppWindow,
    tray: &AssistantTray,
    state: &RefCell<WindowState>,
    config: &Rc<RefCell<AppConfig>>,
    config_saver: &ConfigSaveHandle,
) -> bool {
    let collapsed_position = {
        let mut state = state.borrow_mut();
        if state.collapse().is_none() {
            return false;
        }
        let collapsed_position = state.take_collapsed_position();
        sync_tray_state(tray, &state);
        collapsed_position
    };
    ui.set_expanded(false);
    if let Some(position) = collapsed_position {
        restore_collapsed_position_later(ui, position, Rc::clone(config), config_saver.clone());
    }
    true
}

fn hide_window(ui: &AppWindow, tray: &AssistantTray, state: &RefCell<WindowState>) {
    {
        let mut state = state.borrow_mut();
        if !state.hide() {
            return;
        }
        sync_tray_state(tray, &state);
    }

    ui.window()
        .with_winit_window(|window| window.set_visible(false));
    if let Err(error) = ui.hide() {
        eprintln!("failed to hide the assistant window: {error}");
    }
}

fn restore_window(ui: &AppWindow, tray: &AssistantTray, state: &RefCell<WindowState>) {
    let mut state = state.borrow_mut();
    if state.phase() == ApplicationPhase::Exiting {
        return;
    }
    if let Some(layout) = state.restore() {
        ui.set_expanded(layout.width > app_core::WindowMode::COLLAPSED_LAYOUT.width);
    }
    sync_tray_state(tray, &state);
    drop(state);

    if let Err(error) = ui.show() {
        eprintln!("failed to show the assistant window: {error}");
        return;
    }
    show_native_window(ui, true);
}

fn open_settings(ui: &AppWindow, tray: &AssistantTray, state: &RefCell<WindowState>) {
    restore_window(ui, tray, state);

    if state.borrow().phase() == ApplicationPhase::Collapsed {
        remember_collapsed_position(ui, state);
        state.borrow_mut().toggle();
    }
    let state = state.borrow();
    if state.phase() != ApplicationPhase::Expanded {
        return;
    }
    ui.set_expanded(true);
    ui.set_active_tab(2);
    sync_tray_state(tray, &state);
}

fn toggle_always_on_top(
    ui: &AppWindow,
    tray: &AssistantTray,
    state: &RefCell<WindowState>,
    config: &RefCell<AppConfig>,
    config_saver: &ConfigSaveHandle,
) {
    let enabled = state.borrow_mut().toggle_always_on_top();
    ui.set_always_on_top_enabled(enabled);
    tray.set_always_on_top_enabled(enabled);
    let snapshot = {
        let mut config = config.borrow_mut();
        config.always_on_top = enabled;
        config.clone()
    };
    config_saver.request(snapshot);
}

fn toggle_position_lock(
    ui: &AppWindow,
    tray: &AssistantTray,
    state: &RefCell<WindowState>,
    gesture: &RefCell<DragGesture>,
    config: &RefCell<AppConfig>,
    config_saver: &ConfigSaveHandle,
) {
    let locked = state.borrow_mut().toggle_position_locked();
    if locked {
        gesture.borrow_mut().cancel();
    }
    ui.set_position_locked(locked);
    tray.set_position_locked(locked);
    let snapshot = {
        let mut config = config.borrow_mut();
        config.position_locked = locked;
        config.clone()
    };
    config_saver.request(snapshot);
}

fn toggle_startup(ui: &AppWindow, tray: &AssistantTray, enabled: &Cell<bool>) {
    let requested = !enabled.get();
    if let Err(error) = startup::set_enabled(requested) {
        eprintln!("failed to update startup state: {error}");
        return;
    }

    enabled.set(requested);
    ui.set_startup_enabled(requested);
    tray.set_startup_enabled(requested);
}

fn sync_tray_state(tray: &AssistantTray, state: &WindowState) {
    tray.set_window_visible(matches!(
        state.phase(),
        ApplicationPhase::Collapsed | ApplicationPhase::Expanded
    ));
    tray.set_expanded(state.phase() == ApplicationPhase::Expanded);
    tray.set_always_on_top_enabled(state.always_on_top());
    tray.set_position_locked(state.position_locked());
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    use app_core::{
        PhysicalPosition,
        config::{AppConfig, ConfigStore},
    };
    use system_monitor::{MemoryUsage, MetricValue, SourceStatus, VideoMemoryUsage};

    use super::{
        ConfigWorker, format_gpu, format_memory, format_rate, format_temperature,
        format_video_memory,
    };

    static NEXT_TEMP_DIR: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn config_worker_flushes_the_latest_pending_save_when_dropped() {
        let sequence = NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!(
            "xiaoxi-config-worker-{}-{sequence}",
            std::process::id()
        ));
        let path = directory.join("settings.json");
        let store = ConfigStore::new(&path);
        let worker = ConfigWorker::start(store.clone()).expect("start config worker");
        let handle = worker.handle();
        handle.request(AppConfig::default());
        let mut expected = AppConfig::default();
        expected.window_position = Some(PhysicalPosition { x: 320, y: 180 });
        expected.always_on_top = false;
        expected.position_locked = true;
        handle.request(expected.clone());

        drop(worker);

        assert_eq!(store.load().expect("load saved config").config, expected);
        let _ = fs::remove_dir_all(directory);
    }

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

    #[test]
    fn temperature_text_identifies_the_acpi_source() {
        let (value, detail) = format_temperature(MetricValue {
            value: Some(42.4),
            status: SourceStatus::Available,
        });

        assert_eq!(value, "42°C");
        assert_eq!(detail, "ACPI 热区");
    }

    #[test]
    fn unavailable_temperature_is_explicit() {
        let (value, detail) = format_temperature(MetricValue::<f32> {
            value: None,
            status: SourceStatus::Unavailable,
        });

        assert_eq!(value, "不可用");
        assert_eq!(detail, "未检测到可用温度源");
    }
}

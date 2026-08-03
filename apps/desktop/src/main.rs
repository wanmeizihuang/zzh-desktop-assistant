#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ai_management;
mod conversation;
mod display;
mod single_instance;
mod startup;

use std::{
    cell::{Cell, RefCell},
    env,
    error::Error,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc, Mutex,
        mpsc::{self, RecvTimeoutError, Sender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use agent_cli::{CodexCliConfig, CodexCliConnector, discover_codex_executable};
use agent_core::{
    AgentConnector, AgentEvent, AgentRun, CancellationToken, ConnectorCatalog, ConnectorError,
    ConnectorErrorCode, RequestId, RunPhase,
};
use agent_http::{OpenAiCompatibleConfig, OpenAiCompatibleConnector};
use app_core::{
    ApplicationPhase, DragDecision, DragGesture, PhysicalPosition as CorePhysicalPosition,
    PhysicalSize as CorePhysicalSize, PointerPosition, WindowMode, WindowState,
    agent_profiles::{
        AgentProfileDraft, AgentProfileKind, profile_kind, validate_agent_profile_draft,
    },
    clamp_window_for_mode,
    config::{
        AgentConnectionProfile, AgentProfile, AppConfig, ConfigLoadStatus, ConfigStore, MascotId,
        MonitorSampleInterval,
    },
    credentials::CredentialStore,
};
use conversation::{
    BeginError, BeginRequest, ConversationController, ConversationRole, ConversationView,
};
use slint::winit_030::winit::platform::windows::WindowExtWindows;
use slint::winit_030::{EventResult, WinitWindowAccessor, winit};
use slint::{ComponentHandle, ModelRc, VecModel};
use system_monitor::{
    GpuTemperatureTarget, MetricHistory, MetricSnapshot, MetricValue, SensorServiceStatus,
    SourceStatus, SystemSampler, TemperatureInfo, TemperatureSensor, TemperatureSource,
    prepare_gpu_temperature_sources,
};

slint::include_modules!();

fn main() -> Result<(), Box<dyn Error>> {
    // SAFETY: This is the first operation and no application worker threads exist yet.
    unsafe { prepare_gpu_temperature_sources() };

    let primary_instance = match single_instance::acquire()? {
        single_instance::AcquireResult::Primary(instance) => instance,
        single_instance::AcquireResult::SecondaryNotified => return Ok(()),
    };

    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("software".into())
        .select()?;

    let config_store = ConfigStore::new(local_config_path()?);
    let mut loaded_config = load_config_or_default(&config_store);
    if initialize_managed_models(&mut loaded_config, discover_codex_executable()) {
        log_config_save_result(&config_store, save_config(&config_store, &loaded_config));
    }
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
    let monitor_worker =
        MonitorWorker::start(ui.as_weak(), config.borrow().monitor_sample_interval)?;
    let monitor_control = monitor_worker.control();
    let conversation_runtime = Rc::new(ConversationRuntime::from_config(&config.borrow()));

    state.borrow_mut().start();
    ui.set_always_on_top_enabled(state.borrow().always_on_top());
    ui.set_position_locked(state.borrow().position_locked());
    ui.set_startup_enabled(startup_enabled.get());
    ui.set_mascot_id(config.borrow().mascot_id.ui_index());
    ui.set_monitor_sample_interval(config.borrow().monitor_sample_interval.ui_index());
    initialize_conversation_ui(&ui, &conversation_runtime);
    sync_ai_management_ui(&ui, &config.borrow(), &conversation_runtime);
    sync_tray_state(&tray, &state);
    tray.set_startup_enabled(startup_enabled.get());

    let monitor_control_for_ui = monitor_control.clone();
    ui.on_monitor_activity_changed(move |window_visible, expanded, active_tab| {
        monitor_control_for_ui.set_activity(monitor_activity(window_visible, expanded, active_tab));
    });

    let monitor_control_for_interval = monitor_control.clone();
    let config_for_interval = Rc::clone(&config);
    let saver_for_interval = config_worker.handle();
    let ui_weak = ui.as_weak();
    ui.on_monitor_sample_interval_selection_requested(move |interval_index| {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        select_monitor_sample_interval(
            &ui,
            &config_for_interval,
            &saver_for_interval,
            &monitor_control_for_interval,
            interval_index,
        );
    });
    notify_monitor_activity(&ui);

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
    register_conversation_callbacks(
        &ui,
        Rc::clone(&conversation_runtime),
        Rc::clone(&config),
        config_worker.handle(),
    );
    register_ai_management_callbacks(
        &ui,
        Rc::clone(&conversation_runtime),
        Rc::clone(&config),
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
    conversation_runtime.cancel_active();
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

    config_path_for_local_app_data(Path::new(&local_app_data))
}

fn config_path_for_local_app_data(local_app_data: &Path) -> io::Result<PathBuf> {
    let current_path = local_app_data
        .join("ZZH Desktop Assistant")
        .join("settings.json");
    let legacy_path = local_app_data
        .join("Xiaoxi Desktop Assistant")
        .join("settings.json");

    migrate_legacy_config(&legacy_path, &current_path)?;
    Ok(current_path)
}

fn migrate_legacy_config(legacy_path: &Path, current_path: &Path) -> io::Result<bool> {
    if current_path.exists() || !legacy_path.is_file() {
        return Ok(false);
    }

    let Some(parent) = current_path.parent() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "configuration path has no parent directory",
        ));
    };
    fs::create_dir_all(parent)?;

    let temporary_path = current_path.with_extension("migration.tmp");
    fs::copy(legacy_path, &temporary_path)?;
    if let Err(error) = fs::rename(&temporary_path, current_path) {
        let _ = fs::remove_file(&temporary_path);
        return Err(error);
    }

    Ok(true)
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

#[derive(Clone, Debug)]
struct RuntimeAgentProfile {
    id: String,
    name: String,
    status: String,
    available: bool,
}

struct ConversationRuntime {
    profiles: RefCell<Vec<RuntimeAgentProfile>>,
    catalog: RefCell<ConnectorCatalog>,
    selected_index: Cell<usize>,
    controller: Arc<Mutex<ConversationController>>,
    active_cancel: Arc<Mutex<Option<(RequestId, CancellationToken)>>>,
}

impl ConversationRuntime {
    fn from_config(config: &AppConfig) -> Self {
        let (profiles, catalog, selected_index) = build_runtime_profiles(config);
        Self {
            profiles: RefCell::new(profiles),
            catalog: RefCell::new(catalog),
            selected_index: Cell::new(selected_index),
            controller: Arc::new(Mutex::new(ConversationController::new())),
            active_cancel: Arc::new(Mutex::new(None)),
        }
    }

    fn reload_profiles(&self, config: &AppConfig) {
        let (profiles, catalog, selected_index) = build_runtime_profiles(config);
        self.profiles.replace(profiles);
        self.catalog.replace(catalog);
        self.selected_index.set(selected_index);
        lock_unpoisoned(&self.controller).clear_retry();
    }

    fn selected_profile(&self) -> Option<RuntimeAgentProfile> {
        self.profiles
            .borrow()
            .get(self.selected_index.get())
            .cloned()
    }

    fn is_busy(&self) -> bool {
        lock_unpoisoned(&self.controller).view().busy
    }

    fn cancel_active(&self) {
        if let Some((_, token)) = lock_unpoisoned(&self.active_cancel).take() {
            token.cancel();
        }
    }
}

fn build_runtime_profiles(
    config: &AppConfig,
) -> (Vec<RuntimeAgentProfile>, ConnectorCatalog, usize) {
    let mut profiles = Vec::new();
    let mut catalog = ConnectorCatalog::new();

    for profile in &config.agent_profiles {
        let (status, connector) = build_configured_connector(profile);
        push_runtime_profile(
            &mut profiles,
            &mut catalog,
            &profile.id,
            &profile.display_name,
            &status,
            connector,
        );
    }

    let selected_index = config
        .selected_agent_profile_id
        .as_deref()
        .and_then(|selected| profiles.iter().position(|profile| profile.id == selected))
        .or_else(|| profiles.iter().position(|profile| profile.available))
        .unwrap_or(0);

    (profiles, catalog, selected_index)
}

fn initialize_managed_models(config: &mut AppConfig, codex_executable: Option<PathBuf>) -> bool {
    if config.agent_management_initialized {
        return false;
    }
    if config.agent_profiles.is_empty()
        && let Some(executable) = codex_executable
    {
        config.agent_profiles.push(AgentProfile {
            id: "codex-cli".into(),
            display_name: "Codex CLI".into(),
            connection: AgentConnectionProfile::Cli {
                executable,
                arguments: Vec::new(),
            },
            credential_reference: None,
        });
        config.selected_agent_profile_id = Some("codex-cli".into());
    }
    config.agent_management_initialized = true;
    true
}

fn build_configured_connector(
    profile: &AgentProfile,
) -> (String, Result<Arc<dyn AgentConnector>, String>) {
    match &profile.connection {
        AgentConnectionProfile::Http {
            endpoint,
            model,
            deployment,
        } => {
            let token = match &profile.credential_reference {
                Some(reference) => match CredentialStore::new().read(&reference.target) {
                    Ok(Some(secret)) => Some(secret.expose_secret().to_owned()),
                    Ok(None) => {
                        return (
                            "HTTP · 缺少凭据".into(),
                            Err("Windows 凭据管理器中未找到所配置的凭据".into()),
                        );
                    }
                    Err(error) => {
                        return (
                            "HTTP · 凭据不可用".into(),
                            Err(format!("无法读取所配置的凭据：{error}")),
                        );
                    }
                },
                None => None,
            };
            let config = OpenAiCompatibleConfig::new(endpoint, model)
                .with_local_execution(*deployment == app_core::config::HttpDeployment::Local);
            let connector = OpenAiCompatibleConnector::new(
                profile.id.clone(),
                profile.display_name.clone(),
                config,
                token,
            )
            .map(|connector| Arc::new(connector) as Arc<dyn AgentConnector>)
            .map_err(|error| error.message);
            let location = if *deployment == app_core::config::HttpDeployment::Local {
                "本地服务"
            } else {
                "云端 API"
            };
            (format!("{location} · {model}"), connector)
        }
        AgentConnectionProfile::Cli {
            executable,
            arguments,
        } => {
            let config = CodexCliConfig::new(executable).map(|config| {
                config.with_extra_args(normalize_codex_arguments(arguments).map(OsString::from))
            });
            let connector = config
                .map(|config| {
                    CodexCliConnector::new_managed(
                        profile.id.clone(),
                        profile.display_name.clone(),
                        config,
                    )
                })
                .map(|connector| Arc::new(connector) as Arc<dyn AgentConnector>)
                .map_err(|error| error.message);
            ("本地 CLI · 可用".into(), connector)
        }
    }
}

fn normalize_codex_arguments(arguments: &[String]) -> impl Iterator<Item = &str> {
    let mut start = 0;
    if arguments.get(start).is_some_and(|value| value == "exec") {
        start += 1;
    }
    if arguments.get(start).is_some_and(|value| value == "--json") {
        start += 1;
    }
    arguments[start..].iter().map(String::as_str)
}

fn push_runtime_profile(
    profiles: &mut Vec<RuntimeAgentProfile>,
    catalog: &mut ConnectorCatalog,
    id: &str,
    name: &str,
    available_status: &str,
    connector: Result<Arc<dyn AgentConnector>, String>,
) {
    let (available, status) = match connector {
        Ok(connector) => match catalog.register(connector) {
            Ok(()) => (true, available_status.to_owned()),
            Err(error) => (false, format!("配置不可用：{error}")),
        },
        Err(error) => (false, error),
    };
    profiles.push(RuntimeAgentProfile {
        id: id.to_owned(),
        name: name.to_owned(),
        status,
        available,
    });
}

fn initialize_conversation_ui(ui: &AppWindow, runtime: &ConversationRuntime) {
    let choices = runtime
        .profiles
        .borrow()
        .iter()
        .map(|profile| AgentChoice {
            name: profile.name.clone().into(),
            status: profile.status.clone().into(),
            available: profile.available,
        })
        .collect::<Vec<_>>();
    ui.set_agent_choices(ModelRc::new(VecModel::from(choices)));
    sync_selected_agent(ui, runtime);
    let view = lock_unpoisoned(&runtime.controller).view();
    apply_conversation_view(ui, &view);
}

fn sync_ai_management_ui(ui: &AppWindow, config: &AppConfig, runtime: &ConversationRuntime) {
    let runtime_profiles = runtime.profiles.borrow();
    let rows = config
        .agent_profiles
        .iter()
        .map(|profile| {
            let runtime_profile = runtime_profiles
                .iter()
                .find(|candidate| candidate.id == profile.id);
            ManagedAiRow {
                name: profile.display_name.clone().into(),
                kind: match profile_kind(profile) {
                    AgentProfileKind::CloudHttp => "云端 API",
                    AgentProfileKind::LocalHttp => "本地服务",
                    AgentProfileKind::CodexCli => "Codex CLI",
                }
                .into(),
                detail: runtime_profile
                    .map_or_else(
                        || "配置尚未加载".to_owned(),
                        |profile| profile.status.clone(),
                    )
                    .into(),
                available: runtime_profile.is_some_and(|profile| profile.available),
            }
        })
        .collect::<Vec<_>>();
    ui.set_managed_ai_models(ModelRc::new(VecModel::from(rows)));
}

fn register_ai_management_callbacks(
    ui: &AppWindow,
    runtime: Rc<ConversationRuntime>,
    config: Rc<RefCell<AppConfig>>,
    config_saver: ConfigSaveHandle,
) {
    let ui_weak = ui.as_weak();
    ui.on_ai_manager_open_requested(move || {
        if let Some(ui) = ui_weak.upgrade() {
            reset_ai_editor(&ui);
            ui.set_ai_manager_open(true);
        }
    });

    let ui_weak = ui.as_weak();
    ui.on_ai_manager_close_requested(move || {
        if let Some(ui) = ui_weak.upgrade() {
            reset_ai_editor(&ui);
            ui.set_ai_manager_open(false);
        }
    });

    let ui_weak = ui.as_weak();
    let runtime_for_new = Rc::clone(&runtime);
    ui.on_ai_model_new_requested(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        if runtime_for_new.is_busy() {
            return;
        }
        reset_ai_editor(&ui);
        ui.set_ai_editor_open(true);
    });

    let ui_weak = ui.as_weak();
    let runtime_for_edit = Rc::clone(&runtime);
    let config_for_edit = Rc::clone(&config);
    ui.on_ai_model_edit_requested(move |index| {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        if runtime_for_edit.is_busy() || index < 0 {
            return;
        }
        let config = config_for_edit.borrow();
        let Some(profile) = config.agent_profiles.get(index as usize) else {
            return;
        };
        let draft = ai_management::draft_from_profile(profile);
        ui.set_ai_editor_index(index);
        ui.set_ai_editor_kind(draft.kind.ui_index());
        ui.set_ai_editor_name(draft.display_name.into());
        ui.set_ai_editor_endpoint(draft.endpoint.into());
        ui.set_ai_editor_model(draft.model.into());
        ui.set_ai_editor_executable(draft.executable.into());
        ui.set_ai_editor_arguments(ai_management::arguments_editor_text(&draft.arguments).into());
        ui.set_ai_editor_has_api_key(profile.credential_reference.is_some());
        ui.set_ai_editor_clear_api_key(false);
        ui.set_ai_editor_api_key("".into());
        ui.set_ai_management_message("".into());
        ui.set_ai_editor_open(true);
    });

    let config_for_check = Rc::clone(&config);
    ui.on_ai_model_check_requested(
        move |index, kind, name, endpoint, model, executable, arguments| {
            let config = config_for_check.borrow();
            match draft_from_editor(
                &config,
                index,
                kind,
                name.as_str(),
                endpoint.as_str(),
                model.as_str(),
                executable.as_str(),
                arguments.as_str(),
            )
            .and_then(|draft| check_model_draft(&draft, &config))
            {
                Ok(()) => "配置格式有效".into(),
                Err(error) => error.into(),
            }
        },
    );

    let ui_weak = ui.as_weak();
    let runtime_for_save = Rc::clone(&runtime);
    let config_for_save = Rc::clone(&config);
    let saver_for_save = config_saver.clone();
    ui.on_ai_model_save_requested(
        move |index, kind, name, endpoint, model, executable, arguments, api_key, clear_api_key| {
            let Some(ui) = ui_weak.upgrade() else {
                return false;
            };
            if runtime_for_save.is_busy() {
                ui.set_ai_management_message("回答进行中，暂不能修改模型".into());
                return false;
            }
            let result = {
                let current = config_for_save.borrow();
                draft_from_editor(
                    &current,
                    index,
                    kind,
                    name.as_str(),
                    endpoint.as_str(),
                    model.as_str(),
                    executable.as_str(),
                    arguments.as_str(),
                )
                .and_then(|draft| {
                    let credential_edit = if !api_key.trim().is_empty() {
                        ai_management::CredentialEdit::Replace(api_key.to_string())
                    } else if clear_api_key {
                        ai_management::CredentialEdit::Clear
                    } else {
                        ai_management::CredentialEdit::Keep
                    };
                    ai_management::save_profile(
                        &current,
                        &draft,
                        credential_edit,
                        profile_id_seed(),
                        &CredentialStore::new(),
                        &saver_for_save,
                    )
                    .map_err(|error| error.to_string())
                })
            };
            match result {
                Ok(next) => {
                    config_for_save.replace(next.clone());
                    runtime_for_save.reload_profiles(&next);
                    initialize_conversation_ui(&ui, &runtime_for_save);
                    sync_ai_management_ui(&ui, &next, &runtime_for_save);
                    true
                }
                Err(error) => {
                    ui.set_ai_management_message(error.into());
                    false
                }
            }
        },
    );

    let ui_weak = ui.as_weak();
    let runtime_for_delete_request = Rc::clone(&runtime);
    ui.on_ai_model_delete_requested(move |index| {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        if runtime_for_delete_request.is_busy() || index < 0 {
            return;
        }
        ui.set_ai_editor_index(index);
        ui.set_ai_delete_confirm_open(true);
    });

    let ui_weak = ui.as_weak();
    let runtime_for_delete = Rc::clone(&runtime);
    let config_for_delete = Rc::clone(&config);
    ui.on_ai_model_delete_confirmed(move |index| {
        let Some(ui) = ui_weak.upgrade() else {
            return false;
        };
        if runtime_for_delete.is_busy() || index < 0 {
            return false;
        }
        let result = {
            let current = config_for_delete.borrow();
            let Some(profile) = current.agent_profiles.get(index as usize) else {
                ui.set_ai_management_message("要删除的模型已不存在".into());
                return false;
            };
            ai_management::delete_profile(
                &current,
                &profile.id,
                &CredentialStore::new(),
                &config_saver,
            )
            .map_err(|error| error.to_string())
        };
        match result {
            Ok(next) => {
                config_for_delete.replace(next.clone());
                runtime_for_delete.reload_profiles(&next);
                initialize_conversation_ui(&ui, &runtime_for_delete);
                sync_ai_management_ui(&ui, &next, &runtime_for_delete);
                ui.set_ai_editor_index(-1);
                true
            }
            Err(error) => {
                ui.set_ai_management_message(error.into());
                false
            }
        }
    });
}

fn reset_ai_editor(ui: &AppWindow) {
    ui.set_ai_editor_open(false);
    ui.set_ai_delete_confirm_open(false);
    ui.set_ai_editor_index(-1);
    ui.set_ai_editor_kind(0);
    ui.set_ai_editor_name("".into());
    ui.set_ai_editor_endpoint("".into());
    ui.set_ai_editor_model("".into());
    ui.set_ai_editor_executable("codex.exe".into());
    ui.set_ai_editor_arguments("".into());
    ui.set_ai_editor_api_key("".into());
    ui.set_ai_editor_has_api_key(false);
    ui.set_ai_editor_clear_api_key(false);
    ui.set_ai_management_message("".into());
}

#[allow(clippy::too_many_arguments)]
fn draft_from_editor(
    config: &AppConfig,
    index: i32,
    kind: i32,
    name: &str,
    endpoint: &str,
    model: &str,
    executable: &str,
    arguments: &str,
) -> Result<AgentProfileDraft, String> {
    let kind = AgentProfileKind::from_ui_index(kind).ok_or_else(|| "模型类型无效".to_owned())?;
    let existing_id = if index < 0 {
        None
    } else {
        Some(
            config
                .agent_profiles
                .get(index as usize)
                .ok_or_else(|| "要编辑的模型已不存在".to_owned())?
                .id
                .clone(),
        )
    };
    Ok(AgentProfileDraft {
        existing_id,
        display_name: name.to_owned(),
        kind,
        endpoint: endpoint.to_owned(),
        model: model.to_owned(),
        executable: executable.to_owned(),
        arguments: ai_management::parse_argument_lines(arguments),
    })
}

fn check_model_draft(draft: &AgentProfileDraft, config: &AppConfig) -> Result<(), String> {
    validate_agent_profile_draft(draft, &config.agent_profiles)
        .map_err(|error| error.to_string())?;
    if draft.kind == AgentProfileKind::CodexCli {
        CodexCliConfig::new(PathBuf::from(draft.executable.trim()))
            .map_err(|error| error.message)?;
    }
    Ok(())
}

fn profile_id_seed() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}

fn register_conversation_callbacks(
    ui: &AppWindow,
    runtime: Rc<ConversationRuntime>,
    config: Rc<RefCell<AppConfig>>,
    config_saver: ConfigSaveHandle,
) {
    let ui_weak = ui.as_weak();
    let runtime_for_selection = Rc::clone(&runtime);
    let config_for_selection = Rc::clone(&config);
    let saver_for_selection = config_saver.clone();
    ui.on_agent_selection_requested(move |index| {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        if index < 0
            || lock_unpoisoned(&runtime_for_selection.controller)
                .view()
                .busy
        {
            return;
        }
        let index = index as usize;
        let Some(profile) = runtime_for_selection.profiles.borrow().get(index).cloned() else {
            return;
        };
        runtime_for_selection.selected_index.set(index);
        lock_unpoisoned(&runtime_for_selection.controller).clear_retry();
        let snapshot = {
            let mut config = config_for_selection.borrow_mut();
            config.selected_agent_profile_id = Some(profile.id.clone());
            config.clone()
        };
        saver_for_selection.request(snapshot);
        sync_selected_agent(&ui, &runtime_for_selection);
        let view = lock_unpoisoned(&runtime_for_selection.controller).view();
        apply_conversation_view(&ui, &view);
    });

    let ui_weak = ui.as_weak();
    let runtime_for_send = Rc::clone(&runtime);
    ui.on_agent_send_requested(move |prompt| {
        let Some(ui) = ui_weak.upgrade() else {
            return false;
        };
        let Some(profile) = runtime_for_send.selected_profile() else {
            return false;
        };
        if !profile.available {
            return false;
        }
        let begin = match lock_unpoisoned(&runtime_for_send.controller).begin(
            &profile.id,
            &profile.name,
            prompt.as_str(),
        ) {
            Ok(begin) => begin,
            Err(BeginError::Busy | BeginError::InvalidPrompt | BeginError::RetryUnavailable) => {
                return false;
            }
        };
        let view = lock_unpoisoned(&runtime_for_send.controller).view();
        apply_conversation_view(&ui, &view);
        start_conversation_request(&ui, &runtime_for_send, begin);
        true
    });

    let ui_weak = ui.as_weak();
    let runtime_for_stop = Rc::clone(&runtime);
    ui.on_agent_stop_requested(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let request_id = lock_unpoisoned(&runtime_for_stop.controller).request_stop();
        if let Some(request_id) = request_id
            && let Some((active_id, token)) =
                lock_unpoisoned(&runtime_for_stop.active_cancel).as_ref()
            && *active_id == request_id
        {
            token.cancel();
        }
        let view = lock_unpoisoned(&runtime_for_stop.controller).view();
        apply_conversation_view(&ui, &view);
    });

    let ui_weak = ui.as_weak();
    let runtime_for_retry = Rc::clone(&runtime);
    ui.on_agent_retry_requested(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let begin = match lock_unpoisoned(&runtime_for_retry.controller).retry() {
            Ok(begin) => begin,
            Err(_) => return,
        };
        let view = lock_unpoisoned(&runtime_for_retry.controller).view();
        apply_conversation_view(&ui, &view);
        start_conversation_request(&ui, &runtime_for_retry, begin);
    });
}

fn start_conversation_request(ui: &AppWindow, runtime: &ConversationRuntime, begin: BeginRequest) {
    let Some(connector) = runtime.catalog.borrow().connector(&begin.connector_id) else {
        fail_conversation_start(
            ui,
            &runtime.controller,
            ConnectorError::new(ConnectorErrorCode::Configuration, "所选智能体连接器不可用"),
        );
        return;
    };
    let request_id = begin.request.id;
    let run = match connector.start(begin.request) {
        Ok(run) => run,
        Err(error) => {
            fail_conversation_start(ui, &runtime.controller, error);
            return;
        }
    };
    let token = run.cancellation_token();
    *lock_unpoisoned(&runtime.active_cancel) = Some((request_id, token));

    let controller = Arc::clone(&runtime.controller);
    let active_cancel = Arc::clone(&runtime.active_cancel);
    let failure_controller = Arc::clone(&runtime.controller);
    let failure_active_cancel = Arc::clone(&runtime.active_cancel);
    let ui_weak = ui.as_weak();
    let spawn_result = thread::Builder::new()
        .name(format!("agent-run-{}", request_id.0))
        .spawn(move || {
            consume_agent_run(run, request_id, controller, active_cancel, ui_weak);
        });
    if let Err(error) = spawn_result {
        clear_active_cancellation(&failure_active_cancel, request_id);
        fail_conversation_start(
            ui,
            &failure_controller,
            ConnectorError::new(
                ConnectorErrorCode::Process,
                format!("无法启动智能体事件线程：{error}"),
            ),
        );
    }
}

fn fail_conversation_start(
    ui: &AppWindow,
    controller: &Arc<Mutex<ConversationController>>,
    error: ConnectorError,
) {
    if let Err(sequence_error) = lock_unpoisoned(controller).fail_to_start(error) {
        eprintln!("failed to record connector start failure: {sequence_error:?}");
    }
    let view = lock_unpoisoned(controller).view();
    apply_conversation_view(ui, &view);
}

fn consume_agent_run(
    run: AgentRun,
    request_id: RequestId,
    controller: Arc<Mutex<ConversationController>>,
    active_cancel: Arc<Mutex<Option<(RequestId, CancellationToken)>>>,
    ui_weak: slint::Weak<AppWindow>,
) {
    const UI_THROTTLE: Duration = Duration::from_millis(50);
    let mut last_dispatch = Instant::now();
    loop {
        match run.recv_timeout(UI_THROTTLE) {
            Ok(event) => {
                let dispatch_immediately = !matches!(event, AgentEvent::TextDelta { .. });
                let outcome = lock_unpoisoned(&controller).apply_event(event);
                match outcome {
                    Ok(outcome) => {
                        if outcome.cancel_transport {
                            run.cancel();
                        }
                        if outcome.terminal {
                            clear_active_cancellation(&active_cancel, request_id);
                        }
                        if dispatch_immediately
                            || outcome.terminal
                            || last_dispatch.elapsed() >= UI_THROTTLE
                        {
                            dispatch_conversation_view(
                                &ui_weak,
                                lock_unpoisoned(&controller).view(),
                            );
                            last_dispatch = Instant::now();
                        }
                        if outcome.terminal {
                            return;
                        }
                    }
                    Err(error) => {
                        eprintln!("rejected invalid agent event: {error:?}");
                        run.cancel();
                        finish_closed_conversation(
                            request_id,
                            &controller,
                            &active_cancel,
                            &ui_weak,
                        );
                        return;
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                finish_closed_conversation(request_id, &controller, &active_cancel, &ui_weak);
                return;
            }
        }
    }
}

fn finish_closed_conversation(
    request_id: RequestId,
    controller: &Arc<Mutex<ConversationController>>,
    active_cancel: &Arc<Mutex<Option<(RequestId, CancellationToken)>>>,
    ui_weak: &slint::Weak<AppWindow>,
) {
    if let Err(error) = lock_unpoisoned(controller).channel_closed(request_id) {
        eprintln!("failed to close interrupted agent stream: {error:?}");
    }
    clear_active_cancellation(active_cancel, request_id);
    dispatch_conversation_view(ui_weak, lock_unpoisoned(controller).view());
}

fn clear_active_cancellation(
    active_cancel: &Arc<Mutex<Option<(RequestId, CancellationToken)>>>,
    request_id: RequestId,
) {
    let mut active = lock_unpoisoned(active_cancel);
    if active
        .as_ref()
        .is_some_and(|(active_id, _)| *active_id == request_id)
    {
        active.take();
    }
}

fn dispatch_conversation_view(ui_weak: &slint::Weak<AppWindow>, view: ConversationView) {
    if ui_weak
        .upgrade_in_event_loop(move |ui| apply_conversation_view(&ui, &view))
        .is_err()
    {
        eprintln!("failed to dispatch agent state to the UI event loop");
    }
}

fn apply_conversation_view(ui: &AppWindow, view: &ConversationView) {
    let rows = view
        .messages
        .iter()
        .map(|message| ConversationRow {
            kind: match message.role {
                ConversationRole::User => 0,
                ConversationRole::Assistant => 1,
            },
            author: message.author.clone().into(),
            text: message.text.clone().into(),
            pending: message.pending,
            failed: message.failed,
        })
        .collect::<Vec<_>>();
    ui.set_conversation_messages(ModelRc::new(VecModel::from(rows)));
    ui.set_conversation_busy(view.busy);
    ui.set_conversation_stopping(view.stopping);
    ui.set_conversation_can_retry(view.can_retry);
    ui.set_conversation_status(conversation_status(view).into());
}

fn conversation_status(view: &ConversationView) -> String {
    if view.stopping {
        return "正在停止…".into();
    }
    if let Some(error) = &view.error {
        return error.clone();
    }
    match view.phase {
        Some(RunPhase::AwaitingStart) => "正在连接…".into(),
        Some(RunPhase::Streaming) => "正在回答…".into(),
        Some(RunPhase::Completed) => "已完成".into(),
        Some(RunPhase::Cancelled) => "已停止".into(),
        Some(RunPhase::Failed) => "请求失败".into(),
        None => "暂无对话".into(),
    }
}

fn sync_selected_agent(ui: &AppWindow, runtime: &ConversationRuntime) {
    let Some(profile) = runtime.selected_profile() else {
        ui.set_selected_agent_index(-1);
        ui.set_selected_agent_name("未添加模型".into());
        ui.set_selected_agent_status("请在设置的 AI 管理中添加".into());
        ui.set_selected_agent_available(false);
        return;
    };
    ui.set_selected_agent_index(runtime.selected_index.get() as i32);
    ui.set_selected_agent_name(profile.name.clone().into());
    ui.set_selected_agent_status(profile.status.clone().into());
    ui.set_selected_agent_available(profile.available);
}

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

enum ConfigCommand {
    Save(AppConfig),
    SaveImmediately(AppConfig, mpsc::SyncSender<Result<(), String>>),
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

    fn save_immediately(&self, config: AppConfig) -> Result<(), String> {
        let (reply_sender, reply_receiver) = mpsc::sync_channel(1);
        self.sender
            .send(ConfigCommand::SaveImmediately(config, reply_sender))
            .map_err(|_| "配置写入线程已停止".to_owned())?;
        reply_receiver
            .recv()
            .map_err(|_| "配置写入线程未返回保存结果".to_owned())?
    }
}

impl ai_management::ConfigPersistence for ConfigSaveHandle {
    fn save_immediately(&self, config: &AppConfig) -> Result<(), String> {
        self.save_immediately(config.clone())
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
            let mut pending = match command {
                ConfigCommand::Save(config) => config,
                ConfigCommand::SaveImmediately(config, reply) => {
                    let _ = reply.send(save_config(&store, &config));
                    continue;
                }
                ConfigCommand::Stop => return,
            };

            loop {
                match receiver.recv_timeout(Self::SAVE_DELAY) {
                    Ok(ConfigCommand::Save(config)) => pending = config,
                    Ok(ConfigCommand::SaveImmediately(config, reply)) => {
                        let _ = reply.send(save_config(&store, &config));
                        break;
                    }
                    Ok(ConfigCommand::Stop) | Err(RecvTimeoutError::Disconnected) => {
                        log_config_save_result(&store, save_config(&store, &pending));
                        return;
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        log_config_save_result(&store, save_config(&store, &pending));
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

fn save_config(store: &ConfigStore, config: &AppConfig) -> Result<(), String> {
    store
        .save(config)
        .map_err(|error| format!("无法保存配置到 {}：{error}", store.path().display()))
}

fn log_config_save_result(store: &ConfigStore, result: Result<(), String>) {
    if let Err(error) = result {
        eprintln!(
            "failed to save configuration to {}: {error}",
            store.path().display()
        );
    }
}

enum MonitorCommand {
    SetActivity(MonitorActivity),
    SetInteractiveInterval(MonitorSampleInterval),
    Stop,
}

#[derive(Clone)]
struct MonitorControl {
    sender: Sender<MonitorCommand>,
}

impl MonitorControl {
    fn set_activity(&self, activity: MonitorActivity) {
        if self
            .sender
            .send(MonitorCommand::SetActivity(activity))
            .is_err()
        {
            eprintln!("monitor worker is no longer available");
        }
    }

    fn set_interactive_interval(&self, interval: MonitorSampleInterval) {
        if self
            .sender
            .send(MonitorCommand::SetInteractiveInterval(interval))
            .is_err()
        {
            eprintln!("monitor worker is no longer available");
        }
    }
}

struct MonitorWorker {
    control: MonitorControl,
    thread: Option<JoinHandle<()>>,
}

const TREND_POINT_LIMIT: usize = 48;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MonitorActivity {
    Interactive,
    Background,
    Hidden,
}

impl MonitorActivity {
    fn sample_interval(self, interactive_interval: MonitorSampleInterval) -> Duration {
        match self {
            Self::Interactive => Duration::from_secs(interactive_interval.seconds()),
            Self::Background => Duration::from_secs(10),
            Self::Hidden => Duration::from_secs(20),
        }
    }

    fn should_dispatch_ui(self) -> bool {
        self == Self::Interactive
    }
}

fn monitor_activity(window_visible: bool, expanded: bool, active_tab: i32) -> MonitorActivity {
    if !window_visible {
        MonitorActivity::Hidden
    } else if expanded && active_tab == 0 {
        MonitorActivity::Interactive
    } else {
        MonitorActivity::Background
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct TrendSeriesPoint {
    primary: Option<f32>,
    secondary: Option<f32>,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct MonitorTrends {
    cpu: Vec<TrendSeriesPoint>,
    memory: Vec<TrendSeriesPoint>,
    network_received: Vec<TrendSeriesPoint>,
    network_transmitted: Vec<TrendSeriesPoint>,
}

fn build_monitor_trends(history: &MetricHistory) -> MonitorTrends {
    let cpu = history
        .trend_points(TREND_POINT_LIMIT, |snapshot| {
            available_value(snapshot.cpu_total_percent)
        })
        .into_iter()
        .map(|point| point.value)
        .collect();
    let memory = history
        .trend_points(TREND_POINT_LIMIT, |snapshot| {
            available_value(snapshot.memory).map(|memory| memory.used_percent())
        })
        .into_iter()
        .map(|point| point.value)
        .collect();
    let received = history
        .trend_points(TREND_POINT_LIMIT, |snapshot| {
            available_value(snapshot.network).map(|network| network.received_bytes_per_second)
        })
        .into_iter()
        .map(|point| point.value)
        .collect();
    let transmitted = history
        .trend_points(TREND_POINT_LIMIT, |snapshot| {
            available_value(snapshot.network).map(|network| network.transmitted_bytes_per_second)
        })
        .into_iter()
        .map(|point| point.value)
        .collect();

    let (network_received, network_transmitted) = normalize_network_trends(received, transmitted);
    MonitorTrends {
        cpu: normalize_percent_trends(cpu),
        memory: normalize_percent_trends(memory),
        network_received,
        network_transmitted,
    }
}

fn available_value<T: Copy>(metric: MetricValue<T>) -> Option<T> {
    (metric.status == SourceStatus::Available)
        .then_some(metric.value)
        .flatten()
}

fn normalize_percent_trends(values: Vec<Option<f32>>) -> Vec<TrendSeriesPoint> {
    values
        .into_iter()
        .map(|value| TrendSeriesPoint {
            primary: value
                .filter(|value| value.is_finite())
                .map(|value| value.clamp(0.0, 100.0) / 100.0),
            secondary: None,
        })
        .collect()
}

fn normalize_network_trends(
    received: Vec<Option<f64>>,
    transmitted: Vec<Option<f64>>,
) -> (Vec<TrendSeriesPoint>, Vec<TrendSeriesPoint>) {
    let valid_value = |value: f64| value.is_finite().then_some(value.max(0.0));
    let scale = received
        .iter()
        .chain(&transmitted)
        .filter_map(|value| value.and_then(valid_value))
        .fold(0.0_f64, f64::max);
    let normalize = |value: Option<f64>| {
        value.and_then(valid_value).map(|value| {
            if scale <= f64::EPSILON {
                0.0
            } else {
                (value / scale).clamp(0.0, 1.0) as f32
            }
        })
    };
    let output_len = received.len().max(transmitted.len());

    let normalized_received = (0..output_len)
        .map(|index| TrendSeriesPoint {
            primary: normalize(received.get(index).copied().flatten()),
            secondary: None,
        })
        .collect();
    let normalized_transmitted = (0..output_len)
        .map(|index| TrendSeriesPoint {
            primary: normalize(transmitted.get(index).copied().flatten()),
            secondary: None,
        })
        .collect();

    (normalized_received, normalized_transmitted)
}

fn should_update_trends(window_visible: bool, expanded: bool, active_tab: i32) -> bool {
    window_visible && expanded && active_tab == 0
}

impl MonitorWorker {
    fn start(
        ui_weak: slint::Weak<AppWindow>,
        initial_interactive_interval: MonitorSampleInterval,
    ) -> std::io::Result<Self> {
        let (command_sender, command_receiver) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("system-monitor".into())
            .spawn(move || {
                let mut sampler = SystemSampler::new();
                let mut history = MetricHistory::default();
                let mut activity = MonitorActivity::Background;
                let mut interactive_interval = initial_interactive_interval;
                let mut delay = Duration::from_millis(250);

                loop {
                    match command_receiver.recv_timeout(delay) {
                        Ok(MonitorCommand::SetActivity(next_activity)) => {
                            if next_activity == activity {
                                continue;
                            }
                            activity = next_activity;
                            if !activity.should_dispatch_ui() {
                                delay = activity.sample_interval(interactive_interval);
                                continue;
                            }
                        }
                        Ok(MonitorCommand::SetInteractiveInterval(next_interval)) => {
                            interactive_interval = next_interval;
                            delay = activity.sample_interval(interactive_interval);
                            continue;
                        }
                        Ok(MonitorCommand::Stop) | Err(RecvTimeoutError::Disconnected) => break,
                        Err(RecvTimeoutError::Timeout) => {}
                    }

                    let snapshot = sampler.sample();
                    if !history.push(snapshot) {
                        delay = activity.sample_interval(interactive_interval);
                        continue;
                    }

                    if activity.should_dispatch_ui() {
                        let trends = build_monitor_trends(&history);
                        if ui_weak
                            .upgrade_in_event_loop(move |ui| {
                                if !should_update_trends(
                                    ui.get_window_visible(),
                                    ui.get_expanded(),
                                    ui.get_active_tab(),
                                ) {
                                    return;
                                }
                                apply_snapshot(&ui, snapshot);
                                apply_trends(&ui, trends);
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    delay = activity.sample_interval(interactive_interval);
                }
            })?;

        Ok(Self {
            control: MonitorControl {
                sender: command_sender,
            },
            thread: Some(thread),
        })
    }

    fn control(&self) -> MonitorControl {
        self.control.clone()
    }
}

impl Drop for MonitorWorker {
    fn drop(&mut self) {
        let _ = self.control.sender.send(MonitorCommand::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn notify_monitor_activity(ui: &AppWindow) {
    ui.invoke_monitor_activity_changed(
        ui.get_window_visible(),
        ui.get_expanded(),
        ui.get_active_tab(),
    );
}

fn apply_snapshot(ui: &AppWindow, snapshot: MetricSnapshot) {
    let (cpu_value, cpu_detail) = format_cpu(snapshot.cpu_total_percent);
    ui.set_cpu_value(cpu_value.into());
    ui.set_cpu_detail(cpu_detail.into());

    let (cpu_temperature_value, cpu_temperature_detail) = format_cpu_temperature(
        snapshot.cpu_temperature_celsius,
        snapshot.cpu_temperature_info,
        snapshot.sensor_service_status,
    );
    ui.set_cpu_temperature_value(cpu_temperature_value.into());
    ui.set_cpu_temperature_detail(cpu_temperature_detail.into());

    let (memory_value, memory_detail) = format_memory(snapshot.memory);
    ui.set_memory_value(memory_value.into());
    ui.set_memory_detail(memory_detail.into());

    let ((received_value, received_detail), (transmitted_value, transmitted_detail)) =
        format_network(snapshot.network);
    ui.set_network_received_value(received_value.into());
    ui.set_network_received_detail(received_detail.into());
    ui.set_network_transmitted_value(transmitted_value.into());
    ui.set_network_transmitted_detail(transmitted_detail.into());

    let (gpu_value, gpu_detail) = format_gpu(snapshot.gpu_total_percent);
    ui.set_gpu_value(gpu_value.into());
    ui.set_gpu_detail(gpu_detail.into());

    let (video_memory_value, video_memory_detail) = format_video_memory(snapshot.video_memory);
    ui.set_video_memory_value(video_memory_value.into());
    ui.set_video_memory_detail(video_memory_detail.into());

    let (temperature_value, temperature_detail) = format_temperature(
        snapshot.temperature_celsius,
        snapshot.temperature_info,
        snapshot.temperature_target,
        snapshot.sensor_service_status,
    );
    ui.set_temperature_value(temperature_value.into());
    ui.set_temperature_detail(temperature_detail.into());
}

fn apply_trends(ui: &AppWindow, trends: MonitorTrends) {
    ui.set_cpu_trend(to_trend_segment_model(trends.cpu));
    ui.set_memory_trend(to_trend_segment_model(trends.memory));
    ui.set_network_received_trend(to_trend_segment_model(trends.network_received));
    ui.set_network_transmitted_trend(to_trend_segment_model(trends.network_transmitted));
}

fn to_trend_segment_model(points: Vec<TrendSeriesPoint>) -> ModelRc<TrendSegment> {
    if points.len() < 2 {
        return ModelRc::new(VecModel::default());
    }

    let last_index = (points.len() - 1) as f32;
    let point_coordinates = |index: usize, value: f32| {
        let x = 1.0 + index as f32 * 98.0 / last_index;
        let y = 5.0 + (1.0 - value.clamp(0.0, 1.0)) * 80.0;
        (x, y)
    };
    let mut segments = Vec::with_capacity((points.len() - 1) * 2);

    for secondary in [false, true] {
        for (index, pair) in points.windows(2).enumerate() {
            let values = if secondary {
                (pair[0].secondary, pair[1].secondary)
            } else {
                (pair[0].primary, pair[1].primary)
            };
            let (Some(start_value), Some(end_value)) = values else {
                continue;
            };
            let (start_x, start_y) = point_coordinates(index, start_value);
            let (end_x, end_y) = point_coordinates(index + 1, end_value);
            segments.push(TrendSegment {
                commands: format!("M {start_x:.3} {start_y:.3} L {end_x:.3} {end_y:.3}").into(),
                secondary,
            });
        }
    }

    ModelRc::new(VecModel::from(segments))
}

fn format_cpu(metric: MetricValue<f32>) -> (String, String) {
    match (metric.status, metric.value) {
        (SourceStatus::Available, Some(value)) => (format!("{value:.0}%"), "系统总占用".into()),
        (SourceStatus::WarmingUp, _) => warming_up_text(),
        _ => source_status_text(metric.status),
    }
}

fn format_cpu_temperature(
    metric: MetricValue<f32>,
    info: Option<TemperatureInfo>,
    service_status: SensorServiceStatus,
) -> (String, String) {
    match (metric.status, metric.value) {
        (SourceStatus::Available, Some(value)) => (
            format!("{value:.0}°C"),
            info.map_or_else(|| "CPU 温度".into(), format_temperature_info),
        ),
        (SourceStatus::WarmingUp, _) => ("采样中".into(), "正在读取 CPU Package".into()),
        (SourceStatus::Unavailable, _) => (
            "不支持".into(),
            temperature_service_detail(service_status, "未读取到 CPU Package/Tctl-Tdie"),
        ),
        (SourceStatus::TemporarilyUnavailable, _) | (SourceStatus::Available, None) => (
            "暂不可用".into(),
            temperature_service_detail(service_status, "CPU 温度读取失败，将重试"),
        ),
    }
}

fn format_temperature_info(info: TemperatureInfo) -> String {
    let sensor = match info.sensor {
        TemperatureSensor::CpuPackage => "CPU Package",
        TemperatureSensor::TctlTdie => "Tctl/Tdie",
        TemperatureSensor::GpuCore => "GPU Core",
        TemperatureSensor::GpuEdge => "GPU Edge",
    };
    let source = match info.source {
        TemperatureSource::LibreHardwareMonitor => "LHM",
        TemperatureSource::IntelLevelZero => "Level Zero",
        TemperatureSource::NvidiaNvml => "NVML",
    };
    format!("{sensor} · {source}")
}

fn temperature_service_detail(status: SensorServiceStatus, fallback: &str) -> String {
    match status {
        SensorServiceStatus::NotInstalled => "需安装温度传感器服务".into(),
        SensorServiceStatus::PawnIoNotInstalled => "需安装 PawnIO 硬件驱动".into(),
        SensorServiceStatus::PawnIoDeviceUnavailable => {
            "PawnIO 驱动不可用，可能需要重启电脑".into()
        }
        SensorServiceStatus::InitializationFailed => "温度传感器初始化失败".into(),
        SensorServiceStatus::Starting => "温度传感器服务正在启动".into(),
        SensorServiceStatus::TemporarilyUnavailable => "温度传感器服务暂不可用".into(),
        SensorServiceStatus::Ready => fallback.into(),
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
        _ => source_status_text(metric.status),
    }
}

type MetricText = (String, String);

fn format_network(
    metric: MetricValue<system_monitor::NetworkThroughput>,
) -> (MetricText, MetricText) {
    match (metric.status, metric.value) {
        (SourceStatus::Available, Some(network)) => (
            (
                format_rate(network.received_bytes_per_second),
                "实时接收".into(),
            ),
            (
                format_rate(network.transmitted_bytes_per_second),
                "实时发送".into(),
            ),
        ),
        (SourceStatus::WarmingUp, _) => (warming_up_text(), warming_up_text()),
        _ => (
            source_status_text(metric.status),
            source_status_text(metric.status),
        ),
    }
}

fn format_gpu(metric: MetricValue<f32>) -> (String, String) {
    match (metric.status, metric.value) {
        (SourceStatus::Available, Some(value)) => (format!("{value:.0}%"), "最忙引擎".into()),
        (SourceStatus::WarmingUp, _) => warming_up_text(),
        _ => source_status_text(metric.status),
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
        _ => source_status_text(metric.status),
    }
}

fn format_temperature(
    metric: MetricValue<f32>,
    info: Option<TemperatureInfo>,
    target: Option<GpuTemperatureTarget>,
    service_status: SensorServiceStatus,
) -> (String, String) {
    let target_label = match target {
        Some(GpuTemperatureTarget::Integrated) => "核显",
        Some(GpuTemperatureTarget::Dedicated) => "独显",
        None => "GPU",
    };
    match (metric.status, metric.value) {
        (SourceStatus::Available, Some(value)) => (
            format!("{value:.0}°C"),
            info.map_or_else(|| format!("{target_label}温度"), format_temperature_info),
        ),
        (SourceStatus::WarmingUp, _) => ("采样中".into(), format!("正在读取{target_label}温度")),
        (SourceStatus::TemporarilyUnavailable, _) => (
            "暂不可用".into(),
            temperature_service_detail(
                service_status,
                &format!("{target_label} GPU Core/Edge 读取失败，将重试"),
            ),
        ),
        (SourceStatus::Unavailable, _) => target.map_or_else(
            || ("不支持".into(), "未检测到物理显卡".into()),
            |_| {
                (
                    "不支持".into(),
                    temperature_service_detail(
                        service_status,
                        &format!("{target_label}未提供 GPU Core/Edge"),
                    ),
                )
            },
        ),
        (SourceStatus::Available, None) => source_status_text(SourceStatus::TemporarilyUnavailable),
    }
}

fn warming_up_text() -> (String, String) {
    ("采样中".into(), "正在建立差分基线".into())
}

fn source_status_text(status: SourceStatus) -> (String, String) {
    match status {
        SourceStatus::Unavailable => ("不支持".into(), "当前硬件不支持".into()),
        SourceStatus::TemporarilyUnavailable | SourceStatus::Available => {
            ("暂不可用".into(), "读取失败，将自动重试".into())
        }
        SourceStatus::WarmingUp => warming_up_text(),
    }
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
    let config_for_mascot = Rc::clone(&config);
    let saver_for_mascot = config_saver.clone();
    ui.on_mascot_selection_requested(move |mascot_index| {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };

        select_mascot(&ui, &config_for_mascot, &saver_for_mascot, mascot_index);
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
        ui.set_mascot_animation_running(false);
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
        window.set_skip_taskbar(true);
        if let Err(error) = display::enforce_tool_window(window) {
            eprintln!("failed to hide the assistant taskbar entry: {error}");
        }
        if focus {
            window.focus_window();
        }
    });
    // A hidden software-rendered window can lose its backing buffer while Slint still considers
    // the item tree clean. Changing a parent opacity invalidates the complete visible subtree.
    ui.set_repaint_cycle(!ui.get_repaint_cycle());
    ui.window().request_redraw();
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
    state.borrow_mut().toggle();
    ui.set_expanded(true);
    notify_monitor_activity(ui);
    sync_tray_state(tray, state);
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
        state.take_collapsed_position()
    };
    sync_tray_state(tray, state);
    ui.set_expanded(false);
    notify_monitor_activity(ui);
    if let Some(position) = collapsed_position {
        restore_collapsed_position_later(ui, position, Rc::clone(config), config_saver.clone());
    }
    true
}

fn hide_window(ui: &AppWindow, tray: &AssistantTray, state: &RefCell<WindowState>) {
    if !state.borrow_mut().hide() {
        return;
    }
    sync_tray_state(tray, state);
    ui.set_mascot_animation_running(false);
    ui.set_window_visible(false);
    notify_monitor_activity(ui);

    if let Err(error) = ui.hide() {
        eprintln!("failed to hide the assistant window: {error}");
    }
}

fn restore_window(ui: &AppWindow, tray: &AssistantTray, state: &RefCell<WindowState>) {
    let restored_layout = {
        let mut state = state.borrow_mut();
        if state.phase() == ApplicationPhase::Exiting {
            return;
        }
        state.restore()
    };
    if let Some(layout) = restored_layout {
        ui.set_expanded(layout.width > app_core::WindowMode::COLLAPSED_LAYOUT.width);
    }
    sync_tray_state(tray, state);

    if let Err(error) = ui.show() {
        eprintln!("failed to show the assistant window: {error}");
        return;
    }
    ui.set_window_visible(true);
    ui.set_mascot_animation_running(true);
    notify_monitor_activity(ui);
    show_native_window(ui, true);
}

fn open_settings(ui: &AppWindow, tray: &AssistantTray, state: &RefCell<WindowState>) {
    ui.set_active_tab(2);
    restore_window(ui, tray, state);

    if state.borrow().phase() == ApplicationPhase::Collapsed {
        remember_collapsed_position(ui, state);
        state.borrow_mut().toggle();
    }
    if state.borrow().phase() != ApplicationPhase::Expanded {
        return;
    }
    ui.set_expanded(true);
    notify_monitor_activity(ui);
    sync_tray_state(tray, state);
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

fn select_mascot(
    ui: &AppWindow,
    config: &RefCell<AppConfig>,
    config_saver: &ConfigSaveHandle,
    mascot_index: i32,
) {
    let Some(mascot_id) = MascotId::from_ui_index(mascot_index) else {
        return;
    };
    ui.set_mascot_id(mascot_id.ui_index());

    let snapshot = {
        let mut config = config.borrow_mut();
        if config.mascot_id == mascot_id {
            return;
        }
        config.mascot_id = mascot_id;
        config.clone()
    };
    config_saver.request(snapshot);
}

fn select_monitor_sample_interval(
    ui: &AppWindow,
    config: &RefCell<AppConfig>,
    config_saver: &ConfigSaveHandle,
    monitor_control: &MonitorControl,
    interval_index: i32,
) {
    let Some(interval) = MonitorSampleInterval::from_ui_index(interval_index) else {
        return;
    };
    ui.set_monitor_sample_interval(interval.ui_index());
    monitor_control.set_interactive_interval(interval);

    let snapshot = {
        let mut config = config.borrow_mut();
        if config.monitor_sample_interval == interval {
            return;
        }
        config.monitor_sample_interval = interval;
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

fn sync_tray_state(tray: &AssistantTray, state: &RefCell<WindowState>) {
    let (phase, always_on_top, position_locked) = {
        let state = state.borrow();
        (
            state.phase(),
            state.always_on_top(),
            state.position_locked(),
        )
    };
    tray.set_window_visible(matches!(
        phase,
        ApplicationPhase::Collapsed | ApplicationPhase::Expanded
    ));
    tray.set_expanded(phase == ApplicationPhase::Expanded);
    tray.set_always_on_top_enabled(always_on_top);
    tray.set_position_locked(position_locked);
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        sync::Arc,
        sync::atomic::{AtomicU64, Ordering},
        time::Duration,
    };

    use agent_core::RunPhase;
    use app_core::{
        PhysicalPosition,
        config::{
            AgentConnectionProfile, AgentProfile, AppConfig, ConfigStore, HttpDeployment,
            MonitorSampleInterval,
        },
    };
    use slint::Model;
    use system_monitor::{
        GpuTemperatureTarget, MemoryUsage, MetricValue, NetworkThroughput, SensorServiceStatus,
        SourceStatus, TemperatureInfo, TemperatureSensor, TemperatureSource, VideoMemoryUsage,
    };

    use super::{
        ConfigWorker, ConversationRuntime, ConversationView, MonitorActivity, TrendSeriesPoint,
        config_path_for_local_app_data, conversation_status, format_cpu_temperature, format_gpu,
        format_memory, format_network, format_rate, format_temperature, format_video_memory,
        initialize_managed_models, monitor_activity, normalize_codex_arguments,
        normalize_network_trends, normalize_percent_trends, should_update_trends,
        to_trend_segment_model,
    };

    static NEXT_TEMP_DIR: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn legacy_config_is_copied_to_the_zzh_directory_once() {
        let sequence = NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed);
        let local_app_data = std::env::temp_dir().join(format!(
            "zzh-config-migration-{}-{sequence}",
            std::process::id()
        ));
        let legacy_path = local_app_data
            .join("Xiaoxi Desktop Assistant")
            .join("settings.json");
        fs::create_dir_all(legacy_path.parent().unwrap()).expect("create legacy directory");
        fs::write(&legacy_path, b"legacy configuration").expect("write legacy configuration");

        let current_path =
            config_path_for_local_app_data(&local_app_data).expect("resolve current config path");

        assert_eq!(
            current_path,
            local_app_data
                .join("ZZH Desktop Assistant")
                .join("settings.json")
        );
        assert_eq!(
            fs::read(&current_path).expect("read migrated configuration"),
            b"legacy configuration"
        );
        assert!(legacy_path.is_file());
        let _ = fs::remove_dir_all(local_app_data);
    }

    #[test]
    fn existing_zzh_config_is_never_overwritten_by_legacy_config() {
        let sequence = NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed);
        let local_app_data = std::env::temp_dir().join(format!(
            "zzh-config-precedence-{}-{sequence}",
            std::process::id()
        ));
        let legacy_path = local_app_data
            .join("Xiaoxi Desktop Assistant")
            .join("settings.json");
        let current_path = local_app_data
            .join("ZZH Desktop Assistant")
            .join("settings.json");
        fs::create_dir_all(legacy_path.parent().unwrap()).expect("create legacy directory");
        fs::create_dir_all(current_path.parent().unwrap()).expect("create current directory");
        fs::write(&legacy_path, b"legacy configuration").expect("write legacy configuration");
        fs::write(&current_path, b"current configuration").expect("write current configuration");

        let resolved =
            config_path_for_local_app_data(&local_app_data).expect("resolve current config path");

        assert_eq!(resolved, current_path);
        assert_eq!(
            fs::read(&resolved).expect("read current configuration"),
            b"current configuration"
        );
        let _ = fs::remove_dir_all(local_app_data);
    }

    #[test]
    fn first_management_initialization_seeds_codex_only_once() {
        let mut config = AppConfig::default();
        assert!(initialize_managed_models(
            &mut config,
            Some(PathBuf::from("C:/tools/codex.exe"))
        ));
        assert!(config.agent_management_initialized);
        assert_eq!(config.agent_profiles.len(), 1);
        assert_eq!(config.agent_profiles[0].id, "codex-cli");

        config.agent_profiles.clear();
        config.selected_agent_profile_id = None;
        assert!(!initialize_managed_models(
            &mut config,
            Some(PathBuf::from("C:/tools/codex.exe"))
        ));
        assert!(config.agent_profiles.is_empty());
    }

    #[test]
    fn reloading_managed_profiles_preserves_the_conversation_controller() {
        let runtime = ConversationRuntime::from_config(&AppConfig::default());
        let controller = Arc::clone(&runtime.controller);
        let mut config = AppConfig::default();
        config.agent_profiles.push(AgentProfile {
            id: "local-model".into(),
            display_name: "Local Model".into(),
            connection: AgentConnectionProfile::Http {
                endpoint: "http://127.0.0.1:11434/v1/chat/completions".into(),
                model: "qwen".into(),
                deployment: HttpDeployment::Local,
            },
            credential_reference: None,
        });

        runtime.reload_profiles(&config);

        assert!(Arc::ptr_eq(&controller, &runtime.controller));
        assert_eq!(runtime.selected_profile().unwrap().id, "local-model");
    }

    #[test]
    fn codex_profile_arguments_remove_the_builtin_command_prefix() {
        let arguments = vec![
            "exec".to_owned(),
            "--json".to_owned(),
            "--model".to_owned(),
            "gpt-5".to_owned(),
        ];

        assert_eq!(
            normalize_codex_arguments(&arguments).collect::<Vec<_>>(),
            vec!["--model", "gpt-5"]
        );
    }

    #[test]
    fn conversation_status_prioritizes_stopping_and_error_details() {
        let mut view = ConversationView {
            messages: Vec::new(),
            busy: true,
            stopping: true,
            can_retry: false,
            phase: Some(RunPhase::Streaming),
            error: Some("稍后重试".into()),
        };
        assert_eq!(conversation_status(&view), "正在停止…");

        view.stopping = false;
        view.busy = false;
        view.phase = Some(RunPhase::Failed);
        assert_eq!(conversation_status(&view), "稍后重试");
    }

    #[test]
    fn monitor_activity_matches_visibility_and_active_view() {
        assert_eq!(
            monitor_activity(true, true, 0),
            MonitorActivity::Interactive
        );
        assert_eq!(
            monitor_activity(true, false, 0),
            MonitorActivity::Background
        );
        assert_eq!(monitor_activity(true, true, 1), MonitorActivity::Background);
        assert_eq!(monitor_activity(false, true, 0), MonitorActivity::Hidden);
        assert_eq!(monitor_activity(false, false, 2), MonitorActivity::Hidden);
    }

    #[test]
    fn monitor_activity_uses_lower_background_sample_rates() {
        assert_eq!(
            MonitorActivity::Interactive.sample_interval(MonitorSampleInterval::TwoSeconds),
            Duration::from_secs(2)
        );
        assert_eq!(
            MonitorActivity::Interactive.sample_interval(MonitorSampleInterval::FiveSeconds),
            Duration::from_secs(5)
        );
        assert_eq!(
            MonitorActivity::Interactive.sample_interval(MonitorSampleInterval::TenSeconds),
            Duration::from_secs(10)
        );
        assert_eq!(
            MonitorActivity::Background.sample_interval(MonitorSampleInterval::TwoSeconds),
            Duration::from_secs(10)
        );
        assert_eq!(
            MonitorActivity::Hidden.sample_interval(MonitorSampleInterval::TwoSeconds),
            Duration::from_secs(20)
        );
    }

    #[test]
    fn only_interactive_monitoring_dispatches_ui_updates() {
        assert!(MonitorActivity::Interactive.should_dispatch_ui());
        assert!(!MonitorActivity::Background.should_dispatch_ui());
        assert!(!MonitorActivity::Hidden.should_dispatch_ui());
    }

    #[test]
    fn percent_trends_are_normalized_and_clamped() {
        let points = normalize_percent_trends(vec![Some(-10.0), Some(50.0), None, Some(125.0)]);

        assert_eq!(
            points,
            vec![
                TrendSeriesPoint {
                    primary: Some(0.0),
                    secondary: None,
                },
                TrendSeriesPoint {
                    primary: Some(0.5),
                    secondary: None,
                },
                TrendSeriesPoint {
                    primary: None,
                    secondary: None,
                },
                TrendSeriesPoint {
                    primary: Some(1.0),
                    secondary: None,
                },
            ]
        );
    }

    #[test]
    fn network_trends_share_one_scale_and_keep_missing_values() {
        let (received, transmitted) = normalize_network_trends(
            vec![Some(10.0), None, Some(5.0)],
            vec![Some(20.0), Some(5.0), None],
        );

        assert_eq!(
            received,
            vec![
                TrendSeriesPoint {
                    primary: Some(0.5),
                    secondary: None,
                },
                TrendSeriesPoint {
                    primary: None,
                    secondary: None,
                },
                TrendSeriesPoint {
                    primary: Some(0.25),
                    secondary: None,
                },
            ]
        );
        assert_eq!(
            transmitted,
            vec![
                TrendSeriesPoint {
                    primary: Some(1.0),
                    secondary: None,
                },
                TrendSeriesPoint {
                    primary: Some(0.25),
                    secondary: None,
                },
                TrendSeriesPoint {
                    primary: None,
                    secondary: None,
                },
            ]
        );
    }

    #[test]
    fn trends_update_only_on_the_visible_expanded_monitor_tab() {
        assert!(should_update_trends(true, true, 0));
        assert!(!should_update_trends(false, true, 0));
        assert!(!should_update_trends(true, false, 0));
        assert!(!should_update_trends(true, true, 1));
        assert!(!should_update_trends(true, true, 2));
    }

    #[test]
    fn slint_trend_model_connects_adjacent_values_and_breaks_on_missing_samples() {
        let model = to_trend_segment_model(vec![
            TrendSeriesPoint {
                primary: Some(0.25),
                secondary: Some(0.5),
            },
            TrendSeriesPoint {
                primary: Some(0.75),
                secondary: Some(0.25),
            },
            TrendSeriesPoint {
                primary: None,
                secondary: Some(1.0),
            },
        ]);

        assert_eq!(model.row_count(), 3);
        let primary = model.row_data(0).expect("primary trend segment");
        assert_eq!(primary.commands.as_str(), "M 1.000 65.000 L 50.000 25.000");
        assert!(!primary.secondary);
        let first_secondary = model.row_data(1).expect("first network send segment");
        assert_eq!(
            first_secondary.commands.as_str(),
            "M 1.000 45.000 L 50.000 65.000"
        );
        assert!(first_secondary.secondary);
        let second_secondary = model.row_data(2).expect("second network send segment");
        assert_eq!(
            second_secondary.commands.as_str(),
            "M 50.000 65.000 L 99.000 5.000"
        );
        assert!(second_secondary.secondary);
    }

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
    fn immediate_config_save_acknowledges_the_latest_snapshot() {
        let sequence = NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!(
            "zzh-config-immediate-{}-{sequence}",
            std::process::id()
        ));
        let path = directory.join("settings.json");
        let store = ConfigStore::new(&path);
        let worker = ConfigWorker::start(store.clone()).expect("start config worker");
        let handle = worker.handle();
        handle.request(AppConfig::default());
        let mut expected = AppConfig::default();
        expected.agent_management_initialized = true;

        handle
            .save_immediately(expected.clone())
            .expect("save managed model snapshot immediately");

        assert_eq!(
            store.load().expect("load immediate config").config,
            expected
        );
        drop(worker);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn network_rate_uses_compact_binary_units() {
        assert_eq!(format_rate(512.0), "512 B/s");
        assert_eq!(format_rate(2.0 * 1024.0), "2.0 KB/s");
        assert_eq!(format_rate(3.5 * 1024.0 * 1024.0), "3.5 MB/s");
    }

    #[test]
    fn network_text_is_split_into_receive_and_send_cards() {
        let (received, transmitted) = format_network(MetricValue {
            value: Some(NetworkThroughput {
                received_bytes_per_second: 2.0 * 1024.0,
                transmitted_bytes_per_second: 512.0,
            }),
            status: SourceStatus::Available,
        });

        assert_eq!(received, ("2.0 KB/s".into(), "实时接收".into()));
        assert_eq!(transmitted, ("512 B/s".into(), "实时发送".into()));
    }

    #[test]
    fn cpu_temperature_identifies_the_package_and_lhm_source() {
        let (value, detail) = format_cpu_temperature(
            MetricValue {
                value: Some(42.4),
                status: SourceStatus::Available,
            },
            Some(TemperatureInfo {
                sensor: TemperatureSensor::CpuPackage,
                source: TemperatureSource::LibreHardwareMonitor,
            }),
            SensorServiceStatus::Ready,
        );

        assert_eq!(value, "42°C");
        assert_eq!(detail, "CPU Package · LHM");
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

        assert_eq!(value, "不支持");
        assert_eq!(detail, "当前硬件不支持");
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
    fn temporary_gpu_failure_is_shown_as_retryable() {
        let (value, detail) = format_gpu(MetricValue::<f32> {
            value: None,
            status: SourceStatus::TemporarilyUnavailable,
        });

        assert_eq!(value, "暂不可用");
        assert_eq!(detail, "读取失败，将自动重试");
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
    fn unsupported_video_memory_is_distinct_from_a_temporary_failure() {
        let (value, detail) = format_video_memory(MetricValue::<VideoMemoryUsage> {
            value: None,
            status: SourceStatus::Unavailable,
        });

        assert_eq!(value, "不支持");
        assert_eq!(detail, "当前硬件不支持");
    }

    #[test]
    fn temperature_text_identifies_the_integrated_gpu_source() {
        let (value, detail) = format_temperature(
            MetricValue {
                value: Some(42.4),
                status: SourceStatus::Available,
            },
            Some(TemperatureInfo {
                sensor: TemperatureSensor::GpuCore,
                source: TemperatureSource::IntelLevelZero,
            }),
            Some(GpuTemperatureTarget::Integrated),
            SensorServiceStatus::NotInstalled,
        );

        assert_eq!(value, "42°C");
        assert_eq!(detail, "GPU Core · Level Zero");
    }

    #[test]
    fn unavailable_integrated_gpu_temperature_is_explicit() {
        let (value, detail) = format_temperature(
            MetricValue::<f32> {
                value: None,
                status: SourceStatus::Unavailable,
            },
            None,
            Some(GpuTemperatureTarget::Integrated),
            SensorServiceStatus::Ready,
        );

        assert_eq!(value, "不支持");
        assert_eq!(detail, "核显未提供 GPU Core/Edge");
    }

    #[test]
    fn temporary_dedicated_gpu_temperature_failure_keeps_its_target() {
        let (value, detail) = format_temperature(
            MetricValue::<f32> {
                value: None,
                status: SourceStatus::TemporarilyUnavailable,
            },
            None,
            Some(GpuTemperatureTarget::Dedicated),
            SensorServiceStatus::TemporarilyUnavailable,
        );

        assert_eq!(value, "暂不可用");
        assert_eq!(detail, "温度传感器服务暂不可用");
    }

    #[test]
    fn missing_pawnio_is_not_reported_as_unsupported_cpu_hardware() {
        let (value, detail) = format_cpu_temperature(
            MetricValue::<f32> {
                value: None,
                status: SourceStatus::TemporarilyUnavailable,
            },
            None,
            SensorServiceStatus::PawnIoNotInstalled,
        );

        assert_eq!(value, "暂不可用");
        assert_eq!(detail, "需安装 PawnIO 硬件驱动");
    }
}

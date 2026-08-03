use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufReader, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::PhysicalPosition;

pub const CURRENT_CONFIG_VERSION: u32 = 5;

static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MascotId {
    #[default]
    Emerald,
    Orb,
}

impl MascotId {
    pub const fn ui_index(self) -> i32 {
        match self {
            Self::Emerald => 0,
            Self::Orb => 1,
        }
    }

    pub const fn from_ui_index(index: i32) -> Option<Self> {
        match index {
            0 => Some(Self::Emerald),
            1 => Some(Self::Orb),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MonitorSampleInterval {
    #[default]
    TwoSeconds,
    FiveSeconds,
    TenSeconds,
}

impl MonitorSampleInterval {
    pub const fn seconds(self) -> u64 {
        match self {
            Self::TwoSeconds => 2,
            Self::FiveSeconds => 5,
            Self::TenSeconds => 10,
        }
    }

    pub const fn ui_index(self) -> i32 {
        match self {
            Self::TwoSeconds => 0,
            Self::FiveSeconds => 1,
            Self::TenSeconds => 2,
        }
    }

    pub const fn from_ui_index(index: i32) -> Option<Self> {
        match index {
            0 => Some(Self::TwoSeconds),
            1 => Some(Self::FiveSeconds),
            2 => Some(Self::TenSeconds),
            _ => None,
        }
    }
}

impl Serialize for MonitorSampleInterval {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u64(self.seconds())
    }
}

impl<'de> Deserialize<'de> for MonitorSampleInterval {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let seconds = u64::deserialize(deserializer)?;
        Ok(match seconds {
            5 => Self::FiveSeconds,
            10 => Self::TenSeconds,
            _ => Self::TwoSeconds,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CredentialReference {
    pub target: String,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HttpDeployment {
    #[default]
    Cloud,
    Local,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "transport", rename_all = "snake_case")]
pub enum AgentConnectionProfile {
    Http {
        endpoint: String,
        model: String,
        #[serde(default)]
        deployment: HttpDeployment,
    },
    Cli {
        executable: PathBuf,
        arguments: Vec<String>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentProfile {
    pub id: String,
    pub display_name: String,
    pub connection: AgentConnectionProfile,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_reference: Option<CredentialReference>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    version: u32,
    pub window_position: Option<PhysicalPosition>,
    pub always_on_top: bool,
    pub position_locked: bool,
    pub mascot_id: MascotId,
    pub monitor_sample_interval: MonitorSampleInterval,
    pub agent_profiles: Vec<AgentProfile>,
    pub selected_agent_profile_id: Option<String>,
    pub agent_management_initialized: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: CURRENT_CONFIG_VERSION,
            window_position: None,
            always_on_top: true,
            position_locked: false,
            mascot_id: MascotId::default(),
            monitor_sample_interval: MonitorSampleInterval::default(),
            agent_profiles: Vec::new(),
            selected_agent_profile_id: None,
            agent_management_initialized: false,
        }
    }
}

impl AppConfig {
    fn migrate(mut self) -> Self {
        if self.version < 2 {
            self.mascot_id = MascotId::Emerald;
        }
        if self.version < 5 {
            self.agent_management_initialized = !self.agent_profiles.is_empty();
        }
        if self
            .selected_agent_profile_id
            .as_ref()
            .is_some_and(|selected| {
                !self
                    .agent_profiles
                    .iter()
                    .any(|profile| profile.id == *selected)
            })
        {
            self.selected_agent_profile_id = None;
        }
        self.version = CURRENT_CONFIG_VERSION;
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConfigLoadStatus {
    Missing,
    Loaded,
    RecoveredCorrupt { backup_path: PathBuf },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadedConfig {
    pub config: AppConfig,
    pub status: ConfigLoadStatus,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigStore {
    path: PathBuf,
}

impl ConfigStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> io::Result<LoadedConfig> {
        let file = match File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(LoadedConfig {
                    config: AppConfig::default(),
                    status: ConfigLoadStatus::Missing,
                });
            }
            Err(error) => return Err(error),
        };

        match serde_json::from_reader(BufReader::new(file)) {
            Ok(config) => Ok(LoadedConfig {
                config: AppConfig::migrate(config),
                status: ConfigLoadStatus::Loaded,
            }),
            Err(_) => {
                let backup_path = self.next_corrupt_backup_path();
                fs::rename(&self.path, &backup_path)?;
                Ok(LoadedConfig {
                    config: AppConfig::default(),
                    status: ConfigLoadStatus::RecoveredCorrupt { backup_path },
                })
            }
        }
    }

    pub fn save(&self, config: &AppConfig) -> io::Result<()> {
        let Some(parent) = self.path.parent() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "configuration path has no parent directory",
            ));
        };
        fs::create_dir_all(parent)?;

        let temporary_path = self.next_temporary_path();
        let write_result = (|| {
            let mut temporary_file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary_path)?;
            serde_json::to_writer_pretty(&mut temporary_file, config).map_err(io::Error::other)?;
            temporary_file.write_all(b"\n")?;
            temporary_file.sync_all()?;
            drop(temporary_file);
            replace_file(&temporary_path, &self.path)
        })();

        if write_result.is_err() {
            let _ = fs::remove_file(&temporary_path);
        }
        write_result
    }

    fn next_corrupt_backup_path(&self) -> PathBuf {
        let file_name = self
            .path
            .file_name()
            .map_or_else(|| "settings.json".into(), |name| name.to_string_lossy());
        let first_candidate = self.path.with_file_name(format!("{file_name}.corrupt"));
        if !first_candidate.exists() {
            return first_candidate;
        }

        for sequence in 1_u32.. {
            let candidate = self
                .path
                .with_file_name(format!("{file_name}.corrupt.{sequence}"));
            if !candidate.exists() {
                return candidate;
            }
        }
        unreachable!("the corrupt backup sequence is unbounded")
    }

    fn next_temporary_path(&self) -> PathBuf {
        let sequence = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
        let file_name = self
            .path
            .file_name()
            .map_or_else(|| "settings.json".into(), |name| name.to_string_lossy());
        self.path.with_file_name(format!(
            ".{file_name}.{}.{}.tmp",
            std::process::id(),
            sequence
        ))
    }
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    use windows::{
        Win32::Storage::FileSystem::{
            MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        },
        core::PCWSTR,
    };

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();

    unsafe {
        MoveFileExW(
            PCWSTR(source.as_ptr()),
            PCWSTR(destination.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
        .map_err(io::Error::other)
    }
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };

    use crate::PhysicalPosition;

    use super::{
        AgentConnectionProfile, AgentProfile, AppConfig, ConfigLoadStatus, ConfigStore,
        CredentialReference, HttpDeployment, MascotId, MonitorSampleInterval,
    };

    static NEXT_TEMP_DIR: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let sequence = NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "xiaoxi-config-{label}-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("create isolated test directory");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn missing_config_uses_defaults_without_creating_a_file() {
        let directory = TestDirectory::new("missing");
        let path = directory.path().join("settings.json");
        let store = ConfigStore::new(&path);

        let loaded = store.load().expect("load missing config");

        assert_eq!(loaded.config, AppConfig::default());
        assert_eq!(loaded.status, ConfigLoadStatus::Missing);
        assert!(!path.exists());
    }

    #[test]
    fn config_round_trips_all_desktop_settings() {
        let directory = TestDirectory::new("round-trip");
        let path = directory.path().join("settings.json");
        let store = ConfigStore::new(&path);
        let expected = AppConfig {
            window_position: Some(PhysicalPosition { x: -320, y: 144 }),
            always_on_top: false,
            position_locked: true,
            monitor_sample_interval: MonitorSampleInterval::TenSeconds,
            ..AppConfig::default()
        };

        store.save(&expected).expect("save config");
        let loaded = store.load().expect("reload config");

        assert_eq!(loaded.config, expected);
        assert_eq!(loaded.status, ConfigLoadStatus::Loaded);
    }

    #[test]
    fn legacy_config_without_a_mascot_uses_the_default() {
        let directory = TestDirectory::new("legacy-mascot");
        let path = directory.path().join("settings.json");
        fs::write(
            &path,
            br#"{"version":1,"window_position":null,"always_on_top":true,"position_locked":false}"#,
        )
        .expect("write legacy config");

        let loaded = ConfigStore::new(&path).load().expect("load legacy config");

        assert_eq!(loaded.config.mascot_id, MascotId::Emerald);
        assert_eq!(loaded.status, ConfigLoadStatus::Loaded);
    }

    #[test]
    fn mascot_ui_index_rejects_unavailable_options() {
        assert_eq!(MascotId::from_ui_index(0), Some(MascotId::Emerald));
        assert_eq!(MascotId::Emerald.ui_index(), 0);
        assert_eq!(MascotId::from_ui_index(1), Some(MascotId::Orb));
        assert_eq!(MascotId::Orb.ui_index(), 1);
        assert_eq!(MascotId::from_ui_index(2), None);
    }

    #[test]
    fn version_one_orb_config_migrates_to_emerald() {
        let directory = TestDirectory::new("v1-orb");
        let path = directory.path().join("settings.json");
        fs::write(
            &path,
            br#"{"version":1,"window_position":null,"always_on_top":true,"position_locked":false,"mascot_id":"orb"}"#,
        )
        .expect("write version one config");

        let loaded = ConfigStore::new(&path)
            .load()
            .expect("load version one config");

        assert_eq!(loaded.config.mascot_id, MascotId::Emerald);
        assert_eq!(loaded.status, ConfigLoadStatus::Loaded);
    }

    #[test]
    fn legacy_config_without_a_monitor_interval_defaults_to_two_seconds() {
        let directory = TestDirectory::new("legacy-monitor-interval");
        let path = directory.path().join("settings.json");
        fs::write(
            &path,
            br#"{"version":2,"window_position":null,"always_on_top":true,"position_locked":false,"mascot_id":"emerald"}"#,
        )
        .expect("write version two config");

        let loaded = ConfigStore::new(&path)
            .load()
            .expect("load version two config");

        assert_eq!(
            loaded.config.monitor_sample_interval,
            MonitorSampleInterval::TwoSeconds
        );
        assert_eq!(loaded.status, ConfigLoadStatus::Loaded);
    }

    #[test]
    fn unsupported_monitor_interval_falls_back_to_two_seconds() {
        let directory = TestDirectory::new("invalid-monitor-interval");
        let path = directory.path().join("settings.json");
        fs::write(
            &path,
            br#"{"version":3,"window_position":null,"always_on_top":true,"position_locked":false,"mascot_id":"emerald","monitor_sample_interval":3}"#,
        )
        .expect("write invalid monitor interval");

        let loaded = ConfigStore::new(&path)
            .load()
            .expect("load invalid monitor interval");

        assert_eq!(
            loaded.config.monitor_sample_interval,
            MonitorSampleInterval::TwoSeconds
        );
        assert_eq!(loaded.status, ConfigLoadStatus::Loaded);
    }

    #[test]
    fn monitor_interval_ui_index_accepts_only_available_options() {
        assert_eq!(
            MonitorSampleInterval::from_ui_index(0),
            Some(MonitorSampleInterval::TwoSeconds)
        );
        assert_eq!(
            MonitorSampleInterval::from_ui_index(1),
            Some(MonitorSampleInterval::FiveSeconds)
        );
        assert_eq!(
            MonitorSampleInterval::from_ui_index(2),
            Some(MonitorSampleInterval::TenSeconds)
        );
        assert_eq!(MonitorSampleInterval::from_ui_index(3), None);
        assert_eq!(MonitorSampleInterval::TenSeconds.ui_index(), 2);
    }

    #[test]
    fn agent_profiles_round_trip_without_a_raw_secret_field() {
        let directory = TestDirectory::new("agent-profiles");
        let path = directory.path().join("settings.json");
        let store = ConfigStore::new(&path);
        let expected = AppConfig {
            agent_profiles: vec![
                AgentProfile {
                    id: "openai-compatible".into(),
                    display_name: "OpenAI Compatible".into(),
                    connection: AgentConnectionProfile::Http {
                        endpoint: "https://example.invalid/v1/chat/completions".into(),
                        model: "example-model".into(),
                        deployment: HttpDeployment::Cloud,
                    },
                    credential_reference: Some(CredentialReference {
                        target: "ZZH Desktop Assistant/openai-compatible".into(),
                    }),
                },
                AgentProfile {
                    id: "codex-cli".into(),
                    display_name: "Codex CLI".into(),
                    connection: AgentConnectionProfile::Cli {
                        executable: PathBuf::from("codex.exe"),
                        arguments: vec!["exec".into(), "--json".into()],
                    },
                    credential_reference: None,
                },
            ],
            selected_agent_profile_id: Some("openai-compatible".into()),
            agent_management_initialized: true,
            ..AppConfig::default()
        };

        store.save(&expected).expect("save agent profiles");
        let serialized = fs::read_to_string(&path).expect("read agent profile JSON");
        let loaded = store.load().expect("reload agent profiles");

        assert_eq!(loaded.config, expected);
        assert!(!serialized.contains("api_key"));
        assert!(!serialized.contains("test-raw-secret-value"));
        assert!(serialized.contains("credential_reference"));
    }

    #[test]
    fn version_three_config_migrates_to_empty_agent_profiles() {
        let directory = TestDirectory::new("legacy-agent-profiles");
        let path = directory.path().join("settings.json");
        fs::write(
            &path,
            br#"{"version":3,"window_position":null,"always_on_top":true,"position_locked":false,"mascot_id":"emerald","monitor_sample_interval":5}"#,
        )
        .expect("write version three config");

        let loaded = ConfigStore::new(&path)
            .load()
            .expect("load version three config");

        assert!(loaded.config.agent_profiles.is_empty());
        assert_eq!(loaded.config.selected_agent_profile_id, None);
        assert!(!loaded.config.agent_management_initialized);
        assert_eq!(loaded.status, ConfigLoadStatus::Loaded);
    }

    #[test]
    fn version_four_profiles_migrate_into_initialized_management() {
        let directory = TestDirectory::new("managed-agent-profiles");
        let path = directory.path().join("settings.json");
        fs::write(
            &path,
            br#"{"version":4,"agent_profiles":[{"id":"local","display_name":"Local","connection":{"transport":"http","endpoint":"http://127.0.0.1:11434/v1/chat/completions","model":"qwen"}}]}"#,
        )
        .expect("write version four config");

        let loaded = ConfigStore::new(&path)
            .load()
            .expect("load version four config");

        assert!(loaded.config.agent_management_initialized);
        assert!(matches!(
            loaded.config.agent_profiles[0].connection,
            AgentConnectionProfile::Http {
                deployment: HttpDeployment::Cloud,
                ..
            }
        ));
    }

    #[test]
    fn dangling_selected_agent_profile_is_cleared_during_load() {
        let directory = TestDirectory::new("dangling-agent-selection");
        let path = directory.path().join("settings.json");
        fs::write(
            &path,
            br#"{"version":4,"window_position":null,"always_on_top":true,"position_locked":false,"mascot_id":"emerald","monitor_sample_interval":2,"agent_profiles":[],"selected_agent_profile_id":"missing"}"#,
        )
        .expect("write dangling agent selection");

        let loaded = ConfigStore::new(&path)
            .load()
            .expect("load dangling agent selection");

        assert_eq!(loaded.config.selected_agent_profile_id, None);
        assert_eq!(loaded.status, ConfigLoadStatus::Loaded);
    }

    #[test]
    fn corrupt_config_is_preserved_before_defaults_are_returned() {
        let directory = TestDirectory::new("corrupt");
        let path = directory.path().join("settings.json");
        fs::write(&path, b"{not-json").expect("write corrupt fixture");
        let store = ConfigStore::new(&path);

        let loaded = store.load().expect("recover corrupt config");

        let ConfigLoadStatus::RecoveredCorrupt { backup_path } = loaded.status else {
            panic!("expected corrupt recovery status");
        };
        assert_eq!(loaded.config, AppConfig::default());
        assert_eq!(fs::read(backup_path).expect("read backup"), b"{not-json");
        assert!(!path.exists());
    }

    #[test]
    fn repeated_save_atomically_replaces_the_previous_config() {
        let directory = TestDirectory::new("replace");
        let path = directory.path().join("settings.json");
        let store = ConfigStore::new(&path);
        store
            .save(&AppConfig::default())
            .expect("save initial config");
        let replacement = AppConfig {
            window_position: Some(PhysicalPosition { x: 640, y: 360 }),
            always_on_top: false,
            position_locked: true,
            ..AppConfig::default()
        };

        store.save(&replacement).expect("replace config");

        assert_eq!(store.load().expect("load replacement").config, replacement);
        let temporary_files = fs::read_dir(directory.path())
            .expect("read config directory")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(temporary_files, 0);
    }
}

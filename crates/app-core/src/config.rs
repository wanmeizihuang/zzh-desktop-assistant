use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufReader, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use serde::{Deserialize, Serialize};

use crate::PhysicalPosition;

pub const CURRENT_CONFIG_VERSION: u32 = 1;

static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    version: u32,
    pub window_position: Option<PhysicalPosition>,
    pub always_on_top: bool,
    pub position_locked: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: CURRENT_CONFIG_VERSION,
            window_position: None,
            always_on_top: true,
            position_locked: false,
        }
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
                config,
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

    use super::{AppConfig, ConfigLoadStatus, ConfigStore};

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
            ..AppConfig::default()
        };

        store.save(&expected).expect("save config");
        let loaded = store.load().expect("reload config");

        assert_eq!(loaded.config, expected);
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

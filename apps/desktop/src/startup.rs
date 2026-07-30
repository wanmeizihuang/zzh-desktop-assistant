use std::{
    env,
    ffi::{OsStr, OsString},
    io,
    mem::size_of,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::Path,
    slice,
};

use windows::{
    Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR},
        System::Registry::{
            HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_SZ,
            RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegGetValueW, RegOpenKeyExW,
            RegSetValueExW,
        },
    },
    core::PCWSTR,
};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "XiaoxiDesktopAssistant";

pub fn is_enabled() -> io::Result<bool> {
    let expected = startup_command(&env::current_exe()?);
    Ok(read_run_value(OsStr::new(VALUE_NAME))?
        .is_some_and(|actual| commands_match(&actual, &expected)))
}

pub fn set_enabled(enabled: bool) -> io::Result<()> {
    if enabled {
        write_run_value(
            OsStr::new(VALUE_NAME),
            &startup_command(&env::current_exe()?),
        )
    } else {
        delete_run_value(OsStr::new(VALUE_NAME))
    }
}

fn startup_command(executable: &Path) -> OsString {
    let mut command = Vec::with_capacity(executable.as_os_str().encode_wide().count() + 2);
    command.push(u16::from(b'"'));
    command.extend(executable.as_os_str().encode_wide());
    command.push(u16::from(b'"'));
    OsString::from_wide(&command)
}

fn commands_match(actual: &OsStr, expected: &OsStr) -> bool {
    actual
        .to_string_lossy()
        .eq_ignore_ascii_case(&expected.to_string_lossy())
}

fn read_run_value(value_name: &OsStr) -> io::Result<Option<OsString>> {
    let key = wide_string(OsStr::new(RUN_KEY));
    let name = wide_string(value_name);
    let mut byte_count = 0u32;
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            PCWSTR(name.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut byte_count),
        )
    };
    if result == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    check_result(result)?;

    let mut buffer = vec![0u16; (byte_count as usize).div_ceil(size_of::<u16>())];
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            PCWSTR(name.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut byte_count),
        )
    };
    check_result(result)?;
    let length = buffer
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(buffer.len());
    Ok(Some(OsString::from_wide(&buffer[..length])))
}

fn write_run_value(value_name: &OsStr, command: &OsStr) -> io::Result<()> {
    let subkey = wide_string(OsStr::new(RUN_KEY));
    let name = wide_string(value_name);
    let command = wide_string(command);
    let mut key = HKEY::default();
    check_result(unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut key,
            None,
        )
    })?;
    let key = RegistryKey(key);
    let bytes = unsafe {
        slice::from_raw_parts(
            command.as_ptr().cast::<u8>(),
            command.len() * size_of::<u16>(),
        )
    };
    check_result(unsafe { RegSetValueExW(key.0, PCWSTR(name.as_ptr()), None, REG_SZ, Some(bytes)) })
}

fn delete_run_value(value_name: &OsStr) -> io::Result<()> {
    let subkey = wide_string(OsStr::new(RUN_KEY));
    let name = wide_string(value_name);
    let mut key = HKEY::default();
    let result = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            None,
            KEY_SET_VALUE,
            &mut key,
        )
    };
    if result == ERROR_FILE_NOT_FOUND {
        return Ok(());
    }
    check_result(result)?;
    let key = RegistryKey(key);
    let result = unsafe { RegDeleteValueW(key.0, PCWSTR(name.as_ptr())) };
    if result == ERROR_FILE_NOT_FOUND {
        return Ok(());
    }
    check_result(result)
}

fn wide_string(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}

fn check_result(result: WIN32_ERROR) -> io::Result<()> {
    if result == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(result.0 as i32))
    }
}

struct RegistryKey(HKEY);

impl Drop for RegistryKey {
    fn drop(&mut self) {
        let _ = unsafe { RegCloseKey(self.0) };
    }
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsStr, path::Path};

    use super::{
        commands_match, delete_run_value, read_run_value, startup_command, write_run_value,
    };

    #[test]
    fn startup_command_quotes_paths_with_spaces() {
        assert_eq!(
            startup_command(Path::new(r"C:\Program Files\Xiaoxi\desktop-assistant.exe")),
            OsStr::new(r#""C:\Program Files\Xiaoxi\desktop-assistant.exe""#)
        );
    }

    #[test]
    fn command_match_is_case_insensitive_but_rejects_a_different_path() {
        assert!(commands_match(
            OsStr::new(r#""C:\APPS\XIAOXI.EXE""#),
            OsStr::new(r#""c:\apps\xiaoxi.exe""#)
        ));
        assert!(!commands_match(
            OsStr::new(r#""C:\apps\old.exe""#),
            OsStr::new(r#""C:\apps\xiaoxi.exe""#)
        ));
    }

    #[test]
    #[ignore = "writes an isolated current-user Run value"]
    fn current_user_run_value_round_trips_without_admin_rights() {
        let value_name = format!("XiaoxiDesktopAssistantTest{}", std::process::id());
        let value_name = OsStr::new(&value_name);
        let command = OsStr::new(r#""C:\Program Files\Xiaoxi Test\desktop-assistant.exe""#);
        let _cleanup = RunValueCleanup(value_name);

        delete_run_value(value_name).expect("remove stale test value");
        write_run_value(value_name, command).expect("write current-user Run value");
        assert_eq!(
            read_run_value(value_name).expect("read current-user Run value"),
            Some(command.to_os_string())
        );
        delete_run_value(value_name).expect("delete current-user Run value");
        assert_eq!(
            read_run_value(value_name).expect("confirm deleted Run value"),
            None
        );
    }

    struct RunValueCleanup<'a>(&'a OsStr);

    impl Drop for RunValueCleanup<'_> {
        fn drop(&mut self) {
            let _ = delete_run_value(self.0);
        }
    }
}

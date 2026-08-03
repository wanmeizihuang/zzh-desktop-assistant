use std::{error::Error, fmt};

use zeroize::Zeroizing;

const CREDENTIAL_NAMESPACE: &str = "ZZHDesktopAssistant/agent/";
const MAX_PROFILE_ID_BYTES: usize = 128;
const MAX_SECRET_BYTES: usize = 2560;
const MAX_TARGET_UTF16_UNITS: usize = 32_767;
pub const REDACTED_SECRET: &str = "[REDACTED]";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CredentialErrorKind {
    InvalidTarget,
    InvalidSecret,
    InvalidData,
    UnsupportedPlatform,
    System,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CredentialError {
    kind: CredentialErrorKind,
    message: String,
}

impl CredentialError {
    fn new(kind: CredentialErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub const fn kind(&self) -> CredentialErrorKind {
        self.kind
    }
}

impl fmt::Display for CredentialError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for CredentialError {}

pub struct SecretString(Zeroizing<String>);

impl SecretString {
    pub fn new(value: impl Into<String>) -> Result<Self, CredentialError> {
        let value = Zeroizing::new(value.into());
        validate_secret(&value)?;
        Ok(Self(value))
    }

    pub fn expose_secret(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretString([REDACTED])")
    }
}

pub fn credential_target(profile_id: &str) -> Result<String, CredentialError> {
    if profile_id.is_empty()
        || profile_id.len() > MAX_PROFILE_ID_BYTES
        || !profile_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(CredentialError::new(
            CredentialErrorKind::InvalidTarget,
            "credential profile ID must use 1-128 ASCII letters, digits, dots, dashes or underscores",
        ));
    }

    let target = format!("{CREDENTIAL_NAMESPACE}{profile_id}");
    validate_target(&target)?;
    Ok(target)
}

pub fn redact_secret(text: &str, secret: &SecretString) -> String {
    text.replace(secret.expose_secret(), REDACTED_SECRET)
}

fn validate_target(target: &str) -> Result<(), CredentialError> {
    if target.is_empty()
        || target.contains('\0')
        || target.encode_utf16().count() > MAX_TARGET_UTF16_UNITS
    {
        return Err(CredentialError::new(
            CredentialErrorKind::InvalidTarget,
            "credential target is empty, contains a null character or is too long",
        ));
    }
    Ok(())
}

fn validate_secret(secret: &str) -> Result<(), CredentialError> {
    if secret.trim().is_empty() || secret.contains('\0') || secret.len() > MAX_SECRET_BYTES {
        return Err(CredentialError::new(
            CredentialErrorKind::InvalidSecret,
            "credential secret is empty, contains a null character or exceeds 2560 bytes",
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CredentialStore;

impl CredentialStore {
    pub const fn new() -> Self {
        Self
    }

    pub fn write(&self, target: &str, secret: &SecretString) -> Result<(), CredentialError> {
        validate_target(target)?;
        validate_secret(secret.expose_secret())?;

        #[cfg(windows)]
        {
            windows_store::write(target, secret)
        }
        #[cfg(not(windows))]
        {
            let _ = (target, secret);
            Err(unsupported_platform_error())
        }
    }

    pub fn read(&self, target: &str) -> Result<Option<SecretString>, CredentialError> {
        validate_target(target)?;

        #[cfg(windows)]
        {
            windows_store::read(target)
        }
        #[cfg(not(windows))]
        {
            let _ = target;
            Err(unsupported_platform_error())
        }
    }

    pub fn delete(&self, target: &str) -> Result<bool, CredentialError> {
        validate_target(target)?;

        #[cfg(windows)]
        {
            windows_store::delete(target)
        }
        #[cfg(not(windows))]
        {
            let _ = target;
            Err(unsupported_platform_error())
        }
    }
}

#[cfg(not(windows))]
fn unsupported_platform_error() -> CredentialError {
    CredentialError::new(
        CredentialErrorKind::UnsupportedPlatform,
        "Windows Credential Manager is unavailable on this platform",
    )
}

#[cfg(windows)]
mod windows_store {
    use std::{ptr, slice, str};

    use windows::{
        Win32::{
            Foundation::ERROR_NOT_FOUND,
            Security::Credentials::{
                CRED_MAX_CREDENTIAL_BLOB_SIZE, CRED_MAX_GENERIC_TARGET_NAME_LENGTH,
                CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree,
                CredReadW, CredWriteW,
            },
        },
        core::{HRESULT, PCWSTR, PWSTR},
    };
    use zeroize::{Zeroize, Zeroizing};

    use super::{
        CredentialError, CredentialErrorKind, MAX_SECRET_BYTES, MAX_TARGET_UTF16_UNITS,
        SecretString,
    };

    pub(super) fn write(target: &str, secret: &SecretString) -> Result<(), CredentialError> {
        debug_assert_eq!(MAX_SECRET_BYTES, CRED_MAX_CREDENTIAL_BLOB_SIZE as usize);
        debug_assert_eq!(
            MAX_TARGET_UTF16_UNITS,
            CRED_MAX_GENERIC_TARGET_NAME_LENGTH as usize
        );

        let mut target = wide_null(target);
        let mut comment = wide_null("ZZH Desktop Assistant agent credential");
        let mut blob = Zeroizing::new(secret.expose_secret().as_bytes().to_vec());
        let credential = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            TargetName: PWSTR(target.as_mut_ptr()),
            Comment: PWSTR(comment.as_mut_ptr()),
            CredentialBlobSize: blob.len() as u32,
            CredentialBlob: blob.as_mut_ptr(),
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            ..Default::default()
        };

        unsafe { CredWriteW(&credential, 0) }.map_err(|error| system_error("write", error))
    }

    pub(super) fn read(target: &str) -> Result<Option<SecretString>, CredentialError> {
        let target = wide_null(target);
        let mut credential = ptr::null_mut();
        if let Err(error) = unsafe {
            CredReadW(
                PCWSTR(target.as_ptr()),
                CRED_TYPE_GENERIC,
                None,
                &mut credential,
            )
        } {
            if is_not_found(&error) {
                return Ok(None);
            }
            return Err(system_error("read", error));
        }
        if credential.is_null() {
            return Err(CredentialError::new(
                CredentialErrorKind::InvalidData,
                "Windows Credential Manager returned an empty credential pointer",
            ));
        }

        let buffer = CredentialBuffer(credential);
        let credential = buffer.credential();
        let blob_size = credential.CredentialBlobSize as usize;
        if blob_size == 0 || blob_size > MAX_SECRET_BYTES || credential.CredentialBlob.is_null() {
            return Err(CredentialError::new(
                CredentialErrorKind::InvalidData,
                "stored credential contains an invalid secret blob",
            ));
        }

        let bytes = unsafe { slice::from_raw_parts(credential.CredentialBlob, blob_size) };
        let text = str::from_utf8(bytes).map_err(|_| {
            CredentialError::new(
                CredentialErrorKind::InvalidData,
                "stored credential secret is not valid UTF-8",
            )
        })?;
        SecretString::new(text.to_owned()).map(Some)
    }

    pub(super) fn delete(target: &str) -> Result<bool, CredentialError> {
        let target = wide_null(target);
        match unsafe { CredDeleteW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None) } {
            Ok(()) => Ok(true),
            Err(error) if is_not_found(&error) => Ok(false),
            Err(error) => Err(system_error("delete", error)),
        }
    }

    fn wide_null(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }

    fn is_not_found(error: &windows::core::Error) -> bool {
        error.code() == HRESULT::from_win32(ERROR_NOT_FOUND.0)
    }

    fn system_error(operation: &str, error: windows::core::Error) -> CredentialError {
        CredentialError::new(
            CredentialErrorKind::System,
            format!("Windows Credential Manager {operation} failed: {error}"),
        )
    }

    struct CredentialBuffer(*mut CREDENTIALW);

    impl CredentialBuffer {
        fn credential(&self) -> &CREDENTIALW {
            unsafe { &*self.0 }
        }
    }

    impl Drop for CredentialBuffer {
        fn drop(&mut self) {
            unsafe {
                let credential = &mut *self.0;
                let blob_size = credential.CredentialBlobSize as usize;
                if !credential.CredentialBlob.is_null()
                    && blob_size > 0
                    && blob_size <= MAX_SECRET_BYTES
                {
                    slice::from_raw_parts_mut(credential.CredentialBlob, blob_size).zeroize();
                }
                CredFree(self.0.cast());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        CredentialErrorKind, CredentialStore, SecretString, credential_target, redact_secret,
    };

    #[cfg(windows)]
    struct IsolatedCredential {
        store: CredentialStore,
        target: String,
    }

    #[cfg(windows)]
    impl Drop for IsolatedCredential {
        fn drop(&mut self) {
            let _ = self.store.delete(&self.target);
        }
    }

    #[test]
    fn credential_target_uses_a_stable_application_namespace() {
        assert_eq!(
            credential_target("openai-primary").unwrap(),
            "ZZHDesktopAssistant/agent/openai-primary"
        );
        assert_eq!(
            credential_target("codex_cli.2").unwrap(),
            "ZZHDesktopAssistant/agent/codex_cli.2"
        );
    }

    #[test]
    fn credential_target_rejects_empty_or_unsafe_profile_ids() {
        for invalid in ["", " ", "../other", "agent/name", "中文", "has space"] {
            let error = credential_target(invalid).unwrap_err();
            assert_eq!(error.kind(), CredentialErrorKind::InvalidTarget);
        }
    }

    #[test]
    fn secret_validation_rejects_empty_nul_and_oversized_values() {
        for invalid in [String::new(), "contains\0nul".into(), "x".repeat(2561)] {
            let error = SecretString::new(invalid.clone()).unwrap_err();
            assert_eq!(error.kind(), CredentialErrorKind::InvalidSecret);
            if !invalid.is_empty() {
                assert!(!format!("{error:?}").contains(&invalid));
            }
        }
    }

    #[test]
    fn secret_debug_and_redaction_never_include_plaintext() {
        let secret = SecretString::new("test-redaction-value").unwrap();
        let debug = format!("{secret:?}");
        let redacted = redact_secret("authorization failed for test-redaction-value", &secret);

        assert_eq!(debug, "SecretString([REDACTED])");
        assert_eq!(redacted, "authorization failed for [REDACTED]");
        assert!(!debug.contains(secret.expose_secret()));
        assert!(!redacted.contains(secret.expose_secret()));
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "writes and deletes one isolated current-user Generic Credential"]
    fn current_user_generic_credential_round_trips_in_isolation() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let target = format!(
            "ZZHDesktopAssistant/test/credential-round-trip-{}-{nonce}",
            std::process::id()
        );
        let secret = SecretString::new(format!("isolated-secret-{nonce}")).unwrap();
        let store = CredentialStore::new();
        let _cleanup = IsolatedCredential {
            store,
            target: target.clone(),
        };

        store.write(&target, &secret).unwrap();
        let loaded = store.read(&target).unwrap().expect("stored credential");
        assert_eq!(loaded.expose_secret(), secret.expose_secret());
        assert!(store.delete(&target).unwrap());
        assert!(store.read(&target).unwrap().is_none());
        assert!(!store.delete(&target).unwrap());
    }
}

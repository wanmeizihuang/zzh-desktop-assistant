use std::{error::Error, fmt};

use app_core::{
    agent_profiles::{
        AgentProfileDraft, AgentProfileKind, allocate_agent_profile_id, build_agent_profile,
        validate_agent_profile_draft,
    },
    config::{
        AgentConnectionProfile, AgentProfile, AppConfig, CredentialReference, HttpDeployment,
    },
    credentials::{CredentialStore, SecretString, credential_target},
};

pub enum CredentialEdit {
    Keep,
    Replace(String),
    Clear,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AiManagementError {
    message: String,
}

impl AiManagementError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for AiManagementError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for AiManagementError {}

pub trait CredentialStorage {
    fn read_secret(&self, target: &str) -> Result<Option<SecretString>, String>;
    fn write_secret(&self, target: &str, secret: &SecretString) -> Result<(), String>;
    fn delete_secret(&self, target: &str) -> Result<bool, String>;
}

impl CredentialStorage for CredentialStore {
    fn read_secret(&self, target: &str) -> Result<Option<SecretString>, String> {
        self.read(target).map_err(|error| error.to_string())
    }

    fn write_secret(&self, target: &str, secret: &SecretString) -> Result<(), String> {
        self.write(target, secret)
            .map_err(|error| error.to_string())
    }

    fn delete_secret(&self, target: &str) -> Result<bool, String> {
        self.delete(target).map_err(|error| error.to_string())
    }
}

pub trait ConfigPersistence {
    fn save_immediately(&self, config: &AppConfig) -> Result<(), String>;
}

pub fn parse_argument_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

pub fn arguments_editor_text(arguments: &[String]) -> String {
    arguments.join("\n")
}

pub fn draft_from_profile(profile: &AgentProfile) -> AgentProfileDraft {
    let (kind, endpoint, model, executable, arguments) = match &profile.connection {
        AgentConnectionProfile::Http {
            endpoint,
            model,
            deployment,
        } => (
            if *deployment == HttpDeployment::Cloud {
                AgentProfileKind::CloudHttp
            } else {
                AgentProfileKind::LocalHttp
            },
            endpoint.clone(),
            model.clone(),
            String::new(),
            Vec::new(),
        ),
        AgentConnectionProfile::Cli {
            executable,
            arguments,
        } => (
            AgentProfileKind::CodexCli,
            String::new(),
            String::new(),
            executable.to_string_lossy().into_owned(),
            arguments.clone(),
        ),
    };
    AgentProfileDraft {
        existing_id: Some(profile.id.clone()),
        display_name: profile.display_name.clone(),
        kind,
        endpoint,
        model,
        executable,
        arguments,
    }
}

pub fn save_profile<C: CredentialStorage, P: ConfigPersistence>(
    current: &AppConfig,
    draft: &AgentProfileDraft,
    credential_edit: CredentialEdit,
    id_seed: u64,
    credentials: &C,
    persistence: &P,
) -> Result<AppConfig, AiManagementError> {
    validate_agent_profile_draft(draft, &current.agent_profiles)
        .map_err(|error| AiManagementError::new(error.to_string()))?;

    let existing = draft.existing_id.as_deref().and_then(|id| {
        current
            .agent_profiles
            .iter()
            .find(|profile| profile.id == id)
    });
    if draft.existing_id.is_some() && existing.is_none() {
        return Err(AiManagementError::new("要编辑的模型已不存在"));
    }

    let id = existing.map_or_else(
        || allocate_agent_profile_id(&current.agent_profiles, id_seed),
        |profile| profile.id.clone(),
    );
    let old_target = existing.and_then(|profile| {
        profile
            .credential_reference
            .as_ref()
            .map(|reference| reference.target.clone())
    });

    let (new_reference, replacement_secret) = match draft.kind {
        AgentProfileKind::CodexCli => {
            if matches!(credential_edit, CredentialEdit::Replace(_)) {
                return Err(AiManagementError::new("Codex CLI 不使用 API Key"));
            }
            (None, None)
        }
        AgentProfileKind::CloudHttp | AgentProfileKind::LocalHttp => match credential_edit {
            CredentialEdit::Keep => (
                existing.and_then(|profile| profile.credential_reference.clone()),
                None,
            ),
            CredentialEdit::Replace(secret) => {
                let target = credential_target(&id)
                    .map_err(|error| AiManagementError::new(error.to_string()))?;
                let secret = SecretString::new(secret)
                    .map_err(|error| AiManagementError::new(error.to_string()))?;
                (Some(CredentialReference { target }), Some(secret))
            }
            CredentialEdit::Clear => (None, None),
        },
    };

    let mut next = current.clone();
    let profile = build_agent_profile(draft, id.clone(), new_reference.clone());
    if let Some(index) = next
        .agent_profiles
        .iter()
        .position(|profile| profile.id == id)
    {
        next.agent_profiles[index] = profile;
    } else {
        next.agent_profiles.push(profile);
    }
    next.agent_management_initialized = true;
    if next.selected_agent_profile_id.is_none() {
        next.selected_agent_profile_id = Some(id);
    }

    commit_with_credentials(
        &next,
        old_target.as_deref(),
        new_reference
            .as_ref()
            .map(|reference| reference.target.as_str()),
        replacement_secret.as_ref(),
        credentials,
        persistence,
    )?;
    Ok(next)
}

pub fn delete_profile<C: CredentialStorage, P: ConfigPersistence>(
    current: &AppConfig,
    profile_id: &str,
    credentials: &C,
    persistence: &P,
) -> Result<AppConfig, AiManagementError> {
    let Some(index) = current
        .agent_profiles
        .iter()
        .position(|profile| profile.id == profile_id)
    else {
        return Err(AiManagementError::new("要删除的模型已不存在"));
    };
    let old_target = current.agent_profiles[index]
        .credential_reference
        .as_ref()
        .map(|reference| reference.target.as_str());
    let mut next = current.clone();
    next.agent_profiles.remove(index);
    if next.selected_agent_profile_id.as_deref() == Some(profile_id) {
        next.selected_agent_profile_id = None;
    }
    next.agent_management_initialized = true;

    commit_with_credentials(&next, old_target, None, None, credentials, persistence)?;
    Ok(next)
}

fn commit_with_credentials<C: CredentialStorage, P: ConfigPersistence>(
    next: &AppConfig,
    old_target: Option<&str>,
    new_target: Option<&str>,
    replacement_secret: Option<&SecretString>,
    credentials: &C,
    persistence: &P,
) -> Result<(), AiManagementError> {
    let targets_differ = old_target != new_target;
    let credential_changes = replacement_secret.is_some() || targets_differ;
    if !credential_changes {
        return persistence
            .save_immediately(next)
            .map_err(AiManagementError::new);
    }

    let old_secret = old_target
        .map(|target| credentials.read_secret(target))
        .transpose()
        .map_err(AiManagementError::new)?
        .flatten();

    if let (Some(target), Some(secret)) = (new_target, replacement_secret) {
        credentials
            .write_secret(target, secret)
            .map_err(AiManagementError::new)?;
        if targets_differ
            && let Some(old_target) = old_target
            && let Err(error) = credentials.delete_secret(old_target)
        {
            if target != old_target {
                let _ = credentials.delete_secret(target);
            }
            return Err(AiManagementError::new(error));
        }
    } else if targets_differ && let Some(target) = old_target {
        credentials
            .delete_secret(target)
            .map_err(AiManagementError::new)?;
    }

    if let Err(save_error) = persistence.save_immediately(next) {
        let rollback =
            rollback_credentials(old_target, new_target, old_secret.as_ref(), credentials);
        return Err(AiManagementError::new(match rollback {
            Ok(()) => save_error,
            Err(rollback_error) => {
                format!("{save_error}；凭据回滚也失败：{rollback_error}")
            }
        }));
    }

    Ok(())
}

fn rollback_credentials<C: CredentialStorage>(
    old_target: Option<&str>,
    new_target: Option<&str>,
    old_secret: Option<&SecretString>,
    credentials: &C,
) -> Result<(), String> {
    if let Some(target) = new_target
        && Some(target) != old_target
    {
        credentials.delete_secret(target)?;
    }
    if let Some(target) = old_target {
        if let Some(secret) = old_secret {
            credentials.write_secret(target, secret)?;
        } else {
            credentials.delete_secret(target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::HashMap, path::PathBuf};

    use super::*;

    #[derive(Default)]
    struct MemoryCredentials(RefCell<HashMap<String, String>>);

    impl CredentialStorage for MemoryCredentials {
        fn read_secret(&self, target: &str) -> Result<Option<SecretString>, String> {
            self.0
                .borrow()
                .get(target)
                .cloned()
                .map(SecretString::new)
                .transpose()
                .map_err(|error| error.to_string())
        }

        fn write_secret(&self, target: &str, secret: &SecretString) -> Result<(), String> {
            self.0
                .borrow_mut()
                .insert(target.into(), secret.expose_secret().into());
            Ok(())
        }

        fn delete_secret(&self, target: &str) -> Result<bool, String> {
            Ok(self.0.borrow_mut().remove(target).is_some())
        }
    }

    struct MemoryPersistence {
        fail: bool,
        saved: RefCell<Option<AppConfig>>,
    }

    impl ConfigPersistence for MemoryPersistence {
        fn save_immediately(&self, config: &AppConfig) -> Result<(), String> {
            if self.fail {
                Err("配置保存失败".into())
            } else {
                self.saved.replace(Some(config.clone()));
                Ok(())
            }
        }
    }

    fn draft(existing_id: Option<&str>) -> AgentProfileDraft {
        AgentProfileDraft {
            existing_id: existing_id.map(str::to_owned),
            display_name: "DeepSeek".into(),
            kind: AgentProfileKind::CloudHttp,
            endpoint: "https://api.example.com/v1/chat/completions".into(),
            model: "deepseek-chat".into(),
            executable: String::new(),
            arguments: Vec::new(),
        }
    }

    #[test]
    fn replacing_a_key_commits_only_after_configuration_save() {
        let credentials = MemoryCredentials::default();
        let persistence = MemoryPersistence {
            fail: false,
            saved: RefCell::new(None),
        };
        let next = save_profile(
            &AppConfig::default(),
            &draft(None),
            CredentialEdit::Replace("new-secret".into()),
            7,
            &credentials,
            &persistence,
        )
        .unwrap();
        let target = next.agent_profiles[0]
            .credential_reference
            .as_ref()
            .unwrap()
            .target
            .clone();

        assert_eq!(credentials.0.borrow().get(&target).unwrap(), "new-secret");
        assert_eq!(persistence.saved.borrow().as_ref(), Some(&next));
    }

    #[test]
    fn failed_configuration_save_restores_the_previous_key() {
        let credentials = MemoryCredentials::default();
        let target = credential_target("managed").unwrap();
        credentials
            .0
            .borrow_mut()
            .insert(target.clone(), "old-secret".into());
        let mut current = AppConfig::default();
        current.agent_profiles = vec![AgentProfile {
            id: "managed".into(),
            display_name: "DeepSeek".into(),
            connection: AgentConnectionProfile::Http {
                endpoint: "https://old.example/v1/chat/completions".into(),
                model: "old".into(),
                deployment: HttpDeployment::Cloud,
            },
            credential_reference: Some(CredentialReference {
                target: target.clone(),
            }),
        }];
        let persistence = MemoryPersistence {
            fail: true,
            saved: RefCell::new(None),
        };

        assert!(
            save_profile(
                &current,
                &draft(Some("managed")),
                CredentialEdit::Replace("new-secret".into()),
                8,
                &credentials,
                &persistence,
            )
            .is_err()
        );
        assert_eq!(credentials.0.borrow().get(&target).unwrap(), "old-secret");
    }

    #[test]
    fn deleting_the_selected_profile_allows_runtime_to_choose_a_fallback() {
        let profile = |id: &str| AgentProfile {
            id: id.into(),
            display_name: id.into(),
            connection: AgentConnectionProfile::Cli {
                executable: PathBuf::from("codex.exe"),
                arguments: Vec::new(),
            },
            credential_reference: None,
        };
        let mut current = AppConfig::default();
        current.agent_profiles = vec![profile("first"), profile("second")];
        current.selected_agent_profile_id = Some("first".into());
        let persistence = MemoryPersistence {
            fail: false,
            saved: RefCell::new(None),
        };

        let next = delete_profile(
            &current,
            "first",
            &MemoryCredentials::default(),
            &persistence,
        )
        .unwrap();

        assert_eq!(next.selected_agent_profile_id, None);
    }

    #[test]
    fn argument_editor_uses_one_literal_argument_per_line() {
        let parsed = parse_argument_lines("--model\ngpt-5\nvalue with spaces\n");
        assert_eq!(parsed, ["--model", "gpt-5", "value with spaces"]);
        assert_eq!(
            arguments_editor_text(&parsed),
            "--model\ngpt-5\nvalue with spaces"
        );
    }
}

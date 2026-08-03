use std::{error::Error, fmt, path::PathBuf};

use url::Url;

use crate::config::{AgentConnectionProfile, AgentProfile, CredentialReference, HttpDeployment};

const MAX_DISPLAY_NAME_CHARS: usize = 64;
const MAX_MODEL_CHARS: usize = 128;
const MAX_ENDPOINT_CHARS: usize = 2048;
const MAX_EXECUTABLE_CHARS: usize = 1024;
const MAX_ARGUMENTS: usize = 32;
const MAX_ARGUMENT_CHARS: usize = 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentProfileKind {
    CloudHttp,
    LocalHttp,
    CodexCli,
}

impl AgentProfileKind {
    pub const fn ui_index(self) -> i32 {
        match self {
            Self::CloudHttp => 0,
            Self::LocalHttp => 1,
            Self::CodexCli => 2,
        }
    }

    pub const fn from_ui_index(index: i32) -> Option<Self> {
        match index {
            0 => Some(Self::CloudHttp),
            1 => Some(Self::LocalHttp),
            2 => Some(Self::CodexCli),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentProfileDraft {
    pub existing_id: Option<String>,
    pub display_name: String,
    pub kind: AgentProfileKind,
    pub endpoint: String,
    pub model: String,
    pub executable: String,
    pub arguments: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentProfileField {
    Name,
    Endpoint,
    Model,
    Executable,
    Arguments,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentProfileValidationError {
    pub field: AgentProfileField,
    pub message: String,
}

impl AgentProfileValidationError {
    fn new(field: AgentProfileField, message: impl Into<String>) -> Self {
        Self {
            field,
            message: message.into(),
        }
    }
}

impl fmt::Display for AgentProfileValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for AgentProfileValidationError {}

pub fn allocate_agent_profile_id(existing: &[AgentProfile], seed: u64) -> String {
    for suffix in 0u32.. {
        let candidate = if suffix == 0 {
            format!("model-{seed:x}")
        } else {
            format!("model-{seed:x}-{suffix}")
        };
        if !existing.iter().any(|profile| profile.id == candidate) {
            return candidate;
        }
    }
    unreachable!("u32 profile ID suffix space is exhausted")
}

pub fn validate_agent_profile_draft(
    draft: &AgentProfileDraft,
    existing: &[AgentProfile],
) -> Result<(), AgentProfileValidationError> {
    let name = draft.display_name.trim();
    if name.is_empty() || name.chars().count() > MAX_DISPLAY_NAME_CHARS {
        return Err(AgentProfileValidationError::new(
            AgentProfileField::Name,
            "模型名称需要包含 1 至 64 个字符",
        ));
    }
    if existing.iter().any(|profile| {
        draft.existing_id.as_deref() != Some(profile.id.as_str())
            && profile.display_name.trim().eq_ignore_ascii_case(name)
    }) {
        return Err(AgentProfileValidationError::new(
            AgentProfileField::Name,
            "模型名称已存在",
        ));
    }

    match draft.kind {
        AgentProfileKind::CloudHttp | AgentProfileKind::LocalHttp => {
            validate_endpoint(&draft.endpoint)?;
            let model = draft.model.trim();
            if model.is_empty() || model.chars().count() > MAX_MODEL_CHARS {
                return Err(AgentProfileValidationError::new(
                    AgentProfileField::Model,
                    "模型 ID 需要包含 1 至 128 个字符",
                ));
            }
        }
        AgentProfileKind::CodexCli => {
            let executable = draft.executable.trim();
            if executable.is_empty() || executable.chars().count() > MAX_EXECUTABLE_CHARS {
                return Err(AgentProfileValidationError::new(
                    AgentProfileField::Executable,
                    "需要填写 Codex 可执行文件路径或命令名",
                ));
            }
            if draft.arguments.len() > MAX_ARGUMENTS
                || draft
                    .arguments
                    .iter()
                    .any(|argument| argument.contains('\0') || argument.len() > MAX_ARGUMENT_CHARS)
            {
                return Err(AgentProfileValidationError::new(
                    AgentProfileField::Arguments,
                    "附加参数最多 32 项，每项不能超过 1024 字节",
                ));
            }
        }
    }
    Ok(())
}

fn validate_endpoint(endpoint: &str) -> Result<(), AgentProfileValidationError> {
    let endpoint = endpoint.trim();
    if endpoint.is_empty() || endpoint.chars().count() > MAX_ENDPOINT_CHARS {
        return Err(AgentProfileValidationError::new(
            AgentProfileField::Endpoint,
            "需要填写有效的 HTTP 接口地址",
        ));
    }
    let parsed = Url::parse(endpoint).map_err(|_| {
        AgentProfileValidationError::new(AgentProfileField::Endpoint, "接口地址格式无效")
    })?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err(AgentProfileValidationError::new(
            AgentProfileField::Endpoint,
            "接口地址必须是无内嵌凭据的 HTTP 或 HTTPS URL",
        ));
    }
    Ok(())
}

pub fn build_agent_profile(
    draft: &AgentProfileDraft,
    id: String,
    credential_reference: Option<CredentialReference>,
) -> AgentProfile {
    let connection = match draft.kind {
        AgentProfileKind::CloudHttp | AgentProfileKind::LocalHttp => AgentConnectionProfile::Http {
            endpoint: draft.endpoint.trim().to_owned(),
            model: draft.model.trim().to_owned(),
            deployment: if draft.kind == AgentProfileKind::CloudHttp {
                HttpDeployment::Cloud
            } else {
                HttpDeployment::Local
            },
        },
        AgentProfileKind::CodexCli => AgentConnectionProfile::Cli {
            executable: PathBuf::from(draft.executable.trim()),
            arguments: draft.arguments.clone(),
        },
    };
    AgentProfile {
        id,
        display_name: draft.display_name.trim().to_owned(),
        connection,
        credential_reference,
    }
}

pub fn profile_kind(profile: &AgentProfile) -> AgentProfileKind {
    match profile.connection {
        AgentConnectionProfile::Http {
            deployment: HttpDeployment::Cloud,
            ..
        } => AgentProfileKind::CloudHttp,
        AgentConnectionProfile::Http {
            deployment: HttpDeployment::Local,
            ..
        } => AgentProfileKind::LocalHttp,
        AgentConnectionProfile::Cli { .. } => AgentProfileKind::CodexCli,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http_draft() -> AgentProfileDraft {
        AgentProfileDraft {
            existing_id: None,
            display_name: "本地千问".into(),
            kind: AgentProfileKind::LocalHttp,
            endpoint: "http://127.0.0.1:11434/v1/chat/completions".into(),
            model: "qwen3".into(),
            executable: String::new(),
            arguments: Vec::new(),
        }
    }

    #[test]
    fn valid_http_draft_builds_a_local_profile() {
        let draft = http_draft();
        validate_agent_profile_draft(&draft, &[]).unwrap();
        let profile = build_agent_profile(&draft, "model-1".into(), None);

        assert!(matches!(
            profile.connection,
            AgentConnectionProfile::Http {
                deployment: HttpDeployment::Local,
                ..
            }
        ));
    }

    #[test]
    fn duplicate_names_are_rejected_but_the_edited_profile_is_excluded() {
        let mut draft = http_draft();
        let existing = vec![build_agent_profile(&draft, "model-1".into(), None)];
        assert_eq!(
            validate_agent_profile_draft(&draft, &existing)
                .unwrap_err()
                .field,
            AgentProfileField::Name
        );

        draft.existing_id = Some("model-1".into());
        validate_agent_profile_draft(&draft, &existing).unwrap();
    }

    #[test]
    fn endpoint_rejects_non_http_and_embedded_credentials() {
        for endpoint in ["file:///tmp/model", "https://user:secret@example.com/v1"] {
            let mut draft = http_draft();
            draft.endpoint = endpoint.into();
            assert_eq!(
                validate_agent_profile_draft(&draft, &[]).unwrap_err().field,
                AgentProfileField::Endpoint
            );
        }
    }

    #[test]
    fn stable_id_allocation_skips_collisions() {
        let draft = http_draft();
        let existing = vec![
            build_agent_profile(&draft, "model-2a".into(), None),
            build_agent_profile(&draft, "model-2a-1".into(), None),
        ];

        assert_eq!(allocate_agent_profile_id(&existing, 42), "model-2a-2");
    }
}

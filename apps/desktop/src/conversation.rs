use agent_core::{
    AgentEvent, AgentMessage, AgentRequest, ConnectorError, ConnectorErrorCode, EventSequenceError,
    MessageRole, RequestId, RunPhase, RunTranscript,
};

pub const MAX_RESPONSE_BYTES: usize = 64 * 1024;
pub const MAX_HISTORY_MESSAGES: usize = 24;
const MAX_HISTORY_BYTES: usize = 256 * 1024;
const MAX_PROMPT_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConversationRole {
    User,
    Assistant,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConversationMessage {
    pub role: ConversationRole,
    pub author: String,
    pub text: String,
    pub pending: bool,
    pub failed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConversationView {
    pub messages: Vec<ConversationMessage>,
    pub busy: bool,
    pub stopping: bool,
    pub can_retry: bool,
    pub phase: Option<RunPhase>,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct BeginRequest {
    pub connector_id: String,
    pub request: AgentRequest,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BeginError {
    Busy,
    InvalidPrompt,
    RetryUnavailable,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ApplyOutcome {
    pub terminal: bool,
    pub cancel_transport: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConversationError {
    NoActiveRequest,
    EventSequence(EventSequenceError),
}

struct ActiveRequest {
    transcript: RunTranscript,
    assistant_index: usize,
    stop_requested: bool,
}

#[derive(Clone)]
struct RetrySeed {
    connector_id: String,
    connector_name: String,
}

pub struct ConversationController {
    next_request_id: u64,
    messages: Vec<ConversationMessage>,
    active: Option<ActiveRequest>,
    retry_seed: Option<RetrySeed>,
    can_retry: bool,
    last_phase: Option<RunPhase>,
    last_error: Option<String>,
}

impl ConversationController {
    pub fn new() -> Self {
        Self {
            next_request_id: 1,
            messages: Vec::new(),
            active: None,
            retry_seed: None,
            can_retry: false,
            last_phase: None,
            last_error: None,
        }
    }

    pub fn begin(
        &mut self,
        connector_id: &str,
        connector_name: &str,
        prompt: &str,
    ) -> Result<BeginRequest, BeginError> {
        let prompt = prompt.trim();
        if prompt.is_empty() || prompt.len() > MAX_PROMPT_BYTES {
            return Err(BeginError::InvalidPrompt);
        }
        if self.active.is_some() {
            return Err(BeginError::Busy);
        }

        self.trim_history(MAX_HISTORY_MESSAGES.saturating_sub(2));
        let request_id = self.allocate_request_id();
        let mut request = self.request_from_history(request_id);
        request.push_message(MessageRole::User, prompt);
        self.messages.push(ConversationMessage {
            role: ConversationRole::User,
            author: "你".into(),
            text: prompt.to_owned(),
            pending: false,
            failed: false,
        });
        let result = self.begin_with_request(connector_id, connector_name, request);
        self.retry_seed = Some(RetrySeed {
            connector_id: connector_id.to_owned(),
            connector_name: connector_name.to_owned(),
        });
        Ok(result)
    }

    pub fn retry(&mut self) -> Result<BeginRequest, BeginError> {
        if self.active.is_some() {
            return Err(BeginError::Busy);
        }
        if !self.can_retry {
            return Err(BeginError::RetryUnavailable);
        }
        let seed = self
            .retry_seed
            .clone()
            .ok_or(BeginError::RetryUnavailable)?;

        self.trim_history(MAX_HISTORY_MESSAGES.saturating_sub(1));
        let request_id = self.allocate_request_id();
        let request = self.request_from_history(request_id);
        Ok(self.begin_with_request(&seed.connector_id, &seed.connector_name, request))
    }

    pub fn clear_retry(&mut self) {
        if self.active.is_none() {
            self.can_retry = false;
            self.retry_seed = None;
        }
    }

    pub fn request_stop(&mut self) -> Option<RequestId> {
        let active = self.active.as_mut()?;
        if active.stop_requested {
            return None;
        }
        active.stop_requested = true;
        Some(active.transcript.request_id())
    }

    pub fn fail_to_start(
        &mut self,
        error: ConnectorError,
    ) -> Result<ApplyOutcome, ConversationError> {
        let request_id = self
            .active
            .as_ref()
            .ok_or(ConversationError::NoActiveRequest)?
            .transcript
            .request_id();
        self.apply_event(AgentEvent::Started { request_id })?;
        self.apply_event(AgentEvent::Failed { request_id, error })
    }

    pub fn channel_closed(
        &mut self,
        request_id: RequestId,
    ) -> Result<ApplyOutcome, ConversationError> {
        let phase = self
            .active
            .as_ref()
            .filter(|active| active.transcript.request_id() == request_id)
            .ok_or(ConversationError::NoActiveRequest)?
            .transcript
            .phase();
        if phase == RunPhase::AwaitingStart {
            self.apply_event(AgentEvent::Started { request_id })?;
        }
        self.apply_event(AgentEvent::Failed {
            request_id,
            error: ConnectorError::new(
                ConnectorErrorCode::Transport,
                "agent event stream ended before a terminal event",
            ),
        })
    }

    pub fn apply_event(&mut self, event: AgentEvent) -> Result<ApplyOutcome, ConversationError> {
        let active = self
            .active
            .as_mut()
            .ok_or(ConversationError::NoActiveRequest)?;
        if event.request_id() != active.transcript.request_id() {
            return Err(ConversationError::EventSequence(
                EventSequenceError::RequestIdMismatch {
                    expected: active.transcript.request_id(),
                    actual: event.request_id(),
                },
            ));
        }

        if let AgentEvent::TextDelta { text, .. } = &event
            && active
                .transcript
                .response_text()
                .len()
                .saturating_add(text.len())
                > MAX_RESPONSE_BYTES
        {
            let request_id = active.transcript.request_id();
            active
                .transcript
                .apply_event(AgentEvent::Failed {
                    request_id,
                    error: ConnectorError::new(
                        ConnectorErrorCode::Protocol,
                        "agent response exceeded the desktop transcript limit",
                    ),
                })
                .map_err(ConversationError::EventSequence)?;
            self.finish_active();
            return Ok(ApplyOutcome {
                terminal: true,
                cancel_transport: true,
            });
        }

        active
            .transcript
            .apply_event(event)
            .map_err(ConversationError::EventSequence)?;
        let terminal = active.transcript.phase().is_terminal();
        self.sync_active_message();
        if terminal {
            self.finish_active();
        }
        Ok(ApplyOutcome {
            terminal,
            cancel_transport: false,
        })
    }

    pub fn view(&self) -> ConversationView {
        ConversationView {
            messages: self.messages.clone(),
            busy: self.active.is_some(),
            stopping: self
                .active
                .as_ref()
                .is_some_and(|active| active.stop_requested),
            can_retry: self.can_retry,
            phase: self
                .active
                .as_ref()
                .map(|active| active.transcript.phase())
                .or(self.last_phase),
            error: self.last_error.clone(),
        }
    }

    fn begin_with_request(
        &mut self,
        connector_id: &str,
        connector_name: &str,
        request: AgentRequest,
    ) -> BeginRequest {
        let assistant_index = self.messages.len();
        self.messages.push(ConversationMessage {
            role: ConversationRole::Assistant,
            author: connector_name.to_owned(),
            text: String::new(),
            pending: true,
            failed: false,
        });
        self.active = Some(ActiveRequest {
            transcript: RunTranscript::new(request.id),
            assistant_index,
            stop_requested: false,
        });
        self.can_retry = false;
        self.last_phase = Some(RunPhase::AwaitingStart);
        self.last_error = None;
        BeginRequest {
            connector_id: connector_id.to_owned(),
            request,
        }
    }

    fn finish_active(&mut self) {
        self.sync_active_message();
        let Some(active) = self.active.take() else {
            return;
        };
        let phase = active.transcript.phase();
        self.last_phase = Some(phase);
        self.can_retry = active.transcript.is_retryable();
        self.last_error = active.transcript.error().map(|error| error.message.clone());
        if let Some(message) = self.messages.get_mut(active.assistant_index) {
            message.pending = false;
            message.failed = phase == RunPhase::Failed;
            if message.text.is_empty() {
                message.text = match phase {
                    RunPhase::Cancelled => "已停止".into(),
                    RunPhase::Failed => self
                        .last_error
                        .clone()
                        .unwrap_or_else(|| "智能体请求失败".into()),
                    RunPhase::Completed => "回答为空".into(),
                    RunPhase::AwaitingStart | RunPhase::Streaming => String::new(),
                };
            }
        }
        self.trim_history(MAX_HISTORY_MESSAGES);
    }

    fn sync_active_message(&mut self) {
        let Some(active) = self.active.as_ref() else {
            return;
        };
        if let Some(message) = self.messages.get_mut(active.assistant_index) {
            message.text = active.transcript.response_text().to_owned();
            message.pending = !active.transcript.phase().is_terminal();
            message.failed = active.transcript.phase() == RunPhase::Failed;
        }
    }

    fn request_from_history(&self, request_id: RequestId) -> AgentRequest {
        let messages = self
            .messages
            .iter()
            .filter(|message| !message.pending && !message.failed && !message.text.is_empty())
            .map(|message| AgentMessage {
                role: match message.role {
                    ConversationRole::User => MessageRole::User,
                    ConversationRole::Assistant => MessageRole::Assistant,
                },
                content: message.text.clone(),
            })
            .collect();
        AgentRequest {
            id: request_id,
            messages,
            attachments: Vec::new(),
        }
    }

    fn allocate_request_id(&mut self) -> RequestId {
        let request_id = RequestId(self.next_request_id);
        self.next_request_id = self.next_request_id.checked_add(1).unwrap_or(1);
        request_id
    }

    fn trim_history(&mut self, message_limit: usize) {
        while self.messages.len() > message_limit
            || self
                .messages
                .iter()
                .map(|message| message.text.len())
                .sum::<usize>()
                > MAX_HISTORY_BYTES
        {
            self.messages.remove(0);
        }
        while self
            .messages
            .first()
            .is_some_and(|message| message.role == ConversationRole::Assistant)
        {
            self.messages.remove(0);
        }
    }
}

impl Default for ConversationController {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use agent_core::{AgentEvent, ConnectorError, ConnectorErrorCode, RequestId, RunPhase};

    use super::{BeginError, ConversationController, MAX_HISTORY_MESSAGES, MAX_RESPONSE_BYTES};

    fn start_streaming(
        controller: &mut ConversationController,
        connector_id: &str,
        prompt: &str,
    ) -> RequestId {
        let request_id = controller
            .begin(connector_id, connector_id, prompt)
            .unwrap()
            .request
            .id;
        controller
            .apply_event(AgentEvent::Started { request_id })
            .unwrap();
        request_id
    }

    #[test]
    fn only_one_request_can_be_active() {
        let mut controller = ConversationController::new();
        let first = controller.begin("codex-cli", "Codex CLI", "first").unwrap();

        let error = controller
            .begin("codex-cli", "Codex CLI", "second")
            .unwrap_err();

        assert_eq!(first.request.id, RequestId(1));
        assert_eq!(error, BeginError::Busy);
        assert!(controller.view().busy);
    }

    #[test]
    fn retry_reuses_the_prompt_but_allocates_a_new_request_id() {
        let mut controller = ConversationController::new();
        let first_id = start_streaming(&mut controller, "openai", "try this");
        controller
            .apply_event(AgentEvent::Failed {
                request_id: first_id,
                error: ConnectorError::new(ConnectorErrorCode::RateLimited, "try later"),
            })
            .unwrap();

        assert!(controller.view().can_retry);
        let retry = controller.retry().unwrap();

        assert_eq!(retry.request.id, RequestId(2));
        assert_eq!(retry.connector_id, "openai");
        assert_eq!(retry.request.latest_user_text(), Some("try this"));
        assert!(controller.view().busy);
    }

    #[test]
    fn completed_message_keeps_its_original_agent_name_after_switching() {
        let mut controller = ConversationController::new();
        let first_id = controller
            .begin("codex-cli", "Codex CLI", "first")
            .unwrap()
            .request
            .id;
        controller
            .apply_event(AgentEvent::Started {
                request_id: first_id,
            })
            .unwrap();
        controller
            .apply_event(AgentEvent::Completed {
                request_id: first_id,
            })
            .unwrap();

        controller.begin("deepseek", "DeepSeek", "second").unwrap();
        let view = controller.view();

        assert_eq!(view.messages[1].author, "Codex CLI");
        assert_eq!(view.messages.last().unwrap().author, "DeepSeek");
    }

    #[test]
    fn stop_is_idempotent_while_waiting_for_the_cancelled_event() {
        let mut controller = ConversationController::new();
        let request_id = start_streaming(&mut controller, "codex-cli", "stop this");

        assert_eq!(controller.request_stop(), Some(request_id));
        assert_eq!(controller.request_stop(), None);
        assert!(controller.view().stopping);

        controller
            .apply_event(AgentEvent::Cancelled { request_id })
            .unwrap();
        assert!(!controller.view().busy);
        assert_eq!(controller.view().phase, Some(RunPhase::Cancelled));
    }

    #[test]
    fn retry_is_offered_only_for_retryable_failures() {
        let mut controller = ConversationController::new();
        let request_id = start_streaming(&mut controller, "openai", "auth check");
        controller
            .apply_event(AgentEvent::Failed {
                request_id,
                error: ConnectorError::new(ConnectorErrorCode::Unauthorized, "not authorized"),
            })
            .unwrap();

        assert!(!controller.view().can_retry);
        assert!(matches!(
            controller.retry(),
            Err(BeginError::RetryUnavailable)
        ));
    }

    #[test]
    fn oversized_response_is_failed_and_requests_transport_cancellation() {
        let mut controller = ConversationController::new();
        let request_id = start_streaming(&mut controller, "codex-cli", "large response");
        controller
            .apply_event(AgentEvent::TextDelta {
                request_id,
                text: "x".repeat(MAX_RESPONSE_BYTES),
            })
            .unwrap();

        let outcome = controller
            .apply_event(AgentEvent::TextDelta {
                request_id,
                text: "overflow".into(),
            })
            .unwrap();
        let view = controller.view();

        assert!(outcome.cancel_transport);
        assert!(outcome.terminal);
        assert_eq!(view.phase, Some(RunPhase::Failed));
        assert!(view.messages.last().unwrap().text.len() <= MAX_RESPONSE_BYTES);
        assert!(!view.can_retry);
    }

    #[test]
    fn completed_history_stays_within_the_message_limit() {
        let mut controller = ConversationController::new();

        for index in 0..(MAX_HISTORY_MESSAGES + 8) {
            let request_id =
                start_streaming(&mut controller, "codex-cli", &format!("question {index}"));
            controller
                .apply_event(AgentEvent::TextDelta {
                    request_id,
                    text: format!("answer {index}"),
                })
                .unwrap();
            controller
                .apply_event(AgentEvent::Completed { request_id })
                .unwrap();
        }

        assert!(controller.view().messages.len() <= MAX_HISTORY_MESSAGES);
        assert!(
            controller
                .view()
                .messages
                .last()
                .unwrap()
                .text
                .contains("answer")
        );
    }
}

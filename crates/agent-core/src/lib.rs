use std::{
    error::Error,
    fmt,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, RecvError, RecvTimeoutError, TryRecvError},
    },
    time::Duration,
};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RequestId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransportKind {
    Http,
    Cli,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttachmentMode {
    Unsupported,
    Upload,
    LocalPath,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConnectorCapabilities {
    pub streaming: bool,
    pub attachment_mode: AttachmentMode,
    pub image_input: bool,
    pub local_execution: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectorDescriptor {
    pub id: String,
    pub display_name: String,
    pub transport: TransportKind,
    pub capabilities: ConnectorCapabilities,
}

impl ConnectorDescriptor {
    pub fn new(
        id: impl Into<String>,
        display_name: impl Into<String>,
        transport: TransportKind,
        capabilities: ConnectorCapabilities,
    ) -> Self {
        Self {
            id: id.into(),
            display_name: display_name.into(),
            transport,
            capabilities,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessageRole {
    System,
    User,
    Assistant,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentMessage {
    pub role: MessageRole,
    pub content: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttachmentKind {
    File,
    Image,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttachmentRef {
    pub path: PathBuf,
    pub size_bytes: u64,
    pub kind: AttachmentKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentRequest {
    pub id: RequestId,
    pub messages: Vec<AgentMessage>,
    pub attachments: Vec<AttachmentRef>,
}

impl AgentRequest {
    pub fn new(id: RequestId) -> Self {
        Self {
            id,
            messages: Vec::new(),
            attachments: Vec::new(),
        }
    }

    pub fn single_user_message(id: RequestId, content: impl Into<String>) -> Self {
        let mut request = Self::new(id);
        request.push_message(MessageRole::User, content);
        request
    }

    pub fn push_message(&mut self, role: MessageRole, content: impl Into<String>) {
        self.messages.push(AgentMessage {
            role,
            content: content.into(),
        });
    }

    pub fn latest_user_text(&self) -> Option<&str> {
        self.messages
            .iter()
            .rev()
            .find(|message| message.role == MessageRole::User)
            .map(|message| message.content.as_str())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectorErrorCode {
    Configuration,
    InvalidRequest,
    Unauthorized,
    RateLimited,
    Timeout,
    Transport,
    Protocol,
    Process,
    UnsupportedCapability,
}

impl ConnectorErrorCode {
    pub const fn is_retryable(self) -> bool {
        matches!(
            self,
            Self::RateLimited | Self::Timeout | Self::Transport | Self::Process
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectorError {
    pub code: ConnectorErrorCode,
    pub message: String,
}

impl ConnectorError {
    pub fn new(code: ConnectorErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub const fn is_retryable(&self) -> bool {
        self.code.is_retryable()
    }
}

impl fmt::Display for ConnectorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.message)
    }
}

impl Error for ConnectorError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentEvent {
    Started {
        request_id: RequestId,
    },
    TextDelta {
        request_id: RequestId,
        text: String,
    },
    Completed {
        request_id: RequestId,
    },
    Cancelled {
        request_id: RequestId,
    },
    Failed {
        request_id: RequestId,
        error: ConnectorError,
    },
}

#[derive(Clone, Debug)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

pub struct AgentRun {
    events: Receiver<AgentEvent>,
    cancellation: CancellationToken,
}

impl AgentRun {
    pub fn new(events: Receiver<AgentEvent>, cancellation: CancellationToken) -> Self {
        Self {
            events,
            cancellation,
        }
    }

    pub fn cancel(&self) {
        self.cancellation.cancel();
    }

    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    pub fn recv(&self) -> Result<AgentEvent, RecvError> {
        self.events.recv()
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Result<AgentEvent, RecvTimeoutError> {
        self.events.recv_timeout(timeout)
    }

    pub fn try_recv(&self) -> Result<AgentEvent, TryRecvError> {
        self.events.try_recv()
    }
}

impl Drop for AgentRun {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

pub trait AgentConnector: Send + Sync {
    fn descriptor(&self) -> &ConnectorDescriptor;

    fn start(&self, request: AgentRequest) -> Result<AgentRun, ConnectorError>;
}

pub fn validate_request(
    request: &AgentRequest,
    capabilities: ConnectorCapabilities,
) -> Result<(), ConnectorError> {
    if request
        .latest_user_text()
        .is_none_or(|text| text.trim().is_empty())
    {
        return Err(ConnectorError::new(
            ConnectorErrorCode::InvalidRequest,
            "request must contain a non-empty user message",
        ));
    }

    if !request.attachments.is_empty()
        && capabilities.attachment_mode == AttachmentMode::Unsupported
    {
        return Err(ConnectorError::new(
            ConnectorErrorCode::UnsupportedCapability,
            "connector does not support attachments",
        ));
    }

    if !capabilities.image_input
        && request
            .attachments
            .iter()
            .any(|attachment| attachment.kind == AttachmentKind::Image)
    {
        return Err(ConnectorError::new(
            ConnectorErrorCode::UnsupportedCapability,
            "connector does not support image input",
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, sync::mpsc, time::Duration};

    use super::{
        AgentEvent, AgentRequest, AgentRun, AttachmentKind, AttachmentMode, AttachmentRef,
        CancellationToken, ConnectorCapabilities, ConnectorErrorCode, MessageRole, RequestId,
        validate_request,
    };

    fn text_capabilities() -> ConnectorCapabilities {
        ConnectorCapabilities {
            streaming: true,
            attachment_mode: AttachmentMode::Unsupported,
            image_input: false,
            local_execution: false,
        }
    }

    #[test]
    fn empty_request_is_rejected_before_connector_work_starts() {
        let error = validate_request(&AgentRequest::new(RequestId(7)), text_capabilities())
            .expect_err("empty requests must be rejected");

        assert_eq!(error.code, ConnectorErrorCode::InvalidRequest);
        assert!(!error.is_retryable());
    }

    #[test]
    fn attachments_require_a_declared_connector_capability() {
        let mut request = AgentRequest::single_user_message(RequestId(8), "summarize this");
        request.attachments.push(AttachmentRef {
            path: PathBuf::from("report.txt"),
            size_bytes: 42,
            kind: AttachmentKind::File,
        });

        let error = validate_request(&request, text_capabilities())
            .expect_err("unsupported attachments must fail before transport work");

        assert_eq!(error.code, ConnectorErrorCode::UnsupportedCapability);
    }

    #[test]
    fn retry_semantics_are_derived_from_machine_readable_error_codes() {
        assert!(ConnectorErrorCode::RateLimited.is_retryable());
        assert!(ConnectorErrorCode::Timeout.is_retryable());
        assert!(ConnectorErrorCode::Transport.is_retryable());
        assert!(!ConnectorErrorCode::Unauthorized.is_retryable());
        assert!(!ConnectorErrorCode::InvalidRequest.is_retryable());
    }

    #[test]
    fn cancellation_is_shared_and_idempotent() {
        let token = CancellationToken::new();
        let worker_token = token.clone();

        token.cancel();
        token.cancel();

        assert!(token.is_cancelled());
        assert!(worker_token.is_cancelled());
    }

    #[test]
    fn run_receives_transport_neutral_events() {
        let (sender, receiver) = mpsc::channel();
        let run = AgentRun::new(receiver, CancellationToken::new());
        sender
            .send(AgentEvent::Started {
                request_id: RequestId(9),
            })
            .unwrap();

        assert_eq!(
            run.recv_timeout(Duration::from_millis(10)).unwrap(),
            AgentEvent::Started {
                request_id: RequestId(9)
            }
        );
    }

    #[test]
    fn dropping_a_run_cancels_its_worker_token() {
        let (_sender, receiver) = mpsc::channel();
        let run = AgentRun::new(receiver, CancellationToken::new());
        let worker_token = run.cancellation_token();

        drop(run);

        assert!(worker_token.is_cancelled());
    }

    #[test]
    fn latest_user_text_ignores_system_and_assistant_messages() {
        let mut request = AgentRequest::new(RequestId(10));
        request.push_message(MessageRole::System, "be concise");
        request.push_message(MessageRole::User, "first question");
        request.push_message(MessageRole::Assistant, "first answer");
        request.push_message(MessageRole::User, "latest question");

        assert_eq!(request.latest_user_text(), Some("latest question"));
    }
}

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentEventKind {
    Started,
    TextDelta,
    Completed,
    Cancelled,
    Failed,
}

impl AgentEvent {
    pub const fn request_id(&self) -> RequestId {
        match self {
            Self::Started { request_id }
            | Self::TextDelta { request_id, .. }
            | Self::Completed { request_id }
            | Self::Cancelled { request_id }
            | Self::Failed { request_id, .. } => *request_id,
        }
    }

    pub const fn kind(&self) -> AgentEventKind {
        match self {
            Self::Started { .. } => AgentEventKind::Started,
            Self::TextDelta { .. } => AgentEventKind::TextDelta,
            Self::Completed { .. } => AgentEventKind::Completed,
            Self::Cancelled { .. } => AgentEventKind::Cancelled,
            Self::Failed { .. } => AgentEventKind::Failed,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunPhase {
    AwaitingStart,
    Streaming,
    Completed,
    Cancelled,
    Failed,
}

impl RunPhase {
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::Failed)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventSequenceError {
    RequestIdMismatch {
        expected: RequestId,
        actual: RequestId,
    },
    UnexpectedEvent {
        phase: RunPhase,
        event: AgentEventKind,
    },
}

impl fmt::Display for EventSequenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RequestIdMismatch { expected, actual } => write!(
                formatter,
                "event request ID {} does not match active request {}",
                actual.0, expected.0
            ),
            Self::UnexpectedEvent { phase, event } => {
                write!(
                    formatter,
                    "event {event:?} is invalid while run is {phase:?}"
                )
            }
        }
    }
}

impl Error for EventSequenceError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunTranscript {
    request_id: RequestId,
    phase: RunPhase,
    response_text: String,
    error: Option<ConnectorError>,
}

impl RunTranscript {
    pub fn new(request_id: RequestId) -> Self {
        Self {
            request_id,
            phase: RunPhase::AwaitingStart,
            response_text: String::new(),
            error: None,
        }
    }

    pub const fn request_id(&self) -> RequestId {
        self.request_id
    }

    pub const fn phase(&self) -> RunPhase {
        self.phase
    }

    pub fn response_text(&self) -> &str {
        &self.response_text
    }

    pub const fn error(&self) -> Option<&ConnectorError> {
        self.error.as_ref()
    }

    pub fn is_retryable(&self) -> bool {
        self.error
            .as_ref()
            .is_some_and(ConnectorError::is_retryable)
    }

    pub fn apply_event(&mut self, event: AgentEvent) -> Result<(), EventSequenceError> {
        let actual_request_id = event.request_id();
        if actual_request_id != self.request_id {
            return Err(EventSequenceError::RequestIdMismatch {
                expected: self.request_id,
                actual: actual_request_id,
            });
        }

        let event_kind = event.kind();
        match (self.phase, event) {
            (RunPhase::AwaitingStart, AgentEvent::Started { .. }) => {
                self.phase = RunPhase::Streaming;
            }
            (RunPhase::Streaming, AgentEvent::TextDelta { text, .. }) => {
                self.response_text.push_str(&text);
            }
            (RunPhase::Streaming, AgentEvent::Completed { .. }) => {
                self.phase = RunPhase::Completed;
            }
            (RunPhase::Streaming, AgentEvent::Cancelled { .. }) => {
                self.phase = RunPhase::Cancelled;
            }
            (RunPhase::Streaming, AgentEvent::Failed { error, .. }) => {
                self.phase = RunPhase::Failed;
                self.error = Some(error);
            }
            (phase, _) => {
                return Err(EventSequenceError::UnexpectedEvent {
                    phase,
                    event: event_kind,
                });
            }
        }

        Ok(())
    }
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

struct ConnectorCatalogEntry {
    id: String,
    connector: Arc<dyn AgentConnector>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConnectorCatalogError {
    EmptyId,
    DuplicateId(String),
    NotFound(String),
}

impl fmt::Display for ConnectorCatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyId => write!(formatter, "connector ID must not be empty"),
            Self::DuplicateId(id) => write!(formatter, "connector ID '{id}' is already registered"),
            Self::NotFound(id) => write!(formatter, "connector ID '{id}' is not registered"),
        }
    }
}

impl Error for ConnectorCatalogError {}

#[derive(Default)]
pub struct ConnectorCatalog {
    entries: Vec<ConnectorCatalogEntry>,
    selected_index: Option<usize>,
}

impl ConnectorCatalog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn register(
        &mut self,
        connector: Arc<dyn AgentConnector>,
    ) -> Result<(), ConnectorCatalogError> {
        let id = connector.descriptor().id.clone();
        if id.trim().is_empty() {
            return Err(ConnectorCatalogError::EmptyId);
        }
        if self.entries.iter().any(|entry| entry.id == id) {
            return Err(ConnectorCatalogError::DuplicateId(id));
        }

        self.entries.push(ConnectorCatalogEntry { id, connector });
        if self.selected_index.is_none() {
            self.selected_index = Some(0);
        }
        Ok(())
    }

    pub fn ids(&self) -> impl ExactSizeIterator<Item = &str> {
        self.entries.iter().map(|entry| entry.id.as_str())
    }

    pub fn connector(&self, id: &str) -> Option<Arc<dyn AgentConnector>> {
        self.entries
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| Arc::clone(&entry.connector))
    }

    pub fn select(&mut self, id: &str) -> Result<(), ConnectorCatalogError> {
        let Some(index) = self.entries.iter().position(|entry| entry.id == id) else {
            return Err(ConnectorCatalogError::NotFound(id.into()));
        };
        self.selected_index = Some(index);
        Ok(())
    }

    pub fn selected_id(&self) -> Option<&str> {
        self.selected_index
            .and_then(|index| self.entries.get(index))
            .map(|entry| entry.id.as_str())
    }

    pub fn selected(&self) -> Option<Arc<dyn AgentConnector>> {
        self.selected_index
            .and_then(|index| self.entries.get(index))
            .map(|entry| Arc::clone(&entry.connector))
    }
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
    use std::{
        path::PathBuf,
        sync::{Arc, mpsc},
        time::Duration,
    };

    use super::{
        AgentEvent, AgentEventKind, AgentRequest, AgentRun, AttachmentKind, AttachmentMode,
        AttachmentRef, CancellationToken, ConnectorCapabilities, ConnectorCatalog,
        ConnectorCatalogError, ConnectorDescriptor, ConnectorError, ConnectorErrorCode,
        EventSequenceError, MessageRole, RequestId, RunPhase, RunTranscript, TransportKind,
        validate_request,
    };

    struct StubConnector {
        descriptor: ConnectorDescriptor,
    }

    impl StubConnector {
        fn new(id: &str) -> Self {
            Self {
                descriptor: ConnectorDescriptor::new(
                    id,
                    id.to_uppercase(),
                    TransportKind::Http,
                    text_capabilities(),
                ),
            }
        }
    }

    impl super::AgentConnector for StubConnector {
        fn descriptor(&self) -> &ConnectorDescriptor {
            &self.descriptor
        }

        fn start(&self, _request: AgentRequest) -> Result<AgentRun, ConnectorError> {
            let (_sender, receiver) = mpsc::channel();
            Ok(AgentRun::new(receiver, CancellationToken::new()))
        }
    }

    fn text_capabilities() -> ConnectorCapabilities {
        ConnectorCapabilities {
            streaming: true,
            attachment_mode: AttachmentMode::Unsupported,
            image_input: false,
            local_execution: false,
        }
    }

    #[test]
    fn connector_catalog_preserves_insertion_order_and_selects_the_first_entry() {
        let mut catalog = ConnectorCatalog::new();
        catalog
            .register(Arc::new(StubConnector::new("codex-cli")))
            .unwrap();
        catalog
            .register(Arc::new(StubConnector::new("openai-http")))
            .unwrap();

        assert_eq!(
            catalog.ids().collect::<Vec<_>>(),
            vec!["codex-cli", "openai-http"]
        );
        assert_eq!(catalog.selected_id(), Some("codex-cli"));
        assert_eq!(catalog.len(), 2);
        assert!(!catalog.is_empty());
    }

    #[test]
    fn connector_catalog_rejects_empty_and_duplicate_ids_without_mutation() {
        let mut catalog = ConnectorCatalog::new();
        assert_eq!(
            catalog.register(Arc::new(StubConnector::new("  "))),
            Err(ConnectorCatalogError::EmptyId)
        );
        catalog
            .register(Arc::new(StubConnector::new("codex-cli")))
            .unwrap();

        assert_eq!(
            catalog.register(Arc::new(StubConnector::new("codex-cli"))),
            Err(ConnectorCatalogError::DuplicateId("codex-cli".into()))
        );
        assert_eq!(catalog.ids().collect::<Vec<_>>(), vec!["codex-cli"]);
    }

    #[test]
    fn connector_catalog_selects_by_stable_id_and_reports_missing_connectors() {
        let mut catalog = ConnectorCatalog::new();
        catalog
            .register(Arc::new(StubConnector::new("codex-cli")))
            .unwrap();
        catalog
            .register(Arc::new(StubConnector::new("openai-http")))
            .unwrap();

        catalog.select("openai-http").unwrap();

        assert_eq!(catalog.selected_id(), Some("openai-http"));
        assert_eq!(
            catalog
                .selected()
                .expect("selected connector")
                .descriptor()
                .id,
            "openai-http"
        );
        assert!(catalog.connector("codex-cli").is_some());
        assert!(catalog.connector("missing").is_none());
        assert_eq!(
            catalog.select("missing"),
            Err(ConnectorCatalogError::NotFound("missing".into()))
        );
        assert_eq!(catalog.selected_id(), Some("openai-http"));
    }

    #[test]
    fn valid_streaming_events_reduce_into_one_completed_transcript() {
        let request_id = RequestId(40);
        let mut transcript = RunTranscript::new(request_id);

        transcript
            .apply_event(AgentEvent::Started { request_id })
            .unwrap();
        transcript
            .apply_event(AgentEvent::TextDelta {
                request_id,
                text: "Hello".into(),
            })
            .unwrap();
        transcript
            .apply_event(AgentEvent::TextDelta {
                request_id,
                text: " world".into(),
            })
            .unwrap();
        transcript
            .apply_event(AgentEvent::Completed { request_id })
            .unwrap();

        assert_eq!(transcript.request_id(), request_id);
        assert_eq!(transcript.phase(), RunPhase::Completed);
        assert_eq!(transcript.response_text(), "Hello world");
        assert_eq!(transcript.error(), None);
    }

    #[test]
    fn event_from_another_request_is_rejected_without_mutating_state() {
        let mut transcript = RunTranscript::new(RequestId(41));

        let error = transcript
            .apply_event(AgentEvent::Started {
                request_id: RequestId(99),
            })
            .unwrap_err();

        assert_eq!(
            error,
            EventSequenceError::RequestIdMismatch {
                expected: RequestId(41),
                actual: RequestId(99),
            }
        );
        assert_eq!(transcript.phase(), RunPhase::AwaitingStart);
    }

    #[test]
    fn text_delta_before_started_is_rejected() {
        let request_id = RequestId(42);
        let mut transcript = RunTranscript::new(request_id);

        let error = transcript
            .apply_event(AgentEvent::TextDelta {
                request_id,
                text: "too early".into(),
            })
            .unwrap_err();

        assert_eq!(
            error,
            EventSequenceError::UnexpectedEvent {
                phase: RunPhase::AwaitingStart,
                event: AgentEventKind::TextDelta,
            }
        );
        assert!(transcript.response_text().is_empty());
    }

    #[test]
    fn duplicate_started_event_is_rejected() {
        let request_id = RequestId(43);
        let mut transcript = RunTranscript::new(request_id);
        transcript
            .apply_event(AgentEvent::Started { request_id })
            .unwrap();

        let error = transcript
            .apply_event(AgentEvent::Started { request_id })
            .unwrap_err();

        assert_eq!(
            error,
            EventSequenceError::UnexpectedEvent {
                phase: RunPhase::Streaming,
                event: AgentEventKind::Started,
            }
        );
    }

    #[test]
    fn cancelled_and_failed_events_preserve_terminal_details() {
        let cancelled_id = RequestId(44);
        let mut cancelled = RunTranscript::new(cancelled_id);
        cancelled
            .apply_event(AgentEvent::Started {
                request_id: cancelled_id,
            })
            .unwrap();
        cancelled
            .apply_event(AgentEvent::Cancelled {
                request_id: cancelled_id,
            })
            .unwrap();
        assert_eq!(cancelled.phase(), RunPhase::Cancelled);
        assert!(!cancelled.is_retryable());

        let failed_id = RequestId(45);
        let mut failed = RunTranscript::new(failed_id);
        failed
            .apply_event(AgentEvent::Started {
                request_id: failed_id,
            })
            .unwrap();
        failed
            .apply_event(AgentEvent::Failed {
                request_id: failed_id,
                error: ConnectorError::new(ConnectorErrorCode::RateLimited, "try later"),
            })
            .unwrap();
        assert_eq!(failed.phase(), RunPhase::Failed);
        assert_eq!(
            failed.error().map(|error| error.message.as_str()),
            Some("try later")
        );
        assert!(failed.is_retryable());
    }

    #[test]
    fn event_after_terminal_state_is_rejected() {
        let request_id = RequestId(46);
        let mut transcript = RunTranscript::new(request_id);
        transcript
            .apply_event(AgentEvent::Started { request_id })
            .unwrap();
        transcript
            .apply_event(AgentEvent::Completed { request_id })
            .unwrap();

        let error = transcript
            .apply_event(AgentEvent::TextDelta {
                request_id,
                text: "too late".into(),
            })
            .unwrap_err();

        assert_eq!(
            error,
            EventSequenceError::UnexpectedEvent {
                phase: RunPhase::Completed,
                event: AgentEventKind::TextDelta,
            }
        );
        assert!(transcript.response_text().is_empty());
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

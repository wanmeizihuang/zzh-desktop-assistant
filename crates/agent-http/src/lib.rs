use std::{
    fmt,
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant},
};

use agent_core::{
    AgentConnector, AgentEvent, AgentRequest, AgentRun, AttachmentMode, CancellationToken,
    ConnectorCapabilities, ConnectorDescriptor, ConnectorError, ConnectorErrorCode, MessageRole,
    TransportKind, validate_request,
};
use eventsource_stream::{EventStreamError, Eventsource};
use futures_util::StreamExt;
use reqwest::{
    Client, Url,
    header::{ACCEPT, CONTENT_TYPE},
};
use serde::{Deserialize, Serialize};

const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const DEFAULT_MAX_STREAM_BYTES: usize = 4 * 1024 * 1024;
const DEFAULT_MAX_RESPONSE_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenAiCompatibleConfig {
    endpoint: String,
    model: String,
    connect_timeout: Duration,
    request_timeout: Duration,
    max_stream_bytes: usize,
    max_response_bytes: usize,
    local_execution: bool,
}

impl OpenAiCompatibleConfig {
    pub fn new(endpoint: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            model: model.into(),
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            max_stream_bytes: DEFAULT_MAX_STREAM_BYTES,
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
            local_execution: false,
        }
    }

    pub fn with_connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = timeout;
        self
    }

    pub fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    pub fn with_limits(mut self, max_stream_bytes: usize, max_response_bytes: usize) -> Self {
        self.max_stream_bytes = max_stream_bytes;
        self.max_response_bytes = max_response_bytes;
        self
    }

    pub fn with_local_execution(mut self, local_execution: bool) -> Self {
        self.local_execution = local_execution;
        self
    }
}

pub struct OpenAiCompatibleConnector {
    descriptor: ConnectorDescriptor,
    client: Client,
    endpoint: Url,
    model: String,
    bearer_token: Option<Arc<str>>,
    max_stream_bytes: usize,
    max_response_bytes: usize,
}

impl OpenAiCompatibleConnector {
    pub fn new(
        id: impl Into<String>,
        display_name: impl Into<String>,
        config: OpenAiCompatibleConfig,
        bearer_token: Option<String>,
    ) -> Result<Self, ConnectorError> {
        let id = id.into();
        let display_name = display_name.into();
        if id.trim().is_empty() || display_name.trim().is_empty() {
            return Err(configuration_error(
                "connector ID and display name must not be empty",
            ));
        }
        if config.model.trim().is_empty() {
            return Err(configuration_error("model must not be empty"));
        }
        if config.connect_timeout.is_zero() || config.request_timeout.is_zero() {
            return Err(configuration_error(
                "HTTP timeouts must be greater than zero",
            ));
        }
        if config.max_stream_bytes == 0 || config.max_response_bytes == 0 {
            return Err(configuration_error("HTTP limits must be greater than zero"));
        }
        if bearer_token
            .as_ref()
            .is_some_and(|token| token.trim().is_empty())
        {
            return Err(configuration_error("bearer token must not be empty"));
        }

        let endpoint = Url::parse(&config.endpoint)
            .map_err(|_| configuration_error("endpoint must be a valid HTTP or HTTPS URL"))?;
        if !matches!(endpoint.scheme(), "http" | "https") {
            return Err(configuration_error(
                "endpoint must use the HTTP or HTTPS scheme",
            ));
        }

        let client = Client::builder()
            .connect_timeout(config.connect_timeout)
            .timeout(config.request_timeout)
            .build()
            .map_err(|_| configuration_error("failed to build the HTTP client"))?;

        Ok(Self {
            descriptor: ConnectorDescriptor::new(
                id,
                display_name,
                TransportKind::Http,
                ConnectorCapabilities {
                    streaming: true,
                    attachment_mode: AttachmentMode::Unsupported,
                    image_input: false,
                    local_execution: config.local_execution,
                },
            ),
            client,
            endpoint,
            model: config.model,
            bearer_token: bearer_token.map(Arc::from),
            max_stream_bytes: config.max_stream_bytes,
            max_response_bytes: config.max_response_bytes,
        })
    }
}

impl AgentConnector for OpenAiCompatibleConnector {
    fn descriptor(&self) -> &ConnectorDescriptor {
        &self.descriptor
    }

    fn start(&self, request: AgentRequest) -> Result<AgentRun, ConnectorError> {
        validate_request(&request, self.descriptor.capabilities)?;

        let request_id = request.id;
        let client = self.client.clone();
        let endpoint = self.endpoint.clone();
        let model = self.model.clone();
        let bearer_token = self.bearer_token.clone();
        let max_stream_bytes = self.max_stream_bytes;
        let max_response_bytes = self.max_response_bytes;
        let cancellation = CancellationToken::new();
        let worker_cancellation = cancellation.clone();
        let (sender, receiver) = mpsc::channel();

        thread::Builder::new()
            .name("openai-http-agent".into())
            .spawn(move || {
                if sender.send(AgentEvent::Started { request_id }).is_err() {
                    return;
                }
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(_) => {
                        let _ = sender.send(AgentEvent::Failed {
                            request_id,
                            error: ConnectorError::new(
                                ConnectorErrorCode::Transport,
                                "failed to initialize HTTP runtime",
                            ),
                        });
                        return;
                    }
                };

                let outcome = runtime.block_on(run_openai_request(
                    client,
                    endpoint,
                    model,
                    bearer_token,
                    request,
                    max_stream_bytes,
                    max_response_bytes,
                    &sender,
                    worker_cancellation,
                ));
                let event = match outcome {
                    Ok(StreamOutcome::Completed) => AgentEvent::Completed { request_id },
                    Ok(StreamOutcome::Cancelled) => AgentEvent::Cancelled { request_id },
                    Err(error) => AgentEvent::Failed { request_id, error },
                };
                let _ = sender.send(event);
            })
            .map_err(|_| {
                ConnectorError::new(
                    ConnectorErrorCode::Transport,
                    "failed to start HTTP connector worker",
                )
            })?;

        Ok(AgentRun::new(receiver, cancellation))
    }
}

#[derive(Serialize)]
struct ChatCompletionRequest {
    model: String,
    stream: bool,
    messages: Vec<ChatMessage>,
}

#[derive(Serialize)]
struct ChatMessage {
    role: &'static str,
    content: String,
}

impl From<agent_core::AgentMessage> for ChatMessage {
    fn from(message: agent_core::AgentMessage) -> Self {
        let role = match message.role {
            MessageRole::System => "system",
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
        };
        Self {
            role,
            content: message.content,
        }
    }
}

#[derive(Deserialize)]
struct ChatCompletionChunk {
    #[serde(default)]
    choices: Vec<ChatChoice>,
    error: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct ChatChoice {
    delta: ChatDelta,
}

#[derive(Deserialize)]
struct ChatDelta {
    content: Option<String>,
}

enum StreamOutcome {
    Completed,
    Cancelled,
}

#[derive(Debug)]
enum BoundedStreamError {
    Transport(reqwest::Error),
    LimitExceeded,
}

impl fmt::Display for BoundedStreamError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(_) => write!(formatter, "HTTP transport failed"),
            Self::LimitExceeded => write!(formatter, "SSE stream exceeded its byte limit"),
        }
    }
}

impl std::error::Error for BoundedStreamError {}

#[allow(clippy::too_many_arguments)]
async fn run_openai_request(
    client: Client,
    endpoint: Url,
    model: String,
    bearer_token: Option<Arc<str>>,
    request: AgentRequest,
    max_stream_bytes: usize,
    max_response_bytes: usize,
    sender: &mpsc::Sender<AgentEvent>,
    cancellation: CancellationToken,
) -> Result<StreamOutcome, ConnectorError> {
    let request_id = request.id;
    let body = ChatCompletionRequest {
        model,
        stream: true,
        messages: request.messages.into_iter().map(Into::into).collect(),
    };
    let mut request_builder = client
        .post(endpoint)
        .header(ACCEPT, "text/event-stream")
        .header(CONTENT_TYPE, "application/json")
        .json(&body);
    if let Some(token) = bearer_token {
        request_builder = request_builder.bearer_auth(token.as_ref());
    }

    let response = tokio::select! {
        _ = wait_until_cancelled(cancellation.clone()) => return Ok(StreamOutcome::Cancelled),
        response = request_builder.send() => response.map_err(map_reqwest_error)?,
    };
    let status = response.status();
    if !status.is_success() {
        return Err(http_status_error(status.as_u16()));
    }
    let is_event_stream = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(';')
                .next()
                .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("text/event-stream"))
        });
    if !is_event_stream {
        return Err(ConnectorError::new(
            ConnectorErrorCode::Protocol,
            "HTTP response is not an SSE stream",
        ));
    }

    let mut stream_bytes = 0_usize;
    let bounded_stream = response.bytes_stream().map(move |result| {
        let bytes = result.map_err(BoundedStreamError::Transport)?;
        stream_bytes = stream_bytes
            .checked_add(bytes.len())
            .ok_or(BoundedStreamError::LimitExceeded)?;
        if stream_bytes > max_stream_bytes {
            return Err(BoundedStreamError::LimitExceeded);
        }
        Ok(bytes)
    });
    let mut events = bounded_stream.eventsource();
    let mut response_bytes = 0_usize;

    loop {
        let next = tokio::select! {
            _ = wait_until_cancelled(cancellation.clone()) => return Ok(StreamOutcome::Cancelled),
            next = events.next() => next,
        };
        let Some(event) = next else {
            return Err(ConnectorError::new(
                ConnectorErrorCode::Transport,
                "HTTP stream ended before the completion marker",
            ));
        };
        let event = event.map_err(map_event_stream_error)?;
        if event.data.trim() == "[DONE]" {
            return Ok(StreamOutcome::Completed);
        }

        let chunk: ChatCompletionChunk = serde_json::from_str(&event.data).map_err(|_| {
            ConnectorError::new(
                ConnectorErrorCode::Protocol,
                "HTTP stream contained malformed JSON",
            )
        })?;
        if chunk.error.is_some() {
            return Err(ConnectorError::new(
                ConnectorErrorCode::Protocol,
                "HTTP stream contained a provider error event",
            ));
        }
        let Some(text) = chunk
            .choices
            .into_iter()
            .next()
            .and_then(|choice| choice.delta.content)
            .filter(|text| !text.is_empty())
        else {
            continue;
        };
        response_bytes = response_bytes
            .checked_add(text.len())
            .ok_or_else(response_limit_error)?;
        if response_bytes > max_response_bytes {
            return Err(response_limit_error());
        }
        if sender
            .send(AgentEvent::TextDelta { request_id, text })
            .is_err()
        {
            return Ok(StreamOutcome::Cancelled);
        }
    }
}

async fn wait_until_cancelled(cancellation: CancellationToken) {
    while !cancellation.is_cancelled() {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn configuration_error(message: &'static str) -> ConnectorError {
    ConnectorError::new(ConnectorErrorCode::Configuration, message)
}

fn response_limit_error() -> ConnectorError {
    ConnectorError::new(
        ConnectorErrorCode::Protocol,
        "HTTP response exceeded its text limit",
    )
}

fn map_reqwest_error(error: reqwest::Error) -> ConnectorError {
    let code = if error.is_timeout() {
        ConnectorErrorCode::Timeout
    } else {
        ConnectorErrorCode::Transport
    };
    ConnectorError::new(code, "HTTP transport failed")
}

fn map_event_stream_error(error: EventStreamError<BoundedStreamError>) -> ConnectorError {
    match error {
        EventStreamError::Transport(BoundedStreamError::Transport(error)) => {
            map_reqwest_error(error)
        }
        EventStreamError::Transport(BoundedStreamError::LimitExceeded) => ConnectorError::new(
            ConnectorErrorCode::Protocol,
            "SSE stream exceeded its byte limit",
        ),
        EventStreamError::Utf8(_) | EventStreamError::Parser(_) => ConnectorError::new(
            ConnectorErrorCode::Protocol,
            "HTTP response contained malformed SSE data",
        ),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MockHttpScenario {
    Success { chunks: Vec<String> },
    HttpStatus(u16),
    Disconnect { chunks: Vec<String> },
}

pub struct MockHttpConnector {
    descriptor: ConnectorDescriptor,
    scenario: MockHttpScenario,
    chunk_delay: Duration,
}

impl MockHttpConnector {
    pub fn new(scenario: MockHttpScenario) -> Self {
        Self {
            descriptor: ConnectorDescriptor::new(
                "mock-http",
                "Mock HTTP",
                TransportKind::Http,
                ConnectorCapabilities {
                    streaming: true,
                    attachment_mode: AttachmentMode::Unsupported,
                    image_input: false,
                    local_execution: false,
                },
            ),
            scenario,
            chunk_delay: Duration::ZERO,
        }
    }

    pub fn with_chunk_delay(mut self, delay: Duration) -> Self {
        self.chunk_delay = delay;
        self
    }
}

impl AgentConnector for MockHttpConnector {
    fn descriptor(&self) -> &ConnectorDescriptor {
        &self.descriptor
    }

    fn start(&self, request: AgentRequest) -> Result<AgentRun, ConnectorError> {
        validate_request(&request, self.descriptor.capabilities)?;

        let request_id = request.id;
        let scenario = self.scenario.clone();
        let chunk_delay = self.chunk_delay;
        let cancellation = CancellationToken::new();
        let worker_cancellation = cancellation.clone();
        let (sender, receiver) = mpsc::channel();

        thread::Builder::new()
            .name("mock-http-agent".into())
            .spawn(move || {
                if sender.send(AgentEvent::Started { request_id }).is_err() {
                    return;
                }

                match scenario {
                    MockHttpScenario::HttpStatus(status) => {
                        let _ = sender.send(AgentEvent::Failed {
                            request_id,
                            error: http_status_error(status),
                        });
                    }
                    MockHttpScenario::Success { chunks } => {
                        if send_chunks(
                            &sender,
                            request_id,
                            chunks,
                            chunk_delay,
                            &worker_cancellation,
                        ) {
                            return;
                        }
                        let _ = sender.send(AgentEvent::Completed { request_id });
                    }
                    MockHttpScenario::Disconnect { chunks } => {
                        if send_chunks(
                            &sender,
                            request_id,
                            chunks,
                            chunk_delay,
                            &worker_cancellation,
                        ) {
                            return;
                        }
                        let _ = sender.send(AgentEvent::Failed {
                            request_id,
                            error: ConnectorError::new(
                                ConnectorErrorCode::Transport,
                                "HTTP stream disconnected before completion",
                            ),
                        });
                    }
                }
            })
            .map_err(|error| {
                ConnectorError::new(
                    ConnectorErrorCode::Transport,
                    format!("failed to start HTTP connector worker: {error}"),
                )
            })?;

        Ok(AgentRun::new(receiver, cancellation))
    }
}

fn send_chunks(
    sender: &mpsc::Sender<AgentEvent>,
    request_id: agent_core::RequestId,
    chunks: Vec<String>,
    chunk_delay: Duration,
    cancellation: &CancellationToken,
) -> bool {
    for text in chunks {
        if wait_for_delay_or_cancellation(chunk_delay, cancellation) {
            let _ = sender.send(AgentEvent::Cancelled { request_id });
            return true;
        }

        if sender
            .send(AgentEvent::TextDelta { request_id, text })
            .is_err()
        {
            return true;
        }
    }

    if cancellation.is_cancelled() {
        let _ = sender.send(AgentEvent::Cancelled { request_id });
        return true;
    }

    false
}

fn wait_for_delay_or_cancellation(delay: Duration, cancellation: &CancellationToken) -> bool {
    let deadline = Instant::now() + delay;
    loop {
        if cancellation.is_cancelled() {
            return true;
        }

        let now = Instant::now();
        if now >= deadline {
            return false;
        }
        thread::sleep((deadline - now).min(Duration::from_millis(5)));
    }
}

fn http_status_error(status: u16) -> ConnectorError {
    let code = match status {
        401 | 403 => ConnectorErrorCode::Unauthorized,
        408 | 504 => ConnectorErrorCode::Timeout,
        429 => ConnectorErrorCode::RateLimited,
        404 => ConnectorErrorCode::Configuration,
        400 | 409 | 413 | 422 => ConnectorErrorCode::InvalidRequest,
        _ => ConnectorErrorCode::Transport,
    };
    ConnectorError::new(code, format!("HTTP request failed with status {status}"))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use agent_core::{AgentConnector, AgentEvent, AgentRequest, ConnectorErrorCode, RequestId};

    use super::{
        MockHttpConnector, MockHttpScenario, OpenAiCompatibleConfig, OpenAiCompatibleConnector,
    };

    fn configuration_error(
        config: OpenAiCompatibleConfig,
        bearer_token: Option<String>,
    ) -> ConnectorErrorCode {
        OpenAiCompatibleConnector::new("test", "Test", config, bearer_token)
            .err()
            .expect("invalid connector configuration")
            .code
    }

    #[test]
    fn invalid_openai_configuration_is_rejected_before_start() {
        assert_eq!(
            configuration_error(OpenAiCompatibleConfig::new("not a URL", "model"), None),
            ConnectorErrorCode::Configuration
        );
        assert_eq!(
            configuration_error(OpenAiCompatibleConfig::new("http://127.0.0.1", "   "), None,),
            ConnectorErrorCode::Configuration
        );
        assert_eq!(
            configuration_error(
                OpenAiCompatibleConfig::new("http://127.0.0.1", "model").with_limits(0, 1),
                None,
            ),
            ConnectorErrorCode::Configuration
        );
        assert_eq!(
            configuration_error(
                OpenAiCompatibleConfig::new("http://127.0.0.1", "model"),
                Some("  ".into()),
            ),
            ConnectorErrorCode::Configuration
        );
    }

    #[test]
    fn local_openai_service_declares_local_execution() {
        let connector = OpenAiCompatibleConnector::new(
            "local-model",
            "Local Model",
            OpenAiCompatibleConfig::new("http://127.0.0.1:11434/v1/chat/completions", "qwen")
                .with_local_execution(true),
            None,
        )
        .unwrap();

        assert!(connector.descriptor().capabilities.local_execution);
    }

    #[test]
    fn successful_http_script_emits_ordered_text_deltas() {
        let connector = MockHttpConnector::new(MockHttpScenario::Success {
            chunks: vec!["Hello".into(), " world".into()],
        });
        let run = connector
            .start(AgentRequest::single_user_message(RequestId(1), "hi"))
            .unwrap();

        assert_eq!(
            run.recv_timeout(Duration::from_secs(1)).unwrap(),
            AgentEvent::Started {
                request_id: RequestId(1)
            }
        );
        assert_eq!(
            run.recv_timeout(Duration::from_secs(1)).unwrap(),
            AgentEvent::TextDelta {
                request_id: RequestId(1),
                text: "Hello".into()
            }
        );
        assert_eq!(
            run.recv_timeout(Duration::from_secs(1)).unwrap(),
            AgentEvent::TextDelta {
                request_id: RequestId(1),
                text: " world".into()
            }
        );
        assert_eq!(
            run.recv_timeout(Duration::from_secs(1)).unwrap(),
            AgentEvent::Completed {
                request_id: RequestId(1)
            }
        );
    }

    #[test]
    fn http_statuses_map_to_stable_error_codes() {
        for (status, expected) in [
            (401, ConnectorErrorCode::Unauthorized),
            (408, ConnectorErrorCode::Timeout),
            (429, ConnectorErrorCode::RateLimited),
            (404, ConnectorErrorCode::Configuration),
            (422, ConnectorErrorCode::InvalidRequest),
            (503, ConnectorErrorCode::Transport),
        ] {
            let connector = MockHttpConnector::new(MockHttpScenario::HttpStatus(status));
            let run = connector
                .start(AgentRequest::single_user_message(RequestId(2), "hi"))
                .unwrap();
            let _ = run.recv_timeout(Duration::from_secs(1)).unwrap();
            let AgentEvent::Failed { error, .. } =
                run.recv_timeout(Duration::from_secs(1)).unwrap()
            else {
                panic!("expected a failed event");
            };

            assert_eq!(error.code, expected);
        }
    }

    #[test]
    fn disconnect_after_chunks_preserves_received_text_then_fails() {
        let connector = MockHttpConnector::new(MockHttpScenario::Disconnect {
            chunks: vec!["partial".into()],
        });
        let run = connector
            .start(AgentRequest::single_user_message(RequestId(3), "hi"))
            .unwrap();
        let _ = run.recv_timeout(Duration::from_secs(1)).unwrap();

        assert!(matches!(
            run.recv_timeout(Duration::from_secs(1)).unwrap(),
            AgentEvent::TextDelta { text, .. } if text == "partial"
        ));
        assert!(matches!(
            run.recv_timeout(Duration::from_secs(1)).unwrap(),
            AgentEvent::Failed { error, .. }
                if error.code == ConnectorErrorCode::Transport
        ));
    }

    #[test]
    fn cancellation_ends_the_stream_without_completion() {
        let connector = MockHttpConnector::new(MockHttpScenario::Success {
            chunks: vec!["too late".into()],
        })
        .with_chunk_delay(Duration::from_millis(100));
        let run = connector
            .start(AgentRequest::single_user_message(RequestId(4), "hi"))
            .unwrap();
        let _ = run.recv_timeout(Duration::from_secs(1)).unwrap();

        run.cancel();

        assert_eq!(
            run.recv_timeout(Duration::from_secs(1)).unwrap(),
            AgentEvent::Cancelled {
                request_id: RequestId(4)
            }
        );
    }
}

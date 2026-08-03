use std::{
    collections::HashMap,
    env,
    ffi::OsString,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};

use agent_core::{
    AgentConnector, AgentEvent, AgentRequest, AgentRun, AttachmentMode, CancellationToken,
    ConnectorCapabilities, ConnectorDescriptor, ConnectorError, ConnectorErrorCode, RequestId,
    TransportKind, validate_request,
};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

const STDERR_LIMIT_BYTES: u64 = 8 * 1024;
const CODEX_EXECUTABLE_NAME: &str = "codex";

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CliCommandSpec {
    pub executable: PathBuf,
    pub args: Vec<OsString>,
}

impl CliCommandSpec {
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
            args: Vec::new(),
        }
    }

    pub fn with_args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }
}

pub struct CliConnector {
    descriptor: ConnectorDescriptor,
    command: CliCommandSpec,
    poll_interval: Duration,
    output_format: CliOutputFormat,
}

impl CliConnector {
    pub fn new(command: CliCommandSpec) -> Self {
        Self {
            descriptor: ConnectorDescriptor::new(
                "local-cli",
                "Local CLI",
                TransportKind::Cli,
                ConnectorCapabilities {
                    streaming: true,
                    attachment_mode: AttachmentMode::Unsupported,
                    image_input: false,
                    local_execution: true,
                },
            ),
            command,
            poll_interval: Duration::from_millis(20),
            output_format: CliOutputFormat::TextLines,
        }
    }

    pub fn with_poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = interval.max(Duration::from_millis(1));
        self
    }
}

impl AgentConnector for CliConnector {
    fn descriptor(&self) -> &ConnectorDescriptor {
        &self.descriptor
    }

    fn start(&self, request: AgentRequest) -> Result<AgentRun, ConnectorError> {
        validate_request(&request, self.descriptor.capabilities)?;
        let prompt = request
            .latest_user_text()
            .expect("validated request has a user message")
            .to_owned();
        let request_id = request.id;
        let command = self.command.clone();
        let poll_interval = self.poll_interval;
        let output_format = self.output_format;
        let cancellation = CancellationToken::new();
        let worker_cancellation = cancellation.clone();
        let (sender, receiver) = mpsc::channel();

        thread::Builder::new()
            .name("cli-agent".into())
            .spawn(move || {
                run_cli_process(
                    command,
                    prompt,
                    request_id,
                    poll_interval,
                    output_format,
                    worker_cancellation,
                    sender,
                );
            })
            .map_err(|error| {
                ConnectorError::new(
                    ConnectorErrorCode::Process,
                    format!("failed to start CLI connector worker: {error}"),
                )
            })?;

        Ok(AgentRun::new(receiver, cancellation))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexCliConfig {
    executable: PathBuf,
    extra_args: Vec<OsString>,
}

impl CodexCliConfig {
    pub fn discover() -> Result<Self, ConnectorError> {
        let executable = discover_codex_executable().ok_or_else(missing_codex_error)?;
        Ok(Self {
            executable,
            extra_args: Vec::new(),
        })
    }

    pub fn new(executable: impl Into<PathBuf>) -> Result<Self, ConnectorError> {
        let requested = executable.into();
        let resolved = resolve_codex_executable(&requested).ok_or_else(missing_codex_error)?;
        Ok(Self {
            executable: resolved,
            extra_args: Vec::new(),
        })
    }

    pub fn with_extra_args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.extra_args = args.into_iter().map(Into::into).collect();
        self
    }

    pub fn executable(&self) -> &Path {
        &self.executable
    }

    pub fn command_spec(&self) -> CliCommandSpec {
        let mut args = [
            "exec",
            "--json",
            "--skip-git-repo-check",
            "--sandbox",
            "read-only",
        ]
        .map(OsString::from)
        .to_vec();
        args.extend(self.extra_args.iter().cloned());
        args.push(OsString::from("-"));
        CliCommandSpec::new(&self.executable).with_args(args)
    }
}

pub struct CodexCliConnector {
    config: CodexCliConfig,
    inner: CliConnector,
}

impl CodexCliConnector {
    pub fn new(config: CodexCliConfig) -> Self {
        Self::new_managed("codex-cli", "Codex CLI", config)
    }

    pub fn new_managed(
        id: impl Into<String>,
        display_name: impl Into<String>,
        config: CodexCliConfig,
    ) -> Self {
        let descriptor = ConnectorDescriptor::new(
            id,
            display_name,
            TransportKind::Cli,
            ConnectorCapabilities {
                streaming: true,
                attachment_mode: AttachmentMode::Unsupported,
                image_input: false,
                local_execution: true,
            },
        );
        let inner = CliConnector {
            descriptor,
            command: config.command_spec(),
            poll_interval: Duration::from_millis(20),
            output_format: CliOutputFormat::CodexJsonLines,
        };
        Self { config, inner }
    }

    pub fn discover() -> Result<Self, ConnectorError> {
        CodexCliConfig::discover().map(Self::new)
    }

    pub fn config(&self) -> &CodexCliConfig {
        &self.config
    }

    pub fn with_poll_interval(mut self, interval: Duration) -> Self {
        self.inner.poll_interval = interval.max(Duration::from_millis(1));
        self
    }
}

impl AgentConnector for CodexCliConnector {
    fn descriptor(&self) -> &ConnectorDescriptor {
        self.inner.descriptor()
    }

    fn start(&self, request: AgentRequest) -> Result<AgentRun, ConnectorError> {
        self.inner.start(request)
    }
}

pub fn discover_codex_executable() -> Option<PathBuf> {
    let search_paths = env::var_os("PATH")
        .as_deref()
        .map(env::split_paths)
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    discover_codex_executable_in(search_paths.iter().map(PathBuf::as_path))
}

pub fn discover_codex_executable_in<'a>(
    search_paths: impl IntoIterator<Item = &'a Path>,
) -> Option<PathBuf> {
    discover_named_executable_in(Path::new(CODEX_EXECUTABLE_NAME), search_paths)
}

fn resolve_codex_executable(requested: &Path) -> Option<PathBuf> {
    if requested.is_file() {
        return Some(requested.to_path_buf());
    }
    if requested.is_absolute() || requested.components().count() != 1 {
        return None;
    }

    let search_paths = env::var_os("PATH")
        .as_deref()
        .map(env::split_paths)
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    discover_named_executable_in(requested, search_paths.iter().map(PathBuf::as_path))
}

fn discover_named_executable_in<'a>(
    requested: &Path,
    search_paths: impl IntoIterator<Item = &'a Path>,
) -> Option<PathBuf> {
    let candidates = executable_candidates(requested);
    search_paths.into_iter().find_map(|directory| {
        candidates
            .iter()
            .map(|candidate| directory.join(candidate))
            .find(|candidate| candidate.is_file())
    })
}

fn executable_candidates(requested: &Path) -> Vec<OsString> {
    let requested_name = requested.as_os_str().to_os_string();
    #[cfg(windows)]
    if requested.extension().is_none() {
        let mut executable_name = requested_name.clone();
        executable_name.push(".exe");
        return vec![executable_name, requested_name];
    }
    vec![requested_name]
}

fn missing_codex_error() -> ConnectorError {
    ConnectorError::new(
        ConnectorErrorCode::Configuration,
        "Codex executable was not found; install Codex CLI or configure its executable path",
    )
}

#[derive(Clone, Copy)]
enum CliOutputFormat {
    TextLines,
    CodexJsonLines,
}

fn run_cli_process(
    spec: CliCommandSpec,
    prompt: String,
    request_id: RequestId,
    poll_interval: Duration,
    output_format: CliOutputFormat,
    cancellation: CancellationToken,
    sender: mpsc::Sender<AgentEvent>,
) {
    let mut command = Command::new(&spec.executable);
    command
        .args(&spec.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            let _ = sender.send(failed_event(
                request_id,
                format!("failed to start CLI process: {error}"),
            ));
            return;
        }
    };

    if sender.send(AgentEvent::Started { request_id }).is_err() {
        terminate_child(&mut child);
        return;
    }

    let write_result = child
        .stdin
        .take()
        .ok_or_else(|| "CLI stdin pipe is unavailable".to_owned())
        .and_then(|mut stdin| {
            stdin
                .write_all(prompt.as_bytes())
                .map_err(|error| format!("failed to write CLI prompt: {error}"))
        });
    if let Err(message) = write_result {
        terminate_child(&mut child);
        let _ = sender.send(failed_event(request_id, message));
        return;
    }

    let Some(stdout) = child.stdout.take() else {
        terminate_child(&mut child);
        let _ = sender.send(failed_event(request_id, "CLI stdout pipe is unavailable"));
        return;
    };
    let Some(stderr) = child.stderr.take() else {
        terminate_child(&mut child);
        let _ = sender.send(failed_event(request_id, "CLI stderr pipe is unavailable"));
        return;
    };

    let (output_sender, output_receiver) = mpsc::channel();
    let stdout_thread = thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let output = line.map(OutputMessage::Line).unwrap_or_else(|error| {
                OutputMessage::ReadError(format!("failed to read CLI stdout: {error}"))
            });
            let is_error = matches!(output, OutputMessage::ReadError(_));
            if output_sender.send(output).is_err() || is_error {
                break;
            }
        }
    });
    let stderr_thread = thread::spawn(move || read_bounded_stderr(stderr));
    let mut output_adapter = OutputAdapter::new(output_format);
    let mut output_error = None;

    loop {
        if !forward_output(
            &output_receiver,
            &sender,
            request_id,
            &mut output_adapter,
            &mut output_error,
        ) {
            terminate_child(&mut child);
            let _ = stdout_thread.join();
            let _ = stderr_thread.join();
            return;
        }
        if let Some(error) = output_error.take() {
            terminate_child(&mut child);
            let _ = stdout_thread.join();
            let _ = stderr_thread.join();
            let _ = sender.send(AgentEvent::Failed { request_id, error });
            return;
        }

        if cancellation.is_cancelled() {
            terminate_child(&mut child);
            let _ = stdout_thread.join();
            let _ = stderr_thread.join();
            let _ = sender.send(AgentEvent::Cancelled { request_id });
            return;
        }

        match child.try_wait() {
            Ok(Some(status)) => {
                let _ = stdout_thread.join();
                let stderr = stderr_thread.join().unwrap_or_default();
                if !forward_output(
                    &output_receiver,
                    &sender,
                    request_id,
                    &mut output_adapter,
                    &mut output_error,
                ) {
                    return;
                }
                send_process_terminal_event(
                    &sender,
                    request_id,
                    status,
                    stderr,
                    output_error,
                    &output_adapter,
                );
                return;
            }
            Ok(None) => thread::sleep(poll_interval),
            Err(error) => {
                terminate_child(&mut child);
                let _ = stdout_thread.join();
                let _ = stderr_thread.join();
                let _ = sender.send(failed_event(
                    request_id,
                    format!("failed to poll CLI process: {error}"),
                ));
                return;
            }
        }
    }
}

enum OutputMessage {
    Line(String),
    ReadError(String),
}

enum OutputAdapter {
    TextLines,
    CodexJson(CodexJsonOutput),
}

impl OutputAdapter {
    fn new(format: CliOutputFormat) -> Self {
        match format {
            CliOutputFormat::TextLines => Self::TextLines,
            CliOutputFormat::CodexJsonLines => Self::CodexJson(CodexJsonOutput::default()),
        }
    }

    fn parse_line(&mut self, line: String) -> Result<Option<String>, ConnectorError> {
        match self {
            Self::TextLines => Ok(Some(line)),
            Self::CodexJson(output) => output.parse_line(&line),
        }
    }

    fn validate_success(&self) -> Result<(), ConnectorError> {
        match self {
            Self::TextLines => Ok(()),
            Self::CodexJson(output) if output.turn_completed => Ok(()),
            Self::CodexJson(_) => Err(ConnectorError::new(
                ConnectorErrorCode::Protocol,
                "Codex process exited before reporting turn.completed",
            )),
        }
    }
}

#[derive(Default)]
struct CodexJsonOutput {
    message_snapshots: HashMap<String, String>,
    turn_completed: bool,
}

impl CodexJsonOutput {
    fn parse_line(&mut self, line: &str) -> Result<Option<String>, ConnectorError> {
        let value: serde_json::Value = serde_json::from_str(line).map_err(|error| {
            ConnectorError::new(
                ConnectorErrorCode::Protocol,
                format!("invalid Codex JSONL event: {error}"),
            )
        })?;
        let event_type = value
            .get("type")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                ConnectorError::new(
                    ConnectorErrorCode::Protocol,
                    "Codex JSONL event is missing its type",
                )
            })?;

        match event_type {
            "item.updated" | "item.completed" => self.parse_item(&value),
            "turn.completed" => {
                self.turn_completed = true;
                Ok(None)
            }
            "turn.failed" | "error" => {
                let message = value
                    .pointer("/error/message")
                    .or_else(|| value.get("message"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unknown Codex error");
                Err(ConnectorError::new(
                    ConnectorErrorCode::Protocol,
                    format!("Codex turn failed: {message}"),
                ))
            }
            _ => Ok(None),
        }
    }

    fn parse_item(&mut self, event: &serde_json::Value) -> Result<Option<String>, ConnectorError> {
        let Some(item) = event.get("item") else {
            return Err(ConnectorError::new(
                ConnectorErrorCode::Protocol,
                "Codex item event is missing its item",
            ));
        };
        if item.get("type").and_then(serde_json::Value::as_str) != Some("agent_message") {
            return Ok(None);
        }

        let id = item
            .get("id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                ConnectorError::new(
                    ConnectorErrorCode::Protocol,
                    "Codex agent message is missing its id",
                )
            })?;
        let text = item
            .get("text")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                ConnectorError::new(
                    ConnectorErrorCode::Protocol,
                    "Codex agent message is missing its text",
                )
            })?;
        let previous = self.message_snapshots.entry(id.to_owned()).or_default();
        let Some(delta) = text.strip_prefix(previous.as_str()) else {
            return Err(ConnectorError::new(
                ConnectorErrorCode::Protocol,
                "Codex agent message update replaced previously emitted text",
            ));
        };
        if delta.is_empty() {
            return Ok(None);
        }
        let delta = delta.to_owned();
        *previous = text.to_owned();
        Ok(Some(delta))
    }
}

fn forward_output(
    receiver: &mpsc::Receiver<OutputMessage>,
    sender: &mpsc::Sender<AgentEvent>,
    request_id: RequestId,
    output_adapter: &mut OutputAdapter,
    output_error: &mut Option<ConnectorError>,
) -> bool {
    while let Ok(output) = receiver.try_recv() {
        match output {
            OutputMessage::Line(text) => match output_adapter.parse_line(text) {
                Ok(Some(text)) => {
                    if sender
                        .send(AgentEvent::TextDelta { request_id, text })
                        .is_err()
                    {
                        return false;
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    *output_error = Some(error);
                    break;
                }
            },
            OutputMessage::ReadError(error) => {
                *output_error = Some(ConnectorError::new(ConnectorErrorCode::Protocol, error));
                break;
            }
        }
    }
    true
}

fn send_process_terminal_event(
    sender: &mpsc::Sender<AgentEvent>,
    request_id: RequestId,
    status: ExitStatus,
    stderr: String,
    output_error: Option<ConnectorError>,
    output_adapter: &OutputAdapter,
) {
    let event = if let Some(error) = output_error {
        AgentEvent::Failed { request_id, error }
    } else if status.success() {
        match output_adapter.validate_success() {
            Ok(()) => AgentEvent::Completed { request_id },
            Err(error) => AgentEvent::Failed { request_id, error },
        }
    } else {
        let exit_code = status
            .code()
            .map_or_else(|| "unknown".into(), |code| code.to_string());
        let detail = stderr.trim();
        let message = if detail.is_empty() {
            format!("CLI process exited with code {exit_code}")
        } else {
            format!("CLI process exited with code {exit_code}: {detail}")
        };
        failed_event(request_id, message)
    };
    let _ = sender.send(event);
}

fn read_bounded_stderr(stderr: impl Read) -> String {
    let mut bytes = Vec::new();
    let _ = stderr.take(STDERR_LIMIT_BYTES).read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).into_owned()
}

fn terminate_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn failed_event(request_id: RequestId, message: impl Into<String>) -> AgentEvent {
    AgentEvent::Failed {
        request_id,
        error: ConnectorError::new(ConnectorErrorCode::Process, message),
    }
}

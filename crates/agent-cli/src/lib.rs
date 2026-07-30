use std::{
    ffi::OsString,
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
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

fn run_cli_process(
    spec: CliCommandSpec,
    prompt: String,
    request_id: RequestId,
    poll_interval: Duration,
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
    let mut output_error = None;

    loop {
        if !forward_output(
            &output_receiver,
            &sender,
            request_id,
            &mut output_error,
        ) {
            terminate_child(&mut child);
            let _ = stdout_thread.join();
            let _ = stderr_thread.join();
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

fn forward_output(
    receiver: &mpsc::Receiver<OutputMessage>,
    sender: &mpsc::Sender<AgentEvent>,
    request_id: RequestId,
    output_error: &mut Option<String>,
) -> bool {
    while let Ok(output) = receiver.try_recv() {
        match output {
            OutputMessage::Line(text) => {
                if sender
                    .send(AgentEvent::TextDelta { request_id, text })
                    .is_err()
                {
                    return false;
                }
            }
            OutputMessage::ReadError(error) => *output_error = Some(error),
        }
    }
    true
}

fn send_process_terminal_event(
    sender: &mpsc::Sender<AgentEvent>,
    request_id: RequestId,
    status: ExitStatus,
    stderr: String,
    output_error: Option<String>,
) {
    let event = if let Some(error) = output_error {
        AgentEvent::Failed {
            request_id,
            error: ConnectorError::new(ConnectorErrorCode::Protocol, error),
        }
    } else if status.success() {
        AgentEvent::Completed { request_id }
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

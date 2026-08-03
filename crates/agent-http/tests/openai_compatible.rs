use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::mpsc,
    thread,
    time::Duration,
};

use agent_core::{AgentConnector, AgentEvent, AgentRequest, ConnectorErrorCode, RequestId};
use agent_http::{OpenAiCompatibleConfig, OpenAiCompatibleConnector};

#[derive(Clone)]
struct ResponseChunk {
    delay: Duration,
    bytes: Vec<u8>,
}

impl ResponseChunk {
    fn immediate(bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            delay: Duration::ZERO,
            bytes: bytes.into(),
        }
    }

    fn delayed(delay: Duration, bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            delay,
            bytes: bytes.into(),
        }
    }
}

struct ResponseScript {
    status: u16,
    chunks: Vec<ResponseChunk>,
    finish_chunked_body: bool,
    response_delay: Duration,
}

impl ResponseScript {
    fn sse(chunks: Vec<ResponseChunk>) -> Self {
        Self {
            status: 200,
            chunks,
            finish_chunked_body: true,
            response_delay: Duration::ZERO,
        }
    }

    fn status(status: u16) -> Self {
        Self {
            status,
            chunks: Vec::new(),
            finish_chunked_body: true,
            response_delay: Duration::ZERO,
        }
    }
}

struct ScriptedServer {
    endpoint: String,
    request: mpsc::Receiver<String>,
}

impl ScriptedServer {
    fn start(script: ResponseScript) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind scripted server");
        let address = listener.local_addr().expect("scripted server address");
        let (request_sender, request_receiver) = mpsc::channel();

        thread::Builder::new()
            .name("openai-http-test-server".into())
            .spawn(move || {
                let (mut stream, _) = listener.accept().expect("accept connector request");
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .expect("set request timeout");
                let request = read_http_request(&mut stream);
                request_sender.send(request).ok();

                thread::sleep(script.response_delay);
                write_scripted_response(&mut stream, script);
            })
            .expect("spawn scripted server");

        Self {
            endpoint: format!("http://{address}/v1/chat/completions"),
            request: request_receiver,
        }
    }

    fn connector(
        &self,
        request_timeout: Duration,
        max_stream_bytes: usize,
        max_response_bytes: usize,
        bearer_token: Option<&str>,
    ) -> OpenAiCompatibleConnector {
        let config = OpenAiCompatibleConfig::new(&self.endpoint, "test-model")
            .with_connect_timeout(Duration::from_secs(1))
            .with_request_timeout(request_timeout)
            .with_limits(max_stream_bytes, max_response_bytes);
        OpenAiCompatibleConnector::new(
            "test-openai",
            "Test OpenAI",
            config,
            bearer_token.map(str::to_owned),
        )
        .expect("create OpenAI-compatible connector")
    }

    fn captured_request(&self) -> String {
        self.request
            .recv_timeout(Duration::from_secs(2))
            .expect("captured connector request")
    }
}

fn read_http_request(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 2048];
    let header_end = loop {
        let count = stream.read(&mut buffer).expect("read request");
        assert!(count > 0, "request closed before headers completed");
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(index) = find_bytes(&bytes, b"\r\n\r\n") {
            break index + 4;
        }
    };
    let headers = String::from_utf8_lossy(&bytes[..header_end]);
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().expect("content length"))
        })
        .unwrap_or_default();
    while bytes.len() < header_end + content_length {
        let count = stream.read(&mut buffer).expect("read request body");
        assert!(count > 0, "request closed before body completed");
        bytes.extend_from_slice(&buffer[..count]);
    }
    String::from_utf8(bytes).expect("UTF-8 HTTP request")
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn write_scripted_response(stream: &mut TcpStream, script: ResponseScript) {
    let reason = match script.status {
        200 => "OK",
        401 => "Unauthorized",
        429 => "Too Many Requests",
        _ => "Error",
    };
    write!(
        stream,
        "HTTP/1.1 {} {}\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
        script.status, reason
    )
    .expect("write response headers");
    stream.flush().expect("flush response headers");

    for chunk in script.chunks {
        thread::sleep(chunk.delay);
        if write!(stream, "{:X}\r\n", chunk.bytes.len()).is_err()
            || stream.write_all(&chunk.bytes).is_err()
            || stream.write_all(b"\r\n").is_err()
            || stream.flush().is_err()
        {
            return;
        }
    }

    if script.finish_chunked_body {
        let _ = stream.write_all(b"0\r\n\r\n");
        let _ = stream.flush();
    }
}

fn start(connector: &OpenAiCompatibleConnector, request_id: u64) -> agent_core::AgentRun {
    connector
        .start(AgentRequest::single_user_message(
            RequestId(request_id),
            "hello",
        ))
        .expect("start connector")
}

fn next_event(run: &agent_core::AgentRun) -> AgentEvent {
    run.recv_timeout(Duration::from_secs(2))
        .expect("next connector event")
}

#[test]
fn ordered_sse_deltas_complete_only_after_done() {
    let server = ScriptedServer::start(ResponseScript::sse(vec![
        ResponseChunk::immediate(b"data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n"),
        ResponseChunk::immediate(b"data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\n"),
        ResponseChunk::immediate(b"data: [DONE]\n\n"),
    ]));
    let connector = server.connector(
        Duration::from_secs(2),
        64 * 1024,
        16 * 1024,
        Some("secret-token"),
    );
    let run = start(&connector, 1);

    assert!(matches!(next_event(&run), AgentEvent::Started { .. }));
    assert!(matches!(next_event(&run), AgentEvent::TextDelta { text, .. } if text == "Hel"));
    assert!(matches!(next_event(&run), AgentEvent::TextDelta { text, .. } if text == "lo"));
    assert!(matches!(next_event(&run), AgentEvent::Completed { .. }));

    let request = server.captured_request();
    assert!(request.starts_with("POST /v1/chat/completions HTTP/1.1\r\n"));
    assert!(
        request
            .to_ascii_lowercase()
            .contains("authorization: bearer secret-token")
    );
    assert!(request.contains("\"model\":\"test-model\""));
    assert!(request.contains("\"stream\":true"));
    assert!(request.contains("\"role\":\"user\""));
    assert!(request.contains("\"content\":\"hello\""));
}

#[test]
fn malformed_json_is_a_protocol_failure_without_secret_leakage() {
    let server = ScriptedServer::start(ResponseScript::sse(vec![ResponseChunk::immediate(
        b"data: {not-json}\n\n",
    )]));
    let connector = server.connector(
        Duration::from_secs(2),
        64 * 1024,
        16 * 1024,
        Some("never-log-this"),
    );
    let run = start(&connector, 2);
    let _ = next_event(&run);

    let AgentEvent::Failed { error, .. } = next_event(&run) else {
        panic!("expected protocol failure");
    };
    assert_eq!(error.code, ConnectorErrorCode::Protocol);
    assert!(!error.message.contains("never-log-this"));
}

#[test]
fn disconnect_before_done_is_a_transport_failure() {
    let mut script = ResponseScript::sse(vec![ResponseChunk::immediate(
        b"data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n",
    )]);
    script.finish_chunked_body = false;
    let server = ScriptedServer::start(script);
    let connector = server.connector(Duration::from_secs(2), 64 * 1024, 16 * 1024, None);
    let run = start(&connector, 3);
    let _ = next_event(&run);

    assert!(matches!(next_event(&run), AgentEvent::TextDelta { text, .. } if text == "partial"));
    assert!(matches!(
        next_event(&run),
        AgentEvent::Failed { error, .. } if error.code == ConnectorErrorCode::Transport
    ));
}

#[test]
fn request_timeout_maps_to_timeout() {
    let mut script = ResponseScript::sse(Vec::new());
    script.response_delay = Duration::from_millis(250);
    let server = ScriptedServer::start(script);
    let connector = server.connector(Duration::from_millis(50), 64 * 1024, 16 * 1024, None);
    let run = start(&connector, 4);
    let _ = next_event(&run);

    assert!(matches!(
        next_event(&run),
        AgentEvent::Failed { error, .. } if error.code == ConnectorErrorCode::Timeout
    ));
}

#[test]
fn authentication_and_rate_limit_statuses_keep_stable_error_codes() {
    for (request_id, status, expected) in [
        (5, 401, ConnectorErrorCode::Unauthorized),
        (6, 429, ConnectorErrorCode::RateLimited),
    ] {
        let server = ScriptedServer::start(ResponseScript::status(status));
        let connector = server.connector(Duration::from_secs(2), 64 * 1024, 16 * 1024, None);
        let run = start(&connector, request_id);
        let _ = next_event(&run);

        assert!(matches!(
            next_event(&run),
            AgentEvent::Failed { error, .. } if error.code == expected
        ));
    }
}

#[test]
fn cancellation_interrupts_a_stalled_stream() {
    let server = ScriptedServer::start(ResponseScript::sse(vec![ResponseChunk::delayed(
        Duration::from_secs(1),
        b"data: [DONE]\n\n",
    )]));
    let connector = server.connector(Duration::from_secs(3), 64 * 1024, 16 * 1024, None);
    let run = start(&connector, 7);
    let _ = next_event(&run);

    run.cancel();

    assert!(matches!(next_event(&run), AgentEvent::Cancelled { .. }));
}

#[test]
fn stream_and_response_limits_fail_before_unbounded_growth() {
    let server = ScriptedServer::start(ResponseScript::sse(vec![ResponseChunk::immediate(
        b"data: {\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}\n\n",
    )]));
    let connector = server.connector(Duration::from_secs(2), 16, 16 * 1024, None);
    let run = start(&connector, 8);
    let _ = next_event(&run);
    assert!(matches!(
        next_event(&run),
        AgentEvent::Failed { error, .. } if error.code == ConnectorErrorCode::Protocol
    ));

    let server = ScriptedServer::start(ResponseScript::sse(vec![ResponseChunk::immediate(
        b"data: {\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}\n\n",
    )]));
    let connector = server.connector(Duration::from_secs(2), 64 * 1024, 4, None);
    let run = start(&connector, 9);
    let _ = next_event(&run);
    assert!(matches!(
        next_event(&run),
        AgentEvent::Failed { error, .. } if error.code == ConnectorErrorCode::Protocol
    ));
}

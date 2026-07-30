use std::{
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use agent_core::{
    AgentConnector, AgentEvent, AgentRequest, AgentRun, AttachmentMode, CancellationToken,
    ConnectorCapabilities, ConnectorDescriptor, ConnectorError, ConnectorErrorCode, TransportKind,
    validate_request,
};

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

fn wait_for_delay_or_cancellation(
    delay: Duration,
    cancellation: &CancellationToken,
) -> bool {
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
        429 => ConnectorErrorCode::RateLimited,
        _ => ConnectorErrorCode::Transport,
    };
    ConnectorError::new(code, format!("HTTP request failed with status {status}"))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use agent_core::{AgentConnector, AgentEvent, AgentRequest, ConnectorErrorCode, RequestId};

    use super::{MockHttpConnector, MockHttpScenario};

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
            (429, ConnectorErrorCode::RateLimited),
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

use std::{ffi::OsString, path::PathBuf, time::Duration};

use agent_cli::{CliCommandSpec, CliConnector};
use agent_core::{AgentConnector, AgentEvent, AgentRequest, ConnectorErrorCode, RequestId};

fn fixture_command(mode: &str) -> CliCommandSpec {
    CliCommandSpec::new(PathBuf::from(env!("CARGO_BIN_EXE_agent-cli-fixture")))
        .with_args([OsString::from(mode)])
}

#[test]
fn cli_streams_real_child_process_stdout_in_order() {
    let connector = CliConnector::new(fixture_command("stream"));
    let run = connector
        .start(AgentRequest::single_user_message(RequestId(11), "hello"))
        .unwrap();

    assert!(matches!(
        run.recv_timeout(Duration::from_secs(2)).unwrap(),
        AgentEvent::Started { .. }
    ));
    assert!(matches!(
        run.recv_timeout(Duration::from_secs(2)).unwrap(),
        AgentEvent::TextDelta { text, .. } if text == "received:hello"
    ));
    assert!(matches!(
        run.recv_timeout(Duration::from_secs(2)).unwrap(),
        AgentEvent::TextDelta { text, .. } if text == "done"
    ));
    assert!(matches!(
        run.recv_timeout(Duration::from_secs(2)).unwrap(),
        AgentEvent::Completed { .. }
    ));
}

#[test]
fn prompt_shell_metacharacters_are_passed_literally_through_stdin() {
    let connector = CliConnector::new(fixture_command("stream"));
    let run = connector
        .start(AgentRequest::single_user_message(
            RequestId(12),
            "hello & echo injected",
        ))
        .unwrap();
    let _ = run.recv_timeout(Duration::from_secs(2)).unwrap();

    assert!(matches!(
        run.recv_timeout(Duration::from_secs(2)).unwrap(),
        AgentEvent::TextDelta { text, .. }
            if text == "received:hello & echo injected"
    ));
}

#[test]
fn non_zero_exit_becomes_a_process_error_with_bounded_stderr() {
    let connector = CliConnector::new(fixture_command("fail"));
    let run = connector
        .start(AgentRequest::single_user_message(RequestId(13), "hi"))
        .unwrap();
    let _ = run.recv_timeout(Duration::from_secs(2)).unwrap();
    let AgentEvent::Failed { error, .. } = run.recv_timeout(Duration::from_secs(2)).unwrap() else {
        panic!("expected a failed event");
    };

    assert_eq!(error.code, ConnectorErrorCode::Process);
    assert!(error.message.contains("7"));
    assert!(error.message.contains("fixture failure"));
}

#[test]
fn cancellation_kills_the_child_and_emits_cancelled() {
    let connector =
        CliConnector::new(fixture_command("wait")).with_poll_interval(Duration::from_millis(5));
    let run = connector
        .start(AgentRequest::single_user_message(RequestId(14), "hi"))
        .unwrap();
    let _ = run.recv_timeout(Duration::from_secs(2)).unwrap();

    run.cancel();

    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        let event = run
            .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
            .unwrap();
        if matches!(event, AgentEvent::Cancelled { .. }) {
            break;
        }
        assert!(!matches!(event, AgentEvent::Completed { .. }));
    }
}

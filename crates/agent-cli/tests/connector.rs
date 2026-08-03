use std::{
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use agent_cli::{
    CliCommandSpec, CliConnector, CodexCliConfig, CodexCliConnector, discover_codex_executable_in,
};
use agent_core::{AgentConnector, AgentEvent, AgentRequest, ConnectorErrorCode, RequestId};

fn fixture_command(mode: &str) -> CliCommandSpec {
    CliCommandSpec::new(PathBuf::from(env!("CARGO_BIN_EXE_agent-cli-fixture")))
        .with_args([OsString::from(mode)])
}

fn codex_fixture(extra_args: impl IntoIterator<Item = &'static str>) -> CodexCliConnector {
    let config = CodexCliConfig::new(PathBuf::from(env!("CARGO_BIN_EXE_agent-cli-fixture")))
        .unwrap()
        .with_extra_args(extra_args);
    CodexCliConnector::new(config)
}

#[test]
fn managed_codex_connectors_keep_distinct_profile_identities() {
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_agent-cli-fixture"));
    let first = CodexCliConnector::new_managed(
        "codex-fast",
        "Codex Fast",
        CodexCliConfig::new(&executable).unwrap(),
    );
    let second = CodexCliConnector::new_managed(
        "codex-deep",
        "Codex Deep",
        CodexCliConfig::new(&executable).unwrap(),
    );

    assert_eq!(first.descriptor().id, "codex-fast");
    assert_eq!(first.descriptor().display_name, "Codex Fast");
    assert_eq!(second.descriptor().id, "codex-deep");
    assert_eq!(second.descriptor().display_name, "Codex Deep");
}

fn temporary_directory(label: &str) -> PathBuf {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "zzh-desktop-assistant-{label}-{}-{id}",
        std::process::id()
    ));
    fs::create_dir_all(&path).unwrap();
    path
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

#[test]
fn codex_executable_discovery_uses_the_first_matching_search_directory() {
    let first = temporary_directory("codex-discovery-first");
    let second = temporary_directory("codex-discovery-second");
    let executable_name = if cfg!(windows) { "codex.exe" } else { "codex" };
    let expected = second.join(executable_name);
    fs::write(&expected, b"fixture").unwrap();

    let discovered = discover_codex_executable_in([first.as_path(), second.as_path()]);

    assert_eq!(discovered.as_deref(), Some(expected.as_path()));
    fs::remove_dir_all(first).unwrap();
    fs::remove_dir_all(second).unwrap();
}

#[test]
fn codex_argument_construction_is_fixed_and_keeps_extra_arguments_literal() {
    let config = CodexCliConfig::new(PathBuf::from(env!("CARGO_BIN_EXE_agent-cli-fixture")))
        .unwrap()
        .with_extra_args(["--model", "gpt-test", "value & not-a-command"]);

    let spec = config.command_spec();

    assert_eq!(spec.executable, config.executable().to_path_buf());
    assert_eq!(
        spec.args,
        [
            "exec",
            "--json",
            "--skip-git-repo-check",
            "--sandbox",
            "read-only",
            "--model",
            "gpt-test",
            "value & not-a-command",
            "-",
        ]
        .map(OsString::from)
    );
}

#[test]
fn codex_streams_jsonl_agent_message_updates_without_duplicate_text() {
    let connector = codex_fixture([]);
    let run = connector
        .start(AgentRequest::single_user_message(RequestId(21), "hello"))
        .unwrap();

    assert!(matches!(
        run.recv_timeout(Duration::from_secs(2)).unwrap(),
        AgentEvent::Started {
            request_id: RequestId(21)
        }
    ));
    assert!(matches!(
        run.recv_timeout(Duration::from_secs(2)).unwrap(),
        AgentEvent::TextDelta { text, .. } if text == "received:"
    ));
    assert!(matches!(
        run.recv_timeout(Duration::from_secs(2)).unwrap(),
        AgentEvent::TextDelta { text, .. } if text == "hello"
    ));
    assert!(matches!(
        run.recv_timeout(Duration::from_secs(2)).unwrap(),
        AgentEvent::Completed {
            request_id: RequestId(21)
        }
    ));
}

#[test]
fn codex_turn_failure_is_reported_as_a_protocol_error() {
    let connector = codex_fixture(["--fixture-fail"]);
    let run = connector
        .start(AgentRequest::single_user_message(RequestId(22), "hello"))
        .unwrap();
    let _ = run.recv_timeout(Duration::from_secs(2)).unwrap();
    let AgentEvent::Failed { error, .. } = run.recv_timeout(Duration::from_secs(2)).unwrap() else {
        panic!("expected Codex failure event");
    };

    assert_eq!(error.code, ConnectorErrorCode::Protocol);
    assert_eq!(error.message, "Codex turn failed: fixture turn failure");
}

#[test]
fn codex_cancellation_terminates_the_process() {
    let connector = codex_fixture(["--fixture-wait"]).with_poll_interval(Duration::from_millis(5));
    let run = connector
        .start(AgentRequest::single_user_message(RequestId(23), "hello"))
        .unwrap();
    let _ = run.recv_timeout(Duration::from_secs(2)).unwrap();

    run.cancel();

    let event = run.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(matches!(
        event,
        AgentEvent::Cancelled {
            request_id: RequestId(23)
        }
    ));
}

#[test]
fn missing_codex_executable_is_rejected_before_starting_a_run() {
    let directory = temporary_directory("missing-codex");
    let missing = directory.join("definitely-missing-codex.exe");

    let error = CodexCliConfig::new(&missing).unwrap_err();

    assert_eq!(error.code, ConnectorErrorCode::Configuration);
    assert!(error.message.contains("Codex executable was not found"));
    assert!(!Path::new(&missing).exists());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn codex_connector_uses_a_stable_descriptor() {
    let connector = codex_fixture([]);

    assert_eq!(connector.descriptor().id, "codex-cli");
    assert_eq!(connector.descriptor().display_name, "Codex CLI");
    assert_eq!(
        connector.config().executable().as_os_str(),
        OsStr::new(env!("CARGO_BIN_EXE_agent-cli-fixture"))
    );
}

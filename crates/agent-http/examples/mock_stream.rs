use agent_core::{AgentConnector, AgentEvent, AgentRequest, RequestId};
use agent_http::{MockHttpConnector, MockHttpScenario};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let connector = MockHttpConnector::new(MockHttpScenario::Success {
        chunks: vec!["Hello".into(), " from".into(), " Mock HTTP".into()],
    });
    let run = connector.start(AgentRequest::single_user_message(
        RequestId(1),
        "introduce yourself",
    ))?;

    loop {
        let event = run.recv()?;
        println!("{event:?}");
        if matches!(
            event,
            AgentEvent::Completed { .. } | AgentEvent::Cancelled { .. } | AgentEvent::Failed { .. }
        ) {
            break;
        }
    }

    Ok(())
}

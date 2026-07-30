use std::{env, io::Read};

use agent_cli::{CliCommandSpec, CliConnector};
use agent_core::{AgentConnector, AgentEvent, AgentRequest, RequestId};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if env::args().nth(1).as_deref() == Some("--fixture") {
        let mut prompt = String::new();
        std::io::stdin().read_to_string(&mut prompt)?;
        println!("received:{prompt}");
        println!("done");
        return Ok(());
    }

    let command = CliCommandSpec::new(env::current_exe()?).with_args(["--fixture"]);
    let connector = CliConnector::new(command);
    let run = connector.start(AgentRequest::single_user_message(
        RequestId(1),
        "hello from CLI",
    ))?;

    loop {
        let event = run.recv()?;
        println!("{event:?}");
        if matches!(
            event,
            AgentEvent::Completed { .. }
                | AgentEvent::Cancelled { .. }
                | AgentEvent::Failed { .. }
        ) {
            break;
        }
    }

    Ok(())
}

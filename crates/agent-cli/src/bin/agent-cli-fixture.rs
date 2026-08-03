use std::{
    env,
    io::{self, Read, Write},
    process, thread,
    time::Duration,
};

use serde_json::json;

fn main() -> io::Result<()> {
    let mode = env::args().nth(1).unwrap_or_default();
    match mode.as_str() {
        "exec" => run_codex_fixture(),
        "stream" => {
            let mut prompt = String::new();
            io::stdin().read_to_string(&mut prompt)?;
            println!("received:{prompt}");
            io::stdout().flush()?;
            thread::sleep(Duration::from_millis(20));
            println!("done");
        }
        "fail" => {
            eprintln!("fixture failure");
            process::exit(7);
        }
        "wait" => {
            println!("waiting");
            io::stdout().flush()?;
            thread::sleep(Duration::from_secs(10));
        }
        _ => process::exit(2),
    }
    Ok(())
}

fn run_codex_fixture() {
    let args = env::args().skip(2).collect::<Vec<_>>();
    println!(
        "{}",
        json!({"type": "thread.started", "thread_id": "fixture-thread"})
    );
    println!("{}", json!({"type": "turn.started"}));
    io::stdout().flush().expect("flush fixture start events");

    if args.iter().any(|argument| argument == "--fixture-wait") {
        thread::sleep(Duration::from_secs(10));
        return;
    }

    if args.iter().any(|argument| argument == "--fixture-fail") {
        println!(
            "{}",
            json!({
                "type": "turn.failed",
                "error": {"message": "fixture turn failure"}
            })
        );
        return;
    }

    let mut prompt = String::new();
    io::stdin()
        .read_to_string(&mut prompt)
        .expect("read fixture prompt");
    println!(
        "{}",
        json!({
            "type": "item.updated",
            "item": {
                "id": "item-1",
                "type": "agent_message",
                "text": "received:"
            }
        })
    );
    io::stdout().flush().expect("flush first fixture delta");
    thread::sleep(Duration::from_millis(20));
    println!(
        "{}",
        json!({
            "type": "item.completed",
            "item": {
                "id": "item-1",
                "type": "agent_message",
                "text": format!("received:{prompt}")
            }
        })
    );
    println!("{}", json!({"type": "turn.completed"}));
}

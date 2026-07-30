use std::{
    env,
    io::{self, Read, Write},
    process, thread,
    time::Duration,
};

fn main() -> io::Result<()> {
    let mode = env::args().nth(1).unwrap_or_default();
    match mode.as_str() {
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

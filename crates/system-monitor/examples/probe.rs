use std::{thread, time::Duration};

use system_monitor::SystemSampler;

fn main() {
    let mut sampler = SystemSampler::new();

    for _ in 0..3 {
        thread::sleep(Duration::from_secs(1));
        println!("{:#?}", sampler.sample());
    }
}

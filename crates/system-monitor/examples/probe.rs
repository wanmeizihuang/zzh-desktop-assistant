use std::{thread, time::Duration};

use system_monitor::{SystemSampler, prepare_gpu_temperature_sources};

fn main() {
    // SAFETY: No worker threads exist before the sampler probe starts.
    unsafe { prepare_gpu_temperature_sources() };
    let mut sampler = SystemSampler::new();

    for _ in 0..3 {
        thread::sleep(Duration::from_secs(1));
        println!("{:#?}", sampler.sample());
    }
}

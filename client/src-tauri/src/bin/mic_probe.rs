//! 麦克风采集探针：每秒打印采集样本数与 RMS 能量（说话时明显增大）。
use std::time::{Duration, Instant};

use echoroom_client_lib::audio::capture::MicCapture;

fn rms(pcm: &[i16]) -> f64 {
    if pcm.is_empty() {
        return 0.0;
    }
    let s: f64 = pcm.iter().map(|&x| (x as f64) * (x as f64)).sum();
    (s / pcm.len() as f64).sqrt()
}

fn main() -> anyhow::Result<()> {
    let mic = MicCapture::open(None)?;
    println!(
        "mic_probe: 采样率 {}Hz，每秒打印一次（Ctrl+C 退出）",
        mic.sample_rate()
    );

    let mut acc: Vec<i16> = Vec::with_capacity(48_000 * 2);
    let mut window_start = Instant::now();
    loop {
        mic.pump(&mut acc)?;
        if window_start.elapsed() >= Duration::from_secs(1) {
            println!("samples={} rms={:.0}", acc.len(), rms(&acc));
            acc.clear();
            window_start = Instant::now();
        }
    }
}

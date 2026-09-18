//! 录音工具：采集默认麦克风 N 秒并保存为 WAV（可用系统播放器回放）。
//! 用法：cargo run --bin mic_record -- [秒数] [输出文件]（默认 5 秒，输出 target/record.wav）
use std::time::{Duration, Instant};

use echoroom_client_lib::audio::capture::MicCapture;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let secs: f64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5.0);
    let path = args.get(2).cloned().unwrap_or_else(|| {
        // 默认输出到 workspace 的 target/ 目录（构建产物区），避免污染源码目录
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace 根目录")
            .join("target")
            .join("record.wav")
            .to_string_lossy()
            .into_owned()
    });

    let mic = MicCapture::open()?;
    let rate = mic.sample_rate();
    let mut audio: Vec<i16> = Vec::new();

    println!("开始录音 {secs} 秒（现在说话）...");
    let start = Instant::now();
    let mut last_report = start;
    while start.elapsed() < Duration::from_secs_f64(secs) {
        mic.pump(&mut audio)?;
        if last_report.elapsed() >= Duration::from_secs(1) {
            println!(
                "  ...{:.0}s / {secs:.0}s（{} 样本）",
                start.elapsed().as_secs_f64(),
                audio.len()
            );
            last_report = Instant::now();
        }
    }

    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(&path, spec)?;
    for &s in &audio {
        writer.write_sample(s)?;
    }
    writer.finalize()?;
    println!(
        "已保存: {path}（{} 样本 ≈ {:.1}s）",
        audio.len(),
        audio.len() as f64 / rate as f64
    );
    Ok(())
}

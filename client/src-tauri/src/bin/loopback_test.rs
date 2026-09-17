//! 自听工具：麦克风 →（直接回放）→ 扬声器/耳机。
//! ⚠ 必须戴耳机测试：外放会形成回声正反馈（啸叫）。
use std::sync::mpsc::sync_channel;

use echoroom_client_lib::audio::capture::MicCapture;
use echoroom_client_lib::audio::denoise::Denoiser;
use echoroom_client_lib::audio::playback::spawn_player;
use echoroom_protocol::FRAME_SAMPLES;

fn main() -> anyhow::Result<()> {
    println!("loopback_test: 麦克风直通自听（Ctrl+C 退出）");
    println!("⚠ 必须戴耳机！外放会啸叫。");

    let mic = MicCapture::open()?;
    // 容量 2 块（~40ms）：满则丢弃（实时优先，低延迟）
    let (tx, rx) = sync_channel::<Vec<i16>>(2);

    // 播放回调：消化通道里的块；不足部分填 0（静音）
    let mut pending: Vec<i16> = Vec::new();
    let _player = spawn_player(move |out: &mut [i16]| {
        let mut written = 0usize;
        if !pending.is_empty() {
            let n = pending.len().min(out.len());
            out[..n].copy_from_slice(&pending[..n]);
            pending.drain(..n);
            written = n;
        }
        while written < out.len() {
            match rx.try_recv() {
                Ok(block) => {
                    let n = block.len().min(out.len() - written);
                    out[written..written + n].copy_from_slice(&block[..n]);
                    written += n;
                    if n < block.len() {
                        pending.extend_from_slice(&block[n..]);
                    }
                }
                Err(_) => break,
            }
        }
        if written < out.len() {
            out[written..].fill(0);
        }
    })?;
    println!("自听已启动，请对着麦克风说话（Ctrl+C 退出）");

    // 主线程 = 采集线程：按 960 样本（20ms）切块；RNNoise 降噪后推入通道
    let mut acc: Vec<i16> = Vec::with_capacity(FRAME_SAMPLES * 2);
    let mut denoiser = Denoiser::new();
    loop {
        mic.pump(&mut acc)?;
        while acc.len() >= FRAME_SAMPLES {
            let mut block: Vec<i16> = acc.drain(..FRAME_SAMPLES).collect();
            denoiser.process(&mut block); // RNNoise 语音降噪（自带 VAD 静音抑制）
            let _ = tx.try_send(block); // 通道满则丢弃本块
        }
    }
}

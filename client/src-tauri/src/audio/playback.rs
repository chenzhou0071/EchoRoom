//! WASAPI 播放：默认输出设备、48kHz i16 输出（信号单声道，双声道左右同源）、事件驱动 + 填充回调。
//! COM 必须在同一线程初始化与使用：设备对象全部在播放线程内创建并持有。
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;

use anyhow::{Context, Result};
use wasapi::{AudioRenderClient, BufferFlags, Direction, SampleType, ShareMode, WaveFormat};

use echoroom_protocol::SAMPLE_RATE;

/// 输出按双声道 i16 请求：每帧 4 字节（信号仍单声道，左右同源复制）
const OUT_BYTES_PER_FRAME: usize = 4;

/// wasapi 0.15 的错误类型为 `Box<dyn Error>`（非 Send+Sync，无法直接进 anyhow），
/// 统一转成字符串错误。
fn err2any(e: Box<dyn std::error::Error>) -> anyhow::Error {
    anyhow::anyhow!(e.to_string())
}

/// 播放器句柄：`stop` 置位后播放线程退出；Drop 时自动置位。
pub struct PlayerHandle {
    pub stop: Arc<AtomicBool>,
}

impl Drop for PlayerHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// 启动播放线程：`fill` 回调负责填充每个播放周期的单声道样本（长度 = 本次可写帧数）。
/// 返回前会等待设备初始化完成；初始化失败直接报错。
pub fn spawn_player<F>(fill: F) -> Result<PlayerHandle>
where
    F: FnMut(&mut [i16]) + Send + 'static,
{
    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = Arc::clone(&stop);
    let (ready_tx, ready_rx) = channel::<Result<()>>();

    std::thread::spawn(move || run_player(fill, &stop_thread, &ready_tx));

    match ready_rx.recv() {
        Ok(Ok(())) => Ok(PlayerHandle { stop }),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(anyhow::anyhow!("player thread exited before init")),
    }
}

fn run_player<F>(mut fill: F, stop: &AtomicBool, ready: &Sender<Result<()>>)
where
    F: FnMut(&mut [i16]) + Send + 'static,
{
    let (client, render, event) = match init_render() {
        Ok(triple) => triple,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    let _ = ready.send(Ok(()));
    if let Err(e) = render_loop(&mut fill, &client, &render, &event, stop) {
        eprintln!("[audio] 播放线程异常退出: {e:#}");
    }
}

/// 设备初始化：返回 (AudioClient, AudioRenderClient, 事件句柄)。
fn init_render() -> Result<(wasapi::AudioClient, AudioRenderClient, wasapi::Handle)> {
    wasapi::initialize_mta()
        .ok()
        .context("initialize COM (MTA)")?;
    let device = wasapi::get_default_device(&Direction::Render)
        .map_err(err2any)
        .context("default render device")?;
    let mut client = device
        .get_iaudioclient()
        .map_err(err2any)
        .context("IAudioClient")?;
    // 双声道请求：信号仍为单声道，写出前由本模块复制左右声道（听感居中、双耳平衡）；
    // 共享模式 + convert=true：输出侧同样走 AUTOCONVERTPCM，统一 48k/i16。
    let format = WaveFormat::new(16, 16, &SampleType::Int, SAMPLE_RATE as usize, 2, None);
    client
        .initialize_client(
            &format,
            200_000, // 20ms 缓冲（低延迟；系统会向上对齐到设备周期）
            &Direction::Render,
            &ShareMode::Shared,
            true,
        )
        .map_err(err2any)
        .context("initialize render client")?;
    let render = client
        .get_audiorenderclient()
        .map_err(err2any)
        .context("render client")?;
    let buf_frames = client.get_bufferframecount().map_err(err2any)? as usize;
    println!(
        "播放缓冲: {} 帧 (~{:.0}ms)",
        buf_frames,
        buf_frames as f64 * 1000.0 / SAMPLE_RATE as f64
    );
    // 启动前把当前可用空间填满静音，避免上电瞬间欠载产生爆音
    let prefill = client.get_available_space_in_frames().map_err(err2any)? as usize;
    if prefill > 0 {
        let silence = vec![0u8; prefill * OUT_BYTES_PER_FRAME];
        render
            .write_to_device(prefill, &silence, Some(BufferFlags::none()))
            .map_err(err2any)
            .context("prefill silence")?;
    }
    let event = client
        .set_get_eventhandle()
        .map_err(err2any)
        .context("set event handle")?;
    client
        .start_stream()
        .map_err(err2any)
        .context("start stream")?;
    Ok((client, render, event))
}

fn render_loop<F>(
    fill: &mut F,
    client: &wasapi::AudioClient,
    render: &AudioRenderClient,
    event: &wasapi::Handle,
    stop: &AtomicBool,
) -> Result<()>
where
    F: FnMut(&mut [i16]) + Send + 'static,
{
    while !stop.load(Ordering::Relaxed) {
        // 事件驱动；10ms 超时兜底轮询（事件可能不触发；也保证 stop 快速响应）
        let _ = event.wait_for_event(10);
        let avail = client.get_available_space_in_frames().map_err(err2any)? as usize;
        if avail == 0 {
            continue;
        }
        let mut buf = vec![0i16; avail];
        fill(&mut buf);
        // 单声道样本 → 双声道交错（左右同源；Windows 小端，与采集侧 from_le_bytes 对称）
        let mut bytes = Vec::with_capacity(buf.len() * OUT_BYTES_PER_FRAME);
        for s in &buf {
            let b = s.to_le_bytes();
            bytes.extend_from_slice(&b);
            bytes.extend_from_slice(&b);
        }
        render
            .write_to_device(avail, &bytes, Some(BufferFlags::none()))
            .map_err(err2any)?;
    }
    client.stop_stream().map_err(err2any)?;
    Ok(())
}

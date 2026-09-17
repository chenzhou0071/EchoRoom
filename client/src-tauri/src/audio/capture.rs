//! 麦克风采集：默认输入设备、48kHz 单声道 i16、事件驱动。
//! COM 必须在同一线程初始化与使用：本结构体不实现 Send，采集线程内创建并使用。
use anyhow::{Context, Result};
use wasapi::{AudioCaptureClient, AudioClient, Direction, SampleType, ShareMode, WaveFormat};

use echoroom_protocol::SAMPLE_RATE;

/// 请求格式固定为单声道 i16：每帧 2 字节
const BYTES_PER_FRAME: usize = 2;

/// wasapi 0.15 的错误类型为 `Box<dyn Error>`（非 Send+Sync，无法直接进 anyhow），
/// 统一转成字符串错误。
fn err2any(e: Box<dyn std::error::Error>) -> anyhow::Error {
    anyhow::anyhow!(e.to_string())
}

pub struct MicCapture {
    /// 持有底层 client 存活（drop 即关流）
    _audio_client: AudioClient,
    capture_client: AudioCaptureClient,
    /// 事件句柄：pump 用 wait_for_event 驱动
    _event: wasapi::Handle,
    sample_rate: u32,
}

impl MicCapture {
    pub fn open() -> Result<MicCapture> {
        wasapi::initialize_mta()
            .ok()
            .context("initialize COM (MTA)")?;
        let device = wasapi::get_default_device(&Direction::Capture)
            .map_err(err2any)
            .context("default capture device")?;
        let mut client = device
            .get_iaudioclient()
            .map_err(err2any)
            .context("IAudioClient")?;

        // 仅提示：输出侧靠下方 convert=true 的自动转换保证 48k/单声道/i16
        if let Ok(mix) = client.get_mixformat() {
            if mix.get_samplespersec() != SAMPLE_RATE {
                eprintln!(
                    "[audio] 设备原生采样率 {}Hz != {}Hz，已启用 WASAPI 自动转换",
                    mix.get_samplespersec(),
                    SAMPLE_RATE
                );
            }
        }

        let format = WaveFormat::new(16, 16, &SampleType::Int, SAMPLE_RATE as usize, 1, None);
        // 共享模式 + convert=true：AUTOCONVERTPCM，WASAPI 自动重采样到请求格式
        client
            .initialize_client(
                &format,
                200_000, // 20ms 设备缓冲（低延迟，自听/通话场景关键）
                &Direction::Capture,
                &ShareMode::Shared,
                true,
            )
            .map_err(err2any)
            .context("initialize capture client")?;
        let event = client
            .set_get_eventhandle()
            .map_err(err2any)
            .context("set event handle")?;
        let capture_client = client
            .get_audiocaptureclient()
            .map_err(err2any)
            .context("capture client")?;
        // 必须显式启动流，否则永远读不到数据
        client
            .start_stream()
            .map_err(err2any)
            .context("start stream")?;
        Ok(MicCapture {
            _audio_client: client,
            capture_client,
            _event: event,
            sample_rate: SAMPLE_RATE,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// 阻塞等待声卡事件并读出所有可用样本（追加到 out）；返回本次追加的样本数。
    pub fn pump(&self, out: &mut Vec<i16>) -> Result<usize> {
        // 事件驱动等待新数据；10ms 超时兜底轮询——AUTOCONVERTPCM 下事件回调
        // 可能不按预期触发，超时过长会把每次读取拖到超时点，引入明显延迟。
        let _ = self._event.wait_for_event(10);
        let mut total = 0usize;
        loop {
            let frames = self
                .capture_client
                .get_next_nbr_frames()
                .map_err(err2any)?
                .unwrap_or(0) as usize;
            if frames == 0 {
                break;
            }
            // read_from_device 要求缓冲能容纳整包（不足会报错）
            let mut bytes = vec![0u8; frames * BYTES_PER_FRAME];
            let (read_frames, _flags) = self
                .capture_client
                .read_from_device(&mut bytes)
                .map_err(err2any)
                .context("read from device")?;
            if read_frames == 0 {
                break;
            }
            let n = read_frames as usize;
            out.reserve(n);
            for c in bytes[..n * BYTES_PER_FRAME].chunks_exact(BYTES_PER_FRAME) {
                out.push(i16::from_le_bytes([c[0], c[1]]));
            }
            total += n;
        }
        Ok(total)
    }
}

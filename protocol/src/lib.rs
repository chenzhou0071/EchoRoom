//! EchoRoom 共享协议：TCP 控制消息与 UDP 语音包的编解码。
//! 客户端与服务端共同依赖本 crate（单一事实来源）。

pub mod messages;
pub mod tcp;

/// 默认服务端口（TCP 与 UDP 同号）
pub const DEFAULT_PORT: u16 = 9000;
/// 房间容量（含自己）
pub const ROOM_CAPACITY: usize = 6;
/// 音频帧：20ms @ 48kHz
pub const FRAME_SAMPLES: usize = 960;
/// 采样率
pub const SAMPLE_RATE: u32 = 48000;
/// Opus 目标码率
pub const OPUS_BITRATE: i32 = 40_000;
/// 心跳间隔
pub const HEARTBEAT_INTERVAL_MS: u64 = 2000;
/// UDP 映射超时
pub const UDP_TIMEOUT_MS: u64 = 15_000;

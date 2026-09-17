//! Opus 编解码封装（48kHz 单声道，20ms 帧 = 960 样本）。
use audiopus::coder::{Decoder, Encoder};
use audiopus::{Application, Bitrate, Channels, SampleRate};

use echoroom_protocol::{FRAME_SAMPLES, OPUS_BITRATE};

/// 编码器：麦克风 PCM → Opus 包
pub struct OpusEnc(Encoder);

impl OpusEnc {
    pub fn new() -> Result<Self, audiopus::Error> {
        let mut e = Encoder::new(SampleRate::Hz48000, Channels::Mono, Application::Voip)?;
        e.set_bitrate(Bitrate::BitsPerSecond(OPUS_BITRATE))?;
        Ok(Self(e))
    }

    /// 编码一帧（pcm.len() 必须为 FRAME_SAMPLES）
    pub fn encode(&mut self, pcm: &[i16]) -> Result<Vec<u8>, audiopus::Error> {
        debug_assert_eq!(pcm.len(), FRAME_SAMPLES);
        let mut out = vec![0u8; 512]; // 40kbps×20ms≈100B，512 有充足余量
        let n = self.0.encode(pcm, &mut out)?;
        out.truncate(n);
        Ok(out)
    }
}

/// 解码器：Opus 包 → PCM（None 触发丢包隐藏 PLC）
pub struct OpusDec(Decoder);

impl OpusDec {
    pub fn new() -> Result<Self, audiopus::Error> {
        Ok(Self(Decoder::new(SampleRate::Hz48000, Channels::Mono)?))
    }

    /// 解码一帧到 out（长度须为 FRAME_SAMPLES）；返回解码样本数（正常 = 960）
    pub fn decode(&mut self, frame: Option<&[u8]>, out: &mut [i16]) -> Result<usize, audiopus::Error> {
        debug_assert_eq!(out.len(), FRAME_SAMPLES);
        self.0.decode(frame, out, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine_440() -> Vec<i16> {
        let mut v = Vec::with_capacity(960);
        for i in 0..960 {
            let t = i as f32 / 48000.0;
            v.push((f32::sin(2.0 * std::f32::consts::PI * 440.0 * t) * 12000.0) as i16);
        }
        v
    }

    fn rms(pcm: &[i16]) -> f64 {
        if pcm.is_empty() {
            return 0.0;
        }
        let s: f64 = pcm.iter().map(|&x| (x as f64) * (x as f64)).sum();
        (s / pcm.len() as f64).sqrt()
    }

    #[test]
    fn encode_decode_roundtrip_preserves_signal() {
        let input = sine_440();
        let mut enc = OpusEnc::new().unwrap();
        let mut dec = OpusDec::new().unwrap();
        let packet = enc.encode(&input).unwrap();
        assert!(!packet.is_empty(), "编码输出不应为空");
        let mut out = vec![0i16; 960];
        let n = dec.decode(Some(&packet), &mut out).unwrap();
        assert_eq!(n, 960, "解码应输出整帧");
        // 正弦 440Hz 经 Opus 后能量同量级（保守阈值：>30%）
        assert!(rms(&out) > rms(&input) * 0.3, "能量过度衰减: {} vs {}", rms(&out), rms(&input));
    }

    #[test]
    fn plc_decode_does_not_panic() {
        let mut dec = OpusDec::new().unwrap();
        let mut out = vec![0i16; 960];
        let n = dec.decode(None, &mut out).unwrap();
        assert_eq!(n, 960);
    }
}

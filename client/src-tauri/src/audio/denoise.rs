//! RNNoise 语音降噪封装（48kHz 单声道 i16，帧 480 样本 = 10ms）。
//! 数据约定：nnnoiseless 的 f32 使用 i16 量程（[-32768, 32767]），
//! 与管线内 i16 直接互转即可（不同于常见的 [-1, 1] 归一化）。

use nnnoiseless::DenoiseState;

/// 单帧样本数（RNNoise 固定 10ms @ 48kHz）
pub const FRAME: usize = DenoiseState::FRAME_SIZE;

pub struct Denoiser {
    state: Box<DenoiseState<'static>>,
    in_f32: [f32; FRAME],
    out_f32: [f32; FRAME],
}

impl Denoiser {
    pub fn new() -> Self {
        let mut state = DenoiseState::new();
        // 预热一帧静音：消耗掉首帧输出（文档明确提示含 fade-in 伪影），
        // 避免真实语音的第一帧出现异常。
        let warm_in = [0.0f32; FRAME];
        let mut warm_out = [0.0f32; FRAME];
        state.process_frame(&mut warm_out, &warm_in);
        Self {
            state,
            in_f32: [0.0; FRAME],
            out_f32: [0.0; FRAME],
        }
    }

    /// 原地降噪一个块（长度须为 480 的整倍数）；返回平均 VAD 概率（0~1）。
    pub fn process(&mut self, pcm: &mut [i16]) -> f32 {
        debug_assert_eq!(pcm.len() % FRAME, 0, "块长度须为 480 的整倍数");
        let mut vad = 0.0f32;
        let mut n_frames = 0usize;
        for chunk in pcm.chunks_exact_mut(FRAME) {
            for (i, &s) in chunk.iter().enumerate() {
                self.in_f32[i] = s as f32;
            }
            vad += self.state.process_frame(&mut self.out_f32, &self.in_f32);
            n_frames += 1;
            for (i, s) in chunk.iter_mut().enumerate() {
                // 输出可能略超 i16 量程，clamp 防止回绕
                *s = self.out_f32[i].round().clamp(-32768.0, 32767.0) as i16;
            }
        }
        if n_frames == 0 {
            0.0
        } else {
            vad / n_frames as f32
        }
    }
}

impl Default for Denoiser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(pcm: &[i16]) -> f64 {
        if pcm.is_empty() {
            return 0.0;
        }
        let s: f64 = pcm.iter().map(|&x| (x as f64) * (x as f64)).sum();
        (s / pcm.len() as f64).sqrt()
    }

    #[test]
    fn silence_stays_silent() {
        let mut d = Denoiser::new();
        let mut block = vec![0i16; 960];
        let vad = d.process(&mut block);
        let peak = block.iter().map(|s| s.abs()).max().unwrap();
        assert!(peak < 200, "静音输入应输出近静音, peak={peak}");
        assert!((0.0..=1.0).contains(&vad), "vad 越界: {vad}");
    }

    #[test]
    fn noise_is_attenuated() {
        // 伪随机白噪声：降噪后能量应显著低于输入（RNNoise 对非语音噪声有强抑制）
        let mut d = Denoiser::new();
        let mut seed = 12345u32;
        let mut block: Vec<i16> = (0..960 * 20)
            .map(|_| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                ((seed >> 16) as i16) / 8
            })
            .collect();
        let rms_in = rms(&block);
        d.process(&mut block);
        let rms_out = rms(&block);
        assert!(
            rms_out < rms_in,
            "噪声能量应被衰减: in={rms_in:.1} out={rms_out:.1}"
        );
    }
}

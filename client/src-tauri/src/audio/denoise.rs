//! RNNoise 语音降噪封装（48kHz 单声道 i16，帧 480 样本 = 10ms）。
//! 数据约定：nnnoiseless 的 f32 使用 i16 量程（[-32768, 32767]），
//! 与管线内 i16 直接互转即可（不同于常见的 [-1, 1] 归一化）。
//! 干湿混合：按 RNNoise 自带 VAD 概率自适应保留干信号（语音段留细节，噪声段全湿强抑制）。

use nnnoiseless::DenoiseState;

/// 单帧样本数（RNNoise 固定 10ms @ 48kHz）
pub const FRAME: usize = DenoiseState::FRAME_SIZE;

/// 语音段最多保留的干信号比例（VAD→1 时；兼顾降噪与高频自然度）
const DRY_MIX_MAX: f32 = 0.25;
/// 湿比平滑系数（每 10ms 帧更新）：约 100ms 时间常数，防混合突变泵动
const MIX_SMOOTH: f32 = 0.15;

/// VAD 概率 → 目标湿比：语音段（vad→1）降湿留干；噪声段（vad→0）全湿强抑制
fn wet_coef_for(vad: f32) -> f32 {
    1.0 - DRY_MIX_MAX * vad.clamp(0.0, 1.0)
}

pub struct Denoiser {
    state: Box<DenoiseState<'static>>,
    in_f32: [f32; FRAME],
    out_f32: [f32; FRAME],
    /// 输入副本（干信号；与降噪输出混合用）
    dry_f32: [f32; FRAME],
    /// 平滑后的湿比（1.0 = 全湿，初始全湿随 VAD 收敛）
    mix_wet: f32,
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
            dry_f32: [0.0; FRAME],
            mix_wet: 1.0,
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
                self.dry_f32[i] = s as f32;
            }
            let frame_vad = self.state.process_frame(&mut self.out_f32, &self.in_f32);
            vad += frame_vad;
            n_frames += 1;
            // 自适应干湿混合：湿比平滑逼近目标（防突变泵动）
            let target = wet_coef_for(frame_vad);
            self.mix_wet += (target - self.mix_wet) * MIX_SMOOTH;
            let dry = 1.0 - self.mix_wet;
            for (i, s) in chunk.iter_mut().enumerate() {
                // 输出可能略超 i16 量程，clamp 防止回绕
                *s = (self.out_f32[i] * self.mix_wet + self.dry_f32[i] * dry)
                    .round()
                    .clamp(-32768.0, 32767.0) as i16;
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

    #[test]
    fn silence_and_floor_levels_probe() {
        // VAD 阈值定值探针：观察"静音/真实底噪"经 RNNoise 后的输出能量
        // （cargo test silence_and_floor -- --nocapture 查看）
        let mut d = Denoiser::new();
        let mut zero = vec![0i16; 960 * 30];
        d.process(&mut zero);
        let peak = zero.iter().map(|s| s.abs()).max().unwrap();
        println!("[probe] 全 0 输入 → 输出 rms={:.2} peak={peak}", rms(&zero));
        assert!(rms(&zero) < 0.001, "纯静音输入应输出全 0");

        // 低幅白噪声：模拟真实麦克风底噪（rms ≈ 24，与录得的底噪 < 40 同量级）
        let mut d2 = Denoiser::new();
        let mut seed = 999u32;
        let mut noise: Vec<i16> = (0..960 * 30)
            .map(|_| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                ((seed >> 16) as i16) / 800
            })
            .collect();
        let r_in = rms(&noise);
        d2.process(&mut noise);
        println!("[probe] 底噪输入 rms={r_in:.1} → 输出 rms={:.2}", rms(&noise));
        assert!(rms(&noise) < r_in, "底噪能量应被部分衰减");
    }

    #[test]
    fn wet_coef_interpolates_by_vad() {
        assert!((wet_coef_for(1.0) - 0.75).abs() < 1e-6, "语音段应保留 25% 干信号");
        assert!((wet_coef_for(0.0) - 1.0).abs() < 1e-6, "噪声段应全湿");
        assert_eq!(wet_coef_for(2.0), wet_coef_for(1.0), "越界概率须 clamp 到 1.0");
        assert!(
            wet_coef_for(0.5) > 0.75 && wet_coef_for(0.5) < 1.0,
            "中间概率应线性插值"
        );
    }
}

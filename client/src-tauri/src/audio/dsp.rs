//! 采集端轻量 DSP：高通滤波（去喷麦/低频隆隆）+ 咔嗒/瞬态抑制 + 增益软限幅（防硬削爆音）+ 压缩（治吹麦/近讲持续过响）。
//! 纯 Rust 无依赖；单声道 48kHz i16 语音采集链专用。

/// 麦克风高通截止频率：切掉爆破音（p/b）与桌面震动的低频冲击；
/// 语音基频 85Hz 以上基本无损（100Hz 处 -3dB）。
pub const MIC_HPF_HZ: f32 = 100.0;

/// 双二次 biquad 核心（Direct Form I，状态跨样本连续；f32 逐样本处理）
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Biquad {
    /// RBJ Cookbook 高通（Q = 1/√2 巴特沃斯）
    fn highpass(cutoff_hz: f32, sample_rate: f32) -> Self {
        let w0 = 2.0 * std::f32::consts::PI * cutoff_hz / sample_rate;
        let cos_w0 = w0.cos();
        let alpha = w0.sin() / std::f32::consts::SQRT_2; // 1/(2Q)，Q = 1/√2
        let a0 = 1.0 + alpha;
        Self {
            b0: (1.0 + cos_w0) / 2.0 / a0,
            b1: -(1.0 + cos_w0) / a0,
            b2: (1.0 + cos_w0) / 2.0 / a0,
            a1: -2.0 * cos_w0 / a0,
            a2: (1.0 - alpha) / a0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    /// RBJ Cookbook 低通（Q = 1/√2 巴特沃斯）
    fn lowpass(cutoff_hz: f32, sample_rate: f32) -> Self {
        let w0 = 2.0 * std::f32::consts::PI * cutoff_hz / sample_rate;
        let cos_w0 = w0.cos();
        let alpha = w0.sin() / std::f32::consts::SQRT_2; // 1/(2Q)，Q = 1/√2
        let a0 = 1.0 + alpha;
        Self {
            b0: (1.0 - cos_w0) / 2.0 / a0,
            b1: (1.0 - cos_w0) / a0,
            b2: (1.0 - cos_w0) / 2.0 / a0,
            a1: -2.0 * cos_w0 / a0,
            a2: (1.0 - alpha) / a0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    fn step(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

/// 2 阶巴特沃斯高通（对外封装，i16 原地处理；跨块保状态）
pub struct HighPass(Biquad);

impl HighPass {
    pub fn new(cutoff_hz: f32, sample_rate: f32) -> Self {
        Self(Biquad::highpass(cutoff_hz, sample_rate))
    }

    /// 原地滤波一个块（任意长度；跨块保状态）
    pub fn process(&mut self, pcm: &mut [i16]) {
        for s in pcm.iter_mut() {
            let y = self.0.step(*s as f32);
            *s = y.round().clamp(i16::MIN as f32, i16::MAX as f32) as i16;
        }
    }
}

/// 软限幅拐点（60% 满幅）：以下完全透明（线性，无色变）
const KNEE: f32 = 0.6 * 32767.0;
/// 软限幅渐近线（95% 满幅）：再大的输入也压不过这里（绝不回绕/硬削）
const ASYMPTOTE: f32 = 0.95 * 32767.0;

/// 增益 + 软限幅（f32 域一次完成）：替代"乘增益后硬 clamp"。
/// 拐点以下透明；以上 tanh 平滑压缩（拐点处值与斜率连续），治大声音/高增益削顶爆音。
pub fn gain_and_limit(pcm: &mut [i16], gain: f32) {
    let range = ASYMPTOTE - KNEE;
    for s in pcm.iter_mut() {
        let x = *s as f32 * gain;
        let mag = x.abs();
        let y = if mag <= KNEE {
            x
        } else {
            x.signum() * (KNEE + range * ((mag - KNEE) / range).tanh())
        };
        *s = y.round().clamp(i16::MIN as f32, i16::MAX as f32) as i16;
    }
}

/// 咔嗒/瞬态抑制（de-click）：键盘按键声这类">3kHz 极速脉冲"的专用降噪环节。
/// 原理：3kHz 低通分带 —— 低频带 = LP(x)、高频带 = x − LP(x)，逐样本精确互补
/// （不能用高通输出当高频带：它带相位偏移，压低后相加抵消不彻底）
/// + 快/慢包络突起检测；命中时短促压低高频带增益后重构 out = x·g + (1−g)·LP(x)。
/// g=1 逐样本还原；g→0 等价"只留低频带"：6kHz 咔嗒衰减 80%+，语音主体几乎不动。
const CLICK_CUTOFF_HZ: f32 = 3000.0;
/// 突起判定比：快包络 ≥ 慢包络 ×4（+12dB 瞬时突起）视为瞬态
const CLICK_RATIO: f32 = 4.0;
/// 快包络绝对下限：低于此不触发（防低电平噪声误动作）
const CLICK_FLOOR: f32 = 200.0;
/// 抑制深度（-24dB）
const CLICK_DUCK: f32 = 0.06;
/// 触发保持 8ms（384 样本）：覆盖脉冲本体与混响起头
const CLICK_HOLD: u32 = 384;
/// 快包络衰减 ≈ 5ms
const CLICK_FAST_DECAY: f32 = 0.99584;
/// 慢包络平均 ≈ 150ms
const CLICK_SLOW_COEF: f32 = 0.000139;
/// 抑制增益攻击 ≈ 0.2ms（压住脉冲最前缘）
const CLICK_ATTACK: f32 = 0.104;
/// 抑制增益释放 ≈ 20ms（字间快速恢复，不积压）
const CLICK_RELEASE: f32 = 0.00104;
/// 启动预热 480 样本（10ms）：跳过滤波器冷启动瞬态（数字信号从静默突变的假咔嗒）
const CLICK_WARMUP: u32 = 480;
/// 预热期慢包络收敛系数（≈4ms）：冷启动没有历史基线，先用快系数把环境水平立起来
const CLICK_WARM_SLOW_COEF: f32 = 0.005;

pub struct DeClicker {
    split: Biquad, // 3kHz 低通：高频带 = x − 低通输出（精确互补，无相位失配）
    fast: f32,
    slow: f32,
    gain: f32,
    hold: u32,
    warm: u32,
}

impl DeClicker {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            split: Biquad::lowpass(CLICK_CUTOFF_HZ, sample_rate),
            fast: 0.0,
            slow: 0.0,
            gain: 1.0,
            hold: 0,
            warm: CLICK_WARMUP,
        }
    }

    /// 原地处理一个块（任意长度；跨块保状态）
    pub fn process(&mut self, pcm: &mut [i16]) {
        for s in pcm.iter_mut() {
            let x = *s as f32;
            let low = self.split.step(x);
            let hf = x - low; // 高频带 = 原信号 − 低频带（逐样本互补）
            let mag = hf.abs();
            // 快包络：瞬时攻击、5ms 衰减；慢包络：150ms 平均
            self.fast = if mag > self.fast { mag } else { self.fast * CLICK_FAST_DECAY };
            self.slow += (mag - self.slow) * CLICK_SLOW_COEF;
            // 预热期只听不判：慢包络先快速收敛建立基线（否则冷启动信号被整体误判为"突起"）
            if self.warm > 0 {
                self.slow += (mag - self.slow) * CLICK_WARM_SLOW_COEF;
                self.warm -= 1;
            } else if self.fast > self.slow * CLICK_RATIO && self.fast > CLICK_FLOOR {
                self.hold = CLICK_HOLD;
            } else if self.hold > 0 {
                self.hold -= 1;
            }
            let target = if self.hold > 0 { CLICK_DUCK } else { 1.0 };
            let c = if target < self.gain { CLICK_ATTACK } else { CLICK_RELEASE };
            self.gain += (target - self.gain) * c;
            // 互补重构：out = 低频带 + 高频带×g = x·g + (1−g)·LP(x)；g=1 逐样本还原
            let out = low + hf * self.gain;
            *s = out.round().clamp(i16::MIN as f32, i16::MAX as f32) as i16;
        }
    }
}

/// 压缩器阈值（-12dBFS）：以下完全透明，以上按 4:1 下压
const LEVEL_THRESHOLD: f32 = 0.25 * 32767.0;
/// 4:1 压缩的指数形式（1 − 1/ratio）
const LEVEL_RATIO_EXP: f32 = 0.75;
/// 峰值包络回落 ≈150ms（48kHz）
const LEVEL_ENV_RELEASE: f32 = 0.000139;
/// 增益下压 ≈5ms（48kHz）
const LEVEL_GAIN_ATTACK: f32 = 0.004158;
/// 增益回升 ≈150ms（48kHz）
const LEVEL_GAIN_RELEASE: f32 = 0.000139;

/// 压缩器（Leveller）：治"持续过响"——吹麦、贴麦近讲、喊叫导致接收端超大声/破音。
/// 峰值包络瞬时攻、150ms 放；超阈按 4:1 下压（满幅 → ≈0.35 满幅，-9dB），
/// 阈值以下逐样本透明（正常说话不受影响、无需补偿增益）；下压 5ms/回升 150ms 防抽吸。
pub struct Leveller {
    env: f32,
    gain: f32,
}

impl Leveller {
    pub fn new() -> Self {
        Self { env: 0.0, gain: 1.0 }
    }

    /// 原地处理一个块（任意长度；跨块保状态）
    pub fn process(&mut self, pcm: &mut [i16]) {
        for s in pcm.iter_mut() {
            let x = *s as f32;
            let mag = x.abs();
            // 峰值包络：瞬时攻、150ms 放（持续过响跟得住，回落不迟滞）
            self.env = if mag > self.env {
                mag
            } else {
                self.env * (1.0 - LEVEL_ENV_RELEASE)
            };
            // 静态曲线：env ≤ 阈值 → 1；以上 (T/env)^0.75（等价 4:1）
            let target = if self.env <= LEVEL_THRESHOLD {
                1.0
            } else {
                (LEVEL_THRESHOLD / self.env).powf(LEVEL_RATIO_EXP)
            };
            let c = if target < self.gain {
                LEVEL_GAIN_ATTACK
            } else {
                LEVEL_GAIN_RELEASE
            };
            self.gain += (target - self.gain) * c;
            let out = x * self.gain;
            *s = out.round().clamp(i16::MIN as f32, i16::MAX as f32) as i16;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48000.0;

    fn sine(freq: f32, amp: f32, n: usize) -> Vec<i16> {
        (0..n)
            .map(|i| {
                let t = i as f32 / SR;
                (f32::sin(2.0 * std::f32::consts::PI * freq * t) * amp) as i16
            })
            .collect()
    }

    fn rms(pcm: &[i16]) -> f64 {
        if pcm.is_empty() {
            return 0.0;
        }
        let s: f64 = pcm.iter().map(|&x| (x as f64) * (x as f64)).sum();
        (s / pcm.len() as f64).sqrt()
    }

    #[test]
    fn hpf_attenuates_plosive_band() {
        // 50Hz（爆破音/震动频段，fc 的一半）应被显著衰减（2 阶 ≈ -12dB）
        let mut hp = HighPass::new(MIC_HPF_HZ, SR);
        let mut sig = sine(50.0, 10000.0, 9600);
        hp.process(&mut sig);
        let in_rms = rms(&sine(50.0, 10000.0, 9600)[4800..]);
        let out_rms = rms(&sig[4800..]); // 跳过瞬态建立段
        assert!(
            out_rms < in_rms * 0.35,
            "50Hz 衰减不足: in={in_rms:.0} out={out_rms:.0}"
        );
    }

    #[test]
    fn hpf_passes_voice_band() {
        // 1kHz（语音主体）通过率 > 90%
        let mut hp = HighPass::new(MIC_HPF_HZ, SR);
        let mut sig = sine(1000.0, 10000.0, 9600);
        hp.process(&mut sig);
        let in_rms = rms(&sine(1000.0, 10000.0, 9600)[4800..]);
        let out_rms = rms(&sig[4800..]);
        assert!(
            out_rms > in_rms * 0.9,
            "1kHz 通过率过低: in={in_rms:.0} out={out_rms:.0}"
        );
    }

    #[test]
    fn soft_limit_transparent_below_knee() {
        // 拐点以下：完全线性（±1 舍入误差）
        let mut sig = sine(300.0, 15000.0, 960);
        let orig = sig.clone();
        gain_and_limit(&mut sig, 1.0);
        for (a, b) in sig.iter().zip(orig.iter()) {
            assert!((*a as i32 - *b as i32).abs() <= 1, "小信号被色变: {a} vs {b}");
        }
    }

    #[test]
    fn soft_limit_compresses_without_wraparound() {
        // 满幅输入：平滑压缩到渐近线附近，绝不回绕（翻转/溢出）
        let mut sig = vec![32767i16; 480];
        gain_and_limit(&mut sig, 1.0);
        assert!(
            sig.iter().all(|&v| (28000..=31000).contains(&v)),
            "满幅限幅异常: {:?}",
            &sig[..4]
        );
        // 增益 2× 推入过载区：仍平滑有界
        let mut sig2 = vec![20000i16; 480];
        gain_and_limit(&mut sig2, 2.0);
        assert!(
            sig2.iter().all(|&v| (29000..=32000).contains(&v)),
            "过载限幅异常: {:?}",
            &sig2[..4]
        );
        // 负半周对称（不回绕为正值）
        let mut sig3 = vec![-32767i16; 480];
        gain_and_limit(&mut sig3, 1.0);
        assert!(
            sig3.iter().all(|&v| (-31000..=-28000).contains(&v)),
            "负半周限幅异常: {:?}",
            &sig3[..4]
        );
    }

    /// 低频"语音"衬底（400Hz），用于验证瞬态抑制不伤低频主体
    fn base_tone(n: usize) -> Vec<i16> {
        sine(400.0, 3000.0, n)
    }

    #[test]
    fn declick_low_tone_bit_transparent() {
        // 纯低频（语音主体频段）：低频带 = 原信号 − 高频带，应逐样本恒等还原
        let mut dc = DeClicker::new(SR);
        let mut sig = sine(400.0, 8000.0, 4800);
        let orig = sig.clone();
        dc.process(&mut sig);
        for (a, b) in sig.iter().zip(orig.iter()) {
            assert!((*a as i32 - *b as i32).abs() <= 1, "低频被色变: {a} vs {b}");
        }
    }

    #[test]
    fn declick_suppresses_hf_click() {
        // 叠加在低频衬底上的 5ms 高频咔嗒（近似键盘按键声）应被强力压制
        let mut dc = DeClicker::new(SR);
        let n = 14400;
        let mut sig = base_tone(n);
        for (i, s) in sig.iter_mut().enumerate().take(1440).skip(1200) {
            let t = i as f32 / SR;
            let click = f32::sin(2.0 * std::f32::consts::PI * 6000.0 * t) * 18000.0;
            *s = (*s as f32 + click).clamp(-32768.0, 32767.0) as i16;
        }
        let orig = sig.clone();
        dc.process(&mut sig);
        // 咔嗒段 RMS 降 65% 以上
        let r_in = rms(&orig[1200..1440]);
        let r_out = rms(&sig[1200..1440]);
        assert!(
            r_out < r_in * 0.35,
            "咔嗒压制不足: in={r_in:.0} out={r_out:.0}"
        );
        // 200ms 后完全恢复（低频衬底逐样本还原）
        for (a, b) in sig[9600..].iter().zip(orig[9600..].iter()) {
            assert!((*a as i32 - *b as i32).abs() <= 2, "恢复不完全: {a} vs {b}");
        }
    }

    #[test]
    fn declick_sustained_hf_only_touched_at_onset() {
        // 持续高频（近似擦音 s）：起点短暂压制后恢复，长段能量保留 >95%
        let mut dc = DeClicker::new(SR);
        let mut sig = sine(6000.0, 5000.0, 28800); // 600ms
        let orig = sig.clone();
        dc.process(&mut sig);
        let tail_in = rms(&orig[9600..]);
        let tail_out = rms(&sig[9600..]);
        assert!(
            tail_out > tail_in * 0.95,
            "擦音尾部被过度压制: in={tail_in:.0} out={tail_out:.0}"
        );
    }

    #[test]
    fn leveller_transparent_below_threshold() {
        // 阈值以下（峰值 6000 < 8192）：逐样本透明
        let mut lv = Leveller::new();
        let mut sig = sine(300.0, 6000.0, 4800);
        let orig = sig.clone();
        lv.process(&mut sig);
        for (a, b) in sig.iter().zip(orig.iter()) {
            assert!((*a as i32 - *b as i32).abs() <= 1, "小信号被压缩: {a} vs {b}");
        }
    }

    #[test]
    fn leveller_tames_sustained_full_scale() {
        // 满幅持续（吹麦场景）：稳态压到 ≈0.35 满幅，且平滑无跳变（不产生破音）
        let mut lv = Leveller::new();
        let mut sig = sine(300.0, 32767.0, 9600); // 200ms
        lv.process(&mut sig);
        let tail_peak = sig[7200..].iter().map(|&s| (s as i32).abs()).max().unwrap();
        assert!(
            (9000..=14000).contains(&tail_peak),
            "满幅稳态未压到目标区: {tail_peak}"
        );
        for w in sig[7200..].windows(2) {
            assert!(
                (w[0] as i32 - w[1] as i32).abs() < 4000,
                "出现跳变: {} {}",
                w[0],
                w[1]
            );
        }
    }

    #[test]
    fn leveller_recovers_after_loud_burst() {
        // 过响段结束后 1.5s：增益恢复，正常音量声音复原
        let mut lv = Leveller::new();
        let quiet = sine(300.0, 3000.0, 96000);
        let mut sig = sine(300.0, 32767.0, 4800); // 100ms 过响
        sig.extend(quiet.iter().copied());
        lv.process(&mut sig);
        let restore_from = 4800 + 72000; // 过响结束后 1.5s
        for (i, (&a, &b)) in sig[restore_from..]
            .iter()
            .zip(quiet[72000..].iter())
            .enumerate()
        {
            assert!((a as i32 - b as i32).abs() <= 2, "未恢复: i={i} {a} vs {b}");
        }
    }
}

//! 混音：多路 i16 PCM 逐样本求和（i32 中间态）→ tanh 软限幅 → i16。
//! tanh 映射：y = T * tanh(x / T)，T = 0.8 * 32767 ≈ 26214。
//! 特性：小信号近似线性（无色变），大信号平滑压缩（不爆音、不硬削）。

/// tanh 软限幅的拐点参数 T：0.8 × i16 满幅
pub const SOFT_LIMIT_T: f32 = 26214.0;

pub struct MixAccumulator {
    acc: Vec<i32>,
}

impl MixAccumulator {
    pub fn new(len: usize) -> Self {
        MixAccumulator { acc: vec![0i32; len] }
    }

    /// 累加一路（各路长度可不等；超出部分的样本忽略）
    pub fn add(&mut self, pcm: &[i16]) {
        for (a, &s) in self.acc.iter_mut().zip(pcm.iter()) {
            *a += s as i32;
        }
    }

    pub fn clear(&mut self) {
        self.acc.iter_mut().for_each(|a| *a = 0);
    }

    /// 软限幅写出
    pub fn finalize(&self, out: &mut [i16]) {
        for (o, &a) in out.iter_mut().zip(self.acc.iter()) {
            let x = a as f32;
            let y = SOFT_LIMIT_T * (x / SOFT_LIMIT_T).tanh();
            *o = y.clamp(i16::MIN as f32, i16::MAX as f32) as i16;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const N: usize = 960;

    fn ramp(scale: i16) -> Vec<i16> {
        (0..N)
            .map(|i| ((i as i32 * scale as i32 / N as i32) - scale as i32 / 2) as i16)
            .collect()
    }

    #[test]
    fn silence_plus_signal_equals_signal() {
        let sig = ramp(10000);
        let mut acc = MixAccumulator::new(N);
        acc.add(&vec![0i16; N]);
        acc.add(&sig);
        let mut out = vec![0i16; N];
        acc.finalize(&mut out);
        // 小信号处于近似线性区：输出与输入接近（每样本误差 < 200）
        for (o, s) in out.iter().zip(sig.iter()) {
            assert!((*o as i32 - *s as i32).abs() < 200, "o={o} s={s}");
        }
    }

    #[test]
    fn two_equal_signals_roughly_double_in_linear_zone() {
        let sig = ramp(6000);
        let mut acc = MixAccumulator::new(N);
        acc.add(&sig);
        acc.add(&sig);
        let mut out = vec![0i16; N];
        acc.finalize(&mut out);
        let mid = N / 2;
        let expected = sig[mid] as i32 * 2;
        assert!(
            (out[mid] as i32 - expected).abs() < 400,
            "out={} expected={}",
            out[mid],
            expected
        );
    }

    #[test]
    fn loud_mix_is_soft_limited() {
        // 4 路满幅同相信号：原始和 4×32767，必须被压回 i16 范围且不翻转符号
        let sig = vec![30000i16; N];
        let mut acc = MixAccumulator::new(N);
        for _ in 0..4 {
            acc.add(&sig);
        }
        let mut out = vec![0i16; N];
        acc.finalize(&mut out);
        for &v in &out {
            assert!(v > 20000, "软限幅后仍应是大信号: {v}");
            assert!(v < 32767, "不得削顶到上限: {v}");
        }
    }

    #[test]
    fn clear_resets() {
        let mut acc = MixAccumulator::new(N);
        acc.add(&vec![1000i16; N]);
        acc.clear();
        let mut out = vec![1i16; N];
        acc.finalize(&mut out);
        for &v in &out {
            assert_eq!(v, 0);
        }
    }

    #[test]
    fn empty_mix_is_silence() {
        let acc = MixAccumulator::new(N);
        let mut out = vec![123i16; N];
        acc.finalize(&mut out);
        assert!(out.iter().all(|&v| v == 0));
    }
}

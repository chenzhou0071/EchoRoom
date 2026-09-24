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

/// 咔嗒/瞬态抑制（de-click v3）：键盘按键声这类">3kHz 极速脉冲"的专用降噪环节。
/// 原理：3kHz 低通分带 —— 低频带 = LP(x)、高频带 = x − LP(x)，逐样本精确互补
/// （不能用高通输出当高频带：它带相位偏移，压低后相加抵消不彻底）。
/// 判定："延迟 15ms + 瞬态确认"——快包络突升后持续观察，真瞬态 = 快升快落
/// （回落到事件峰值 25% 以下）→ 申请压制该起点起 8ms；升上去不回落（擦音/语音起跳）
/// → 一直挂起不压（擦音结束时的回落判定因起点过旧自动作废）；观察中再次突升 2× 以上
/// （持续音里的键击）→ 事件起点更新。压制施加在延迟 15ms 的流上：判定完成时触发点
/// 样本恰好尚未输出，故能完整覆盖脉冲本体。命中时短促压低高频带增益后重构
/// out = 低频带 + 高频带×g；g=1 逐样本还原；g→0 等价"只留低频带"：6kHz 咔嗒衰减 80%+，
/// 语音/擦音主体不动（v2 的"条件续期"会误伤擦音 89ms——"高频被降"听感的根因，已废除）。
const CLICK_CUTOFF_HZ: f32 = 3000.0;
/// 突起判定比：快包络 ≥ 慢包络 ×4（+12dB 瞬时突起）视为瞬态
const CLICK_RATIO: f32 = 4.0;
/// 快包络绝对下限：低于此不触发（防低电平噪声误动作）
const CLICK_FLOOR: f32 = 200.0;
/// 抑制深度（-24dB）
const CLICK_DUCK: f32 = 0.06;
/// 压制窗口 8ms（384 样本）：确认后覆盖脉冲本体与混响起尾
const CLICK_HOLD: u32 = 384;
/// 判定延迟 15ms（720 样本）：触发后先观察确认，延迟线保证触发点样本尚未输出
const CLICK_LOOKAHEAD: u32 = 720;
/// 最小观察 3ms（144 样本）：包络成形前不判回落
const CLICK_MIN_OBS: u32 = 144;
/// 回落确认比：快包络 < 事件峰值×0.25 → 快升快落 = 真瞬态
const CLICK_REL_DROP: f32 = 0.25;
/// 观察中再次突升比：> 事件峰值×2 → 更强瞬态（持续音里的键击）→ 事件起点更新
const CLICK_RESET_RATIO: f32 = 2.0;
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
    warm: u32,
    // 瞬态观察器：突升后持续跟踪，等"回落"确认真瞬态（无超时——持续音一直挂起，不压）
    observing: bool,
    obs_elapsed: u32,
    obs_peak: f32,
    obs_t0: u64,
    // 压制计划（输入样本号区间 [plan_start, plan_end)）：支持合并，过旧自动作废
    plan_start: u64,
    plan_end: u64,
    plan_active: bool,
    // 延迟线（15ms）：判定完成时触发点样本恰好尚未输出，故能完整覆盖脉冲
    d_low: [f32; CLICK_LOOKAHEAD as usize],
    d_hf: [f32; CLICK_LOOKAHEAD as usize],
    d_idx: usize,
    pos: u64, // 当前输入样本号（单调递增）
}

impl DeClicker {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            split: Biquad::lowpass(CLICK_CUTOFF_HZ, sample_rate),
            fast: 0.0,
            slow: 0.0,
            gain: 1.0,
            warm: CLICK_WARMUP,
            observing: false,
            obs_elapsed: 0,
            obs_peak: 0.0,
            obs_t0: 0,
            plan_start: 0,
            plan_end: 0,
            plan_active: false,
            d_low: [0.0; CLICK_LOOKAHEAD as usize],
            d_hf: [0.0; CLICK_LOOKAHEAD as usize],
            d_idx: 0,
            pos: 0,
        }
    }

    /// 申请压制 [t0, t0 + CLICK_HOLD)：与现有计划重叠则合并；整体已被输出位置越过（过旧）则作废。
    /// 过旧作废是"擦音结束才回落判定"不误压的关键：那时起点早已输出完毕。
    fn schedule(&mut self, t0: u64, out_pos: i64) {
        let end = t0 + CLICK_HOLD as u64;
        if end as i64 <= out_pos {
            return; // 过旧作废
        }
        if !self.plan_active || t0 >= self.plan_end || end <= self.plan_start {
            self.plan_start = t0; // 无计划/完全不相交：替换
            self.plan_end = end;
        } else {
            self.plan_start = self.plan_start.min(t0); // 重叠：合并（链式判定不互相覆盖）
            self.plan_end = self.plan_end.max(end);
        }
        self.plan_active = true;
    }

    /// 原地处理一个块（任意长度；跨块保状态；输出整体延迟 15ms）
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
            } else if self.observing {
                if self.fast > self.obs_peak * CLICK_RESET_RATIO
                    && self.fast > self.slow * CLICK_RATIO
                    && self.fast > CLICK_FLOOR
                {
                    // 观察中再次突升：更强瞬态（持续音里的键击）→ 事件起点移到它
                    self.obs_elapsed = 1;
                    self.obs_peak = self.fast;
                    self.obs_t0 = self.pos;
                } else {
                    self.obs_elapsed += 1;
                    if self.fast > self.obs_peak {
                        self.obs_peak = self.fast;
                    }
                    // 回落确认：快升快落 = 真瞬态 → 申请压制；持续不回落（擦音/语音）→ 继续挂起
                    if self.obs_elapsed >= CLICK_MIN_OBS
                        && (self.fast < self.obs_peak * CLICK_REL_DROP || self.fast < CLICK_FLOOR)
                    {
                        let out_pos = self.pos as i64 - CLICK_LOOKAHEAD as i64;
                        self.schedule(self.obs_t0, out_pos);
                        self.observing = false;
                    }
                }
            } else if self.fast > self.slow * CLICK_RATIO && self.fast > CLICK_FLOOR {
                self.observing = true;
                self.obs_elapsed = 1;
                self.obs_peak = self.fast;
                self.obs_t0 = self.pos;
            }
            // 延迟线：读出 15ms 前的分带结果、写入当前样本（判定时刻触发点尚未读出）
            let o_low = self.d_low[self.d_idx];
            let o_hf = self.d_hf[self.d_idx];
            self.d_low[self.d_idx] = low;
            self.d_hf[self.d_idx] = hf;
            self.d_idx = (self.d_idx + 1) % CLICK_LOOKAHEAD as usize;
            let out_pos = self.pos as i64 - CLICK_LOOKAHEAD as i64;
            if self.plan_active && self.plan_end as i64 <= out_pos {
                self.plan_active = false; // 计划整体越过输出位置 → 失效
            }
            let target = if self.plan_active && out_pos >= self.plan_start as i64 {
                CLICK_DUCK
            } else {
                1.0
            };
            let c = if target < self.gain { CLICK_ATTACK } else { CLICK_RELEASE };
            self.gain += (target - self.gain) * c;
            // 互补重构：out = 低频带 + 高频带×g；g=1 逐样本还原
            let out = o_low + o_hf * self.gain;
            *s = out.round().clamp(i16::MIN as f32, i16::MAX as f32) as i16;
            self.pos += 1;
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

/// 低频突冲限制器（PopLimiter）：治“噗”——读 3/p/b 时气流冲麦的低频强冲击（实测用户录音：
/// 能量 84-97% 在 300Hz 以下、质心 100-250Hz、峰值满幅 0dB、1ms 极速起跳）。
/// 触发闸门：250Hz 低通包络“快 ≥ 慢×4 且快 > 下限”——只认“低频突冲”（持续浊音不触发、高频咔嗒不触发）。
/// 压制：命中窗口内把全带压向阈值平台 T（-18dB）——但深度动态：冲击多高压多少（小噗轻微、大噗压到平台），
/// 双包络前瞻：当前包络提前降增益（防冲击前缘 1ms 起跳逃逸）、样本包络自限（防衰减段回血）。
/// 10ms 延迟线：判定完成时触发点样本尚未输出，压制能盖住冲击本体；样本与包络同刻读出保持对齐。
/// 与 Leveller 的区别：Leveller 管“持续过响”（慢、软、全时）；本环节管“瞬态突冲”（快、硬、专治低频噗）。
const POP_LP_HZ: f32 = 250.0;
/// 突冲判定比：快包络 ≥ 慢包络 ×4（持续音的峰均比约 2-3，不触发）
const POP_RATIO: f32 = 4.0;
/// 快包络绝对下限（-22dBFS）：正常语音低频（-30dB 级）达不到
const POP_FLOOR: f32 = 0.08 * 32767.0;
/// 压制平台阈值（-18dBFS）：命中时超过此值的部分被压回来
const POP_THRESH: f32 = 0.12 * 32767.0;
/// 增益下限（-24dB）：防极端冲击把增益压成 0
const POP_GAIN_MIN: f32 = 0.06;
/// 压制窗口 30ms：快包络波谷间断期间保持压制
const POP_HOLD: u32 = 1440;
/// 判定延迟 10ms（480 样本）：触发后触发点样本尚未输出
const POP_DELAY: u32 = 480;
/// 快包络衰减 ≈ 5ms
const POP_FAST_DECAY: f32 = 0.99584;
/// 慢包络平均 ≈ 150ms
const POP_SLOW_COEF: f32 = 0.000139;
/// 慢包络门控：快包络超过慢包络 3 倍时不更新慢包络（突冲不抬高环境基线，快速连击不丢触发）
const POP_SLOW_GATE: f32 = 3.0;
/// 慢包络门控下限（≈ -41dBFS）：静音底噪仍参与平均，防止 slow→0 后任何声都像突冲
const POP_SLOW_FLOOR: f32 = 300.0;
/// 全带峰值保持包络衰减 ≈ 30ms（压制深度参考：覆盖冲击整个持续段）
const POP_ENV_DECAY: f32 = 0.99931;
/// 增益下压 ≈ 1ms
const POP_ATTACK: f32 = 0.08;
/// 增益回升 ≈ 20ms
const POP_RELEASE: f32 = 0.001;
/// 启动预热 480 样本（10ms）：跳过滤波器冷启动瞬态
const POP_WARMUP: u32 = 480;
/// 预热期慢包络收敛系数（≈ 4ms）
const POP_WARM_SLOW_COEF: f32 = 0.005;

pub struct PopLimiter {
    det: Biquad, // 250Hz 低通：检测带（噗的能量区）
    fast: f32,
    slow: f32,
    env: f32,  // 全带峰值保持包络（压制额度参考）
    gain: f32,
    warm: u32,
    hold: u32,
    d_x: [f32; POP_DELAY as usize],   // 样本延迟线（10ms）
    d_env: [f32; POP_DELAY as usize], // 包络延迟线（与样本同刻读出，增益永不错位）
    d_idx: usize,
}

impl PopLimiter {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            det: Biquad::lowpass(POP_LP_HZ, sample_rate),
            fast: 0.0,
            slow: 0.0,
            env: 0.0,
            gain: 1.0,
            warm: POP_WARMUP,
            hold: 0,
            d_x: [0.0; POP_DELAY as usize],
            d_env: [0.0; POP_DELAY as usize],
            d_idx: 0,
        }
    }

    /// 原地处理一个块（任意长度；跨块保状态；输出整体延迟 10ms）
    pub fn process(&mut self, pcm: &mut [i16]) {
        for s in pcm.iter_mut() {
            let x = *s as f32;
            let mag = self.det.step(x).abs(); // 检测带：250Hz 低通
            let magf = x.abs(); // 全带峰值：额度参考（输出是全带，额度必须覆盖全带）
            self.fast = if mag > self.fast {
                mag
            } else {
                self.fast * POP_FAST_DECAY
            };
            // 慢包络：突冲期间门控不更新（保持环境基线，快速连击不抬门槛）
            if mag < self.slow.max(POP_SLOW_FLOOR) * POP_SLOW_GATE || self.warm > 0 {
                let c = if self.warm > 0 {
                    POP_WARM_SLOW_COEF
                } else {
                    POP_SLOW_COEF
                };
                self.slow += (mag - self.slow) * c;
            }
            // 全带峰值保持包络（30ms）：压制深度参考
            self.env = if magf > self.env {
                magf
            } else {
                self.env * POP_ENV_DECAY
            };
            if self.warm > 0 {
                self.warm -= 1;
            } else if self.fast > self.slow * POP_RATIO && self.fast > POP_FLOOR {
                self.hold = POP_HOLD;
            } else if self.hold > 0 {
                self.hold -= 1;
            }
            // 延迟线：样本与包络同刻读出（增益永远作用于该样本自己时刻的幅度）
            let xs = self.d_x[self.d_idx];
            let de = self.d_env[self.d_idx];
            self.d_x[self.d_idx] = x;
            self.d_env[self.d_idx] = self.env;
            self.d_idx = (self.d_idx + 1) % POP_DELAY as usize;
            let target = if self.hold > 0 {
                let a = POP_THRESH / (de + 1e-9); // 样本自身幅度：自限（防衰减段回血）
                let b = POP_THRESH / (self.env + 1e-9); // 当前冲击幅度：前瞻（防前缘逃逸）
                a.min(b).min(1.0).max(POP_GAIN_MIN)
            } else {
                1.0
            };
            let c = if target < self.gain {
                POP_ATTACK
            } else {
                POP_RELEASE
            };
            self.gain += (target - self.gain) * c;
            let out = xs * self.gain;
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
        // 纯低频（语音主体频段）：低频带 = 原信号 − 高频带，延迟对齐后逐样本恒等还原
        let mut dc = DeClicker::new(SR);
        let n = 4800;
        let mut sig = sine(400.0, 8000.0, n);
        let orig = sig.clone();
        dc.process(&mut sig);
        // 输出整体延迟 CLICK_LOOKAHEAD：out[D+i] ≡ in[i]；前 D 样本为延迟静默
        let d = CLICK_LOOKAHEAD as usize;
        for &a in &sig[..d] {
            assert_eq!(a, 0, "延迟段应为静默");
        }
        for (a, b) in sig[d..].iter().zip(orig[..n - d].iter()) {
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
        // 咔嗒段 RMS 降 65% 以上（输出延迟 D：咔嗒在 [1200+D, 1440+D)）
        let d = CLICK_LOOKAHEAD as usize;
        let r_in = rms(&orig[1200..1440]);
        let r_out = rms(&sig[1200 + d..1440 + d]);
        assert!(
            r_out < r_in * 0.35,
            "咔嗒压制不足: in={r_in:.0} out={r_out:.0}"
        );
        // 200ms 后完全恢复（低频衬底逐样本还原，含延迟平移）
        for (a, b) in sig[9600 + d..].iter().zip(orig[9600..].iter()) {
            assert!((*a as i32 - *b as i32).abs() <= 2, "恢复不完全: {a} vs {b}");
        }
    }

    #[test]
    fn declick_sustained_hf_not_ducked() {
        // 持续高频（近似擦音 s，读"3"的喷气音）：快包络升上去不回落 → 一直挂起观察、
        // 全程零压制（v2 的"条件续期"会压 89ms——"高频被降"听感的根因，此测试防回归）
        let mut dc = DeClicker::new(SR);
        let n = 28800;
        let mut sig = sine(6000.0, 5000.0, n); // 600ms
        let orig = sig.clone();
        dc.process(&mut sig);
        let d = CLICK_LOOKAHEAD as usize;
        let from = 4800; // 跳过预热/观察建立段
        for (i, (&a, &b)) in sig[from + d..].iter().zip(orig[from..].iter()).enumerate() {
            assert!((a as i32 - b as i32).abs() <= 1, "擦音被压制: i={i} {a} vs {b}");
        }
    }

    #[test]
    fn declick_click_during_sustained_hf_suppressed() {
        // 擦音持续中敲键盘（用户核心场景：说话时打字）：观察中的事件起点应被
        // 新瞬态重置更新 → 键击照常被压（撤销重置路径后此测试会失败）
        let mut dc = DeClicker::new(SR);
        let n = 28800;
        let mut sig = base_tone(n);
        for i in 4800..14400 {
            let t = i as f32 / SR;
            let sib = f32::sin(2.0 * std::f32::consts::PI * 6000.0 * t) * 4000.0;
            sig[i] = (sig[i] as f32 + sib).clamp(-32768.0, 32767.0) as i16;
        }
        for i in 9600..9696 {
            let t = i as f32 / SR;
            let click = f32::sin(2.0 * std::f32::consts::PI * 6000.0 * t) * 16000.0;
            sig[i] = (sig[i] as f32 + click).clamp(-32768.0, 32767.0) as i16;
        }
        let orig = sig.clone();
        dc.process(&mut sig);
        let d = CLICK_LOOKAHEAD as usize;
        let r_in = rms(&orig[9600..9840]);
        let r_out = rms(&sig[9600 + d..9840 + d]);
        assert!(
            r_out < r_in * 0.45,
            "擦音中键击未被压制: in={r_in:.0} out={r_out:.0}"
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

    /// 90Hz 阻尼正弦冲击（噗的核心形态）
    fn pop_burst(amp: f32, n: usize) -> Vec<i16> {
        (0..n)
            .map(|i| {
                let t = i as f32 / SR;
                (f32::sin(2.0 * std::f32::consts::PI * 90.0 * t) * amp * (-(i as f32) / 480.0).exp())
                    as i16
            })
            .collect()
    }

    #[test]
    fn pop_flattens_low_burst() {
        // 满幅低频冲击（用户"噗"的实测形态）：应被压到 -18dB 平台（输出 < 0.17 满幅），
        // 且冲击结束后增益恢复（尾部输出归零）
        let mut pl = PopLimiter::new(SR);
        let mut sig: Vec<i16> = vec![0; 2400];
        sig.extend(pop_burst(30000.0, 5760));
        sig.extend(std::iter::repeat(0).take(4800));
        let mut out = sig.clone();
        pl.process(&mut out);
        let d = POP_DELAY as usize;
        // 冲击位于输入 [2400, 8160) → 输出 [2400+d, 8160+d)
        let peak = out[2400 + d..8160 + d]
            .iter()
            .map(|&v| (v as i32).abs())
            .max()
            .unwrap();
        assert!(peak < 5500, "满幅冲击未被压到平台: peak={peak}");
        let tail = out[8160 + d..]
            .iter()
            .map(|&v| (v as i32).abs())
            .max()
            .unwrap();
        assert!(tail < 500, "冲击后未恢复: tail={tail}");
    }

    #[test]
    fn pop_suppresses_moderate_burst() {
        // 中等幅度突发（0.34 满幅，用户录音里的"小噗"）：同样被压到平台
        let mut pl = PopLimiter::new(SR);
        let mut sig: Vec<i16> = vec![0; 2400];
        sig.extend(pop_burst(11000.0, 5760));
        sig.extend(std::iter::repeat(0).take(4800));
        let mut out = sig.clone();
        pl.process(&mut out);
        let d = POP_DELAY as usize;
        let peak = out[2400 + d..8160 + d]
            .iter()
            .map(|&v| (v as i32).abs())
            .max()
            .unwrap();
        assert!(peak < 5500, "中等冲击未被压到平台: peak={peak}");
    }

    #[test]
    fn pop_transparent_on_voice() {
        // 正常语音（130/260/390Hz 谐波复合）低于快慢比闸门：不得触发，逐样本透明（含 10ms 延迟平移）
        let mut pl = PopLimiter::new(SR);
        let n = 19200;
        let mut sig: Vec<i16> = (0..n)
            .map(|i| {
                let t = i as f32 / SR;
                (f32::sin(2.0 * std::f32::consts::PI * 130.0 * t) * 4000.0
                    + f32::sin(2.0 * std::f32::consts::PI * 260.0 * t) * 2200.0
                    + f32::sin(2.0 * std::f32::consts::PI * 390.0 * t) * 1400.0)
                    as i16
            })
            .collect();
        let orig = sig.clone();
        pl.process(&mut sig);
        let d = POP_DELAY as usize;
        for (i, (&a, &b)) in sig[d..].iter().zip(orig[..n - d].iter()).enumerate() {
            assert!((a as i32 - b as i32).abs() <= 2, "语音被改动: i={i} {a} vs {b}");
        }
    }

    #[test]
    fn pop_transparent_on_sustained_low_tone() {
        // 持续低频浊音（低音嗓）：慢包络跟得上快包络（峰均比 ≈1.6）→ 不触发、透明
        let mut pl = PopLimiter::new(SR);
        let n = 28800;
        let mut sig = sine(150.0, 3000.0, n);
        let orig = sig.clone();
        pl.process(&mut sig);
        let d = POP_DELAY as usize;
        for (i, (&a, &b)) in sig[d..].iter().zip(orig[..n - d].iter()).enumerate() {
            assert!((a as i32 - b as i32).abs() <= 2, "持续浊音被压: i={i} {a} vs {b}");
        }
    }

    #[test]
    fn pop_ignores_hf_click() {
        // 高频脉冲（键盘/擦音属性）：250Hz 检测带里几乎无能量 → 不触发、透明
        let mut pl = PopLimiter::new(SR);
        let n = 9600;
        let mut sig: Vec<i16> = vec![0; n];
        for i in 4800..5760 {
            let t = (i - 4800) as f32 / SR;
            sig[i] = (f32::sin(2.0 * std::f32::consts::PI * 6000.0 * t) * 20000.0
                * (-((i - 4800) as f32) / 96.0).exp()) as i16;
        }
        let orig = sig.clone();
        pl.process(&mut sig);
        let d = POP_DELAY as usize;
        for (&a, &b) in sig[d..].iter().zip(orig[..n - d].iter()) {
            assert!((a as i32 - b as i32).abs() <= 2, "高频脉冲被误动: {a} vs {b}");
        }
    }
}

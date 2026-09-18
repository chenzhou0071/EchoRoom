//! 说话检测：能量双阈值 + 退出保持（防蓝框闪烁）。
use std::time::{Duration, Instant};

pub struct SpeakingDetector {
    enter: f64,
    exit: f64,
    hold: Duration,
    speaking: bool,
    below_since: Option<Instant>,
}

impl SpeakingDetector {
    pub fn new(enter_rms: f64, exit_rms: f64, hold: Duration) -> Self {
        SpeakingDetector {
            enter: enter_rms,
            exit: exit_rms,
            hold,
            speaking: false,
            below_since: None,
        }
    }

    /// 状态翻转时返回 Some(新状态)；否则 None
    pub fn update(&mut self, rms: f64, now: Instant) -> Option<bool> {
        if !self.speaking {
            if rms >= self.enter {
                self.speaking = true;
                self.below_since = None;
                return Some(true);
            }
            None
        } else {
            if rms >= self.exit {
                self.below_since = None;
                return None;
            }
            match self.below_since {
                None => {
                    self.below_since = Some(now);
                    None
                }
                Some(t) if now.duration_since(t) < self.hold => None,
                Some(_) => {
                    self.speaking = false;
                    self.below_since = None;
                    Some(false)
                }
            }
        }
    }

    /// 当前是否处于说话状态（含保持窗口内）
    pub fn speaking(&self) -> bool {
        self.speaking
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn hysteresis_prevents_flicker() {
        let t0 = Instant::now();
        let mut d = SpeakingDetector::new(800.0, 400.0, Duration::from_millis(400));
        // 小噪声（500：低于进入阈值）不触发
        assert_eq!(d.update(500.0, t0), None);
        assert_eq!(d.update(500.0, t0 + Duration::from_millis(100)), None);
        // 说话：超过进入阈值 800 → 变 true
        assert_eq!(d.update(1200.0, t0 + Duration::from_millis(200)), Some(true));
        // 说话中偶尔低于进入但高于退出阈值（500）：保持 true（不闪断）
        assert_eq!(d.update(500.0, t0 + Duration::from_millis(300)), None);
        // 持续低于退出阈值：hold 窗口内保持 true
        assert_eq!(d.update(200.0, t0 + Duration::from_millis(400)), None);
        assert_eq!(d.update(200.0, t0 + Duration::from_millis(700)), None);
        // 超过 hold → 变 false
        assert_eq!(d.update(200.0, t0 + Duration::from_millis(900)), Some(false));
        assert!(!d.speaking(), "退出后应为非说话状态");
    }

    #[test]
    fn immediate_switch_without_hold_for_onset() {
        let t0 = Instant::now();
        let mut d = SpeakingDetector::new(800.0, 400.0, Duration::from_millis(400));
        assert_eq!(d.update(1000.0, t0), Some(true), "进入不加延迟");
        assert!(d.speaking(), "进入后应为说话状态");
    }
}

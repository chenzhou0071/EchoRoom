//! 抖动缓冲：按序列号重排语音帧；水位控制开始播放；缺帧等待后触发 PLC。
//! 每个发送者独立一个实例（调用方按发送者 uid 维护）。

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// 缺帧等待上限：2 帧时长（40ms），超过则触发 PLC
const WAIT_LIMIT: Duration = Duration::from_millis(40);

#[derive(Debug, PartialEq)]
pub enum PopResult {
    /// `Ready(Some(data))`：正常取出一帧；`Ready(None)`：缺帧（调用方用 Opus PLC 补齐）
    Ready(Option<Vec<u8>>),
    /// 水位未到 / 仍在等待缺失帧（调用方本次不出声）
    NotYet,
}

pub struct JitterBuffer {
    target: usize,
    max: usize,
    frames: BTreeMap<u32, Vec<u8>>,
    next_seq: Option<u32>,
    wait_since: Option<Instant>,
    wait_limit: Duration,
}

impl JitterBuffer {
    /// `target`：启动水位（积累到这么多帧才开始出声）；`max`：容量上限（超出裁剪最旧）。
    pub fn new(target: usize, max: usize) -> Self {
        JitterBuffer {
            target,
            max: max.max(target),
            frames: BTreeMap::new(),
            next_seq: None,
            wait_since: None,
            wait_limit: WAIT_LIMIT,
        }
    }

    pub fn insert(&mut self, seq: u32, data: Vec<u8>) {
        if let Some(next) = self.next_seq {
            if seq < next {
                return; // 过老：丢弃
            }
        }
        self.frames.entry(seq).or_insert(data);
        // 超上限：裁剪最旧的（保留最新 max 帧）
        while self.frames.len() > self.max {
            let oldest = *self.frames.keys().next().unwrap();
            self.frames.remove(&oldest);
        }
    }

    pub fn pop(&mut self) -> PopResult {
        // 初始化水位：积累到 target 帧才开始出声
        if self.next_seq.is_none() {
            if self.frames.len() < self.target {
                return PopResult::NotYet;
            }
            self.next_seq = self.frames.keys().next().copied();
        }
        let next = self.next_seq.unwrap();
        if let Some(data) = self.frames.remove(&next) {
            self.next_seq = Some(next.wrapping_add(1));
            self.wait_since = None;
            return PopResult::Ready(Some(data));
        }
        // 缺帧：等待窗口内 NotYet；超时触发 PLC（Ready(None)）
        match self.wait_since {
            None => {
                self.wait_since = Some(Instant::now());
                PopResult::NotYet
            }
            Some(t) if t.elapsed() < self.wait_limit => PopResult::NotYet,
            Some(_) => {
                self.next_seq = Some(next.wrapping_add(1));
                self.wait_since = None;
                PopResult::Ready(None)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(b: u8) -> Vec<u8> {
        vec![b]
    }

    #[test]
    fn fills_watermark_then_pops_in_order() {
        let mut jb = JitterBuffer::new(3, 8);
        jb.insert(0, f(0));
        jb.insert(1, f(1));
        assert_eq!(jb.pop(), PopResult::NotYet, "水位未满不出声");
        jb.insert(2, f(2));
        assert_eq!(jb.pop(), PopResult::Ready(Some(f(0))));
        assert_eq!(jb.pop(), PopResult::Ready(Some(f(1))));
        assert_eq!(jb.pop(), PopResult::Ready(Some(f(2))));
    }

    #[test]
    fn out_of_order_reordered() {
        let mut jb = JitterBuffer::new(2, 8);
        jb.insert(2, f(2));
        jb.insert(0, f(0));
        jb.insert(1, f(1));
        assert_eq!(jb.pop(), PopResult::Ready(Some(f(0))));
        assert_eq!(jb.pop(), PopResult::Ready(Some(f(1))));
        assert_eq!(jb.pop(), PopResult::Ready(Some(f(2))));
    }

    #[test]
    fn missing_frame_triggers_plc_after_wait() {
        let mut jb = JitterBuffer::new(2, 8);
        jb.insert(0, f(0));
        jb.insert(2, f(2));
        assert_eq!(jb.pop(), PopResult::Ready(Some(f(0))));
        // seq=1 缺失：先 NotYet（等待窗口内），超过 40ms 后 Ready(None)（PLC）
        let t0 = std::time::Instant::now();
        let mut plc = false;
        while t0.elapsed() < std::time::Duration::from_millis(200) {
            match jb.pop() {
                PopResult::NotYet => std::thread::sleep(std::time::Duration::from_millis(5)),
                PopResult::Ready(None) => {
                    plc = true;
                    break;
                }
                PopResult::Ready(Some(_)) => panic!("不应有数据"),
            }
        }
        assert!(plc, "缺失帧应在等待超时后触发 PLC");
        assert_eq!(jb.pop(), PopResult::Ready(Some(f(2))), "PLC 后应继续出后续帧");
    }

    #[test]
    fn overflow_drops_oldest() {
        let mut jb = JitterBuffer::new(2, 4);
        for i in 0..10u32 {
            jb.insert(i, f(i as u8));
        }
        // 裁剪保留最新 max=4 帧（6..9）；裁剪后帧数 4 ≥ target=2 → 直接按序出声
        let mut got = Vec::new();
        while let PopResult::Ready(Some(v)) = jb.pop() {
            got.push(v[0]);
        }
        assert_eq!(got, vec![6, 7, 8, 9], "超上限应裁剪最旧、保留最新 4 帧");
    }

    #[test]
    fn late_frame_dropped() {
        let mut jb = JitterBuffer::new(1, 8);
        jb.insert(0, f(0));
        assert_eq!(jb.pop(), PopResult::Ready(Some(f(0))));
        jb.insert(0, f(99)); // 过老的重复帧
        assert_eq!(jb.pop(), PopResult::NotYet);
    }
}

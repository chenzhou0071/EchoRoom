//! 房间状态：成员表、加入/离开、广播、UDP 地址映射与超时。
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use echoroom_protocol::messages::TcpMessage;
use echoroom_protocol::{tcp, ROOM_CAPACITY};

#[derive(Debug, PartialEq)]
pub struct JoinOk {
    pub uid: u16,
    pub token: u32,
    /// 加入前已在房间的成员
    pub members: Vec<(u16, String)>,
}

#[derive(Debug, PartialEq)]
pub enum JoinErr {
    Full,
}

pub struct Member {
    pub uid: u16,
    pub nickname: String,
    pub token: u32,
    pub tx: Sender<Vec<u8>>, // TCP 发送队列（预编码字节）
    pub udp_addr: Option<SocketAddr>,
    pub last_seen: Instant,
}

pub struct Room {
    members: HashMap<u16, Member>,
    next_uid: u16,
    /// 简单确定性伪随机（学习用途：LCG；不引入 rand 依赖）
    rng_state: u64,
}

impl Room {
    pub fn new() -> Room {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E3779B97F4A7C15);
        Room { members: HashMap::new(), next_uid: 1, rng_state: seed | 1 }
    }

    fn next_token(&mut self) -> u32 {
        // LCG（Numerical Recipes 常数）
        self.rng_state = self
            .rng_state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.rng_state >> 32) as u32
    }

    pub fn join(&mut self, nickname: String, tx: Sender<Vec<u8>>) -> Result<JoinOk, JoinErr> {
        if self.members.len() >= ROOM_CAPACITY {
            return Err(JoinErr::Full);
        }
        let members: Vec<(u16, String)> =
            self.members.values().map(|m| (m.uid, m.nickname.clone())).collect();
        let uid = self.next_uid;
        self.next_uid = self.next_uid.wrapping_add(1).max(1);
        let token = self.next_token();
        self.members.insert(
            uid,
            Member {
                uid,
                nickname,
                token,
                tx,
                udp_addr: None,
                last_seen: Instant::now(),
            },
        );
        Ok(JoinOk { uid, token, members })
    }

    pub fn leave(&mut self, uid: u16) -> bool {
        self.members.remove(&uid).is_some()
    }

    pub fn broadcast(&self, except: Option<u16>, msg: &TcpMessage) {
        let bytes = tcp::encode(msg);
        for m in self.members.values() {
            if Some(m.uid) == except {
                continue;
            }
            let _ = m.tx.send(bytes.clone()); // 慢客户端只堵自己的队列
        }
    }

    /// 定向发送给指定成员（预留 API，当前主流程未用）
    #[allow(dead_code)]
    pub fn send_to(&self, uid: u16, msg: &TcpMessage) {
        if let Some(m) = self.members.get(&uid) {
            let _ = m.tx.send(tcp::encode(msg));
        }
    }

    pub fn validate_token(&self, uid: u16, token: u32) -> bool {
        self.members.get(&uid).map(|m| m.token == token).unwrap_or(false)
    }

    pub fn set_udp(&mut self, uid: u16, addr: SocketAddr) {
        if let Some(m) = self.members.get_mut(&uid) {
            m.udp_addr = Some(addr);
            m.last_seen = Instant::now();
        }
    }

    pub fn touch(&mut self, uid: u16) {
        if let Some(m) = self.members.get_mut(&uid) {
            m.last_seen = Instant::now();
        }
    }

    /// 清理超时的 UDP 地址映射（成员保留）；返回被清理的 uid 列表
    pub fn clear_udp_expired(&mut self, timeout: Duration) -> Vec<u16> {
        let now = Instant::now();
        let mut expired = Vec::new();
        for m in self.members.values_mut() {
            if m.udp_addr.is_some() && now.duration_since(m.last_seen) >= timeout {
                m.udp_addr = None;
                expired.push(m.uid);
            }
        }
        expired
    }

    pub fn member_by_addr(&self, addr: SocketAddr) -> Option<u16> {
        self.members.values().find(|m| m.udp_addr == Some(addr)).map(|m| m.uid)
    }

    pub fn others_with_udp(&self, except_uid: u16) -> Vec<SocketAddr> {
        self.members
            .values()
            .filter(|m| m.uid != except_uid && m.udp_addr.is_some())
            .filter_map(|m| m.udp_addr)
            .collect()
    }

    /// 按 uid 查昵称（预留 API，当前主流程未用）
    #[allow(dead_code)]
    pub fn nickname(&self, uid: u16) -> Option<String> {
        self.members.get(&uid).map(|m| m.nickname.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use echoroom_protocol::messages::TcpMessage;
    use std::sync::mpsc;

    #[test]
    fn join_until_capacity_then_reject() {
        let mut room = Room::new();
        let mut rxs = Vec::new();
        for i in 0..ROOM_CAPACITY {
            let (tx, rx) = mpsc::channel();
            let ok = room.join(format!("user{i}"), tx);
            assert!(ok.is_ok());
            rxs.push(rx);
        }
        let (tx, _rx) = mpsc::channel();
        assert_eq!(room.join("第7人".into(), tx), Err(JoinErr::Full));
    }

    #[test]
    fn join_returns_existing_members_and_leaves_removes() {
        let mut room = Room::new();
        let (tx1, _r1) = mpsc::channel();
        let ok1 = room.join("A".into(), tx1).unwrap();
        assert!(ok1.members.is_empty());
        let (tx2, _r2) = mpsc::channel();
        let ok2 = room.join("B".into(), tx2).unwrap();
        assert_eq!(ok2.members, vec![(ok1.uid, "A".to_string())]);
        assert!(room.leave(ok1.uid));
        assert!(!room.leave(ok1.uid)); // 再删为 false
    }

    #[test]
    fn broadcast_skips_except_and_delivers() {
        let mut room = Room::new();
        let (tx1, rx1) = mpsc::channel();
        let (tx2, rx2) = mpsc::channel();
        let a = room.join("A".into(), tx1).unwrap();
        let b = room.join("B".into(), tx2).unwrap();
        room.broadcast(Some(a.uid), &TcpMessage::MemberJoin { uid: b.uid, nickname: "B".into() });
        assert!(rx1.try_recv().is_err(), "A 不应收到（被排除）");
        let bytes = rx2.try_recv().expect("B 应收到");
        let (msg, _) = echoroom_protocol::tcp::try_decode(&bytes).unwrap().unwrap();
        assert_eq!(msg, TcpMessage::MemberJoin { uid: b.uid, nickname: "B".into() });
    }

    #[test]
    fn udp_addr_mapping_and_expiry() {
        use std::time::Duration;
        let mut room = Room::new();
        let (tx, _rx) = mpsc::channel();
        let a = room.join("A".into(), tx).unwrap();
        let addr: std::net::SocketAddr = "127.0.0.1:5555".parse().unwrap();
        assert!(!room.validate_token(a.uid, 999));
        assert!(room.validate_token(a.uid, a.token));
        room.set_udp(a.uid, addr);
        assert_eq!(room.member_by_addr(addr), Some(a.uid));
        assert_eq!(room.others_with_udp(a.uid).len(), 0);
        // 超时 0：立即过期
        let expired = room.clear_udp_expired(Duration::ZERO);
        assert_eq!(expired, vec![a.uid]);
        assert_eq!(room.member_by_addr(addr), None);
    }

    #[test]
    fn send_to_delivers_only_target() {
        let mut room = Room::new();
        let (tx1, rx1) = mpsc::channel();
        let (tx2, rx2) = mpsc::channel();
        let a = room.join("A".into(), tx1).unwrap();
        let _b = room.join("B".into(), tx2).unwrap();
        room.send_to(a.uid, &TcpMessage::LoginReject { reason: "测试".into() });
        let bytes = rx1.try_recv().expect("A 应收到定向消息");
        let (msg, _) = echoroom_protocol::tcp::try_decode(&bytes).unwrap().unwrap();
        assert_eq!(msg, TcpMessage::LoginReject { reason: "测试".into() });
        assert!(rx2.try_recv().is_err(), "B 不应收到定向消息");
    }
}

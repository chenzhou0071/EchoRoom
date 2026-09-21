//! 房间状态：成员表、加入/离开、广播、UDP 地址映射与超时。
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use echoroom_protocol::messages::{MemberInfo, TcpMessage};
use echoroom_protocol::{tcp, ROOM_CAPACITY};

#[derive(Debug, PartialEq)]
pub struct JoinOk {
    pub uid: u16,
    pub token: u32,
    /// 加入前已在房间的成员（五元组：含 has_avatar）
    pub members: Vec<MemberInfo>,
}

#[derive(Debug, PartialEq)]
pub enum JoinErr {
    Full,
}

pub struct Member {
    pub uid: u16,
    /// 持久账号 id（AvatarRequest 查库用）
    pub account_id: i64,
    pub nickname: String,
    pub token: u32,
    /// 是否已上传头像（客户端懒加载标记）
    pub has_avatar: bool,
    pub muted: bool,
    /// 流位图：bit0 = 投屏（屏幕）、bit1 = 摄像头
    pub streams: u8,
    pub tx: Sender<Vec<u8>>, // TCP 发送队列（预编码字节）
    pub udp_addr: Option<SocketAddr>,
    pub last_seen: Instant,
}

pub struct Room {
    members: HashMap<u16, Member>,
    next_uid: u16,
    /// 订阅表：订阅者 uid → 目标 uid（一人最多订阅一人，覆盖式）
    subscriptions: HashMap<u16, u16>,
}

impl Room {
    pub fn new() -> Room {
        Room { members: HashMap::new(), next_uid: 1, subscriptions: HashMap::new() }
    }

    pub fn join(
        &mut self,
        nickname: String,
        account_id: i64,
        has_avatar: bool,
        tx: Sender<Vec<u8>>,
    ) -> Result<JoinOk, JoinErr> {
        if self.members.len() >= ROOM_CAPACITY {
            return Err(JoinErr::Full);
        }
        let members: Vec<MemberInfo> = self
            .members
            .values()
            .map(|m| (m.uid, m.nickname.clone(), m.muted, m.streams, m.has_avatar))
            .collect();
        let uid = self.next_uid;
        self.next_uid = self.next_uid.wrapping_add(1).max(1);
        // UDP 会话 token：真随机（每连接一次性凭证）
        let token = rand::RngCore::next_u32(&mut rand::rngs::OsRng);
        self.members.insert(
            uid,
            Member {
                uid,
                account_id,
                nickname,
                token,
                has_avatar,
                muted: false,
                streams: 0,
                tx,
                udp_addr: None,
                last_seen: Instant::now(),
            },
        );
        Ok(JoinOk { uid, token, members })
    }

    pub fn leave(&mut self, uid: u16) -> bool {
        self.subscriptions.remove(&uid); // 他看别人的记录
        self.subscriptions.retain(|_, t| *t != uid); // 别人看他的记录
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

    /// 定向发送给指定成员（流事件 / 观看名单通知）
    pub fn send_to(&self, uid: u16, msg: &TcpMessage) {
        if let Some(m) = self.members.get(&uid) {
            let _ = m.tx.send(tcp::encode(msg));
        }
    }

    pub fn validate_token(&self, uid: u16, token: u32) -> bool {
        self.members.get(&uid).map(|m| m.token == token).unwrap_or(false)
    }

    /// 更新成员静音状态；成员不存在返回 false
    pub fn set_muted(&mut self, uid: u16, on: bool) -> bool {
        if let Some(m) = self.members.get_mut(&uid) {
            m.muted = on;
            true
        } else {
            false
        }
    }

    /// 更新成员昵称（SetProfile 成功后同步在线状态）；成员不存在 → false
    pub fn update_nickname(&mut self, uid: u16, nickname: &str) -> bool {
        if let Some(m) = self.members.get_mut(&uid) {
            m.nickname = nickname.to_string();
            true
        } else {
            false
        }
    }

    /// 更新头像标记；成员不存在 → false
    pub fn set_has_avatar(&mut self, uid: u16, has: bool) -> bool {
        if let Some(m) = self.members.get_mut(&uid) {
            m.has_avatar = has;
            true
        } else {
            false
        }
    }

    /// 成员对应的账号 id（AvatarRequest 查库用）
    pub fn account_id_of(&self, uid: u16) -> Option<i64> {
        self.members.get(&uid).map(|m| m.account_id)
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

    /// 设置某路流的开/停；返回新位图（成员不存在 → None）
    pub fn set_stream(&mut self, uid: u16, kind: u8, on: bool) -> Option<u8> {
        let m = self.members.get_mut(&uid)?;
        let bit = 1u8 << kind;
        if on {
            m.streams |= bit;
        } else {
            m.streams &= !bit;
        }
        Some(m.streams)
    }

    /// 订阅（覆盖式：直接改写映射）；目标不存在或订阅自己 → false
    pub fn subscribe(&mut self, sub: u16, target: u16) -> bool {
        if sub == target || !self.members.contains_key(&target) {
            return false;
        }
        self.subscriptions.insert(sub, target);
        true
    }

    /// 解除订阅；返回是否存在被解除的订阅
    pub fn unsubscribe(&mut self, sub: u16) -> bool {
        self.subscriptions.remove(&sub).is_some()
    }

    /// 订阅者 → 目标（离开清理 / 覆盖切换通知用）
    pub fn subscription_of(&self, sub: u16) -> Option<u16> {
        self.subscriptions.get(&sub).copied()
    }

    /// 目标的订阅者 uid 列表（按 uid 升序，内容稳定；"谁在看"名单 R1 用）
    pub fn viewers_of(&self, target: u16) -> Vec<u16> {
        let mut v: Vec<u16> = self
            .subscriptions
            .iter()
            .filter(|(_, t)| **t == target)
            .map(|(s, _)| *s)
            .collect();
        v.sort_unstable();
        v
    }

    /// 目标的订阅者中已注册 UDP 地址的列表（视频/屏幕音频的转发目标）
    pub fn subscribers_with_udp(&self, target: u16) -> Vec<SocketAddr> {
        self.subscriptions
            .iter()
            .filter(|(_, t)| **t == target)
            .filter_map(|(s, _)| self.members.get(s)?.udp_addr)
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
    use echoroom_protocol::messages::{TcpMessage, STREAM_CAMERA, STREAM_SCREEN};
    use std::sync::mpsc;

    #[test]
    fn join_until_capacity_then_reject() {
        let mut room = Room::new();
        let mut rxs = Vec::new();
        for i in 0..ROOM_CAPACITY {
            let (tx, rx) = mpsc::channel();
            let ok = room.join(format!("user{i}"), i as i64 + 1, false, tx);
            assert!(ok.is_ok());
            rxs.push(rx);
        }
        let (tx, _rx) = mpsc::channel();
        assert_eq!(room.join("第7人".into(), 7, false, tx), Err(JoinErr::Full));
    }

    #[test]
    fn join_returns_existing_members_and_leaves_removes() {
        let mut room = Room::new();
        let (tx1, _r1) = mpsc::channel();
        let ok1 = room.join("A".into(), 1, false, tx1).unwrap();
        assert!(ok1.members.is_empty());
        let (tx2, _r2) = mpsc::channel();
        let ok2 = room.join("B".into(), 2, true, tx2).unwrap();
        assert_eq!(ok2.members, vec![(ok1.uid, "A".to_string(), false, 0, false)]);
        assert!(room.leave(ok1.uid));
        assert!(!room.leave(ok1.uid)); // 再删为 false
    }

    #[test]
    fn broadcast_skips_except_and_delivers() {
        let mut room = Room::new();
        let (tx1, rx1) = mpsc::channel();
        let (tx2, rx2) = mpsc::channel();
        let a = room.join("A".into(), 1, false, tx1).unwrap();
        let b = room.join("B".into(), 2, false, tx2).unwrap();
        room.broadcast(Some(a.uid), &TcpMessage::MemberJoin { uid: b.uid, nickname: "B".into(), has_avatar: false });
        assert!(rx1.try_recv().is_err(), "A 不应收到（被排除）");
        let bytes = rx2.try_recv().expect("B 应收到");
        let (msg, _) = echoroom_protocol::tcp::try_decode(&bytes).unwrap().unwrap();
        assert_eq!(msg, TcpMessage::MemberJoin { uid: b.uid, nickname: "B".into(), has_avatar: false });
    }

    #[test]
    fn udp_addr_mapping_and_expiry() {
        use std::time::Duration;
        let mut room = Room::new();
        let (tx, _rx) = mpsc::channel();
        let a = room.join("A".into(), 1, false, tx).unwrap();
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
        let a = room.join("A".into(), 1, false, tx1).unwrap();
        let _b = room.join("B".into(), 2, false, tx2).unwrap();
        room.send_to(a.uid, &TcpMessage::AuthReject { reason: "测试".into() });
        let bytes = rx1.try_recv().expect("A 应收到定向消息");
        let (msg, _) = echoroom_protocol::tcp::try_decode(&bytes).unwrap().unwrap();
        assert_eq!(msg, TcpMessage::AuthReject { reason: "测试".into() });
        assert!(rx2.try_recv().is_err(), "B 不应收到定向消息");
    }

    #[test]
    fn set_muted_updates_member_and_join_reports_it() {
        let mut room = Room::new();
        let (tx1, _r1) = mpsc::channel();
        let a = room.join("A".into(), 1, false, tx1).unwrap();
        assert!(room.set_muted(a.uid, true));
        assert!(!room.set_muted(999, true)); // 不存在的成员
        let (tx2, _r2) = mpsc::channel();
        let b = room.join("B".into(), 2, false, tx2).unwrap();
        // 后加入者应看到 A 处于静音
        assert_eq!(b.members, vec![(a.uid, "A".to_string(), true, 0, false)]);
    }

    #[test]
    fn stream_bitmap_set_and_join_reports_it() {
        let mut room = Room::new();
        let (tx1, _r1) = mpsc::channel();
        let a = room.join("A".into(), 1, false, tx1).unwrap();
        assert_eq!(room.set_stream(a.uid, STREAM_SCREEN, true), Some(1));
        assert_eq!(room.set_stream(a.uid, STREAM_CAMERA, true), Some(3));
        assert_eq!(room.set_stream(a.uid, STREAM_SCREEN, false), Some(2));
        assert_eq!(room.set_stream(999, STREAM_SCREEN, true), None, "成员不存在");
        let (tx2, _r2) = mpsc::channel();
        let b = room.join("B".into(), 2, false, tx2).unwrap();
        assert_eq!(b.members, vec![(a.uid, "A".to_string(), false, 2, false)]);
    }

    #[test]
    fn subscribe_unsubscribe_and_viewers() {
        let mut room = Room::new();
        let (txa, _ra) = mpsc::channel();
        let (txb, _rb) = mpsc::channel();
        let a = room.join("A".into(), 1, false, txa).unwrap();
        let b = room.join("B".into(), 2, false, txb).unwrap();
        let addr_b: std::net::SocketAddr = "127.0.0.1:6001".parse().unwrap();
        room.set_udp(b.uid, addr_b);
        assert!(!room.subscribe(b.uid, b.uid), "不能订阅自己");
        assert!(!room.subscribe(b.uid, 999), "目标不存在");
        assert!(room.subscribe(b.uid, a.uid));
        assert_eq!(room.viewers_of(a.uid), vec![b.uid]);
        assert_eq!(room.subscription_of(b.uid), Some(a.uid));
        assert_eq!(room.subscribers_with_udp(a.uid), vec![addr_b]);
        // 覆盖式：C 订阅 A 后再改订阅 B
        let (txc, _rc) = mpsc::channel();
        let c = room.join("C".into(), 3, false, txc).unwrap();
        assert!(room.subscribe(c.uid, a.uid));
        assert_eq!(room.viewers_of(a.uid), vec![b.uid, c.uid], "两人在看，按 uid 升序");
        assert!(room.subscribe(c.uid, b.uid));
        assert_eq!(room.viewers_of(a.uid), vec![b.uid]);
        assert_eq!(room.viewers_of(b.uid), vec![c.uid]);
        assert!(room.unsubscribe(c.uid));
        assert!(!room.unsubscribe(c.uid), "重复退订为 false");
        assert!(room.viewers_of(b.uid).is_empty());
    }

    #[test]
    fn leave_cleans_subscriptions_both_ways() {
        let mut room = Room::new();
        let (txa, _ra) = mpsc::channel();
        let (txb, _rb) = mpsc::channel();
        let a = room.join("A".into(), 1, false, txa).unwrap();
        let b = room.join("B".into(), 2, false, txb).unwrap();
        room.subscribe(b.uid, a.uid); // B 看 A
        room.subscribe(a.uid, b.uid); // A 看 B（互看）
        room.leave(b.uid);
        assert!(room.viewers_of(a.uid).is_empty(), "B 离开后 A 的观众列表清空");
        assert_eq!(room.subscription_of(a.uid), None, "A 看 B 的订阅记录被清");
        assert!(!room.unsubscribe(a.uid));
    }

    #[test]
    fn update_nickname_avatar_flag_and_account_id() {
        let mut room = Room::new();
        let (tx1, _r1) = mpsc::channel();
        let a = room.join("old".into(), 42, true, tx1).unwrap();
        assert_eq!(room.account_id_of(a.uid), Some(42));
        assert_eq!(room.account_id_of(999), None);
        assert!(room.update_nickname(a.uid, "新名字"));
        assert!(!room.update_nickname(999, "x")); // 不存在的成员
        assert!(room.set_has_avatar(a.uid, false));
        assert!(!room.set_has_avatar(999, true));
        // 后加入者看到的五元组应为更新后的昵称与头像标记
        let (tx2, _r2) = mpsc::channel();
        let b = room.join("B".into(), 2, false, tx2).unwrap();
        assert_eq!(b.members, vec![(a.uid, "新名字".to_string(), false, 0, false)]);
    }
}

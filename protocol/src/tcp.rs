//! TCP 控制消息编解码：`[len u32][type u8][payload]`，长度 = type + payload 字节数。
//! 载荷手写序列化：u16/u32 大端；String = [len u16][utf8]；bool = u8(0/1)。
use crate::messages::TcpMessage;

#[derive(Debug, PartialEq)]
pub enum DecodeError {
    UnknownType(u8),
    BadUtf8,
}

// ---------- 写 ----------

fn put_str(out: &mut Vec<u8>, s: &str) {
    let b = s.as_bytes();
    out.extend_from_slice(&(b.len() as u16).to_be_bytes());
    out.extend_from_slice(b);
}

/// bytes 字段：[len u32][原始字节]（头像等大块二进制；u16 前缀不够 64KB 上限）
fn put_bytes(out: &mut Vec<u8>, b: &[u8]) {
    out.extend_from_slice(&(b.len() as u32).to_be_bytes());
    out.extend_from_slice(b);
}

pub fn encode(msg: &TcpMessage) -> Vec<u8> {
    let mut payload = Vec::new();
    match msg {
        TcpMessage::Login { account, password } => {
            put_str(&mut payload, account);
            put_str(&mut payload, password);
        }
        TcpMessage::LoginOk { uid, udp_token, auth_token, members } => {
            payload.extend_from_slice(&uid.to_be_bytes());
            payload.extend_from_slice(&udp_token.to_be_bytes());
            put_str(&mut payload, auth_token);
            payload.extend_from_slice(&(members.len() as u16).to_be_bytes());
            for (uid, name, muted, streams, has_avatar) in members {
                payload.extend_from_slice(&uid.to_be_bytes());
                put_str(&mut payload, name);
                payload.push(if *muted { 1 } else { 0 });
                payload.push(*streams);
                payload.push(if *has_avatar { 1 } else { 0 });
            }
        }
        TcpMessage::MemberJoin { uid, nickname, has_avatar } => {
            payload.extend_from_slice(&uid.to_be_bytes());
            put_str(&mut payload, nickname);
            payload.push(if *has_avatar { 1 } else { 0 });
        }
        TcpMessage::MemberLeave { uid } => payload.extend_from_slice(&uid.to_be_bytes()),
        TcpMessage::Chat { uid, text } => {
            payload.extend_from_slice(&uid.to_be_bytes());
            put_str(&mut payload, text);
        }
        TcpMessage::Speaking { uid, on } => {
            payload.extend_from_slice(&uid.to_be_bytes());
            payload.push(if *on { 1 } else { 0 });
        }
        TcpMessage::Mute { uid, on } | TcpMessage::Muted { uid, on } => {
            payload.extend_from_slice(&uid.to_be_bytes());
            payload.push(if *on { 1 } else { 0 });
        }
        TcpMessage::StreamState { uid, kind, on } => {
            payload.extend_from_slice(&uid.to_be_bytes());
            payload.push(*kind);
            payload.push(if *on { 1 } else { 0 });
        }
        TcpMessage::Subscribe { uid, target } | TcpMessage::RequestKeyframe { uid, target } => {
            payload.extend_from_slice(&uid.to_be_bytes());
            payload.extend_from_slice(&target.to_be_bytes());
        }
        TcpMessage::Unsubscribe { uid } => payload.extend_from_slice(&uid.to_be_bytes()),
        TcpMessage::Viewers { uids } => {
            payload.extend_from_slice(&(uids.len() as u16).to_be_bytes());
            for uid in uids {
                payload.extend_from_slice(&uid.to_be_bytes());
            }
        }
        TcpMessage::AuthReject { reason } => put_str(&mut payload, reason),
        TcpMessage::Register { account, password, invite } => {
            put_str(&mut payload, account);
            put_str(&mut payload, password);
            put_str(&mut payload, invite);
        }
        TcpMessage::Resume { auth_token } => put_str(&mut payload, auth_token),
        TcpMessage::SetProfile { nickname, avatar } => {
            put_str(&mut payload, nickname);
            match avatar {
                Some(b) => {
                    payload.push(1);
                    put_bytes(&mut payload, b);
                }
                None => payload.push(0),
            }
        }
        TcpMessage::ProfileChanged { uid, nickname } => {
            payload.extend_from_slice(&uid.to_be_bytes());
            put_str(&mut payload, nickname);
        }
        TcpMessage::AvatarRequest { uid } => payload.extend_from_slice(&uid.to_be_bytes()),
        TcpMessage::AvatarData { uid, data } => {
            payload.extend_from_slice(&uid.to_be_bytes());
            put_bytes(&mut payload, data);
        }
    }
    let mut out = Vec::with_capacity(5 + payload.len());
    out.extend_from_slice(&((payload.len() + 1) as u32).to_be_bytes());
    out.push(msg.type_id());
    out.extend_from_slice(&payload);
    out
}

// ---------- 读 ----------

enum FieldErr {
    NotEnough,
    BadUtf8,
}

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn need(&self, n: usize) -> Result<(), FieldErr> {
        if self.pos + n <= self.buf.len() {
            Ok(())
        } else {
            Err(FieldErr::NotEnough)
        }
    }
    fn u8(&mut self) -> Result<u8, FieldErr> {
        self.need(1)?;
        let v = self.buf[self.pos];
        self.pos += 1;
        Ok(v)
    }
    fn u16(&mut self) -> Result<u16, FieldErr> {
        self.need(2)?;
        let v = u16::from_be_bytes([self.buf[self.pos], self.buf[self.pos + 1]]);
        self.pos += 2;
        Ok(v)
    }
    fn u32(&mut self) -> Result<u32, FieldErr> {
        self.need(4)?;
        let v = u32::from_be_bytes(self.buf[self.pos..self.pos + 4].try_into().unwrap());
        self.pos += 4;
        Ok(v)
    }
    fn string(&mut self) -> Result<String, FieldErr> {
        let len = self.u16()? as usize;
        self.need(len)?;
        let s = std::str::from_utf8(&self.buf[self.pos..self.pos + len])
            .map_err(|_| FieldErr::BadUtf8)?
            .to_string();
        self.pos += len;
        Ok(s)
    }
    fn bytes(&mut self) -> Result<Vec<u8>, FieldErr> {
        let len = self.u32()? as usize;
        self.need(len)?;
        let b = self.buf[self.pos..self.pos + len].to_vec();
        self.pos += len;
        Ok(b)
    }
}

/// 尝试从缓冲区头部解码一条消息。
/// `Ok(None)` = 数据不足（等待更多字节）；`Ok(Some((msg, n)))` = 成功解码并消费 n 字节（支持粘包循环）；`Err` = 协议错误。
pub fn try_decode(buf: &[u8]) -> Result<Option<(TcpMessage, usize)>, DecodeError> {
    if buf.len() < 5 {
        return Ok(None);
    }
    let len = u32::from_be_bytes(buf[0..4].try_into().unwrap()) as usize;
    // 上限 256KB：容纳头像单帧（≤64KB BLOB + 头部）与成员列表余量
    if len < 1 || len > 256 * 1024 {
        return Err(DecodeError::UnknownType(0)); // 长度异常：视为协议错误
    }
    if buf.len() < 4 + len {
        return Ok(None);
    }
    let type_id = buf[4];
    let body = &buf[5..4 + len];
    let mut r = Reader { buf: body, pos: 0 };

    macro_rules! field {
        ($e:expr) => {
            match $e {
                Ok(v) => v,
                Err(FieldErr::NotEnough) => return Ok(None),
                Err(FieldErr::BadUtf8) => return Err(DecodeError::BadUtf8),
            }
        };
    }

    let msg = match type_id {
        1 => {
            let account = field!(r.string());
            TcpMessage::Login { account, password: field!(r.string()) }
        }
        2 => {
            let uid = field!(r.u16());
            let udp_token = field!(r.u32());
            let auth_token = field!(r.string());
            let count = field!(r.u16()) as usize;
            let mut members = Vec::with_capacity(count.min(64));
            for _ in 0..count {
                let m_uid = field!(r.u16());
                let name = field!(r.string());
                let muted = field!(r.u8()) != 0;
                let streams = field!(r.u8());
                let has_avatar = field!(r.u8()) != 0;
                members.push((m_uid, name, muted, streams, has_avatar));
            }
            TcpMessage::LoginOk { uid, udp_token, auth_token, members }
        }
        3 => {
            let uid = field!(r.u16());
            let nickname = field!(r.string());
            TcpMessage::MemberJoin { uid, nickname, has_avatar: field!(r.u8()) != 0 }
        }
        4 => TcpMessage::MemberLeave { uid: field!(r.u16()) },
        5 => {
            let uid = field!(r.u16());
            TcpMessage::Chat { uid, text: field!(r.string()) }
        }
        6 => {
            let uid = field!(r.u16());
            TcpMessage::Speaking { uid, on: field!(r.u8()) != 0 }
        }
        7 => TcpMessage::AuthReject { reason: field!(r.string()) },
        8 => {
            let uid = field!(r.u16());
            TcpMessage::Mute { uid, on: field!(r.u8()) != 0 }
        }
        9 => {
            let uid = field!(r.u16());
            TcpMessage::Muted { uid, on: field!(r.u8()) != 0 }
        }
        10 => {
            let uid = field!(r.u16());
            let kind = field!(r.u8());
            TcpMessage::StreamState { uid, kind, on: field!(r.u8()) != 0 }
        }
        11 => {
            let uid = field!(r.u16());
            TcpMessage::Subscribe { uid, target: field!(r.u16()) }
        }
        12 => TcpMessage::Unsubscribe { uid: field!(r.u16()) },
        13 => {
            let count = field!(r.u16()) as usize;
            let mut uids = Vec::with_capacity(count.min(64));
            for _ in 0..count {
                uids.push(field!(r.u16()));
            }
            TcpMessage::Viewers { uids }
        }
        14 => {
            let uid = field!(r.u16());
            TcpMessage::RequestKeyframe { uid, target: field!(r.u16()) }
        }
        15 => {
            let account = field!(r.string());
            let password = field!(r.string());
            TcpMessage::Register { account, password, invite: field!(r.string()) }
        }
        16 => TcpMessage::Resume { auth_token: field!(r.string()) },
        17 => {
            let nickname = field!(r.string());
            let has_avatar = field!(r.u8()) != 0;
            let avatar = if has_avatar { Some(field!(r.bytes())) } else { None };
            TcpMessage::SetProfile { nickname, avatar }
        }
        18 => {
            let uid = field!(r.u16());
            TcpMessage::ProfileChanged { uid, nickname: field!(r.string()) }
        }
        19 => TcpMessage::AvatarRequest { uid: field!(r.u16()) },
        20 => {
            let uid = field!(r.u16());
            TcpMessage::AvatarData { uid, data: field!(r.bytes()) }
        }
        other => return Err(DecodeError::UnknownType(other)),
    };
    Ok(Some((msg, 4 + len)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::TcpMessage;

    #[test]
    fn roundtrip_all_variants() {
        let samples = vec![
            TcpMessage::Login { account: "alice".into(), password: "secret123".into() },
            TcpMessage::LoginOk {
                uid: 3,
                udp_token: 0xDEAD_BEEF,
                auth_token: "0123456789abcdef0123456789abcdef".into(),
                members: vec![(1, "小K".into(), false, 0, true), (2, "你".into(), true, 0b11, false)],
            },
            TcpMessage::MemberJoin { uid: 5, nickname: "新来的".into(), has_avatar: true },
            TcpMessage::MemberLeave { uid: 2 },
            TcpMessage::Chat { uid: 1, text: "晚上开黑吗".into() },
            TcpMessage::Speaking { uid: 4, on: true },
            TcpMessage::Mute { uid: 0, on: true },
            TcpMessage::Muted { uid: 4, on: true },
            TcpMessage::StreamState { uid: 0, kind: 1, on: true },
            TcpMessage::StreamState { uid: 4, kind: 0, on: false },
            TcpMessage::Subscribe { uid: 0, target: 4 },
            TcpMessage::Unsubscribe { uid: 0 },
            TcpMessage::Viewers { uids: vec![1, 2, 3] },
            TcpMessage::Viewers { uids: vec![] },
            TcpMessage::RequestKeyframe { uid: 0, target: 4 },
            TcpMessage::AuthReject { reason: "账号或密码错误".into() },
            TcpMessage::Register { account: "bob".into(), password: "pw123456".into(), invite: "echo-2026".into() },
            TcpMessage::Resume { auth_token: "deadbeef".into() },
            TcpMessage::SetProfile { nickname: "阿信".into(), avatar: None },
            TcpMessage::SetProfile { nickname: "阿信".into(), avatar: Some(vec![0xFF, 0xD8, 0xFF, 0xE0]) },
            TcpMessage::ProfileChanged { uid: 3, nickname: "阿信".into() },
            TcpMessage::AvatarRequest { uid: 3 },
            TcpMessage::AvatarData { uid: 3, data: vec![] },
            TcpMessage::AvatarData { uid: 3, data: vec![1, 2, 3, 4] },
        ];
        for msg in samples {
            let bytes = encode(&msg);
            let (decoded, n) = try_decode(&bytes).unwrap().unwrap();
            assert_eq!(decoded, msg);
            assert_eq!(n, bytes.len());
        }
    }

    #[test]
    fn avatar_data_roundtrip_64kb() {
        // 头像上限 64KB：单帧必须可编码可解码（长度上限 256KB 之内）
        let msg = TcpMessage::AvatarData { uid: 7, data: vec![0xAB; 64 * 1024] };
        let bytes = encode(&msg);
        let (decoded, n) = try_decode(&bytes).unwrap().unwrap();
        assert_eq!(decoded, msg);
        assert_eq!(n, bytes.len());
    }

    #[test]
    fn partial_data_returns_none() {
        let bytes = encode(&TcpMessage::Login { account: "abc".into(), password: "x".into() });
        for cut in 0..bytes.len() {
            assert!(try_decode(&bytes[..cut]).unwrap().is_none(), "cut={cut} 应等待更多数据");
        }
    }

    #[test]
    fn two_messages_in_one_buffer() {
        let mut buf = encode(&TcpMessage::MemberLeave { uid: 1 });
        buf.extend(encode(&TcpMessage::Speaking { uid: 2, on: false }));
        let (m1, n1) = try_decode(&buf).unwrap().unwrap();
        assert_eq!(m1, TcpMessage::MemberLeave { uid: 1 });
        let (m2, n2) = try_decode(&buf[n1..]).unwrap().unwrap();
        assert_eq!(m2, TcpMessage::Speaking { uid: 2, on: false });
        assert_eq!(n1 + n2, buf.len());
    }

    #[test]
    fn unknown_type_is_error() {
        // 长度 2、类型 99、载荷 1 字节
        let buf = [0u8, 0, 0, 2, 99, 0];
        assert!(matches!(try_decode(&buf), Err(DecodeError::UnknownType(99))));
    }

    #[test]
    fn bad_utf8_is_error() {
        // Login 消息：载荷 = [len u16=2][0xFF 0xFE]（非法 UTF-8）
        let mut payload = vec![0u8, 2, 0xFF, 0xFE];
        let mut buf = ((payload.len() + 1) as u32).to_be_bytes().to_vec();
        buf.push(1); // type = Login
        buf.append(&mut payload);
        assert!(matches!(try_decode(&buf), Err(DecodeError::BadUtf8)));
    }
}

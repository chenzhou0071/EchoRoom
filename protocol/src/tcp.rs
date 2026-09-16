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

pub fn encode(msg: &TcpMessage) -> Vec<u8> {
    let mut payload = Vec::new();
    match msg {
        TcpMessage::Login { nickname } => put_str(&mut payload, nickname),
        TcpMessage::LoginOk { uid, token, members } => {
            payload.extend_from_slice(&uid.to_be_bytes());
            payload.extend_from_slice(&token.to_be_bytes());
            payload.extend_from_slice(&(members.len() as u16).to_be_bytes());
            for (uid, name) in members {
                payload.extend_from_slice(&uid.to_be_bytes());
                put_str(&mut payload, name);
            }
        }
        TcpMessage::MemberJoin { uid, nickname } => {
            payload.extend_from_slice(&uid.to_be_bytes());
            put_str(&mut payload, nickname);
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
        TcpMessage::LoginReject { reason } => put_str(&mut payload, reason),
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
}

/// 尝试从缓冲区头部解码一条消息。
/// `Ok(None)` = 数据不足（等待更多字节）；`Ok(Some((msg, n)))` = 成功解码并消费 n 字节（支持粘包循环）；`Err` = 协议错误。
pub fn try_decode(buf: &[u8]) -> Result<Option<(TcpMessage, usize)>, DecodeError> {
    if buf.len() < 5 {
        return Ok(None);
    }
    let len = u32::from_be_bytes(buf[0..4].try_into().unwrap()) as usize;
    if len < 1 || len > 64 * 1024 {
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
        1 => TcpMessage::Login { nickname: field!(r.string()) },
        2 => {
            let uid = field!(r.u16());
            let token = field!(r.u32());
            let count = field!(r.u16()) as usize;
            let mut members = Vec::with_capacity(count.min(64));
            for _ in 0..count {
                let m_uid = field!(r.u16());
                let name = field!(r.string());
                members.push((m_uid, name));
            }
            TcpMessage::LoginOk { uid, token, members }
        }
        3 => {
            let uid = field!(r.u16());
            TcpMessage::MemberJoin { uid, nickname: field!(r.string()) }
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
        7 => TcpMessage::LoginReject { reason: field!(r.string()) },
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
            TcpMessage::Login { nickname: "阿信".into() },
            TcpMessage::LoginOk { uid: 3, token: 0xDEAD_BEEF, members: vec![(1, "小K".into()), (2, "你".into())] },
            TcpMessage::MemberJoin { uid: 5, nickname: "新来的".into() },
            TcpMessage::MemberLeave { uid: 2 },
            TcpMessage::Chat { uid: 1, text: "晚上开黑吗".into() },
            TcpMessage::Speaking { uid: 4, on: true },
            TcpMessage::LoginReject { reason: "房间已满（6人）".into() },
        ];
        for msg in samples {
            let bytes = encode(&msg);
            let (decoded, n) = try_decode(&bytes).unwrap().unwrap();
            assert_eq!(decoded, msg);
            assert_eq!(n, bytes.len());
        }
    }

    #[test]
    fn partial_data_returns_none() {
        let bytes = encode(&TcpMessage::Login { nickname: "abc".into() });
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

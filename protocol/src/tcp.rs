//! TCP 控制消息编解码：`[len u32][type u8][payload]`，长度 = type + payload 字节数。
//! 载荷手写序列化：u16/u32 大端；String = [len u16][utf8]；bool = u8(0/1)。
use crate::messages::TcpMessage;

#[derive(Debug, PartialEq)]
pub enum DecodeError {
    UnknownType(u8),
    BadUtf8,
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

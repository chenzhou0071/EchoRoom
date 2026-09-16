//! UDP 包编解码。头 10 字节：magic(0xEC45) / ver(1) / type / uid u16 / seq u32，均大端。
use crate::messages::UdpPacket;

pub const MAGIC: u16 = 0xEC45;
pub const VERSION: u8 = 1;
pub const HEADER_LEN: usize = 10;
/// 语音包上限（防御异常大包）
pub const MAX_PACKET: usize = 1200;

#[derive(Debug, PartialEq)]
pub enum DecodeError {
    TooShort,
    BadMagic,
    BadVersion(u8),
    UnknownType(u8),
}

pub fn encode(uid: u16, seq: u32, pkt: &UdpPacket) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN + 8 + 80);
    out.extend_from_slice(&MAGIC.to_be_bytes());
    out.push(VERSION);
    out.push(pkt.type_id());
    out.extend_from_slice(&uid.to_be_bytes());
    out.extend_from_slice(&seq.to_be_bytes());
    match pkt {
        UdpPacket::Register { token } => out.extend_from_slice(&token.to_be_bytes()),
        UdpPacket::Voice { opus } => out.extend_from_slice(opus),
        UdpPacket::RegisterAck | UdpPacket::RegisterReject | UdpPacket::Heartbeat => {}
    }
    out
}

pub fn decode(buf: &[u8]) -> Result<(u16, u32, UdpPacket), DecodeError> {
    if buf.len() < HEADER_LEN {
        return Err(DecodeError::TooShort);
    }
    if buf.len() > MAX_PACKET {
        return Err(DecodeError::TooShort); // 异常大包按不可用丢弃
    }
    let magic = u16::from_be_bytes([buf[0], buf[1]]);
    if magic != MAGIC {
        return Err(DecodeError::BadMagic);
    }
    let ver = buf[2];
    if ver != VERSION {
        return Err(DecodeError::BadVersion(ver));
    }
    let uid = u16::from_be_bytes([buf[4], buf[5]]);
    let seq = u32::from_be_bytes([buf[6], buf[7], buf[8], buf[9]]);
    let payload = &buf[HEADER_LEN..];
    let pkt = match buf[3] {
        1 => {
            if payload.len() < 4 {
                return Err(DecodeError::TooShort);
            }
            UdpPacket::Register { token: u32::from_be_bytes(payload[..4].try_into().unwrap()) }
        }
        2 => UdpPacket::RegisterAck,
        3 => UdpPacket::RegisterReject,
        4 => UdpPacket::Voice { opus: payload.to_vec() },
        5 => UdpPacket::Heartbeat,
        other => return Err(DecodeError::UnknownType(other)),
    };
    Ok((uid, seq, pkt))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::UdpPacket;

    #[test]
    fn roundtrip_all_variants() {
        let samples = vec![
            UdpPacket::Register { token: 0x1234_5678 },
            UdpPacket::RegisterAck,
            UdpPacket::RegisterReject,
            UdpPacket::Voice { opus: vec![0xAA; 60] },
            UdpPacket::Heartbeat,
        ];
        for (i, pkt) in samples.into_iter().enumerate() {
            let bytes = encode(7, 42 + i as u32, &pkt);
            let (uid, seq, decoded) = decode(&bytes).unwrap();
            assert_eq!(uid, 7);
            assert_eq!(seq, 42 + i as u32);
            assert_eq!(decoded, pkt);
        }
    }

    #[test]
    fn bad_magic_rejected() {
        let mut bytes = encode(1, 0, &UdpPacket::Heartbeat);
        bytes[0] = 0x00;
        assert!(matches!(decode(&bytes), Err(DecodeError::BadMagic)));
    }

    #[test]
    fn bad_version_rejected() {
        let mut bytes = encode(1, 0, &UdpPacket::Heartbeat);
        bytes[2] = 99;
        assert!(matches!(decode(&bytes), Err(DecodeError::BadVersion(99))));
    }

    #[test]
    fn truncated_rejected() {
        let bytes = encode(1, 0, &UdpPacket::Heartbeat);
        assert!(matches!(decode(&bytes[..6]), Err(DecodeError::TooShort)));
    }

    #[test]
    fn voice_payload_preserved() {
        let opus: Vec<u8> = (0..80).map(|i| (i * 3) as u8).collect();
        let bytes = encode(65535, u32::MAX, &UdpPacket::Voice { opus: opus.clone() });
        let (uid, seq, pkt) = decode(&bytes).unwrap();
        assert_eq!((uid, seq), (65535, u32::MAX));
        assert_eq!(pkt, UdpPacket::Voice { opus });
    }
}

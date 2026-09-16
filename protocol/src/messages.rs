//! TCP 控制消息定义。类型号固定，客户端与服务端共用。

#[derive(Debug, Clone, PartialEq)]
pub enum TcpMessage {
    /// C→S：进入房间
    Login { nickname: String },
    /// S→C：登录成功（uid、token、已有成员）
    LoginOk { uid: u16, token: u32, members: Vec<(u16, String)> },
    /// S→C：有成员加入
    MemberJoin { uid: u16, nickname: String },
    /// S→C：有成员离开
    MemberLeave { uid: u16 },
    /// C→S（uid 填 0）公屏消息；S→C（uid 为发送者）广播
    Chat { uid: u16, text: String },
    /// C→S（uid 填 0）说话状态；S→C 广播
    Speaking { uid: u16, on: bool },
    /// S→C：房间满等拒绝原因
    LoginReject { reason: String },
}

impl TcpMessage {
    pub fn type_id(&self) -> u8 {
        match self {
            TcpMessage::Login { .. } => 1,
            TcpMessage::LoginOk { .. } => 2,
            TcpMessage::MemberJoin { .. } => 3,
            TcpMessage::MemberLeave { .. } => 4,
            TcpMessage::Chat { .. } => 5,
            TcpMessage::Speaking { .. } => 6,
            TcpMessage::LoginReject { .. } => 7,
        }
    }
}

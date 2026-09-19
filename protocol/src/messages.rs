//! TCP 控制消息定义。类型号固定，客户端与服务端共用。

/// 流类型常量（u8 位图 bit0 = 屏幕 / bit1 = 摄像头）
pub const STREAM_SCREEN: u8 = 0;
pub const STREAM_CAMERA: u8 = 1;

#[derive(Debug, Clone, PartialEq)]
pub enum TcpMessage {
    /// C→S：进入房间
    Login { nickname: String },
    /// S→C：登录成功（uid、token、已有成员；成员 = (uid, 昵称, 是否静音, 流位图)）
    LoginOk { uid: u16, token: u32, members: Vec<(u16, String, bool, u8)> },
    /// S→C：有成员加入
    MemberJoin { uid: u16, nickname: String },
    /// S→C：有成员离开
    MemberLeave { uid: u16 },
    /// C→S（uid 填 0）公屏消息；S→C（uid 为发送者）广播
    Chat { uid: u16, text: String },
    /// C→S（uid 填 0）说话状态；S→C 广播
    Speaking { uid: u16, on: bool },
    /// C→S（uid 填 0）静音状态；S→C 广播（Muted）
    Mute { uid: u16, on: bool },
    /// S→C 广播静音状态（含发送者本人）
    Muted { uid: u16, on: bool },
    /// 双向：C→S（uid 填 0）上报本端开/停某路流；S→C 广播（uid 为流主）
    StreamState { uid: u16, kind: u8, on: bool },
    /// C→S（uid 填 0）：订阅某人的视频流（覆盖式）
    Subscribe { uid: u16, target: u16 },
    /// C→S（uid 填 0）：取消订阅
    Unsubscribe { uid: u16 },
    /// S→C 定向（发给流主与该流全部订阅者）：当前观看者 uid 名单（人数 = len）
    Viewers { uids: Vec<u16> },
    /// 双向：C→S（uid 填 0）请求目标发关键帧；S→C 转发（uid 为请求者）
    RequestKeyframe { uid: u16, target: u16 },
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
            TcpMessage::Mute { .. } => 8,
            TcpMessage::Muted { .. } => 9,
            TcpMessage::StreamState { .. } => 10,
            TcpMessage::Subscribe { .. } => 11,
            TcpMessage::Unsubscribe { .. } => 12,
            TcpMessage::Viewers { .. } => 13,
            TcpMessage::RequestKeyframe { .. } => 14,
        }
    }
}

/// UDP 语音通道包（头 10 字节：magic u16 / ver u8 / type u8 / uid u16 / seq u32）
#[derive(Debug, Clone, PartialEq)]
pub enum UdpPacket {
    /// C→S：绑定地址映射（token 校验）
    Register { token: u32 },
    /// S→C：绑定成功
    RegisterAck,
    /// S→C：绑定失败（token 不匹配）
    RegisterReject,
    /// 语音帧（C→S 发送自己；S→C 转发时 uid 为来源者）
    Voice { opus: Vec<u8> },
    /// 心跳（seq 用于响应观测）
    Heartbeat,
}

impl UdpPacket {
    pub fn type_id(&self) -> u8 {
        match self {
            UdpPacket::Register { .. } => 1,
            UdpPacket::RegisterAck => 2,
            UdpPacket::RegisterReject => 3,
            UdpPacket::Voice { .. } => 4,
            UdpPacket::Heartbeat => 5,
        }
    }
}

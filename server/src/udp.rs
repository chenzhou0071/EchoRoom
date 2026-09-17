//! UDP 语音转发（T6 实现）。
use std::sync::{Arc, Mutex};
use crate::room::Room;

pub fn spawn_udp_loop(_port: u16, _room: Arc<Mutex<Room>>) {}
pub fn spawn_cleanup_loop(_room: Arc<Mutex<Room>>) {}

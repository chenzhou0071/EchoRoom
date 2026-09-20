//! 客户端配置：昵称、服务器地址与音量设置，持久化到 %APPDATA%\com.echoroom.dev\config.json。
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const APP_IDENTIFIER: &str = "com.echoroom.dev";

fn default_gain() -> f32 {
    1.0
}

fn default_true() -> bool {
    true
}

fn default_share_quality() -> String {
    "720p30".into()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    pub nickname: String,
    pub server_addr: String,
    /// 自己的采集增益（0.0–2.0）
    #[serde(default = "default_gain")]
    pub self_gain: f32,
    /// 自己的静音状态
    #[serde(default)]
    pub muted: bool,
    /// 对他人的播放增益（按昵称）
    #[serde(default)]
    pub peer_gains: std::collections::HashMap<String, f32>,
    /// 观看端：投屏（屏幕）声音的播放增益（0.0–2.0）
    #[serde(default = "default_gain")]
    pub screen_gain: f32,
    /// 投屏画质档位（"720p30" / "1080p15" / "1080p30"）
    #[serde(default = "default_share_quality")]
    pub share_quality: String,
    /// 投屏时是否共享系统声音（Win11+）
    #[serde(default = "default_true")]
    pub share_audio: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            nickname: String::new(),
            server_addr: "127.0.0.1:9000".into(),
            self_gain: 1.0,
            muted: false,
            peer_gains: std::collections::HashMap::new(),
            screen_gain: 1.0,
            share_quality: "720p30".into(),
            share_audio: true,
        }
    }
}

pub fn default_config_path() -> PathBuf {
    let base = std::env::var("APPDATA").unwrap_or_else(|_| ".".into());
    PathBuf::from(base).join(APP_IDENTIFIER).join("config.json")
}

impl Config {
    pub fn load(path: &std::path::Path) -> Config {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self).unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_with_volume_fields() {
        let path = std::env::temp_dir().join("echoroom_cfg_test_roundtrip.json");
        let mut cfg = Config::default();
        cfg.self_gain = 1.5;
        cfg.muted = true;
        cfg.peer_gains.insert("小林".into(), 0.5);
        cfg.screen_gain = 0.8;
        cfg.share_quality = "1080p15".into();
        cfg.share_audio = false;
        cfg.save(&path).unwrap();
        let loaded = Config::load(&path);
        assert_eq!(loaded, cfg);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn old_config_without_volume_fields_loads_defaults() {
        let path = std::env::temp_dir().join("echoroom_cfg_test_old.json");
        std::fs::write(&path, r#"{"nickname":"老用户","server_addr":"127.0.0.1:9000"}"#).unwrap();
        let loaded = Config::load(&path);
        assert_eq!(loaded.nickname, "老用户");
        assert_eq!(loaded.self_gain, 1.0);
        assert!(!loaded.muted);
        assert!(loaded.peer_gains.is_empty());
        assert_eq!(loaded.screen_gain, 1.0);
        assert_eq!(loaded.share_quality, "720p30");
        assert!(loaded.share_audio);
        let _ = std::fs::remove_file(&path);
    }
}

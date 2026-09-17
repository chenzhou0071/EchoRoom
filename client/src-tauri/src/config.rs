//! 客户端配置：昵称与服务器地址，持久化到 %APPDATA%\com.echoroom.dev\config.json。
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const APP_IDENTIFIER: &str = "com.echoroom.dev";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    pub nickname: String,
    pub server_addr: String,
}

impl Default for Config {
    fn default() -> Self {
        Config { nickname: String::new(), server_addr: "127.0.0.1:9000".into() }
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

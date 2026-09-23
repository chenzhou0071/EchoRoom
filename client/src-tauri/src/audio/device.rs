//! 音频设备枚举与查找：wasapi 设备集合（输入/输出）→ 设备信息；按 id 查找指定设备。
//! COM 必须在同一线程初始化与使用：本模块函数在音频线程或命令线程内调用。
use anyhow::{Context, Result};
use wasapi::{Device, DeviceCollection, Direction};

/// wasapi 0.15 的错误类型为 `Box<dyn Error>`（非 Send+Sync，无法直接进 anyhow），统一转字符串。
fn err2any(e: Box<dyn std::error::Error>) -> anyhow::Error {
    anyhow::anyhow!(e.to_string())
}

/// 设备信息（bridge → UI 序列化用；纯数据，可跨线程）
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DeviceInfo {
    /// 端点 id（持久化到 config；wasapi Device::get_id）
    pub id: String,
    /// 友好名（UI 下拉显示）
    pub name: String,
    /// 是否为当前系统默认设备
    pub is_default: bool,
}

/// 枚举某方向的活跃设备（DEVICE_STATE_ACTIVE；含 is_default 标记）
pub fn list(direction: &Direction) -> Result<Vec<DeviceInfo>> {
    // COM 可能已按其他模式初始化（如 Tauri 主线程 STA）：重复初始化 MTA 会失败，忽略即可——
    // COM 此时已可用，wasapi 的 COM 包装在 STA 下同样工作；真正不可用时由下方枚举调用报错。
    let _ = wasapi::initialize_mta();
    let default_id = wasapi::get_default_device(direction)
        .ok()
        .and_then(|d| d.get_id().ok());
    let col = DeviceCollection::new(direction).map_err(err2any).context("设备集合")?;
    let n = col.get_nbr_devices().map_err(err2any).context("设备数量")?;
    let mut out = Vec::with_capacity(n as usize);
    for i in 0..n {
        let Ok(dev) = col.get_device_at_index(i) else { continue };
        let id = dev.get_id().map_err(err2any)?;
        let name = dev.get_friendlyname().map_err(err2any)?;
        out.push(DeviceInfo { is_default: default_id.as_deref() == Some(id.as_str()), id, name });
    }
    Ok(out)
}

/// 按 id 查找设备；找不到返回 None（由调用方决定回退系统默认）
pub fn find(direction: &Direction, id: &str) -> Result<Option<Device>> {
    // 同 list()：COM 已按其他模式初始化时（STA 主线程）忽略 MTA 重复初始化
    let _ = wasapi::initialize_mta();
    let col = DeviceCollection::new(direction).map_err(err2any).context("设备集合")?;
    let n = col.get_nbr_devices().map_err(err2any).context("设备数量")?;
    for i in 0..n {
        let Ok(dev) = col.get_device_at_index(i) else { continue };
        if dev.get_id().map_err(err2any)? == id {
            return Ok(Some(dev));
        }
    }
    Ok(None)
}

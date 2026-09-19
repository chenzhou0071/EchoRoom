//! 隐藏 WebView2 的屏幕共享提示条（屏幕底部"xxx 正在共享你的屏幕"浮条 + 图标）。
//!
//! 该提示条是 Edge WebView2 内核自带的隐私 UI：一个独立置顶窗口（类名
//! Chrome_WidgetWin_1，WS_EX_TOPMOST | WS_EX_NOREDIRECTIONBITMAP），属于本应用的
//! msedgewebview2.exe 子进程。官方没有提供关闭它的 API/启动参数（ScreenCaptureStarting
//! 只能整体取消捕获，WebView2Feedback #2442 至今未实现），因此这里在投屏期间轮询
//! 定位该窗口并 SW_HIDE——等价于自动点了它自带的"隐藏"按钮，共享本身不受影响。
//!
//! 生命周期：`set_share_active(true)` 启动、`set_share_active(false)` 停止（Drop）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use windows::core::BOOL;
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, TRUE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
    TH32CS_SNAPPROCESS,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindowLongPtrW, GetWindowThreadProcessId, IsWindowVisible,
    ShowWindow, GWL_EXSTYLE, SW_HIDE, WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOPMOST,
};

/// 轮询间隔
const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// 提示条隐藏器：Drop 即停（轮询线程随旗标退出）。
pub struct Hider {
    stop: Arc<AtomicBool>,
}

impl Hider {
    /// 启动轮询线程（立即先扫描一次，随后每 500ms 一次）。
    pub fn spawn() -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        std::thread::spawn(move || {
            while !flag.load(Ordering::Relaxed) {
                match hide_bars() {
                    Ok(0) => {}
                    Ok(_) => println!("[indicator] 已隐藏共享提示条"),
                    Err(e) => eprintln!("[indicator] 扫描失败: {e}"),
                }
                std::thread::sleep(POLL_INTERVAL);
            }
        });
        Self { stop }
    }
}

impl Drop for Hider {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// 查找并隐藏本应用 WebView2 的共享提示条；返回本次隐藏的窗口数。
fn hide_bars() -> windows::core::Result<u32> {
    let pids = webview_child_pids()?;
    if pids.is_empty() {
        return Ok(0);
    }
    let mut ctx = Ctx {
        pids: &pids,
        hidden: 0,
    };
    unsafe {
        EnumWindows(Some(enum_hide_cb), LPARAM(&mut ctx as *mut Ctx as isize))?;
    }
    Ok(ctx.hidden)
}

/// 回调上下文：本应用的 WebView2 进程 PID 集合 + 本次已隐藏计数
struct Ctx<'a> {
    pids: &'a [u32],
    hidden: u32,
}

/// 顶层窗口回调：命中提示条签名则 SW_HIDE。
unsafe extern "system" fn enum_hide_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = &mut *(lparam.0 as *mut Ctx);
    if is_indicator_bar(hwnd, ctx.pids) {
        let _ = ShowWindow(hwnd, SW_HIDE);
        ctx.hidden += 1;
    }
    TRUE
}

/// 提示条签名：可见 + 属于本应用的 msedgewebview2 进程 + 类名 Chrome_WidgetWin_1
/// + 置顶 + 无重定向位图（实测本机 WebView2 153，2026-09 验证）。
fn is_indicator_bar(hwnd: HWND, pids: &[u32]) -> bool {
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() {
            return false;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if !pids.contains(&pid) {
            return false;
        }
        let mut buf = [0u16; 64];
        let n = GetClassNameW(hwnd, &mut buf) as usize;
        if String::from_utf16_lossy(&buf[..n]) != "Chrome_WidgetWin_1" {
            return false;
        }
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        ex & WS_EX_TOPMOST.0 != 0 && ex & WS_EX_NOREDIRECTIONBITMAP.0 != 0
    }
}

/// 本应用（宿主进程）直接子进程中的全部 msedgewebview2.exe 进程 PID。
fn webview_child_pids() -> windows::core::Result<Vec<u32>> {
    let my_pid = std::process::id();
    let mut pids = Vec::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)?;
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if Process32FirstW(snap, &mut entry).is_ok() {
            loop {
                let len = entry
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(entry.szExeFile.len());
                let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
                if name.eq_ignore_ascii_case("msedgewebview2.exe")
                    && entry.th32ParentProcessID == my_pid
                {
                    pids.push(entry.th32ProcessID);
                }
                if Process32NextW(snap, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
    }
    Ok(pids)
}

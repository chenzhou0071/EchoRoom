//! 隐藏 WebView2 的屏幕共享提示条（屏幕底部"xxx 正在共享你的屏幕"浮条 + 图标）。
//!
//! 该提示条是 Edge WebView2 内核自带的隐私 UI：一个独立置顶窗口（类名
//! Chrome_WidgetWin_1，WS_EX_TOPMOST | WS_EX_NOREDIRECTIONBITMAP），属于本应用的
//! msedgewebview2.exe 子进程。官方没有提供关闭它的 API/启动参数（ScreenCaptureStarting
//! 只能整体取消捕获，WebView2Feedback #2442 至今未实现），因此这里在投屏期间轮询
//! 定位该窗口并 SW_HIDE——等价于自动点了它自带的"隐藏"按钮，共享本身不受影响。
//!
//! 生命周期：`set_share_active(true)` 启动、`set_share_active(false)` 停止（Drop）。
//!
//! R10（2026-09-20）加固 + 会话日志：用户实测"房间只有自己时正常隐藏、房间有人时失效"，
//! 但本模块的匹配逻辑与房间状态无任何关联 → 需现场数据定位两种场景的实际窗口差异。
//! - 扫描范围：从"直接子进程"扩为**全部后代** msedgewebview2.exe（条窗归属进程可能随版本变化）；
//! - 匹配放宽：`Chrome_WidgetWin*` +（置顶 或 无重定向位）任一命中；标题含"正在共享"兜底；
//! - SW_HIDE 后复查可见性，未生效会如实记录；
//! - 每次投屏会话把窗口枚举明细写入 `%APPDATA%\com.echoroom.dev\indicator.log`。

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use windows::core::BOOL;
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, RECT, TRUE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
    TH32CS_SNAPPROCESS,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindowLongPtrW, GetWindowRect, GetWindowTextW,
    GetWindowThreadProcessId, IsWindowVisible, ShowWindow, GWL_EXSTYLE, SW_HIDE,
    WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOPMOST,
};

/// 轮询间隔
const POLL_INTERVAL: Duration = Duration::from_millis(500);
/// 单次投屏会话的日志行数上限（防无限增长）
const LOG_MAX_LINES: usize = 400;
/// 日志文件超过该大小时下次会话重建（字节）
const LOG_MAX_BYTES: u64 = 256 * 1024;

/// 提示条隐藏器：Drop 即停（轮询线程随旗标退出）。
pub struct Hider {
    stop: Arc<AtomicBool>,
}

impl Hider {
    /// 启动轮询线程（立即先扫描一次，随后每 500ms 一次）。
    pub fn spawn() -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        std::thread::spawn(move || run(flag));
        Self { stop }
    }
}

impl Drop for Hider {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn run(flag: Arc<AtomicBool>) {
    let mut log = SessionLog::open();
    log.write(&format!("=== 投屏会话开始（pid={}）===", std::process::id()));
    let mut scan_no: u64 = 0;
    while !flag.load(Ordering::Relaxed) {
        scan_no += 1;
        match hide_bars() {
            Ok((hidden, pids, records)) => {
                if hidden > 0 {
                    println!("[indicator] 已隐藏共享提示条 ×{hidden}");
                }
                let sig = {
                    use std::hash::{Hash, Hasher};
                    let mut h = std::collections::hash_map::DefaultHasher::new();
                    records.hash(&mut h);
                    h.finish()
                };
                let changed = sig != log.last_sig;
                // 记录时机：会话前 10 秒（前 20 扫）逐次全量；其后仅在窗口集合变化、发生隐藏或每 20 扫时
                let verbose = scan_no <= 20
                    || hidden > 0
                    || (!records.is_empty() && changed)
                    || scan_no % 20 == 0;
                if verbose {
                    log.write(&format!(
                        "scan {scan_no}: 后代 webview 进程={pids:?} 隐藏={hidden}"
                    ));
                    for r in &records {
                        log.write(r);
                    }
                }
                log.last_sig = sig;
            }
            Err(e) => log.write(&format!("scan {scan_no}: 扫描失败 {e}")),
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    log.write("=== 投屏会话结束 ===");
}

/// 日志路径：与 config.json 同目录（%APPDATA%\com.echoroom.dev\indicator.log）
fn log_path() -> PathBuf {
    crate::config::default_config_path().with_file_name("indicator.log")
}

/// 会话日志：追加写入（文件过大时重建），单会话行数封顶。
struct SessionLog {
    file: Option<std::fs::File>,
    lines: usize,
    last_sig: u64,
    started: Instant,
}

impl SessionLog {
    fn open() -> Self {
        let path = log_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let too_big = std::fs::metadata(&path)
            .map(|m| m.len() > LOG_MAX_BYTES)
            .unwrap_or(false);
        let file = if too_big {
            std::fs::File::create(&path).ok()
        } else {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .ok()
        };
        Self {
            file,
            lines: 0,
            last_sig: u64::MAX,
            started: Instant::now(),
        }
    }

    fn write(&mut self, line: &str) {
        if self.lines > LOG_MAX_LINES {
            return;
        }
        if self.lines == LOG_MAX_LINES {
            self.lines += 1;
            self.raw("…（达单会话日志上限，暂停记录）");
            return;
        }
        self.lines += 1;
        let t = self.started.elapsed().as_secs_f32();
        let line = format!("[{t:7.1}s] {line}");
        self.raw(&line);
    }

    fn raw(&mut self, line: &str) {
        if let Some(f) = self.file.as_mut() {
            let _ = writeln!(f, "{line}");
        }
    }
}

/// 查找并隐藏本应用 WebView2 的共享提示条。
/// 返回（本次隐藏数, 后代 webview 进程 PID, 相关窗口明细记录）。
fn hide_bars() -> windows::core::Result<(u32, Vec<u32>, Vec<String>)> {
    let pids = webview_tree_pids()?;
    if pids.is_empty() {
        return Ok((0, pids, Vec::new()));
    }
    let mut ctx = Ctx {
        pids: &pids,
        hidden: 0,
        records: Vec::new(),
    };
    unsafe {
        EnumWindows(Some(enum_cb), LPARAM(&mut ctx as *mut Ctx as isize))?;
    }
    let Ctx { hidden, records, .. } = ctx;
    Ok((hidden, pids, records))
}

/// 回调上下文：本应用 WebView2 进程 PID 集合 + 本次已隐藏计数 + 明细记录
struct Ctx<'a> {
    pids: &'a [u32],
    hidden: u32,
    records: Vec<String>,
}

/// 顶层窗口回调：命中提示条签名则 SW_HIDE，并记录相关窗口明细。
unsafe extern "system" fn enum_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = &mut *(lparam.0 as *mut Ctx);
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if !ctx.pids.contains(&pid) {
        return TRUE;
    }
    let mut cbuf = [0u16; 64];
    let cn = GetClassNameW(hwnd, &mut cbuf).clamp(0, cbuf.len() as i32) as usize;
    let class = String::from_utf16_lossy(&cbuf[..cn]);
    let vis = IsWindowVisible(hwnd).as_bool();
    // 只关注可见窗口或 webview 顶层窗口（不可见且非 Chrome_WidgetWin* 的直接忽略）
    if !vis && !class.starts_with("Chrome_WidgetWin") {
        return TRUE;
    }
    let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
    let mut tbuf = [0u16; 128];
    let tn = GetWindowTextW(hwnd, &mut tbuf).clamp(0, tbuf.len() as i32) as usize;
    let title: String = String::from_utf16_lossy(&tbuf[..tn]).chars().take(60).collect();
    let mut rc = RECT::default();
    let rc_s = if GetWindowRect(hwnd, &mut rc).is_ok() {
        format!("{}x{}", rc.right - rc.left, rc.bottom - rc.top)
    } else {
        "?".into()
    };
    let mut suffix = "";
    if vis && should_hide(&class, &title, ex) {
        let _ = ShowWindow(hwnd, SW_HIDE);
        if IsWindowVisible(hwnd).as_bool() {
            suffix = " → hide未生效(仍可见!)";
        } else {
            suffix = " → 已隐藏";
            ctx.hidden += 1;
        }
    }
    ctx.records.push(format!(
        "  hwnd=0x{:X} pid={pid} cls={class} ex=0x{ex:08X} vis={vis} rect={rc_s} title={title:?}{suffix}",
        hwnd.0 as usize
    ));
    TRUE
}

/// 提示条匹配（R10 放宽）：
/// ① 原验证签名（Chrome_WidgetWin_1 + 置顶 + 无重定向位）放宽为「前置顶或无重定向」任一；
/// ② 标题含"正在共享"/"is sharing"兜底（防类名/样式随 WebView2 版本变化）；
/// DevTools 永远排除。
fn should_hide(class: &str, title: &str, ex: u32) -> bool {
    if title.contains("DevTools") {
        return false;
    }
    let topmost = ex & WS_EX_TOPMOST.0 != 0;
    let no_redir = ex & WS_EX_NOREDIRECTIONBITMAP.0 != 0;
    if class.starts_with("Chrome_WidgetWin") && (topmost || no_redir) {
        return true;
    }
    title.contains("正在共享") || title.to_lowercase().contains("is sharing")
}

/// 本应用（宿主进程）全部后代进程中的 msedgewebview2.exe PID。
/// 直接子进程（browser 进程）与更深的 renderer/gpu/utility 进程一并收集——
/// 提示条窗口的归属进程可能随 WebView2 版本变化。
fn webview_tree_pids() -> windows::core::Result<Vec<u32>> {
    let my_pid = std::process::id();
    let mut rows: Vec<(u32, u32, String)> = Vec::new(); // (pid, parent, exe)
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
                rows.push((
                    entry.th32ProcessID,
                    entry.th32ParentProcessID,
                    String::from_utf16_lossy(&entry.szExeFile[..len]),
                ));
                if Process32NextW(snap, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
    }
    // 从自身出发按父子关系向下遍历（visited 防环）
    let mut pids = Vec::new();
    let mut visited = std::collections::HashSet::new();
    visited.insert(my_pid);
    let mut frontier = vec![my_pid];
    while let Some(p) = frontier.pop() {
        for (pid, parent, name) in &rows {
            if *parent == p && visited.insert(*pid) {
                if name.eq_ignore_ascii_case("msedgewebview2.exe") {
                    pids.push(*pid);
                }
                frontier.push(*pid);
            }
        }
    }
    pids.sort_unstable(); // 顺序稳定，日志可比对
    Ok(pids)
}

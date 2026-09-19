//! WASAPI 进程回路（Application Loopback）屏幕声音采集：排除本进程树的系统混音。
//! 仅 Windows 11（build ≥ 22000）支持；Win10 上 `is_supported()` 返回 false（UI 置灰）。
//! 输出：立体声 i16 交错、每 960 帧（1920 样本）一块经 SyncSender 送出。
//!
//! 注：COM 细节（implement 宏形态 / PROPVARIANT 构造 / GetBuffer 签名）以所用 windows crate
//! 版本编译提示为准；流程与 API 调用顺序与微软官方 ApplicationLoopback 示例一致，并与
//! Task 1 的 screen_probe.rs（已验证）保持相同的避坑处理：
//!  - 进程回路模式 GetMixFormat 不可用 → 手动 WAVEFORMATEX + AUTOCONVERTPCM；
//!  - windows-rs 的 PROPVARIANT 析构会误释放栈指针 → 用完必须 forget；
//!  - 必须先释放全部 COM 对象，最后 CoUninitialize。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::Arc;
use std::time::Duration;

use windows::core::{implement, Interface, Ref, Result as WinResult};
use windows::Win32::Media::Audio::{
    ActivateAudioInterfaceAsync, IActivateAudioInterfaceAsyncOperation,
    IActivateAudioInterfaceCompletionHandler, IActivateAudioInterfaceCompletionHandler_Impl,
    IAudioCaptureClient, IAudioClient, AUDIOCLIENT_ACTIVATION_PARAMS,
    AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK, AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS,
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
    AUDCLNT_STREAMFLAGS_LOOPBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
    PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE, VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
    WAVEFORMATEX,
};
use windows::Win32::System::Com::StructuredStorage::{PROPVARIANT, PROPVARIANT_0_0, PROPVARIANT_0_0_0};
use windows::Win32::System::Com::{BLOB, CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
use windows::Win32::System::Variant::VT_BLOB;

/// 输出格式：48000Hz 立体声 float32（AUTOCONVERTPCM 自动转换来源格式）
const OUT_RATE: u32 = 48_000;
const OUT_CH: u16 = 2;
/// 每 20ms 一块：960 帧 × 2 声道
const CHUNK_SAMPLES: usize = 1920;

/// 采集句柄：Drop 即停止
pub struct ScreenCaptureHandle {
    stop: Arc<AtomicBool>,
}

impl Drop for ScreenCaptureHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// 系统是否支持进程回路（Windows 11 build 22000+）
pub fn is_supported() -> bool {
    use windows::Wdk::System::SystemServices::RtlGetVersion;
    use windows::Win32::System::SystemInformation::OSVERSIONINFOW;
    let mut vi = OSVERSIONINFOW::default();
    vi.dwOSVersionInfoSize = std::mem::size_of::<OSVERSIONINFOW>() as u32;
    let ok = unsafe { RtlGetVersion(&mut vi).is_ok() };
    ok && vi.dwBuildNumber >= 22000
}

#[implement(IActivateAudioInterfaceCompletionHandler)]
struct ActivationHandler {
    done: Arc<AtomicBool>,
}

impl IActivateAudioInterfaceCompletionHandler_Impl for ActivationHandler_Impl {
    fn ActivateCompleted(&self, _op: Ref<'_, IActivateAudioInterfaceAsyncOperation>) -> WinResult<()> {
        self.done.store(true, Ordering::Release);
        Ok(())
    }
}

/// 启动采集线程；返回前等待初始化完成（失败直接报错，句柄 Drop 停止采集）。
pub fn spawn_screen_capture(tx: SyncSender<Vec<i16>>) -> anyhow::Result<ScreenCaptureHandle> {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = stop.clone();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<anyhow::Result<()>>();
    std::thread::spawn(move || match init_capture() {
        Ok((client, capture)) => {
            let _ = ready_tx.send(Ok(()));
            capture_loop(&client, &capture, &stop_thread, &tx);
            // ⚠ COM 释放顺序：先释放全部 COM 对象，最后 CoUninitialize。
            // 若先 CoUninitialize 再让局部变量析构（Release），等于在已反初始化的公寓里调 COM → 堆损坏。
            drop(capture);
            drop(client);
            unsafe { CoUninitialize() }
        }
        Err(e) => {
            let _ = ready_tx.send(Err(e));
        }
    });
    match ready_rx.recv() {
        Ok(Ok(())) => Ok(ScreenCaptureHandle { stop }),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(anyhow::anyhow!("屏幕采集线程提前退出")),
    }
}

/// 激活虚拟设备 + 初始化（流程与 screen_probe 相同）
fn init_capture() -> anyhow::Result<(IAudioClient, IAudioCaptureClient)> {
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? }

    let mut params = AUDIOCLIENT_ACTIVATION_PARAMS::default();
    params.ActivationType = AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK;
    params.Anonymous.ProcessLoopbackParams = AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
        TargetProcessId: std::process::id(),
        ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE,
    };
    let blob = BLOB {
        cbSize: std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
        pBlobData: &mut params as *mut _ as *mut u8,
    };
    let mut prop = PROPVARIANT::default();
    // VT_BLOB 写入：union 字段必须整体赋值（内层是 ManuallyDrop，不能逐字段写）
    prop.Anonymous.Anonymous = core::mem::ManuallyDrop::new(PROPVARIANT_0_0 {
        vt: VT_BLOB,
        wReserved1: 0,
        wReserved2: 0,
        wReserved3: 0,
        Anonymous: PROPVARIANT_0_0_0 { blob },
    });

    let done = Arc::new(AtomicBool::new(false));
    let handler: IActivateAudioInterfaceCompletionHandler =
        ActivationHandler { done: done.clone() }.into();
    let op = unsafe {
        ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID,
            Some(&prop as *const _),
            &handler,
        )?
    };
    // ⚠ windows-rs 给 PROPVARIANT 实现了 Drop（自动调 PropVariantClear）：对 VT_BLOB 会执行
    // CoTaskMemFree(pBlobData)，而 pBlobData 指向栈上的 params → 释放栈指针 → 堆损坏。
    // 用完必须 forget 跳过析构（prop 内没有任何需要释放的堆内存）。
    std::mem::forget(prop);
    while !done.load(Ordering::Acquire) {
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut hr = windows::core::HRESULT(0);
    let mut raw: Option<windows::core::IUnknown> = None;
    unsafe { op.GetActivateResult(&mut hr, &mut raw)? }
    hr.ok()?;
    let client: IAudioClient = raw.unwrap().cast()?;

    // ⚠ 进程回路模式下 GetMixFormat 不可用（表现为堆损坏崩溃）。必须手动指定目标格式 +
    // AUTOCONVERTPCM，由系统把来源格式自动转换过来。
    let wf = WAVEFORMATEX {
        wFormatTag: 3, // WAVE_FORMAT_IEEE_FLOAT
        nChannels: OUT_CH,
        nSamplesPerSec: OUT_RATE,
        nAvgBytesPerSec: OUT_RATE * OUT_CH as u32 * 4,
        nBlockAlign: OUT_CH * 4,
        wBitsPerSample: 32,
        cbSize: 0,
    };
    let flags = AUDCLNT_STREAMFLAGS_LOOPBACK
        | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
        | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
    unsafe {
        client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 200_000, 0, &wf as *const _, None)?
    };
    let capture: IAudioCaptureClient = unsafe { client.GetService()? };
    Ok((client, capture))
}

/// 读循环：f32 交错 → i16 → 满 1920 样本即送出一块（队列满丢块，不积压）
fn capture_loop(
    client: &IAudioClient,
    capture: &IAudioCaptureClient,
    stop: &AtomicBool,
    tx: &SyncSender<Vec<i16>>,
) {
    if let Err(e) = unsafe { client.Start() } {
        eprintln!("[screen] Start 失败: {e}");
        return;
    }
    println!("[screen] 进程回路采集已启动（排除本进程树）");
    let mut pending: Vec<i16> = Vec::with_capacity(CHUNK_SAMPLES * 2);
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(5));
        loop {
            let packet_frames = match unsafe { capture.GetNextPacketSize() } {
                Ok(n) => n,
                Err(_) => break,
            };
            if packet_frames == 0 {
                break;
            }
            let mut data: *mut u8 = std::ptr::null_mut();
            let mut frames: u32 = 0;
            let mut flags: u32 = 0;
            if unsafe { capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None) }.is_err() {
                break;
            }
            let n = frames as usize * OUT_CH as usize;
            if flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) == 0 && !data.is_null() {
                let f = unsafe { std::slice::from_raw_parts(data as *const f32, n) };
                for &x in f {
                    pending.push((x.clamp(-1.0, 1.0) * 32767.0) as i16);
                }
            } else {
                pending.resize(pending.len() + n, 0);
            }
            let _ = unsafe { capture.ReleaseBuffer(frames) };
            while pending.len() >= CHUNK_SAMPLES {
                let block: Vec<i16> = pending.drain(..CHUNK_SAMPLES).collect();
                let _ = tx.try_send(block); // 满则丢块
            }
        }
    }
    let _ = unsafe { client.Stop() };
    println!("[screen] 进程回路采集已停止");
}

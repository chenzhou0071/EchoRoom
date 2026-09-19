//! spike 用：WASAPI 进程回路捕获（排除指定进程树）8 秒 → spike_probe.wav（在 src-tauri 目录运行）。
//! 验证：播放一段音乐 + 开着 EchoRoom 说话，录出的 wav 应有音乐、不应有 EchoRoom 的声音。
//! 用法：cargo run --bin screen_probe -- <要排除的进程 pid>
//!   该 pid 应为 EchoRoom 的 pid（任务管理器查看，或 PowerShell 执行 (Get-Process echoroom-client).Id）；
//!   EchoRoom 的声音应被排除、其他程序（音乐）的声音应被录到。
//!
//! 注：COM 细节（implement 宏形态 / PROPVARIANT 构造 / GetBuffer 签名）以所用 windows crate
//! 版本编译提示为准；本文件的流程与 API 调用顺序与微软官方 ApplicationLoopback 示例一致。
use std::sync::atomic::{AtomicBool, Ordering};
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
use windows::Win32::System::Com::BLOB;
use windows::Win32::System::Com::StructuredStorage::{PROPVARIANT, PROPVARIANT_0_0, PROPVARIANT_0_0_0};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
use windows::Win32::System::Variant::VT_BLOB;

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

fn main() -> anyhow::Result<()> {
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? }

    // 排除目标进程树：命令行传入（EchoRoom 的 pid）；缺省回退自身 pid 并给出提示
    let target_pid: u32 = match std::env::args().nth(1).and_then(|s| s.parse().ok()) {
        Some(pid) => pid,
        None => {
            println!("[probe] 未传 pid 参数，将排除 probe 自身（无法验证 EchoRoom 排除效果！）");
            std::process::id()
        }
    };
    println!("[probe] 排除进程树 pid={target_pid}");

    // 1) 激活参数：进程回路 + 排除目标进程树
    let mut params = AUDIOCLIENT_ACTIVATION_PARAMS::default();
    params.ActivationType = AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK;
    params.Anonymous.ProcessLoopbackParams = AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
        TargetProcessId: target_pid,
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

    // 2) 异步激活虚拟设备
    println!("[probe] 1/5 参数构造完成，开始激活");
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
    // CoTaskMemFree(pBlobData)，而我们的 pBlobData 指向栈上的 params → 释放栈指针 → 退出时
    // 堆损坏 0xC0000374（"报告点≠损坏点"，定位耗时）。用完必须 forget 跳过析构；
    // prop 内没有任何需要释放的堆内存，forget 是正确的。
    std::mem::forget(prop);
    println!("[probe] 2/5 ActivateAudioInterfaceAsync 已返回，等待回调");
    while !done.load(Ordering::Acquire) {
        std::thread::sleep(Duration::from_millis(10));
    }
    println!("[probe] 3/5 回调已触发，取激活结果");
    let mut hr = windows::core::HRESULT(0);
    let mut raw: Option<windows::core::IUnknown> = None;
    unsafe { op.GetActivateResult(&mut hr, &mut raw)? }
    hr.ok()?;
    println!("[probe] 4/5 GetActivateResult ok，cast IAudioClient");
    let client: IAudioClient = raw.unwrap().cast()?;
    println!("[probe] 5/5 IAudioClient 就绪");

    // 3) 格式 + 初始化（loopback + 自动转换）
    // ⚠ 进程回路模式下 IAudioClient::GetMixFormat 不可用（wasapi crate 文档明确记载
    // "get_mixformat just returns Not implemented"；windows-rs 下表现为堆损坏崩溃 0xC0000374）。
    // 必须手动指定目标格式 + AUTOCONVERTPCM 让系统把来源格式自动转换过来。
    const OUT_RATE: u32 = 48_000;
    const OUT_CH: u16 = 2;
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
    println!(
        "[probe] 采集格式: {}Hz {}ch 32bit float（AUTOCONVERTPCM 自动转换）",
        OUT_RATE, OUT_CH
    );
    let capture: IAudioCaptureClient = unsafe { client.GetService()? };
    unsafe { client.Start()? }

    // 4) 读循环：f32 交错 → i16 → wav（8 秒）
    let spec = hound::WavSpec {
        channels: OUT_CH,
        sample_rate: OUT_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut wav = hound::WavWriter::create("spike_probe.wav", spec)?;
    // 第二参数：采集秒数（默认 8，调试用可传 0 跳过读循环）
    let secs: u64 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(8);
    let t_end = std::time::Instant::now() + Duration::from_secs(secs);
    while std::time::Instant::now() < t_end {
        std::thread::sleep(Duration::from_millis(10));
        loop {
            let packet_frames = unsafe { capture.GetNextPacketSize()? };
            if packet_frames == 0 {
                break;
            }
            let mut data: *mut u8 = std::ptr::null_mut();
            let mut frames: u32 = 0;
            let mut flags: u32 = 0;
            unsafe { capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None)? }
            let n = frames as usize * OUT_CH as usize;
            if flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) == 0 && !data.is_null() {
                let f = unsafe { std::slice::from_raw_parts(data as *const f32, n) };
                for &x in f {
                    wav.write_sample((x.clamp(-1.0, 1.0) * 32767.0) as i16)?;
                }
            } else {
                for _ in 0..n {
                    wav.write_sample(0i16)?;
                }
            }
            unsafe { capture.ReleaseBuffer(frames)? }
        }
    }
    unsafe { client.Stop()? }
    wav.finalize()?;
    println!("[probe] 完成 → spike_probe.wav");
    // ⚠ COM 释放顺序：必须先释放全部 COM 对象，最后 CoUninitialize。
    // 若先 CoUninitialize 再让局部变量析构（Release），等于在已反初始化的公寓里调 COM → 堆损坏 0xC0000374。
    println!("[probe] drop capture");
    drop(capture);
    println!("[probe] drop client");
    drop(client);
    println!("[probe] drop op");
    drop(op);
    println!("[probe] drop handler");
    drop(handler);
    println!("[probe] CoUninitialize");
    unsafe { CoUninitialize() }
    println!("[probe] 即将退出");
    Ok(())
}

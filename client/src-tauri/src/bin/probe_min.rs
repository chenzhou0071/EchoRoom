//! 最小复现（step2）：激活流程（ActivateAudioInterfaceAsync → 回调 → GetActivateResult → cast），无 Initialize。
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use windows::core::{implement, Interface, Ref, Result as WinResult};
use windows::Win32::Media::Audio::{
    ActivateAudioInterfaceAsync, IActivateAudioInterfaceAsyncOperation,
    IActivateAudioInterfaceCompletionHandler, IActivateAudioInterfaceCompletionHandler_Impl,
    IAudioClient, AUDIOCLIENT_ACTIVATION_PARAMS, AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
    AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS, PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE,
    VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
};
use windows::Win32::System::Com::BLOB;
use windows::Win32::System::Com::StructuredStorage::{PROPVARIANT, PROPVARIANT_0_0, PROPVARIANT_0_0_0};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
use windows::Win32::System::Variant::VT_BLOB;

#[implement(IActivateAudioInterfaceCompletionHandler)]
struct H {
    done: Arc<AtomicBool>,
}

impl IActivateAudioInterfaceCompletionHandler_Impl for H_Impl {
    fn ActivateCompleted(&self, _op: Ref<'_, IActivateAudioInterfaceAsyncOperation>) -> WinResult<()> {
        self.done.store(true, Ordering::Release);
        Ok(())
    }
}

fn main() -> anyhow::Result<()> {
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? };

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
    prop.Anonymous.Anonymous = core::mem::ManuallyDrop::new(PROPVARIANT_0_0 {
        vt: VT_BLOB,
        wReserved1: 0,
        wReserved2: 0,
        wReserved3: 0,
        Anonymous: PROPVARIANT_0_0_0 { blob },
    });

    let done = Arc::new(AtomicBool::new(false));
    let h: IActivateAudioInterfaceCompletionHandler = H { done: done.clone() }.into();
    // step2b：不调用激活 API，构造完参数直接释放
    // 关键修复验证：PROPVARIANT 的 Drop 会调 PropVariantClear → 对 VT_BLOB 调
    // CoTaskMemFree(pBlobData)；我们的 pBlobData 指向栈上的 params，释放栈指针 = 堆损坏。
    std::mem::forget(prop);
    println!("[min2b] 不调用激活，直接释放 h");
    drop(h);
    println!("[min2b] 即将退出");
    unsafe { CoUninitialize() }
    Ok(())
}

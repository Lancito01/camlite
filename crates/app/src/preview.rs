//! Live preview: a background thread reads "CamLite Camera" itself (a shared
//! stream, so other apps can use it at the same time — and the preview shows
//! exactly what they get), scaled down to a small RGB image. Falls back to the
//! physical camera when the virtual camera isn't set up. Only runs while the
//! window is visible.

use camlite_common::camera::{self, Mode};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
use windows_core::Result;

const FIRST_VIDEO: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
const PREVIEW_WIDTH: u32 = 960;

pub struct Frame {
    pub size: [usize; 2],
    pub rgba: Vec<u8>,
    pub seq: u64,
}

#[derive(Default)]
pub struct Shared {
    pub frame: Mutex<Option<Frame>>,
    pub error: Mutex<Option<String>>,
    stop: AtomicBool,
}

pub struct Preview {
    pub shared: Arc<Shared>,
    pub link: String,
    pub mode: Mode,
    handle: Option<JoinHandle<()>>,
}

impl Preview {
    pub fn start(link: String, mode: Mode, ctx: eframe::egui::Context) -> Self {
        let shared = Arc::new(Shared::default());
        let s = shared.clone();
        let l = link.clone();
        let handle = std::thread::spawn(move || {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            }
            if let Err(e) = run(&l, mode, &s, &ctx) {
                let msg = if e.code() == MF_E_HW_MFT_FAILED_START_STREAMING || e.code().0 as u32 == 0xC00D3704 {
                    "Camera is busy in another format".to_string()
                } else {
                    format!("Preview unavailable ({:#010x})", e.code().0)
                };
                *s.error.lock().unwrap() = Some(msg);
                ctx.request_repaint();
            }
            unsafe { CoUninitialize() };
        });
        Preview { shared, link, mode, handle: Some(handle) }
    }
}

impl Drop for Preview {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn run(link: &str, mode: Mode, shared: &Shared, ctx: &eframe::egui::Context) -> Result<()> {
    unsafe {
        let src = camera::open_device(link)?;
        let mut attrs = None;
        MFCreateAttributes(&mut attrs, 1)?;
        let attrs = attrs.unwrap();
        attrs.SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1)?;
        let reader = MFCreateSourceReaderFromMediaSource(&src, &attrs)?;

        let mut natives = Vec::new();
        let mut i = 0;
        while let Ok(mt) = reader.GetNativeMediaType(FIRST_VIDEO, i) {
            if let Some(info) = camera::type_info(&mt) {
                natives.push((mt, info));
            }
            i += 1;
        }
        // The virtual camera may still be running an older mode for another app;
        // then just show whatever it offers.
        let (native, info) = camera::best_native(&natives, mode)
            .or(natives.first())
            .ok_or_else(|| windows_core::Error::from(MF_E_INVALIDMEDIATYPE))?;
        reader.SetCurrentMediaType(FIRST_VIDEO, None, native)?;
        let mode = info.mode;

        let (w, h) = (PREVIEW_WIDTH, PREVIEW_WIDTH * mode.height / mode.width.max(1));
        let out = MFCreateMediaType()?;
        out.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        out.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
        out.SetUINT64(&MF_MT_FRAME_SIZE, ((w as u64) << 32) | h as u64)?;
        reader.SetCurrentMediaType(FIRST_VIDEO, None, &out)?;

        let mut seq = 0u64;
        let mut rgba = Vec::new();
        while !shared.stop.load(Ordering::SeqCst) {
            let mut flags = 0u32;
            let mut sample = None;
            reader.ReadSample(FIRST_VIDEO, 0, None, Some(&mut flags), None, Some(&mut sample))?;
            if flags & (MF_SOURCE_READERF_ERROR.0 as u32) != 0 {
                return Err(MF_E_VIDEO_RECORDING_DEVICE_INVALIDATED.into());
            }
            let Some(sample) = sample else { continue };
            let buf = sample.ConvertToContiguousBuffer()?;
            let mut ptr = std::ptr::null_mut();
            let mut len = 0u32;
            buf.Lock(&mut ptr, None, Some(&mut len))?;
            let stride = (w * 4) as usize;
            let rows = (len as usize / stride).min(h as usize);
            let data = std::slice::from_raw_parts(ptr, stride * rows);
            rgba.clear();
            rgba.reserve(data.len());
            for px in data.chunks_exact(4) {
                rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
            }
            buf.Unlock()?;
            seq += 1;
            *shared.frame.lock().unwrap() =
                Some(Frame { size: [w as usize, rows], rgba: std::mem::take(&mut rgba), seq });
            ctx.request_repaint();
        }
        drop(reader);
        let _ = src.Shutdown();
    }
    Ok(())
}

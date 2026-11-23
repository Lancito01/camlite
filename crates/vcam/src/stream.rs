//! The single video stream. Frames come from an asynchronous Source Reader on
//! the physical camera (MJPEG → NV12 via Media Foundation's decoder).

use crate::attributes::raw;
use crate::source::ks_not_found;
use camlite_common::camera::{self, Controls, Mode};
use camlite_common::settings::Settings;
use camlite_common::vlog;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use windows::Win32::Foundation::{ERROR_DEVICE_NOT_CONNECTED, S_OK};
use windows::Win32::Media::KernelStreaming::{IKsControl, IKsControl_Impl, KSIDENTIFIER};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows_core::{implement, IUnknown, Interface, Ref, Result, GUID, HRESULT};

/// PINNAME_VIDEO_CAPTURE
const PINNAME_VIDEO_CAPTURE: GUID = GUID::from_u128(0xfb6c4281_0353_11d1_905f_0000c0cc16ba);
const FIRST_VIDEO: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;

#[derive(Default)]
struct State {
    reader: Option<IMFSourceReader>,
    source: Option<IMFMediaSource>,
    tokens: VecDeque<Option<IUnknown>>,
    pending: bool,
    active: bool,
    shut: bool,
    frames: u64,
}

pub struct StreamShared {
    pub attrs: IMFAttributes,
    pub sd: IMFStreamDescriptor,
    queue: IMFMediaEventQueue,
    mode: Mode,
    st: Mutex<State>,
}

fn set_size(mt: &IMFAttributes, key: &GUID, hi: u32, lo: u32) -> Result<()> {
    unsafe { mt.SetUINT64(key, ((hi as u64) << 32) | lo as u64) }
}

fn set_stream_attrs(a: &IMFAttributes) -> Result<()> {
    unsafe {
        a.SetGUID(&MF_DEVICESTREAM_STREAM_CATEGORY, &PINNAME_VIDEO_CAPTURE)?;
        a.SetUINT32(&MF_DEVICESTREAM_STREAM_ID, 0)?;
        a.SetUINT32(&MF_DEVICESTREAM_FRAMESERVER_SHARED, 1)?;
        a.SetUINT32(&MF_DEVICESTREAM_ATTRIBUTE_FRAMESOURCE_TYPES, MFFrameSourceTypes_Color.0 as u32)?;
    }
    Ok(())
}

/// The NV12 type we advertise to apps.
fn output_type(m: Mode) -> Result<IMFMediaType> {
    unsafe {
        let mt = MFCreateMediaType()?;
        mt.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        mt.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
        set_size(&mt, &MF_MT_FRAME_SIZE, m.width, m.height)?;
        set_size(&mt, &MF_MT_FRAME_RATE, m.fps, 1)?;
        set_size(&mt, &MF_MT_PIXEL_ASPECT_RATIO, 1, 1)?;
        mt.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        mt.SetUINT32(&MF_MT_ALL_SAMPLES_INDEPENDENT, 1)?;
        mt.SetUINT32(&MF_MT_FIXED_SIZE_SAMPLES, 1)?;
        mt.SetUINT32(&MF_MT_DEFAULT_STRIDE, m.width)?;
        mt.SetUINT32(&MF_MT_SAMPLE_SIZE, m.width * m.height * 3 / 2)?;
        mt.SetUINT32(&MF_MT_AVG_BITRATE, m.width * m.height * 12 * m.fps)?;
        Ok(mt)
    }
}

impl StreamShared {
    pub fn new(mode: Mode) -> Result<Arc<Self>> {
        unsafe {
            let mt = output_type(mode)?;
            let sd = MFCreateStreamDescriptor(0, &[Some(mt.clone())])?;
            sd.GetMediaTypeHandler()?.SetCurrentMediaType(&mt)?;
            set_stream_attrs(&sd)?;
            let mut attrs = None;
            MFCreateAttributes(&mut attrs, 4)?;
            let attrs = attrs.unwrap();
            set_stream_attrs(&attrs)?;
            Ok(Arc::new(StreamShared {
                attrs,
                sd,
                queue: MFCreateEventQueue()?,
                mode,
                st: Mutex::new(State::default()),
            }))
        }
    }

    pub fn set_source(&self, s: Option<IMFMediaSource>) {
        self.st.lock().unwrap().source = s;
    }

    fn queue_time_event(&self, met: MF_EVENT_TYPE) {
        let now = PROPVARIANT::from(unsafe { MFGetSystemTime() });
        unsafe {
            let _ = self.queue.QueueEventParamVar(met.0 as u32, &GUID::zeroed(), S_OK, &now);
        }
    }

    pub fn start(self: &Arc<Self>) -> Result<()> {
        let mut st = self.st.lock().unwrap();
        if st.shut {
            return Err(MF_E_SHUTDOWN.into());
        }
        if st.reader.is_none() {
            st.reader = Some(self.open_reader()?);
        }
        st.active = true;
        drop(st);
        self.queue_time_event(MEStreamStarted);
        Ok(())
    }

    fn release_reader(&self) {
        let reader = {
            let mut st = self.st.lock().unwrap();
            st.active = false;
            st.pending = false;
            st.tokens.clear();
            st.reader.take()
        };
        // Dropped outside the lock: releasing the reader shuts the camera down
        // and may wait for an in-flight callback that needs the lock.
        drop(reader);
    }

    pub fn stop(&self) {
        self.release_reader();
        self.queue_time_event(MEStreamStopped);
    }

    pub fn shutdown(&self) {
        self.release_reader();
        {
            let mut st = self.st.lock().unwrap();
            st.shut = true;
            st.source = None;
        }
        unsafe {
            let _ = self.queue.Shutdown();
        }
    }

    fn open_reader(self: &Arc<Self>) -> Result<IMFSourceReader> {
        let settings = Settings::load();
        let devices = camera::list_devices()?;
        let dev = camera::pick_device(&devices, &settings.device)
            .ok_or_else(|| windows_core::Error::from(HRESULT::from_win32(ERROR_DEVICE_NOT_CONNECTED.0)))?;
        vlog!("opening '{}' for {}", dev.name, self.mode.key());
        let device = camera::open_device(&dev.link)?;
        Controls::new(&device).apply(&settings.profile(&dev.link));

        unsafe {
            let callback: IMFSourceReaderCallback = ReaderCallback { shared: self.clone() }.into();
            let mut attrs = None;
            MFCreateAttributes(&mut attrs, 3)?;
            let attrs = attrs.unwrap();
            attrs.SetUnknown(&MF_SOURCE_READER_ASYNC_CALLBACK, &callback)?;
            attrs.SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1)?;
            let reader = MFCreateSourceReaderFromMediaSource(&device, &attrs)?;

            let mut natives = Vec::new();
            let mut i = 0;
            while let Ok(mt) = reader.GetNativeMediaType(FIRST_VIDEO, i) {
                if let Some(info) = camera::type_info(&mt) {
                    natives.push((mt, info));
                }
                i += 1;
            }
            let (native, info) = camera::best_native(&natives, self.mode).ok_or_else(|| {
                vlog!("camera has no native type for {}", self.mode.key());
                windows_core::Error::from(MF_E_INVALIDMEDIATYPE)
            })?;
            vlog!("native type {:?} {}", info.subtype, info.mode.key());
            reader.SetCurrentMediaType(FIRST_VIDEO, None, native)?;

            let out = MFCreateMediaType()?;
            out.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            out.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
            set_size(&out, &MF_MT_FRAME_SIZE, self.mode.width, self.mode.height)?;
            reader.SetCurrentMediaType(FIRST_VIDEO, None, &out)?;
            Ok(reader)
        }
    }

    fn request(&self, token: Option<IUnknown>) -> Result<()> {
        let reader = {
            let mut st = self.st.lock().unwrap();
            if st.shut {
                return Err(MF_E_SHUTDOWN.into());
            }
            if !st.active {
                return Err(MF_E_MEDIA_SOURCE_WRONGSTATE.into());
            }
            st.tokens.push_back(token);
            if st.pending {
                return Ok(());
            }
            st.pending = true;
            st.reader.clone()
        };
        if let Some(r) = reader {
            if let Err(e) = unsafe { r.ReadSample(FIRST_VIDEO, 0, None, None, None, None) } {
                self.st.lock().unwrap().pending = false;
                return Err(e);
            }
        }
        Ok(())
    }

    fn on_sample(&self, hr: HRESULT, flags: u32, sample: Option<&IMFSample>) {
        let next = {
            let mut st = self.st.lock().unwrap();
            st.pending = false;
            if !st.active {
                return;
            }
            if hr.is_err() || flags & (MF_SOURCE_READERF_ERROR.0 as u32) != 0 {
                vlog!("reader error {hr:?} flags={flags:#x}");
                let err = if hr.is_err() { hr } else { MF_E_VIDEO_RECORDING_DEVICE_INVALIDATED };
                unsafe {
                    let _ = self.queue.QueueEventParamVar(MEError.0 as u32, &GUID::zeroed(), err, std::ptr::null());
                }
                return;
            }
            if let Some(sample) = sample {
                if let Some(token) = st.tokens.pop_front() {
                    unsafe {
                        let _ = sample.SetSampleTime(MFGetSystemTime());
                        let _ = sample.SetSampleDuration(10_000_000 / self.mode.fps.max(1) as i64);
                        if let Some(t) = &token {
                            let _ = sample.SetUnknown(&MFSampleExtension_Token, t);
                        }
                        let _ = self.queue.QueueEventParamUnk(MEMediaSample.0 as u32, &GUID::zeroed(), S_OK, sample);
                    }
                    st.frames += 1;
                    if st.frames == 1 || st.frames % 1800 == 0 {
                        vlog!("delivered {} frames", st.frames);
                    }
                }
            }
            if st.tokens.is_empty() {
                None
            } else {
                st.pending = true;
                st.reader.clone()
            }
        };
        if let Some(r) = next {
            if unsafe { r.ReadSample(FIRST_VIDEO, 0, None, None, None, None) }.is_err() {
                self.st.lock().unwrap().pending = false;
            }
        }
    }
}

#[implement(IMFSourceReaderCallback)]
struct ReaderCallback {
    shared: Arc<StreamShared>,
}

impl IMFSourceReaderCallback_Impl for ReaderCallback_Impl {
    fn OnReadSample(&self, hr: HRESULT, _idx: u32, flags: u32, _ts: i64, sample: Ref<IMFSample>) -> Result<()> {
        self.shared.on_sample(hr, flags, sample.as_ref());
        Ok(())
    }
    fn OnFlush(&self, _idx: u32) -> Result<()> {
        Ok(())
    }
    fn OnEvent(&self, _idx: u32, _e: Ref<IMFMediaEvent>) -> Result<()> {
        Ok(())
    }
}

#[implement(IMFMediaStream2, IKsControl)]
pub struct Stream {
    pub shared: Arc<StreamShared>,
}

impl IMFMediaEventGenerator_Impl for Stream_Impl {
    fn GetEvent(&self, flags: MEDIA_EVENT_GENERATOR_GET_EVENT_FLAGS) -> Result<IMFMediaEvent> {
        unsafe { self.shared.queue.GetEvent(flags.0) }
    }
    fn BeginGetEvent(&self, cb: Ref<IMFAsyncCallback>, state: Ref<IUnknown>) -> Result<()> {
        let q = &self.shared.queue;
        unsafe { (q.vtable().BeginGetEvent)(q.as_raw(), raw(&cb), raw(&state)).ok() }
    }
    fn EndGetEvent(&self, result: Ref<IMFAsyncResult>) -> Result<IMFMediaEvent> {
        unsafe { self.shared.queue.EndGetEvent(result.as_ref()) }
    }
    fn QueueEvent(&self, met: u32, ext: *const GUID, hr: HRESULT, v: *const PROPVARIANT) -> Result<()> {
        unsafe { self.shared.queue.QueueEventParamVar(met, ext, hr, v) }
    }
}

impl IMFMediaStream_Impl for Stream_Impl {
    fn GetMediaSource(&self) -> Result<IMFMediaSource> {
        self.shared.st.lock().unwrap().source.clone().ok_or_else(|| MF_E_SHUTDOWN.into())
    }
    fn GetStreamDescriptor(&self) -> Result<IMFStreamDescriptor> {
        Ok(self.shared.sd.clone())
    }
    fn RequestSample(&self, token: Ref<IUnknown>) -> Result<()> {
        self.shared.request(token.cloned())
    }
}

impl IMFMediaStream2_Impl for Stream_Impl {
    fn SetStreamState(&self, value: MF_STREAM_STATE) -> Result<()> {
        let active = self.shared.st.lock().unwrap().active;
        if value == MF_STREAM_STATE_RUNNING && !active {
            self.shared.start()
        } else if value == MF_STREAM_STATE_STOPPED && active {
            self.shared.stop();
            Ok(())
        } else {
            Ok(())
        }
    }
    fn GetStreamState(&self) -> Result<MF_STREAM_STATE> {
        let st = self.shared.st.lock().unwrap();
        Ok(if st.active { MF_STREAM_STATE_RUNNING } else { MF_STREAM_STATE_STOPPED })
    }
}

impl IKsControl_Impl for Stream_Impl {
    fn KsProperty(&self, _p: *const KSIDENTIFIER, _pl: u32, _d: *mut core::ffi::c_void, _dl: u32, _r: *mut u32) -> Result<()> {
        Err(ks_not_found())
    }
    fn KsMethod(&self, _p: *const KSIDENTIFIER, _pl: u32, _d: *mut core::ffi::c_void, _dl: u32, _r: *mut u32) -> Result<()> {
        Err(ks_not_found())
    }
    fn KsEvent(&self, _p: *const KSIDENTIFIER, _pl: u32, _d: *mut core::ffi::c_void, _dl: u32, _r: *mut u32) -> Result<()> {
        Err(ks_not_found())
    }
}

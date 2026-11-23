//! The media source the Frame Server talks to. It owns a single video stream.

use crate::attributes::raw;
use crate::forward_attributes;
use crate::stream::{Stream, StreamShared};
use camlite_common::camera::Mode;
use camlite_common::settings::Settings;
use camlite_common::vlog;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use windows::Win32::Foundation::{ERROR_SET_NOT_FOUND, S_OK};
use windows::Win32::Media::KernelStreaming::{IKsControl, IKsControl_Impl, KSIDENTIFIER};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows_core::{implement, IUnknown, Interface, Ref, Result, BOOL, GUID, HRESULT, PCWSTR, PWSTR};

#[implement(IMFMediaSourceEx, IMFGetService, IKsControl, IMFSampleAllocatorControl, IMFAttributes)]
pub struct Source {
    attrs: IMFAttributes,
    queue: IMFMediaEventQueue,
    pd: IMFPresentationDescriptor,
    stream: IMFMediaStream,
    shared: Arc<StreamShared>,
    shut: AtomicBool,
}

pub fn ks_not_found() -> windows_core::Error {
    HRESULT::from_win32(ERROR_SET_NOT_FOUND.0).into()
}

impl Source {
    pub fn create(activate_attrs: &IMFAttributes) -> Result<IMFMediaSource> {
        let settings = Settings::load();
        let mode = camlite_common::camera::list_devices()
            .ok()
            .and_then(|d| camlite_common::camera::pick_device(&d, &settings.device))
            .map(|dev| camlite_common::camera::resolve_mode(&settings, &dev, None))
            .unwrap_or(Mode { width: 1920, height: 1080, fps: 30 });
        vlog!("creating source, mode {}", mode.key());
        unsafe {
            let mut attrs = None;
            MFCreateAttributes(&mut attrs, 4)?;
            let attrs = attrs.unwrap();
            activate_attrs.CopyAllItems(&attrs)?;

            let shared = StreamShared::new(mode)?;
            let stream: IMFMediaStream2 = Stream { shared: shared.clone() }.into();
            let stream: IMFMediaStream = stream.cast()?;
            let pd = MFCreatePresentationDescriptor(Some(&[Some(shared.sd.clone())]))?;
            pd.SelectStream(0)?;

            let source: IMFMediaSourceEx = Source {
                attrs,
                queue: MFCreateEventQueue()?,
                pd,
                stream,
                shared: shared.clone(),
                shut: AtomicBool::new(false),
            }
            .into();
            let source: IMFMediaSource = source.cast()?;
            // Back-reference for IMFMediaStream::GetMediaSource (cleared on Shutdown).
            shared.set_source(Some(source.clone()));
            Ok(source)
        }
    }

    fn check(&self) -> Result<()> {
        if self.shut.load(Ordering::SeqCst) {
            Err(MF_E_SHUTDOWN.into())
        } else {
            Ok(())
        }
    }
}

forward_attributes!(Source_Impl);

impl IMFMediaEventGenerator_Impl for Source_Impl {
    fn GetEvent(&self, flags: MEDIA_EVENT_GENERATOR_GET_EVENT_FLAGS) -> Result<IMFMediaEvent> {
        unsafe { self.queue.GetEvent(flags.0) }
    }
    fn BeginGetEvent(&self, cb: Ref<IMFAsyncCallback>, state: Ref<IUnknown>) -> Result<()> {
        unsafe { (self.queue.vtable().BeginGetEvent)(self.queue.as_raw(), raw(&cb), raw(&state)).ok() }
    }
    fn EndGetEvent(&self, result: Ref<IMFAsyncResult>) -> Result<IMFMediaEvent> {
        unsafe { self.queue.EndGetEvent(result.as_ref()) }
    }
    fn QueueEvent(&self, met: u32, ext: *const GUID, hr: HRESULT, v: *const PROPVARIANT) -> Result<()> {
        unsafe { self.queue.QueueEventParamVar(met, ext, hr, v) }
    }
}

impl IMFMediaSource_Impl for Source_Impl {
    fn GetCharacteristics(&self) -> Result<u32> {
        self.check()?;
        Ok(MFMEDIASOURCE_IS_LIVE.0 as u32)
    }

    fn CreatePresentationDescriptor(&self) -> Result<IMFPresentationDescriptor> {
        self.check()?;
        unsafe { self.pd.Clone() }
    }

    fn Start(&self, pd: Ref<IMFPresentationDescriptor>, fmt: *const GUID, _pos: *const PROPVARIANT) -> Result<()> {
        self.check()?;
        unsafe {
            if !fmt.is_null() && *fmt != GUID::zeroed() {
                return Err(MF_E_UNSUPPORTED_TIME_FORMAT.into());
            }
            let pd = pd.ok()?;
            let mut selected = BOOL(0);
            let mut sd = None;
            pd.GetStreamDescriptorByIndex(0, &mut selected, &mut sd)?;
            vlog!("Start (selected={})", selected.as_bool());
            if selected.as_bool() {
                self.queue.QueueEventParamUnk(MENewStream.0 as u32, &GUID::zeroed(), S_OK, &self.stream)?;
                self.shared.start().inspect_err(|e| vlog!("stream start failed: {e}"))?;
            }
            let now = PROPVARIANT::from(MFGetSystemTime());
            self.queue.QueueEventParamVar(MESourceStarted.0 as u32, &GUID::zeroed(), S_OK, &now)
        }
    }

    fn Stop(&self) -> Result<()> {
        self.check()?;
        vlog!("Stop");
        self.shared.stop();
        let now = PROPVARIANT::from(unsafe { MFGetSystemTime() });
        unsafe { self.queue.QueueEventParamVar(MESourceStopped.0 as u32, &GUID::zeroed(), S_OK, &now) }
    }

    fn Pause(&self) -> Result<()> {
        Err(MF_E_INVALID_STATE_TRANSITION.into())
    }

    fn Shutdown(&self) -> Result<()> {
        if self.shut.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        vlog!("Shutdown");
        self.shared.shutdown();
        unsafe { self.queue.Shutdown() }
    }
}

impl IMFMediaSourceEx_Impl for Source_Impl {
    fn GetSourceAttributes(&self) -> Result<IMFAttributes> {
        self.check()?;
        Ok(self.attrs.clone())
    }
    fn GetStreamAttributes(&self, id: u32) -> Result<IMFAttributes> {
        self.check()?;
        if id != 0 {
            return Err(MF_E_INVALIDSTREAMNUMBER.into());
        }
        Ok(self.shared.attrs.clone())
    }
    fn SetD3DManager(&self, _m: Ref<IUnknown>) -> Result<()> {
        Ok(())
    }
}

impl IMFGetService_Impl for Source_Impl {
    fn GetService(&self, _s: *const GUID, _r: *const GUID, _p: *mut *mut core::ffi::c_void) -> Result<()> {
        Err(MF_E_UNSUPPORTED_SERVICE.into())
    }
}

impl IMFSampleAllocatorControl_Impl for Source_Impl {
    fn SetDefaultAllocator(&self, _id: u32, _alloc: Ref<IUnknown>) -> Result<()> {
        Ok(())
    }
    fn GetAllocatorUsage(&self, id: u32, input: *mut u32, usage: *mut MFSampleAllocatorUsage) -> Result<()> {
        unsafe {
            if !input.is_null() {
                *input = id;
            }
            if !usage.is_null() {
                *usage = MFSampleAllocatorUsage_UsesCustomAllocator;
            }
        }
        Ok(())
    }
}

impl IKsControl_Impl for Source_Impl {
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

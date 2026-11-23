//! `IMFActivate` handed to the Frame Server; it creates the media source.

use crate::attributes::raw;
use crate::forward_attributes;
use camlite_common::vlog;
use std::sync::Mutex;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows_core::{implement, IUnknown, Interface, Ref, Result, BOOL, GUID, PCWSTR, PWSTR};

#[implement(IMFActivate)]
pub struct Activate {
    attrs: IMFAttributes,
    source: Mutex<Option<IMFMediaSource>>,
}

impl Activate {
    pub fn new() -> Result<Self> {
        let mut attrs = None;
        unsafe { MFCreateAttributes(&mut attrs, 4)? };
        Ok(Activate { attrs: attrs.unwrap(), source: Mutex::new(None) })
    }
}

forward_attributes!(Activate_Impl);

impl IMFActivate_Impl for Activate_Impl {
    fn ActivateObject(&self, riid: *const GUID, ppv: *mut *mut core::ffi::c_void) -> Result<()> {
        vlog!("ActivateObject");
        let mut guard = self.source.lock().unwrap();
        let source = match guard.as_ref() {
            Some(s) => s.clone(),
            None => {
                let s = crate::source::Source::create(&self.attrs).inspect_err(|e| vlog!("create source failed: {e}"))?;
                *guard = Some(s.clone());
                s
            }
        };
        unsafe { source.query(riid, ppv).ok() }
    }

    fn ShutdownObject(&self) -> Result<()> {
        vlog!("ShutdownObject");
        if let Some(s) = self.source.lock().unwrap().take() {
            unsafe {
                let _ = s.Shutdown();
            }
        }
        Ok(())
    }

    fn DetachObject(&self) -> Result<()> {
        self.source.lock().unwrap().take();
        Ok(())
    }
}

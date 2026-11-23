//! CamLite virtual camera: a Media Foundation media source that the Windows
//! Camera Frame Server loads on demand. It opens the physical webcam at the
//! chosen HD mode, decodes MJPEG to NV12 and hands frames to whatever app is
//! using "CamLite Camera". Nothing runs while no app has the camera open.

#![allow(non_snake_case)]

mod activate;
mod attributes;
mod source;
mod stream;

use camlite_common::{vlog, VCAM_CLSID};
use std::ffi::c_void;
use windows::Win32::Foundation::{CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_POINTER, S_FALSE};
use windows::Win32::Media::MediaFoundation::IMFActivate;
use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl};
use windows_core::{implement, IUnknown, Interface, Ref, BOOL, GUID, HRESULT};

#[implement(IClassFactory)]
struct Factory;

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(
        &self,
        outer: Ref<IUnknown>,
        riid: *const GUID,
        ppv: *mut *mut c_void,
    ) -> windows_core::Result<()> {
        if ppv.is_null() {
            return Err(E_POINTER.into());
        }
        unsafe { *ppv = std::ptr::null_mut() };
        if outer.is_some() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        let activate: IMFActivate = activate::Activate::new()?.into();
        unsafe { activate.query(riid, ppv).ok() }
    }

    fn LockServer(&self, _lock: BOOL) -> windows_core::Result<()> {
        Ok(())
    }
}

#[no_mangle]
pub unsafe extern "system" fn DllGetClassObject(rclsid: *const GUID, riid: *const GUID, ppv: *mut *mut c_void) -> HRESULT {
    if ppv.is_null() {
        return E_POINTER;
    }
    *ppv = std::ptr::null_mut();
    if *rclsid != VCAM_CLSID {
        return CLASS_E_CLASSNOTAVAILABLE;
    }
    vlog!("DllGetClassObject");
    let factory: IClassFactory = Factory.into();
    factory.query(riid, ppv)
}

#[no_mangle]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    // Stay loaded: the Frame Server keeps sources alive across sessions anyway.
    S_FALSE
}

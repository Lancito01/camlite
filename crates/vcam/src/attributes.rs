//! `IMFAttributes` forwarding, used by objects that must expose an attribute
//! store themselves (the activator and the media source).

/// Implements `IMFAttributes_Impl` for `$t` by forwarding to `self.attrs`.
#[macro_export]
macro_rules! forward_attributes {
    ($t:ty) => {
        impl windows::Win32::Media::MediaFoundation::IMFAttributes_Impl for $t {
            fn GetItem(&self, k: *const GUID, v: *mut PROPVARIANT) -> Result<()> {
                unsafe { (self.attrs.vtable().GetItem)(self.attrs.as_raw(), k, v).ok() }
            }
            fn GetItemType(&self, k: *const GUID) -> Result<MF_ATTRIBUTE_TYPE> {
                unsafe { self.attrs.GetItemType(k) }
            }
            fn CompareItem(&self, k: *const GUID, v: *const PROPVARIANT) -> Result<BOOL> {
                unsafe { self.attrs.CompareItem(k, v) }
            }
            fn Compare(&self, theirs: Ref<IMFAttributes>, mt: MF_ATTRIBUTES_MATCH_TYPE) -> Result<BOOL> {
                unsafe {
                    let mut out = BOOL(0);
                    (self.attrs.vtable().Compare)(self.attrs.as_raw(), raw(&theirs), mt, &mut out).ok()?;
                    Ok(out)
                }
            }
            fn GetUINT32(&self, k: *const GUID) -> Result<u32> {
                unsafe { self.attrs.GetUINT32(k) }
            }
            fn GetUINT64(&self, k: *const GUID) -> Result<u64> {
                unsafe { self.attrs.GetUINT64(k) }
            }
            fn GetDouble(&self, k: *const GUID) -> Result<f64> {
                unsafe { self.attrs.GetDouble(k) }
            }
            fn GetGUID(&self, k: *const GUID) -> Result<GUID> {
                unsafe { self.attrs.GetGUID(k) }
            }
            fn GetStringLength(&self, k: *const GUID) -> Result<u32> {
                unsafe { self.attrs.GetStringLength(k) }
            }
            fn GetString(&self, k: *const GUID, buf: PWSTR, cch: u32, len: *mut u32) -> Result<()> {
                unsafe { (self.attrs.vtable().GetString)(self.attrs.as_raw(), k, buf, cch, len).ok() }
            }
            fn GetAllocatedString(&self, k: *const GUID, v: *mut PWSTR, len: *mut u32) -> Result<()> {
                unsafe { self.attrs.GetAllocatedString(k, v, len) }
            }
            fn GetBlobSize(&self, k: *const GUID) -> Result<u32> {
                unsafe { self.attrs.GetBlobSize(k) }
            }
            fn GetBlob(&self, k: *const GUID, buf: *mut u8, cb: u32, size: *mut u32) -> Result<()> {
                unsafe { (self.attrs.vtable().GetBlob)(self.attrs.as_raw(), k, buf, cb, size).ok() }
            }
            fn GetAllocatedBlob(&self, k: *const GUID, buf: *mut *mut u8, size: *mut u32) -> Result<()> {
                unsafe { self.attrs.GetAllocatedBlob(k, buf, size) }
            }
            fn GetUnknown(&self, k: *const GUID, riid: *const GUID, ppv: *mut *mut core::ffi::c_void) -> Result<()> {
                unsafe { (self.attrs.vtable().GetUnknown)(self.attrs.as_raw(), k, riid, ppv).ok() }
            }
            fn SetItem(&self, k: *const GUID, v: *const PROPVARIANT) -> Result<()> {
                unsafe { self.attrs.SetItem(k, v) }
            }
            fn DeleteItem(&self, k: *const GUID) -> Result<()> {
                unsafe { self.attrs.DeleteItem(k) }
            }
            fn DeleteAllItems(&self) -> Result<()> {
                unsafe { self.attrs.DeleteAllItems() }
            }
            fn SetUINT32(&self, k: *const GUID, v: u32) -> Result<()> {
                unsafe { self.attrs.SetUINT32(k, v) }
            }
            fn SetUINT64(&self, k: *const GUID, v: u64) -> Result<()> {
                unsafe { self.attrs.SetUINT64(k, v) }
            }
            fn SetDouble(&self, k: *const GUID, v: f64) -> Result<()> {
                unsafe { self.attrs.SetDouble(k, v) }
            }
            fn SetGUID(&self, k: *const GUID, v: *const GUID) -> Result<()> {
                unsafe { self.attrs.SetGUID(k, v) }
            }
            fn SetString(&self, k: *const GUID, v: &PCWSTR) -> Result<()> {
                unsafe { self.attrs.SetString(k, *v) }
            }
            fn SetBlob(&self, k: *const GUID, buf: *const u8, cb: u32) -> Result<()> {
                unsafe { (self.attrs.vtable().SetBlob)(self.attrs.as_raw(), k, buf, cb).ok() }
            }
            fn SetUnknown(&self, k: *const GUID, unk: Ref<IUnknown>) -> Result<()> {
                unsafe { (self.attrs.vtable().SetUnknown)(self.attrs.as_raw(), k, raw(&unk)).ok() }
            }
            fn LockStore(&self) -> Result<()> {
                unsafe { self.attrs.LockStore() }
            }
            fn UnlockStore(&self) -> Result<()> {
                unsafe { self.attrs.UnlockStore() }
            }
            fn GetCount(&self) -> Result<u32> {
                unsafe { self.attrs.GetCount() }
            }
            fn GetItemByIndex(&self, i: u32, k: *mut GUID, v: *mut PROPVARIANT) -> Result<()> {
                unsafe { (self.attrs.vtable().GetItemByIndex)(self.attrs.as_raw(), i, k, v).ok() }
            }
            fn CopyAllItems(&self, dest: Ref<IMFAttributes>) -> Result<()> {
                unsafe { (self.attrs.vtable().CopyAllItems)(self.attrs.as_raw(), raw(&dest)).ok() }
            }
        }
    };
}

/// Raw COM pointer of an optional interface reference (null when absent).
pub fn raw<T: windows_core::Interface>(r: &windows_core::Ref<T>) -> *mut core::ffi::c_void {
    r.as_ref().map(|x| x.as_raw()).unwrap_or(core::ptr::null_mut())
}

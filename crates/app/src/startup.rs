//! "Start with Windows": a Run-key entry that re-applies the saved camera
//! settings at sign-in and exits immediately (no resident process).

use windows::Win32::System::Registry::*;
use windows_core::{HSTRING, PCWSTR};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE: &str = "CamLite";

pub fn is_enabled() -> bool {
    let mut len = 0u32;
    unsafe {
        RegGetValueW(HKEY_CURRENT_USER, &HSTRING::from(RUN_KEY), &HSTRING::from(VALUE), RRF_RT_REG_SZ, None, None, Some(&mut len))
            .is_ok()
    }
}

pub fn set_enabled(on: bool) {
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, &HSTRING::from(RUN_KEY), None, KEY_SET_VALUE, &mut key).is_err() {
            return;
        }
        let name = HSTRING::from(VALUE);
        if on {
            let exe = crate::install::preferred_exe();
            let cmd = format!("\"{}\" --apply", exe.display());
            let wide: Vec<u16> = cmd.encode_utf16().chain(Some(0)).collect();
            let bytes = std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2);
            let _ = RegSetValueExW(key, PCWSTR(name.as_ptr()), None, REG_SZ, Some(bytes));
        } else {
            let _ = RegDeleteValueW(key, &name);
        }
        let _ = RegCloseKey(key);
    }
}

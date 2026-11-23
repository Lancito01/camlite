//! Shared code for the CamLite app and the virtual camera DLL.

pub mod camera;
pub mod log;
pub mod settings;

use std::path::PathBuf;
use windows_core::GUID;

/// COM class id of the virtual camera media source.
pub const VCAM_CLSID: GUID = GUID::from_u128(0x3e6b5c1a_8f2d_4a7b_9c41_6d2e0f8a7b13);
pub const VCAM_CLSID_STR: &str = "{3E6B5C1A-8F2D-4A7B-9C41-6D2E0F8A7B13}";
/// Name other apps see in their camera pickers.
pub const VCAM_NAME: &str = "CamLite Camera";

/// `%ProgramData%\CamLite` – readable by the Frame Server service (LocalService),
/// writable by the signed-in user.
pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("ProgramData")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"));
    base.join("CamLite")
}

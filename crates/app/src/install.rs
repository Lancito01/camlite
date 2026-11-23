//! Registering / removing the "CamLite Camera" virtual camera.
//!
//! The DLL is loaded by the Windows Camera Frame Server (runs as LocalService),
//! so it must live somewhere that account can read (Program Files) and its COM
//! class must be registered machine-wide. Both need admin once.

use camlite_common::{data_dir, VCAM_CLSID_STR, VCAM_NAME};
use std::path::{Path, PathBuf};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Registry::*;
use windows_core::{Result, HSTRING, PCWSTR};

pub const DLL_NAME: &str = "camlite_vcam.dll";
pub const EXE_NAME: &str = "camlite.exe";

fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// The installed copy of the app if present, otherwise this exe.
pub fn preferred_exe() -> PathBuf {
    let installed = install_dir().join(EXE_NAME);
    if installed.exists() {
        installed
    } else {
        std::env::current_exe().unwrap_or(installed)
    }
}

pub fn install_dir() -> PathBuf {
    let base = std::env::var_os("ProgramFiles").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Program Files"));
    base.join("CamLite")
}

fn clsid_key() -> String {
    format!(r"SOFTWARE\Classes\CLSID\{VCAM_CLSID_STR}")
}

/// Is the COM class registered and pointing at an existing DLL?
pub fn is_installed() -> bool {
    registered_dll().map(|p| p.exists()).unwrap_or(false)
}

fn registered_dll() -> Option<PathBuf> {
    let key = HSTRING::from(format!(r"{}\InprocServer32", clsid_key()));
    let mut buf = [0u16; 520];
    let mut len = (buf.len() * 2) as u32;
    unsafe {
        RegGetValueW(HKEY_LOCAL_MACHINE, &key, PCWSTR::null(), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as _), Some(&mut len))
            .ok()
            .ok()?;
    }
    let n = buf.iter().position(|&c| c == 0).unwrap_or(0);
    Some(PathBuf::from(String::from_utf16_lossy(&buf[..n])))
}

fn set_string(key: HKEY, name: Option<&str>, value: &str) -> Result<()> {
    let wide: Vec<u16> = value.encode_utf16().chain(Some(0)).collect();
    let bytes = unsafe { std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2) };
    let name = name.map(HSTRING::from);
    unsafe {
        RegSetValueExW(key, name.as_ref().map(|n| PCWSTR(n.as_ptr())).unwrap_or(PCWSTR::null()), None, REG_SZ, Some(bytes))
            .ok()
    }
}

fn register_com(dll: &Path) -> Result<()> {
    unsafe {
        let mut key = HKEY::default();
        let path = HSTRING::from(format!(r"{}\InprocServer32", clsid_key()));
        RegCreateKeyExW(HKEY_LOCAL_MACHINE, &path, None, None, REG_OPTION_NON_VOLATILE, KEY_WRITE, None, &mut key, None).ok()?;
        let r = set_string(key, None, &dll.to_string_lossy()).and_then(|_| set_string(key, Some("ThreadingModel"), "Both"));
        let _ = RegCloseKey(key);
        r?;
        let mut key = HKEY::default();
        RegCreateKeyExW(HKEY_LOCAL_MACHINE, &HSTRING::from(clsid_key()), None, None, REG_OPTION_NON_VOLATILE, KEY_WRITE, None, &mut key, None)
            .ok()?;
        let r = set_string(key, None, VCAM_NAME);
        let _ = RegCloseKey(key);
        r
    }
}

fn virtual_camera() -> Result<IMFVirtualCamera> {
    unsafe {
        MFCreateVirtualCamera(
            MFVirtualCameraType_SoftwareCameraSource,
            MFVirtualCameraLifetime_System,
            MFVirtualCameraAccess_CurrentUser,
            &HSTRING::from(VCAM_NAME),
            &HSTRING::from(VCAM_CLSID_STR),
            None,
        )
    }
}

/// Copies the DLL next to Program Files, registers it, and creates the
/// persistent virtual camera. Must run elevated.
pub fn install() -> Result<()> {
    let exe = std::env::current_exe().map_err(io_err)?;
    let exe_dir = exe.parent().map(Path::to_path_buf).unwrap_or_default();
    let dir = install_dir();
    std::fs::create_dir_all(&dir).map_err(io_err)?;
    for entry in std::fs::read_dir(&dir).map_err(io_err)?.flatten() {
        if entry.path().extension().is_some_and(|e| e == "old") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    if !same_dir(&exe_dir, &dir) {
        for name in [DLL_NAME, EXE_NAME] {
            let dest = dir.join(name);
            if dest.exists() {
                // A loaded DLL / running exe cannot be overwritten, but it can be renamed away.
                let old = dir.join(format!("{name}.{}.old", std::process::id()));
                let _ = std::fs::rename(&dest, &old);
            }
            std::fs::copy(exe_dir.join(name), &dest).map_err(io_err)?;
        }
    }
    register_com(&dir.join(DLL_NAME))?;
    camlite_common::log::log(
        "app.log",
        &format!("install: v{} from {} to {}", env!("CARGO_PKG_VERSION"), exe_dir.display(), dir.display()),
    );

    // Start menu shortcut for all users.
    let lnk = std::env::var("ProgramData").unwrap_or_else(|_| r"C:\ProgramData".into())
        + r"\Microsoft\Windows\Start Menu\Programs\CamLite.lnk";
    let script = format!(
        "$s=(New-Object -ComObject WScript.Shell).CreateShortcut('{}');$s.TargetPath='{}';$s.Save()",
        lnk,
        dir.join(EXE_NAME).display()
    );
    let _ = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags_no_window()
        .status();

    // Settings folder: writable by the user, readable by the Frame Server.
    let data = data_dir();
    let _ = std::fs::create_dir_all(&data);
    let _ = std::process::Command::new("icacls")
        .arg(&data)
        .args(["/grant", "*S-1-5-32-545:(OI)(CI)M", "/q"])
        .creation_flags_no_window()
        .status();

    // The Frame Server keeps an old DLL loaded; stop it so the new one is used.
    // It is demand-start, so Windows restarts it the next time a camera opens.
    restart_frame_server();

    let vcam = virtual_camera()?;
    unsafe { vcam.Start(None)? };
    Ok(())
}

fn restart_frame_server() {
    for svc in ["FrameServerMonitor", "FrameServer"] {
        let _ = std::process::Command::new("net").args(["stop", svc, "/y"]).creation_flags_no_window().status();
    }
}

/// Removes the virtual camera and its registration. Must run elevated.
pub fn uninstall() -> Result<()> {
    if let Ok(vcam) = virtual_camera() {
        unsafe {
            let _ = vcam.Remove();
        }
    }
    unsafe {
        let _ = RegDeleteTreeW(HKEY_LOCAL_MACHINE, &HSTRING::from(clsid_key()));
    }
    restart_frame_server();
    let dir = install_dir();
    for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        // Files still loaded by the Frame Server go away on next reboot.
        if std::fs::remove_file(entry.path()).is_err() {
            let p = HSTRING::from(entry.path().as_os_str());
            unsafe {
                let _ = windows::Win32::Storage::FileSystem::MoveFileExW(
                    &p,
                    PCWSTR::null(),
                    windows::Win32::Storage::FileSystem::MOVEFILE_DELAY_UNTIL_REBOOT,
                );
            }
        }
    }
    let _ = std::fs::remove_dir(&dir);
    let lnk = std::env::var("ProgramData").unwrap_or_else(|_| r"C:\ProgramData".into())
        + r"\Microsoft\Windows\Start Menu\Programs\CamLite.lnk";
    let _ = std::fs::remove_file(lnk);
    Ok(())
}

fn io_err(e: std::io::Error) -> windows_core::Error {
    windows_core::Error::new(windows::Win32::Foundation::E_FAIL, e.to_string())
}

trait NoWindow {
    fn creation_flags_no_window(&mut self) -> &mut Self;
}
impl NoWindow for std::process::Command {
    fn creation_flags_no_window(&mut self) -> &mut Self {
        use std::os::windows::process::CommandExt;
        self.creation_flags(0x0800_0000)
    }
}

/// Re-launches this exe elevated with `arg` and waits for it.
pub fn run_elevated(arg: &str) -> bool {
    match std::env::current_exe() {
        Ok(exe) => run_elevated_exe(&exe, arg),
        Err(_) => false,
    }
}

/// Runs `exe arg` elevated (UAC prompt) and waits; true on exit code 0.
pub fn run_elevated_exe(exe: &Path, arg: &str) -> bool {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
    use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
    let exe = HSTRING::from(exe.as_os_str());
    let verb = HSTRING::from("runas");
    let params = HSTRING::from(arg);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(exe.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        nShow: 0,
        ..Default::default()
    };
    unsafe {
        if ShellExecuteExW(&mut info).is_err() {
            return false;
        }
        WaitForSingleObject(info.hProcess, INFINITE);
        let mut code = 1u32;
        let _ = GetExitCodeProcess(info.hProcess, &mut code);
        let _ = CloseHandle(info.hProcess);
        code == 0
    }
}

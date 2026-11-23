//! Opt-out auto-updater backed by GitHub Releases.
//!
//! On launch (if enabled) it asks the GitHub API for the latest release. When a
//! newer version exists the UI offers an "Update" button, which downloads the
//! release's `camlite.exe` + `camlite_vcam.dll` to a temp folder and runs the
//! new exe's `--install` (one UAC prompt), then restarts the app.
//! Uses the OS HTTP stack (WinHTTP) — no bundled TLS library.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use windows::Win32::Networking::WinHttp::*;
use windows_core::{HSTRING, PCWSTR};

pub const REPO: &str = "Lancito01/camlite";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const ASSETS: [&str; 2] = [crate::install::EXE_NAME, crate::install::DLL_NAME];

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Idle,
    Checking,
    UpToDate,
    Available { version: String, urls: Vec<String> },
    Downloading { version: String },
    Installing { version: String },
    Failed(String),
}

#[derive(Clone)]
pub struct Updater {
    pub status: Arc<Mutex<Status>>,
}

impl Updater {
    pub fn new() -> Self {
        Updater { status: Arc::new(Mutex::new(Status::Idle)) }
    }

    pub fn get(&self) -> Status {
        self.status.lock().unwrap().clone()
    }

    fn set(&self, s: Status, ctx: &eframe::egui::Context) {
        *self.status.lock().unwrap() = s;
        ctx.request_repaint();
    }

    pub fn check(&self, ctx: &eframe::egui::Context) {
        if matches!(self.get(), Status::Checking | Status::Downloading { .. } | Status::Installing { .. }) {
            return;
        }
        self.set(Status::Checking, ctx);
        let (me, ctx) = (self.clone(), ctx.clone());
        std::thread::spawn(move || {
            let status = match latest_release() {
                Ok(Some((version, urls))) if is_newer(&version, VERSION) => Status::Available { version, urls },
                Ok(_) => Status::UpToDate,
                Err(e) => Status::Failed(e),
            };
            me.set(status, &ctx);
        });
    }

    /// Downloads the release and installs it; on success the app restarts.
    pub fn update(&self, ctx: &eframe::egui::Context) {
        let Status::Available { version, urls } = self.get() else { return };
        self.set(Status::Downloading { version: version.clone() }, ctx);
        let (me, ctx) = (self.clone(), ctx.clone());
        std::thread::spawn(move || {
            let dir = std::env::temp_dir().join(format!("camlite-update-{version}"));
            let result = (|| -> Result<PathBuf, String> {
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                for name in ASSETS {
                    let url = urls
                        .iter()
                        .find(|u| u.rsplit('/').next() == Some(name))
                        .ok_or_else(|| format!("release is missing {name}"))?;
                    let bytes = http_get(url)?;
                    if bytes.len() < 1024 || &bytes[..2] != b"MZ" {
                        return Err(format!("{name} download is not a valid binary"));
                    }
                    std::fs::write(dir.join(name), bytes).map_err(|e| e.to_string())?;
                }
                Ok(dir.join(crate::install::EXE_NAME))
            })();
            match result {
                Ok(exe) => {
                    me.set(Status::Installing { version: version.clone() }, &ctx);
                    if crate::install::run_elevated_exe(&exe, "--install") {
                        let _ = std::process::Command::new(crate::install::install_dir().join(crate::install::EXE_NAME))
                            .arg("--after-update")
                            .spawn();
                        std::process::exit(0);
                    }
                    me.set(Status::Failed("Update was cancelled".into()), &ctx);
                }
                Err(e) => me.set(Status::Failed(e), &ctx),
            }
        });
    }
}

/// (version without "v", asset download URLs) of the latest release.
fn latest_release() -> Result<Option<(String, Vec<String>)>, String> {
    let body = http_get(&format!("https://api.github.com/repos/{REPO}/releases/latest"))?;
    let json = String::from_utf8_lossy(&body);
    let Some(tag) = json_strings(&json, "tag_name").into_iter().next() else { return Ok(None) };
    let urls = json_strings(&json, "browser_download_url");
    Ok(Some((tag.trim_start_matches('v').to_string(), urls)))
}

/// All string values of `"key": "value"` pairs (enough for GitHub's API output).
fn json_strings(json: &str, key: &str) -> Vec<String> {
    let needle = format!("\"{key}\"");
    let mut out = Vec::new();
    let mut rest = json;
    while let Some(i) = rest.find(&needle) {
        rest = &rest[i + needle.len()..];
        let after = rest.trim_start();
        let Some(after) = after.strip_prefix(':') else { continue };
        let Some(after) = after.trim_start().strip_prefix('"') else { continue };
        if let Some(end) = after.find('"') {
            out.push(after[..end].replace("\\/", "/"));
        }
    }
    out
}

fn parse_version(v: &str) -> (u32, u32, u32) {
    let mut it = v.split(['.', '-']).map(|p| p.parse::<u32>().unwrap_or(0));
    (it.next().unwrap_or(0), it.next().unwrap_or(0), it.next().unwrap_or(0))
}

fn is_newer(candidate: &str, current: &str) -> bool {
    parse_version(candidate) > parse_version(current)
}

struct Handle(*mut core::ffi::c_void);
impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
}

/// HTTPS GET following redirects (GitHub assets redirect to a CDN).
fn http_get(url: &str) -> Result<Vec<u8>, String> {
    let rest = url.strip_prefix("https://").ok_or("only https is allowed")?;
    let (host, path) = rest.split_once('/').map(|(h, p)| (h, format!("/{p}"))).unwrap_or((rest, "/".into()));
    let err = |what: &str| format!("{what} failed ({})", windows_core::Error::from_thread().code());
    unsafe {
        let ua = HSTRING::from(format!("CamLite/{VERSION}"));
        let session = Handle(WinHttpOpen(&ua, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0));
        if session.0.is_null() {
            return Err(err("WinHttpOpen"));
        }
        let conn = Handle(WinHttpConnect(session.0, &HSTRING::from(host), INTERNET_DEFAULT_HTTPS_PORT, 0));
        if conn.0.is_null() {
            return Err(err("connect"));
        }
        let req = Handle(WinHttpOpenRequest(
            conn.0,
            &HSTRING::from("GET"),
            &HSTRING::from(path),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        ));
        if req.0.is_null() {
            return Err(err("request"));
        }
        let headers: Vec<u16> = "Accept: application/vnd.github+json, application/octet-stream\r\n".encode_utf16().collect();
        WinHttpSendRequest(req.0, Some(&headers), None, 0, 0, 0).map_err(|e| e.message())?;
        WinHttpReceiveResponse(req.0, std::ptr::null_mut()).map_err(|e| e.message())?;
        let mut status = 0u32;
        let mut len = 4u32;
        let mut index = 0u32;
        WinHttpQueryHeaders(
            req.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some(&mut status as *mut u32 as _),
            &mut len,
            &mut index,
        )
        .map_err(|e| e.message())?;
        if status == 404 {
            return Ok(Vec::new());
        }
        if status != 200 {
            return Err(format!("HTTP {status}"));
        }
        let mut body = Vec::new();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let mut read = 0u32;
            WinHttpReadData(req.0, buf.as_mut_ptr() as _, buf.len() as u32, &mut read).map_err(|e| e.message())?;
            if read == 0 {
                break;
            }
            body.extend_from_slice(&buf[..read as usize]);
            if body.len() > 64 * 1024 * 1024 {
                return Err("download too large".into());
            }
        }
        Ok(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn versions() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        let j = r#"{"tag_name": "v0.2.0", "assets":[{"browser_download_url": "https://x/camlite.exe"}]}"#;
        assert_eq!(json_strings(j, "tag_name"), vec!["v0.2.0"]);
        assert_eq!(json_strings(j, "browser_download_url"), vec!["https://x/camlite.exe"]);
    }
}

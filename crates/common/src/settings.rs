//! Persistent settings, stored as a small INI-style file in `%ProgramData%\CamLite`.
//!
//! Each camera gets its own profile (keyed by its device path), so switching
//! or re-plugging cameras restores that camera's resolution and image settings.
//!
//! ```text
//! device=<path of the selected camera>
//! auto_update=1
//! [camera <device path>]
//! mode=1920x1080@30
//! cam.3=150,manual
//! ```

use crate::camera::{Ctrl, Mode};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Settings of one physical camera.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CameraProfile {
    /// Output mode of the virtual camera (None = best available).
    pub mode: Option<Mode>,
    /// Hardware controls the user changed: value + auto flag.
    pub controls: BTreeMap<Ctrl, (i32, bool)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// Device path of the selected camera ("" = pick automatically).
    pub device: String,
    /// Check GitHub for new versions when the app opens.
    pub auto_update: bool,
    /// Per-camera profiles, keyed by lower-cased device path.
    pub cameras: BTreeMap<String, CameraProfile>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { device: String::new(), auto_update: true, cameras: BTreeMap::new() }
    }
}

pub fn path() -> PathBuf {
    crate::data_dir().join("settings.ini")
}

fn key(link: &str) -> String {
    link.to_ascii_lowercase()
}

impl Settings {
    pub fn profile(&self, link: &str) -> CameraProfile {
        self.cameras.get(&key(link)).cloned().unwrap_or_default()
    }

    pub fn profile_mut(&mut self, link: &str) -> &mut CameraProfile {
        self.cameras.entry(key(link)).or_default()
    }

    pub fn load() -> Self {
        std::fs::read_to_string(path()).map(|t| Self::parse(&t)).unwrap_or_default()
    }

    pub fn parse(text: &str) -> Self {
        let mut s = Settings::default();
        let mut section: Option<String> = None;
        let mut legacy = CameraProfile::default();
        for line in text.lines() {
            let line = line.trim();
            if let Some(name) = line.strip_prefix("[camera ").and_then(|l| l.strip_suffix(']')) {
                section = Some(key(name));
                continue;
            }
            let Some((k, v)) = line.split_once('=') else { continue };
            let (k, v) = (k.trim(), v.trim());
            let profile = match &section {
                Some(sec) => s.cameras.entry(sec.clone()).or_default(),
                None => match k {
                    "device" => {
                        s.device = v.to_string();
                        continue;
                    }
                    "auto_update" => {
                        s.auto_update = v != "0";
                        continue;
                    }
                    // Pre-0.1 files kept one global profile.
                    _ => &mut legacy,
                },
            };
            if k == "mode" {
                profile.mode = Mode::parse(v);
            } else if let Some(c) = Ctrl::parse_key(k) {
                let mut it = v.split(',');
                if let Some(val) = it.next().and_then(|x| x.parse().ok()) {
                    profile.controls.insert(c, (val, it.next() == Some("auto")));
                }
            }
        }
        if legacy != CameraProfile::default() && !s.device.is_empty() {
            s.cameras.entry(key(&s.device)).or_insert(legacy);
        }
        s
    }

    pub fn save(&self) -> std::io::Result<()> {
        let out = self.serialize();
        std::fs::create_dir_all(crate::data_dir())?;
        // Write-then-rename so the DLL never reads a half-written file.
        let tmp = path().with_extension("tmp");
        std::fs::write(&tmp, out)?;
        std::fs::rename(tmp, path())
    }

    pub fn serialize(&self) -> String {
        let mut out = format!("device={}\nauto_update={}\n", self.device, self.auto_update as u8);
        for (link, p) in &self.cameras {
            if *p == CameraProfile::default() {
                continue;
            }
            out.push_str(&format!("\n[camera {link}]\n"));
            if let Some(m) = p.mode {
                out.push_str(&format!("mode={}\n", m.key()));
            }
            for (c, (v, auto)) in &p.controls {
                out.push_str(&format!("{}={},{}\n", c.key(), v, if *auto { "auto" } else { "manual" }));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut s = Settings { device: r"\\?\USB#VID_046D".into(), auto_update: false, ..Default::default() };
        let p = s.profile_mut(r"\\?\usb#vid_046d");
        p.mode = Some(Mode { width: 1280, height: 720, fps: 60 });
        p.controls.insert(Ctrl::ZOOM, (200, false));
        p.controls.insert(Ctrl::WHITE_BALANCE, (4000, true));
        s.profile_mut("other").controls.insert(Ctrl::BRIGHTNESS, (90, false));
        assert_eq!(Settings::parse(&s.serialize()), s);
    }

    #[test]
    fn legacy_file_becomes_profile() {
        let s = Settings::parse("device=CamA\nmode=1920x1080@30\ncam.3=150,manual\n");
        let p = s.profile("cama");
        assert_eq!(p.mode, Some(Mode { width: 1920, height: 1080, fps: 30 }));
        assert_eq!(p.controls.get(&Ctrl::ZOOM), Some(&(150, false)));
        assert!(s.auto_update);
    }
}

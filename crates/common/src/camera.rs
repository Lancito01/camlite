//! Media Foundation helpers: device enumeration, native formats and UVC controls.

use crate::settings::{CameraProfile, Settings};
use windows::Win32::Media::DirectShow::{IAMCameraControl, IAMVideoProcAmp};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::CoTaskMemFree;
use windows_core::{Interface, Result, GUID, HSTRING, PWSTR};

pub fn mf_startup() -> Result<()> {
    unsafe { MFStartup(MF_VERSION, MFSTARTUP_LITE) }
}

// ---------------------------------------------------------------- devices --

#[derive(Clone, Debug, PartialEq)]
pub struct DeviceInfo {
    pub name: String,
    pub link: String,
}

fn get_string(a: &IMFAttributes, key: &GUID) -> Option<String> {
    unsafe {
        let mut p = PWSTR::null();
        let mut len = 0u32;
        a.GetAllocatedString(key, &mut p, &mut len).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as _));
        s
    }
}

/// Physical video capture devices (virtual cameras are filtered out).
pub fn list_devices() -> Result<Vec<DeviceInfo>> {
    Ok(enumerate()?.into_iter().filter(|(_, v)| !v).map(|(d, _)| d).collect())
}

/// Our own "CamLite Camera", if registered.
pub fn find_virtual_camera() -> Option<DeviceInfo> {
    enumerate().ok()?.into_iter().find(|(d, v)| *v && d.name.starts_with(crate::VCAM_NAME)).map(|(d, _)| d)
}

/// All devices, flagged `true` when they are Windows virtual cameras.
fn enumerate() -> Result<Vec<(DeviceInfo, bool)>> {
    let mut out = Vec::new();
    unsafe {
        let mut attrs = None;
        MFCreateAttributes(&mut attrs, 1)?;
        let attrs = attrs.unwrap();
        attrs.SetGUID(&MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID)?;
        let mut pp: *mut Option<IMFActivate> = std::ptr::null_mut();
        let mut n = 0u32;
        MFEnumDeviceSources(&attrs, &mut pp, &mut n)?;
        if !pp.is_null() {
            for a in std::slice::from_raw_parts_mut(pp, n as usize) {
                if let Some(act) = a.take() {
                    let name = get_string(&act, &MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME).unwrap_or_default();
                    let link =
                        get_string(&act, &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK).unwrap_or_default();
                    let virtual_cam = name.starts_with(crate::VCAM_NAME) || link.to_ascii_lowercase().contains("vcamdevapi");
                    if !link.is_empty() {
                        out.push((DeviceInfo { name, link }, virtual_cam));
                    }
                }
            }
            CoTaskMemFree(Some(pp as _));
        }
    }
    Ok(out)
}

/// The device the settings point at, or the best default (prefer Logitech).
pub fn pick_device(devices: &[DeviceInfo], wanted: &str) -> Option<DeviceInfo> {
    devices
        .iter()
        .find(|d| !wanted.is_empty() && d.link.eq_ignore_ascii_case(wanted))
        .or_else(|| devices.iter().find(|d| d.link.to_ascii_lowercase().contains("vid_046d")))
        .or_else(|| devices.first())
        .cloned()
}

pub fn open_device(link: &str) -> Result<IMFMediaSource> {
    unsafe {
        let mut attrs = None;
        MFCreateAttributes(&mut attrs, 2)?;
        let attrs = attrs.unwrap();
        attrs.SetGUID(&MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID)?;
        attrs.SetString(&MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK, &HSTRING::from(link))?;
        MFCreateDeviceSource(&attrs)
    }
}

// ------------------------------------------------------------------ modes --

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Mode {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

impl Mode {
    pub fn key(&self) -> String {
        format!("{}x{}@{}", self.width, self.height, self.fps)
    }
    pub fn parse(s: &str) -> Option<Mode> {
        let (wh, fps) = s.split_once('@')?;
        let (w, h) = wh.split_once('x')?;
        Some(Mode { width: w.parse().ok()?, height: h.parse().ok()?, fps: fps.parse().ok()? })
    }
    pub fn label(&self) -> String {
        format!("{}p · {} fps", self.height, self.fps)
    }
}

#[derive(Clone, Debug)]
pub struct NativeType {
    pub subtype: GUID,
    pub mode: Mode,
}

pub fn type_info(mt: &IMFMediaType) -> Option<NativeType> {
    unsafe {
        let subtype = mt.GetGUID(&MF_MT_SUBTYPE).ok()?;
        let size = mt.GetUINT64(&MF_MT_FRAME_SIZE).ok()?;
        let rate = mt.GetUINT64(&MF_MT_FRAME_RATE).ok()?;
        let (num, den) = ((rate >> 32) as u32, rate as u32);
        let fps = if den == 0 { 0 } else { ((num as f64) / (den as f64)).round() as u32 };
        Some(NativeType { subtype, mode: Mode { width: (size >> 32) as u32, height: size as u32, fps } })
    }
}

/// All native media types of the first stream of a device source.
pub fn native_types(source: &IMFMediaSource) -> Result<Vec<(IMFMediaType, NativeType)>> {
    let mut out = Vec::new();
    unsafe {
        let pd = source.CreatePresentationDescriptor()?;
        let mut selected = Default::default();
        let mut sd = None;
        pd.GetStreamDescriptorByIndex(0, &mut selected, &mut sd)?;
        let handler = sd.unwrap().GetMediaTypeHandler()?;
        for i in 0..handler.GetMediaTypeCount()? {
            let mt = handler.GetMediaTypeByIndex(i)?;
            if let Some(info) = type_info(&mt) {
                out.push((mt, info));
            }
        }
    }
    Ok(out)
}

/// Useful output modes: 16:9, at least 720p, best frame rate per resolution.
pub fn modes_from(types: &[NativeType]) -> Vec<Mode> {
    let mut best: Vec<Mode> = Vec::new();
    for t in types {
        let m = t.mode;
        if m.width * 9 != m.height * 16 || m.height < 720 || m.fps < 15 {
            continue;
        }
        match best.iter_mut().find(|b| b.width == m.width && b.height == m.height) {
            Some(b) if b.fps < m.fps => b.fps = m.fps,
            Some(_) => {}
            None => best.push(m),
        }
    }
    best.sort_by(|a, b| b.height.cmp(&a.height));
    best
}

/// The output mode to use for a camera: the saved one if the camera supports
/// it, otherwise its best HD mode (1080p preferred).
pub fn resolve_mode(settings: &Settings, dev: &DeviceInfo, source: Option<&IMFMediaSource>) -> Mode {
    let owned;
    let source = match source {
        Some(s) => Some(s),
        None => {
            owned = open_device(&dev.link).ok();
            owned.as_ref()
        }
    };
    let modes = source
        .and_then(|s| native_types(s).ok())
        .map(|t| modes_from(&t.into_iter().map(|(_, i)| i).collect::<Vec<_>>()))
        .unwrap_or_default();
    let saved = settings.profile(&dev.link).mode;
    if let Some(m) = saved.filter(|m| modes.contains(m)) {
        return m;
    }
    modes
        .iter()
        .find(|m| m.height == 1080)
        .or(modes.first())
        .copied()
        .unwrap_or(Mode { width: 1280, height: 720, fps: 30 })
}

/// Picks the native type to capture `mode` with, preferring compressed MJPEG
/// (raw YUY2 cannot do HD at full frame rate over USB 2).
pub fn best_native<'a, T>(types: &'a [(T, NativeType)], mode: Mode) -> Option<&'a (T, NativeType)> {
    let score = |t: &NativeType| -> i32 {
        if t.mode != mode {
            return -1;
        }
        if t.subtype == MFVideoFormat_MJPG {
            3
        } else if t.subtype == MFVideoFormat_NV12 {
            2
        } else {
            1
        }
    };
    types.iter().filter(|(_, t)| score(t) >= 0).max_by_key(|(_, t)| score(t))
}

// --------------------------------------------------------------- controls --

/// A UVC control, addressed the DirectShow way.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Ctrl {
    /// `VideoProcAmpProperty` (brightness, contrast, …)
    Amp(i32),
    /// `CameraControlProperty` (zoom, pan, exposure, focus, …)
    Cam(i32),
}

impl Ctrl {
    pub const BRIGHTNESS: Ctrl = Ctrl::Amp(0);
    pub const CONTRAST: Ctrl = Ctrl::Amp(1);
    pub const SATURATION: Ctrl = Ctrl::Amp(3);
    pub const SHARPNESS: Ctrl = Ctrl::Amp(4);
    pub const WHITE_BALANCE: Ctrl = Ctrl::Amp(7);
    pub const BACKLIGHT: Ctrl = Ctrl::Amp(8);
    pub const GAIN: Ctrl = Ctrl::Amp(9);
    pub const PAN: Ctrl = Ctrl::Cam(0);
    pub const TILT: Ctrl = Ctrl::Cam(1);
    pub const ZOOM: Ctrl = Ctrl::Cam(3);
    pub const EXPOSURE: Ctrl = Ctrl::Cam(4);
    pub const FOCUS: Ctrl = Ctrl::Cam(6);
    /// UVC "auto-exposure priority": when on (camera default) the camera may
    /// halve the frame rate in dim light. Only on/off (0/1), no range.
    pub const LOW_LIGHT_PRIORITY: Ctrl = Ctrl::Cam(19);

    pub fn key(&self) -> String {
        match self {
            Ctrl::Amp(i) => format!("amp.{i}"),
            Ctrl::Cam(i) => format!("cam.{i}"),
        }
    }
    pub fn parse_key(s: &str) -> Option<Ctrl> {
        let (kind, id) = s.split_once('.')?;
        let id = id.parse().ok()?;
        match kind {
            "amp" => Some(Ctrl::Amp(id)),
            "cam" => Some(Ctrl::Cam(id)),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Range {
    pub min: i32,
    pub max: i32,
    pub step: i32,
    pub default: i32,
    pub can_auto: bool,
}

const FLAG_AUTO: i32 = 1;
const FLAG_MANUAL: i32 = 2;

/// Hardware image controls of one physical camera. Values are applied by the
/// camera itself, so they cost nothing at runtime.
#[derive(Clone)]
pub struct Controls {
    amp: Option<IAMVideoProcAmp>,
    cam: Option<IAMCameraControl>,
}

impl Controls {
    pub fn new(source: &IMFMediaSource) -> Self {
        Controls { amp: source.cast().ok(), cam: source.cast().ok() }
    }

    pub fn range(&self, c: Ctrl) -> Option<Range> {
        let (mut min, mut max, mut step, mut def, mut caps) = (0, 0, 0, 0, 0);
        unsafe {
            match c {
                Ctrl::Amp(p) => self.amp.as_ref()?.GetRange(p, &mut min, &mut max, &mut step, &mut def, &mut caps),
                Ctrl::Cam(p) => self.cam.as_ref()?.GetRange(p, &mut min, &mut max, &mut step, &mut def, &mut caps),
            }
            .ok()?;
        }
        if max <= min {
            return None;
        }
        Some(Range { min, max, step: step.max(1), default: def, can_auto: caps & FLAG_AUTO != 0 })
    }

    pub fn get(&self, c: Ctrl) -> Option<(i32, bool)> {
        let (mut v, mut f) = (0, 0);
        unsafe {
            match c {
                Ctrl::Amp(p) => self.amp.as_ref()?.Get(p, &mut v, &mut f),
                Ctrl::Cam(p) => self.cam.as_ref()?.Get(p, &mut v, &mut f),
            }
            .ok()?;
        }
        Some((v, f & FLAG_AUTO != 0))
    }

    pub fn set(&self, c: Ctrl, value: i32, auto: bool) -> Result<()> {
        let flags = if auto { FLAG_AUTO } else { FLAG_MANUAL };
        let not_supported = || windows_core::Error::from(windows::Win32::Foundation::E_NOTIMPL);
        unsafe {
            match c {
                Ctrl::Amp(p) => self.amp.as_ref().ok_or_else(not_supported)?.Set(p, value, flags),
                Ctrl::Cam(p) => self.cam.as_ref().ok_or_else(not_supported)?.Set(p, value, flags),
            }
        }
    }

    /// Pushes every saved control to the camera. Errors are ignored per control.
    pub fn apply(&self, s: &CameraProfile) {
        // Zoom must go before pan/tilt: most cameras clamp pan/tilt at 1x zoom.
        let mut ordered: Vec<_> = s.controls.iter().collect();
        ordered.sort_by_key(|(c, _)| if **c == Ctrl::ZOOM { 0 } else { 1 });
        for (c, (v, auto)) in ordered {
            let _ = self.set(*c, *v, *auto);
        }
        // Default to a constant frame rate unless the user opted back in.
        if !s.controls.contains_key(&Ctrl::LOW_LIGHT_PRIORITY) {
            let _ = self.set(Ctrl::LOW_LIGHT_PRIORITY, 0, false);
        }
    }
}

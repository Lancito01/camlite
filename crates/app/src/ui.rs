//! The settings window.

use crate::preview::Preview;
use crate::updater::{self, Status, Updater};
use crate::{install, startup};
use camlite_common::camera::{self, Controls, Ctrl, DeviceInfo, Mode, Range};
use camlite_common::settings::{CameraProfile, Settings};
use eframe::egui::{
    self, Align, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, Layout, Margin, RichText, Sense, Stroke,
    TextureHandle, TextureOptions, Vec2,
};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use windows::Win32::Media::MediaFoundation::IMFMediaSource;

/// Set when this process was started by the updater.
pub static AFTER_UPDATE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

const BG: Color32 = Color32::from_rgb(0x0e, 0x0f, 0x11);
const CARD: Color32 = Color32::from_rgb(0x17, 0x18, 0x1b);
const RAISED: Color32 = Color32::from_rgb(0x23, 0x25, 0x29);
const TEXT: Color32 = Color32::from_rgb(0xec, 0xed, 0xef);
const MUTED: Color32 = Color32::from_rgb(0x8a, 0x8f, 0x98);
const ACCENT: Color32 = Color32::from_rgb(0x5b, 0x8c, 0xff);
const GOOD: Color32 = Color32::from_rgb(0x3d, 0xd6, 0x8c);

#[derive(Clone, Copy)]
struct CtrlState {
    range: Range,
    value: i32,
    auto: bool,
}

pub struct App {
    devices: Vec<DeviceInfo>,
    device: Option<DeviceInfo>,
    _device_source: Option<IMFMediaSource>,
    controls: Option<Controls>,
    modes: Vec<Mode>,
    settings: Settings,
    saved: Settings,
    ctrl: BTreeMap<Ctrl, CtrlState>,
    smooth_low_light: bool,
    preview_on: bool,
    preview: Option<Preview>,
    texture: Option<TextureHandle>,
    last_seq: u64,
    changed_at: Option<Instant>,
    installed: bool,
    vcam_link: Option<String>,
    focused: bool,
    job: Option<Arc<Mutex<Option<bool>>>>,
    start_with_windows: bool,
    last_scan: Instant,
    updater: Updater,
    update_checked: bool,
}

const KNOWN: &[Ctrl] = &[
    Ctrl::ZOOM,
    Ctrl::PAN,
    Ctrl::TILT,
    Ctrl::FOCUS,
    Ctrl::BRIGHTNESS,
    Ctrl::CONTRAST,
    Ctrl::SATURATION,
    Ctrl::SHARPNESS,
    Ctrl::WHITE_BALANCE,
    Ctrl::EXPOSURE,
    Ctrl::GAIN,
    Ctrl::BACKLIGHT,
];

pub fn setup_style(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    for (name, file) in [("segoe", r"C:\Windows\Fonts\SegUIVar.ttf"), ("segoe-fallback", r"C:\Windows\Fonts\segoeui.ttf")] {
        if let Ok(bytes) = std::fs::read(file) {
            fonts.font_data.insert(name.into(), Arc::new(FontData::from_owned(bytes)));
            fonts.families.get_mut(&FontFamily::Proportional).unwrap().insert(0, name.into());
            break;
        }
    }
    ctx.set_fonts(fonts);

    let mut style = (*ctx.global_style()).clone();
    style.spacing.item_spacing = Vec2::new(10.0, 10.0);
    style.spacing.slider_rail_height = 4.0;
    style.spacing.interact_size.y = 22.0;
    let v = &mut style.visuals;
    *v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = CARD;
    v.extreme_bg_color = RAISED;
    v.override_text_color = Some(TEXT);
    v.selection.bg_fill = ACCENT;
    v.selection.stroke = Stroke::new(1.0, TEXT);
    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Circle;
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.noninteractive] {
        w.corner_radius = CornerRadius::same(8);
        w.bg_stroke = Stroke::NONE;
    }
    v.widgets.inactive.bg_fill = RAISED;
    v.widgets.inactive.weak_bg_fill = RAISED;
    v.widgets.hovered.bg_fill = Color32::from_rgb(0x2d, 0x30, 0x35);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0x2d, 0x30, 0x35);
    v.widgets.active.bg_fill = ACCENT;
    v.widgets.active.weak_bg_fill = Color32::from_rgb(0x34, 0x37, 0x3d);
    v.widgets.inactive.fg_stroke = Stroke::new(1.5, TEXT);
    v.widgets.hovered.fg_stroke = Stroke::new(1.5, TEXT);
    v.widgets.active.fg_stroke = Stroke::new(1.5, TEXT);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x26, 0x28, 0x2d));
    ctx.set_global_style(style);
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup_style(&cc.egui_ctx);
        let settings = Settings::load();
        let devices = camera::list_devices().unwrap_or_default();
        let mut app = App {
            devices,
            device: None,
            _device_source: None,
            controls: None,
            modes: Vec::new(),
            saved: settings.clone(),
            settings,
            ctrl: BTreeMap::new(),
            smooth_low_light: true,
            preview_on: true,
            preview: None,
            texture: None,
            last_seq: 0,
            changed_at: None,
            installed: install::is_installed(),
            vcam_link: camera::find_virtual_camera().map(|d| d.link),
            focused: true,
            job: None,
            start_with_windows: startup::is_enabled(),
            last_scan: Instant::now(),
            updater: Updater::new(),
            update_checked: false,
        };
        if AFTER_UPDATE.load(std::sync::atomic::Ordering::SeqCst) {
            *app.updater.status.lock().unwrap() = Status::Updated;
            app.update_checked = true;
        }
        let wanted = app.settings.device.clone();
        app.select_device(camera::pick_device(&app.devices, &wanted));
        app
    }

    fn select_device(&mut self, dev: Option<DeviceInfo>) {
        self.preview = None;
        self.texture = None;
        self.ctrl.clear();
        self.modes.clear();
        self.controls = None;
        self._device_source = None;
        self.device = dev.clone();
        let Some(dev) = dev else { return };
        // Each camera keeps its own profile; switching just loads that one.
        self.settings.device = dev.link.clone();
        let Ok(src) = camera::open_device(&dev.link) else { return };
        if let Ok(types) = camera::native_types(&src) {
            let infos: Vec<_> = types.into_iter().map(|(_, t)| t).collect();
            self.modes = camera::modes_from(&infos);
        }
        let mode = camera::resolve_mode(&self.settings, &dev, Some(&src));
        self.settings.profile_mut(&dev.link).mode = Some(mode);
        let controls = Controls::new(&src);
        controls.apply(&self.settings.profile(&dev.link));
        for &c in KNOWN {
            if let (Some(range), Some((value, auto))) = (controls.range(c), controls.get(c)) {
                self.ctrl.insert(c, CtrlState { range, value, auto });
            }
        }
        self.smooth_low_light = controls.get(Ctrl::LOW_LIGHT_PRIORITY).is_none_or(|(v, _)| v == 0);
        self.controls = Some(controls);
        self._device_source = Some(src);
        self.mark_changed();
    }

    fn link(&self) -> String {
        self.device.as_ref().map(|d| d.link.clone()).unwrap_or_default()
    }

    fn mode(&self) -> Option<Mode> {
        self.device.as_ref().and_then(|d| self.settings.profile(&d.link).mode)
    }

    fn profile_mut(&mut self) -> &mut CameraProfile {
        let link = self.link();
        self.settings.profile_mut(&link)
    }

    /// Re-enumerates cameras (hot-plug) and follows the change.
    fn refresh_devices(&mut self) {
        let Ok(devices) = camera::list_devices() else { return };
        if devices == self.devices {
            return;
        }
        self.devices = devices;
        let still_there = self.device.as_ref().is_some_and(|d| self.devices.contains(d));
        // Prefer the camera the user picked; otherwise the best available.
        let wanted = self.settings.device.clone();
        let best = camera::pick_device(&self.devices, &wanted);
        if !still_there || best.as_ref().is_some_and(|b| b.link.eq_ignore_ascii_case(&wanted) && Some(b) != self.device.as_ref()) {
            let keep = self.settings.device.clone();
            self.select_device(best);
            if self.device.is_none() {
                self.settings.device = keep;
            }
        }
    }

    fn mark_changed(&mut self) {
        self.changed_at = Some(Instant::now());
    }

    fn set_ctrl(&mut self, c: Ctrl, value: i32, auto: bool) {
        if let Some(ctrls) = &self.controls {
            let _ = ctrls.set(c, value, auto);
        }
        if let Some(s) = self.ctrl.get_mut(&c) {
            s.value = value;
            s.auto = auto;
        }
        self.profile_mut().controls.insert(c, (value, auto));
        self.mark_changed();
    }

    fn reset(&mut self) {
        self.profile_mut().controls.clear();
        let defaults: Vec<_> = self.ctrl.iter().map(|(c, s)| (*c, s.range.default, s.range.can_auto)).collect();
        // Zoom first so pan/tilt can be reset.
        for (c, v, auto) in defaults.iter().filter(|d| d.0 == Ctrl::ZOOM).chain(defaults.iter().filter(|d| d.0 != Ctrl::ZOOM)) {
            self.set_ctrl(*c, *v, *auto);
        }
        self.profile_mut().controls.clear();
        self.smooth_low_light = true;
        if let Some(ctrls) = &self.controls {
            let _ = ctrls.set(Ctrl::LOW_LIGHT_PRIORITY, 0, false);
        }
        self.mark_changed();
    }

    fn run_job(&mut self, arg: &'static str) {
        let slot = Arc::new(Mutex::new(None));
        let s = slot.clone();
        std::thread::spawn(move || {
            let ok = install::run_elevated(arg);
            *s.lock().unwrap() = Some(ok);
        });
        self.job = Some(slot);
    }

    // ------------------------------------------------------------ logic --

    fn tick(&mut self, ctx: &egui::Context) {
        // Debounced save.
        if let Some(t) = self.changed_at {
            if t.elapsed() > Duration::from_millis(400) {
                if self.settings != self.saved && self.settings.save().is_ok() {
                    self.saved = self.settings.clone();
                }
                self.changed_at = None;
            } else {
                ctx.request_repaint_after(Duration::from_millis(450));
            }
        }

        // Hot-plug: look for camera changes every couple of seconds while open.
        if self.last_scan.elapsed() > Duration::from_secs(2) {
            self.last_scan = Instant::now();
            self.refresh_devices();
        }
        ctx.request_repaint_after(Duration::from_secs(2));

        // Auto-update check once per launch (opt-out).
        if !self.update_checked {
            self.update_checked = true;
            if self.settings.auto_update {
                self.updater.check(ctx);
            }
        }

        // Elevated install/uninstall finished?
        if let Some(job) = &self.job {
            if job.lock().unwrap().is_some() {
                self.job = None;
                self.installed = install::is_installed();
                self.vcam_link = camera::find_virtual_camera().map(|d| d.link);
                self.preview = None;
            } else {
                ctx.request_repaint_after(Duration::from_millis(250));
            }
        }

        // Preview runs only while the window is visible and focused, so it never
        // keeps the camera from an app you switch to (cameras are single-client
        // unless Windows' multi-app camera access is turned on).
        let visible = ctx.input(|i| !i.viewport().minimized.unwrap_or(false) && i.viewport().focused.unwrap_or(true));
        self.focused = visible;
        let want = match (&self.device, self.mode()) {
            (Some(d), Some(m)) if self.preview_on && visible => {
                let link = if self.installed { self.vcam_link.clone() } else { None };
                Some((link.unwrap_or_else(|| d.link.clone()), m))
            }
            _ => None,
        };
        let current = self.preview.as_ref().map(|p| (p.link.clone(), p.mode));
        if want != current {
            self.preview = None;
            if let Some((link, mode)) = want {
                self.preview = Some(Preview::start(link, mode, ctx.clone()));
            }
        }
        if self.preview.is_none() {
            self.texture = None;
        }
        if let Some(p) = &self.preview {
            if let Some(f) = p.shared.frame.lock().unwrap().take() {
                if f.seq != self.last_seq {
                    self.last_seq = f.seq;
                    let img = egui::ColorImage::from_rgba_unmultiplied(f.size, &f.rgba);
                    match &mut self.texture {
                        Some(t) => t.set(img, TextureOptions::LINEAR),
                        None => self.texture = Some(ctx.load_texture("preview", img, TextureOptions::LINEAR)),
                    }
                }
            }
        }
    }

    // --------------------------------------------------------------- ui --

    fn draw(&mut self, ui: &mut egui::Ui) {
        self.header(ui);
        ui.add_space(4.0);
        self.preview_ui(ui);
        ui.add_space(12.0);

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.set_width(ui.available_width());
            if !self.modes.is_empty() {
                section(ui, "Video", |ui| self.video_ui(ui));
            }
            if self.has_any(&[Ctrl::ZOOM, Ctrl::PAN, Ctrl::TILT, Ctrl::FOCUS]) {
                section(ui, "Framing", |ui| self.framing_ui(ui));
            }
            if self.has_any(&[Ctrl::BRIGHTNESS, Ctrl::CONTRAST, Ctrl::SATURATION, Ctrl::SHARPNESS, Ctrl::WHITE_BALANCE]) {
                section(ui, "Color", |ui| self.color_ui(ui));
            }
            if self.has_any(&[Ctrl::EXPOSURE, Ctrl::GAIN, Ctrl::BACKLIGHT]) {
                section(ui, "Light", |ui| self.light_ui(ui));
            }
            section(ui, "App", |ui| self.app_ui(ui));
            ui.add_space(4.0);
        });
    }

    fn has_any(&self, cs: &[Ctrl]) -> bool {
        cs.iter().any(|c| self.ctrl.contains_key(c))
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let mut pick = None;
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(RichText::new("CamLite").size(20.0).strong());
                // Camera picker directly under the title.
                let name = self.device.as_ref().map(|d| d.name.clone()).unwrap_or_else(|| "No camera found".into());
                let mut style = (**ui.style()).clone();
                style.visuals.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
                style.visuals.widgets.inactive.bg_fill = Color32::TRANSPARENT;
                style.spacing.button_padding = Vec2::new(0.0, 2.0);
                ui.set_style(style);
                egui::ComboBox::from_id_salt("camera")
                    .selected_text(RichText::new(name).size(12.5).color(MUTED))
                    .width(0.0)
                    .show_ui(ui, |ui| {
                        if self.devices.is_empty() {
                            ui.label(RichText::new("Plug in a webcam").color(MUTED));
                        }
                        for d in &self.devices {
                            let selected = self.device.as_ref().is_some_and(|x| x.link == d.link);
                            let mut label = d.name.clone();
                            if !d.link.to_ascii_lowercase().contains("vid_046d") {
                                label.push_str("  (non-Logitech, untested)");
                            }
                            if ui.selectable_label(selected, label).clicked() && !selected {
                                pick = Some(d.clone());
                            }
                        }
                    });
            });
            if let Some(d) = pick {
                self.select_device(Some(d));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if self.job.is_some() {
                    ui.spinner();
                } else if self.installed {
                    pill(ui, "Virtual camera ready", GOOD);
                } else if ui.add(accent_button("Set up virtual camera")).clicked() {
                    self.run_job("--install");
                }
            });
        });
    }

    fn preview_ui(&mut self, ui: &mut egui::Ui) {
        let w = ui.available_width();
        let aspect = self.mode().map(|m| m.height as f32 / m.width as f32).unwrap_or(9.0 / 16.0);
        let size = Vec2::new(w, (w * aspect).round());
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        let radius = CornerRadius::same(12);
        match (&self.texture, self.preview_on && self.focused) {
            (Some(tex), true) => {
                egui::Image::new(tex).corner_radius(radius).paint_at(ui, rect);
            }
            _ => {
                ui.painter().rect_filled(rect, radius, CARD);
                let msg = if !self.preview_on {
                    "Preview paused — click to resume".to_string()
                } else if !self.focused {
                    "Preview pauses while CamLite is in the background".to_string()
                } else if self.device.is_none() {
                    "Connect a camera".to_string()
                } else if let Some(e) = self.preview.as_ref().and_then(|p| p.shared.error.lock().unwrap().clone()) {
                    e
                } else {
                    "Starting camera…".to_string()
                };
                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, msg, egui::FontId::proportional(13.0), MUTED);
            }
        }
        if resp.clicked() {
            self.preview_on = !self.preview_on;
            if !self.preview_on {
                self.texture = None;
            }
        }
        resp.on_hover_text(if self.preview_on { "Click to pause preview" } else { "Click to resume preview" });
    }

    fn video_ui(&mut self, ui: &mut egui::Ui) {
        let labels: Vec<String> = self.modes.iter().map(|m| m.label()).collect();
        let mut idx = self.mode().and_then(|m| self.modes.iter().position(|x| *x == m)).unwrap_or(0);
        if segmented(ui, &labels, &mut idx) {
            let mode = self.modes.get(idx).copied();
            self.profile_mut().mode = mode;
            // Saved right away: the preview reopens the virtual camera, which reads it.
            if self.settings.save().is_ok() {
                self.saved = self.settings.clone();
            }
        }
        ui.label(RichText::new("Apps pick up a new resolution the next time they open the camera.").size(11.5).color(MUTED));
    }

    fn framing_ui(&mut self, ui: &mut egui::Ui) {
        let zoomed = self.ctrl.get(&Ctrl::ZOOM).is_some_and(|z| z.value > z.range.min);
        self.slider(ui, Ctrl::ZOOM, "Zoom", true, |v| format!("{:.1}×", v as f32 / 100.0));
        self.slider(ui, Ctrl::PAN, "Pan", zoomed, |v| format!("{v:+}"));
        self.slider(ui, Ctrl::TILT, "Tilt", zoomed, |v| format!("{v:+}"));
        self.auto_slider(ui, Ctrl::FOCUS, "Focus", "Autofocus", |v| v.to_string());
    }

    fn color_ui(&mut self, ui: &mut egui::Ui) {
        self.slider(ui, Ctrl::BRIGHTNESS, "Brightness", true, |v| v.to_string());
        self.slider(ui, Ctrl::CONTRAST, "Contrast", true, |v| v.to_string());
        self.slider(ui, Ctrl::SATURATION, "Saturation", true, |v| v.to_string());
        self.slider(ui, Ctrl::SHARPNESS, "Sharpness", true, |v| v.to_string());
        self.auto_slider(ui, Ctrl::WHITE_BALANCE, "Temperature", "Auto white balance", |v| format!("{v} K"));
    }

    fn light_ui(&mut self, ui: &mut egui::Ui) {
        self.auto_slider(ui, Ctrl::EXPOSURE, "Exposure", "Auto exposure", |v| format!("{v:+}"));
        self.slider(ui, Ctrl::GAIN, "Gain", true, |v| v.to_string());
        if let Some(s) = self.ctrl.get(&Ctrl::BACKLIGHT).copied() {
            let mut on = s.value != 0;
            if toggle_row(ui, "Backlight compensation", &mut on) {
                self.set_ctrl(Ctrl::BACKLIGHT, on as i32, false);
            }
        }
        let mut smooth = self.smooth_low_light;
        if toggle_row(ui, "Keep full frame rate in low light", &mut smooth) {
            self.smooth_low_light = smooth;
            let v = if smooth { 0 } else { 1 };
            if let Some(c) = &self.controls {
                let _ = c.set(Ctrl::LOW_LIGHT_PRIORITY, v, false);
            }
            if smooth {
                self.profile_mut().controls.remove(&Ctrl::LOW_LIGHT_PRIORITY);
            } else {
                self.profile_mut().controls.insert(Ctrl::LOW_LIGHT_PRIORITY, (1, false));
            }
            self.mark_changed();
        }
    }

    fn app_ui(&mut self, ui: &mut egui::Ui) {
        let mut on = self.start_with_windows;
        if toggle_row(ui, "Apply settings when Windows starts", &mut on) {
            startup::set_enabled(on);
            self.start_with_windows = startup::is_enabled();
        }
        let mut auto = self.settings.auto_update;
        if toggle_row(ui, "Check for updates automatically", &mut auto) {
            self.settings.auto_update = auto;
            self.mark_changed();
        }
        ui.horizontal(|ui| {
            let status = self.updater.get();
            let text = match &status {
                Status::Idle => format!("Version {}", updater::VERSION),
                Status::Checking => "Checking for updates…".into(),
                Status::UpToDate => format!("Up to date · v{}", updater::VERSION),
                Status::Updated => format!("Updated to v{} ✓", updater::VERSION),
                Status::Available { version, .. } => format!("Version {version} is available"),
                Status::Downloading { version } => format!("Downloading {version}…"),
                Status::Installing { version } => format!("Installing {version}…"),
                Status::Failed(e) => format!("Update check failed: {e}"),
            };
            ui.label(RichText::new(text).size(12.0).color(MUTED));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| match status {
                Status::Available { .. } => {
                    if ui.add(accent_button("Update")).clicked() {
                        self.updater.update(ui.ctx());
                    }
                }
                Status::Idle | Status::UpToDate | Status::Updated | Status::Failed(_) => {
                    if ui.add(plain_button("Check now")).clicked() {
                        self.updater.check(ui.ctx());
                    }
                }
                _ => {
                    ui.spinner();
                }
            });
        });
        ui.horizontal(|ui| {
            if ui.add(plain_button("Reset to defaults")).clicked() {
                self.reset();
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if self.installed && self.job.is_none() && ui.add(plain_button("Remove virtual camera")).clicked() {
                    self.run_job("--uninstall");
                }
            });
        });
        ui.label(
            RichText::new("In Zoom, Teams, Discord or your browser, choose “CamLite Camera”.")
                .size(11.5)
                .color(MUTED),
        );
    }

    fn slider(&mut self, ui: &mut egui::Ui, c: Ctrl, label: &str, enabled: bool, fmt: impl Fn(i32) -> String) {
        let Some(s) = self.ctrl.get(&c).copied() else { return };
        let mut v = s.value;
        let changed = slider_row(ui, label, &mut v, s.range, enabled && !s.auto, &fmt);
        if changed {
            self.set_ctrl(c, v, false);
        }
    }

    fn auto_slider(&mut self, ui: &mut egui::Ui, c: Ctrl, label: &str, auto_label: &str, fmt: impl Fn(i32) -> String) {
        let Some(s) = self.ctrl.get(&c).copied() else { return };
        if s.range.can_auto {
            let mut auto = s.auto;
            if toggle_row(ui, auto_label, &mut auto) {
                // Switching to manual: start from the value the camera chose.
                let value = self.controls.as_ref().and_then(|ctl| ctl.get(c)).map(|x| x.0).unwrap_or(s.value);
                self.set_ctrl(c, value, auto);
            }
        }
        self.slider(ui, c, label, true, fmt);
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.tick(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Frame::new().fill(BG).inner_margin(Margin::symmetric(18, 16)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(ui.available_height());
            self.draw(ui);
        });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.preview = None;
        if self.settings != self.saved {
            let _ = self.settings.save();
        }
    }
}

// ------------------------------------------------------------ widgets --

fn section(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.label(RichText::new(title.to_uppercase()).size(11.0).color(MUTED).strong());
    ui.add_space(-4.0);
    egui::Frame::new().fill(CARD).corner_radius(12).inner_margin(Margin::symmetric(14, 12)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        add(ui);
    });
    ui.add_space(10.0);
}

const LABEL_W: f32 = 92.0;
const VALUE_W: f32 = 52.0;

fn row(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(Vec2::new(LABEL_W, 22.0), Layout::left_to_right(Align::Center), |ui| {
            ui.set_min_width(LABEL_W);
            ui.label(RichText::new(label).size(13.0));
        });
        add(ui);
    });
}

fn slider_row(ui: &mut egui::Ui, label: &str, v: &mut i32, r: Range, enabled: bool, fmt: &dyn Fn(i32) -> String) -> bool {
    let mut changed = false;
    ui.add_enabled_ui(enabled, |ui| {
        row(ui, label, |ui| {
            ui.spacing_mut().slider_width = (ui.available_width() - VALUE_W - 10.0).max(60.0);
            let slider = egui::Slider::new(v, r.min..=r.max).step_by(r.step as f64).show_value(false);
            let resp = ui.add(slider);
            changed = resp.changed();
            if resp.double_clicked() {
                *v = r.default;
                changed = true;
            }
            resp.on_hover_text("Double-click to reset");
            ui.allocate_ui_with_layout(Vec2::new(VALUE_W, 22.0), Layout::right_to_left(Align::Center), |ui| {
                ui.set_min_width(VALUE_W);
                ui.label(RichText::new(fmt(*v)).size(12.5).color(MUTED));
            });
        });
    });
    changed
}

fn toggle_row(ui: &mut egui::Ui, label: &str, on: &mut bool) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).size(13.0));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            changed = toggle(ui, on).changed();
        });
    });
    changed
}

/// iOS-style switch.
fn toggle(ui: &mut egui::Ui, on: &mut bool) -> egui::Response {
    let size = Vec2::new(34.0, 20.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool_responsive(resp.id, *on);
    let bg = lerp_color(RAISED, ACCENT, t);
    let r = rect.height() / 2.0;
    ui.painter().rect_filled(rect, r, bg);
    let x = egui::lerp((rect.left() + r)..=(rect.right() - r), t);
    ui.painter().circle_filled(egui::pos2(x, rect.center().y), r - 3.0, TEXT);
    resp
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// Segmented control; returns true when the selection changed.
fn segmented(ui: &mut egui::Ui, labels: &[String], selected: &mut usize) -> bool {
    let mut changed = false;
    let h = 32.0;
    let (outer, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), h), Sense::hover());
    ui.painter().rect_filled(outer, 9.0, RAISED);
    let n = labels.len().max(1) as f32;
    let inner = outer.shrink(3.0);
    let w = inner.width() / n;
    for (i, label) in labels.iter().enumerate() {
        let r = egui::Rect::from_min_size(egui::pos2(inner.left() + w * i as f32, inner.top()), Vec2::new(w, inner.height()));
        let resp = ui.interact(r, ui.scope_id().with(("seg", i)), Sense::click());
        if resp.clicked() && *selected != i {
            *selected = i;
            changed = true;
        }
        let active = *selected == i;
        if active {
            ui.painter().rect_filled(r, 7.0, ACCENT);
        } else if resp.hovered() {
            ui.painter().rect_filled(r, 7.0, Color32::from_rgb(0x2d, 0x30, 0x35));
        }
        let color = if active { Color32::WHITE } else { TEXT };
        ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(13.0), color);
    }
    changed
}

fn pill(ui: &mut egui::Ui, text: &str, color: Color32) {
    egui::Frame::new().fill(color.gamma_multiply(0.14)).corner_radius(20).inner_margin(Margin::symmetric(10, 4)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let (r, _) = ui.allocate_exact_size(Vec2::splat(7.0), Sense::hover());
            ui.painter().circle_filled(r.center(), 3.5, color);
            ui.label(RichText::new(text).size(12.0).color(color));
        });
    });
}

fn accent_button(text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.to_string()).size(12.5).color(Color32::WHITE)).fill(ACCENT).corner_radius(8)
}

fn plain_button(text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.to_string()).size(12.5)).fill(RAISED).corner_radius(8)
}

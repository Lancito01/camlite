#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod install;
mod preview;
mod startup;
mod ui;
mod updater;

use camlite_common::camera::{self, Controls};
use camlite_common::settings::Settings;
use eframe::egui;
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, SetForegroundWindow, ShowWindow, SW_RESTORE};
use windows_core::{w, PCWSTR};

fn main() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    let _ = camera::mf_startup();
    let arg = std::env::args().nth(1).unwrap_or_default();
    let code = match arg.as_str() {
        "--install" => report(install::install()),
        "--uninstall" => report(install::uninstall()),
        "--apply" => {
            apply_saved();
            0
        }
        _ => run_ui(),
    };
    std::process::exit(code);
}

/// Pushes the saved hardware settings to the camera, then exits.
fn apply_saved() {
    let s = Settings::load();
    let Ok(devices) = camera::list_devices() else { return };
    if let Some(d) = camera::pick_device(&devices, &s.device) {
        if let Ok(src) = camera::open_device(&d.link) {
            Controls::new(&src).apply(&s.profile(&d.link));
            unsafe {
                let _ = src.Shutdown();
            }
        }
    }
}

fn run_ui() -> i32 {
    // Single instance: focus the existing window instead.
    unsafe {
        let _mutex = CreateMutexW(None, true, w!("CamLite.SingleInstance"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            if let Ok(hwnd) = FindWindowW(PCWSTR::null(), w!("CamLite")) {
                let _ = ShowWindow(hwnd, SW_RESTORE);
                let _ = SetForegroundWindow(hwnd);
            }
            return 0;
        }
        std::mem::forget(_mutex);
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("CamLite")
            .with_inner_size([440.0, 860.0])
            .with_min_inner_size([380.0, 520.0])
            .with_drag_and_drop(false)
            .with_icon(icon()),
        ..Default::default()
    };
    match eframe::run_native("CamLite", options, Box::new(|cc| Ok(Box::new(ui::App::new(cc))))) {
        Ok(()) => 0,
        Err(e) => {
            camlite_common::log::log("app.log", &format!("ui error: {e}"));
            1
        }
    }
}

/// Window icon: the logo pre-rendered to raw 64×64 RGBA by tools/icongen.
fn icon() -> egui::IconData {
    egui::IconData { rgba: include_bytes!("../../../assets/icon-64.rgba").to_vec(), width: 64, height: 64 }
}

fn report(r: windows_core::Result<()>) -> i32 {
    match r {
        Ok(()) => 0,
        Err(e) => {
            camlite_common::log::log("app.log", &format!("error: {e}"));
            1
        }
    }
}

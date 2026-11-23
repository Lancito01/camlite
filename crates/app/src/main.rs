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
        "--after-update" => {
            ui::AFTER_UPDATE.store(true, std::sync::atomic::Ordering::SeqCst);
            run_ui(true)
        }
        _ => run_ui(false),
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

fn run_ui(after_update: bool) -> i32 {
    // Single instance: focus the existing window instead. Right after an
    // update the previous version is still shutting down, so wait for it
    // (up to 15 s) instead of bowing out.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        unsafe {
            let mutex = CreateMutexW(None, true, w!("CamLite.SingleInstance"));
            if GetLastError() != ERROR_ALREADY_EXISTS {
                std::mem::forget(mutex);
                break;
            }
            if let Ok(h) = mutex {
                let _ = windows::Win32::Foundation::CloseHandle(h);
            }
            if !after_update || std::time::Instant::now() > deadline {
                if let Ok(hwnd) = FindWindowW(PCWSTR::null(), w!("CamLite")) {
                    let _ = ShowWindow(hwnd, SW_RESTORE);
                    let _ = SetForegroundWindow(hwnd);
                }
                return 0;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
    if after_update {
        camlite_common::log::log("app.log", &format!("update: now running v{}", updater::VERSION));
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

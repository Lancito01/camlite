//! Tiny append-only file logger (the DLL runs inside a system service, so a
//! file is the only practical way to see what it is doing).

use std::io::Write;

pub fn log(file: &str, msg: &str) {
    let dir = crate::data_dir();
    let path = dir.join(file);
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() > 512 * 1024 {
            let _ = std::fs::remove_file(&path);
        }
    }
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        let _ = writeln!(f, "[{t:.3}] pid={} {msg}", std::process::id());
    }
}

#[macro_export]
macro_rules! vlog {
    ($($arg:tt)*) => { $crate::log::log("vcam.log", &format!($($arg)*)) };
}

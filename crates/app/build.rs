//! Embeds the app icon and version info into camlite.exe.
fn main() {
    println!("cargo:rerun-if-changed=../../assets/camlite.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../assets/camlite.ico")
            .set("ProductName", "CamLite")
            .set("FileDescription", "CamLite")
            .set("LegalCopyright", "MIT License");
        res.compile().expect("failed to embed Windows resources");
    }
}

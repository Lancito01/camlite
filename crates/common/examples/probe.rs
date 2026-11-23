use camlite_common::camera::*;

fn main() -> windows_core::Result<()> {
    unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED).ok()? };
    mf_startup()?;
    for d in list_devices()? {
        println!("{} | {}", d.name, d.link);
        let src = open_device(&d.link)?;
        let types = native_types(&src)?;
        let infos: Vec<_> = types.iter().map(|(_, t)| t.clone()).collect();
        println!("  modes: {:?}", modes_from(&infos));
        let c = Controls::new(&src);
        for i in 0..10 {
            println!("  amp{i}: {:?} cur={:?}", c.range(Ctrl::Amp(i)), c.get(Ctrl::Amp(i)));
        }
        for i in [0, 1, 3, 4, 6, 19] {
            println!("  cam{i}: {:?} cur={:?}", c.range(Ctrl::Cam(i)), c.get(Ctrl::Cam(i)));
        }
        unsafe { src.Shutdown()? };
    }
    Ok(())
}

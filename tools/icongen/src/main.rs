//! Renders assets/logo.svg into the icon formats the app needs.
use resvg::{tiny_skia, usvg};
use std::path::Path;

fn render(tree: &usvg::Tree, size: u32) -> tiny_skia::Pixmap {
    let mut pm = tiny_skia::Pixmap::new(size, size).unwrap();
    let s = size as f32 / tree.size().width();
    resvg::render(tree, tiny_skia::Transform::from_scale(s, s), &mut pm.as_mut());
    pm
}

fn main() {
    let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let svg = std::fs::read(assets.join("logo.svg")).unwrap();
    let tree = usvg::Tree::from_data(&svg, &usvg::Options::default()).unwrap();

    // Multi-size .ico with PNG-compressed entries (Vista+).
    let sizes = [16u32, 20, 24, 32, 40, 48, 64, 128, 256];
    let pngs: Vec<Vec<u8>> = sizes.iter().map(|&s| render(&tree, s).encode_png().unwrap()).collect();
    let mut ico = Vec::new();
    ico.extend_from_slice(&[0, 0, 1, 0]);
    ico.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (s, png) in sizes.iter().zip(&pngs) {
        let d = if *s >= 256 { 0 } else { *s as u8 };
        ico.extend_from_slice(&[d, d, 0, 0]);
        ico.extend_from_slice(&1u16.to_le_bytes());
        ico.extend_from_slice(&32u16.to_le_bytes());
        ico.extend_from_slice(&(png.len() as u32).to_le_bytes());
        ico.extend_from_slice(&offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for png in &pngs {
        ico.extend_from_slice(png);
    }
    std::fs::write(assets.join("camlite.ico"), ico).unwrap();

    // README / social image.
    std::fs::write(assets.join("logo.png"), render(&tree, 512).encode_png().unwrap()).unwrap();

    // Raw straight-alpha RGBA for the window icon (no PNG decoder in the app).
    let pm = render(&tree, 64);
    let rgba: Vec<u8> = pm.pixels().iter().flat_map(|p| {
        let c = p.demultiply();
        [c.red(), c.green(), c.blue(), c.alpha()]
    }).collect();
    std::fs::write(assets.join("icon-64.rgba"), rgba).unwrap();
    println!("icons written to {}", assets.display());
}

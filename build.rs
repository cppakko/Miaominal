#![allow(clippy::disallowed_methods, reason = "build scripts are exempt")]

use image::ExtendedColorType;
use image::codecs::ico::{IcoEncoder, IcoFrame};
use std::env;
use std::error::Error;
use std::fs;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use tiny_skia::{Pixmap, Transform};

const SYSTEM_ICON_PATH: &str = "assets/app_icon_system.svg";
const BUNDLE_ICON_SIZE: u32 = 512;
const BUNDLE_ICON_2X_SIZE: u32 = 1024;
const X11_ICON_SIZE: u32 = 256;
const WINDOWS_ICON_SIZES: &[u32] = &[16, 20, 24, 32, 40, 48, 64, 128, 256];

fn main() {
    if let Err(error) = run() {
        eprintln!("failed to prepare application icons: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={SYSTEM_ICON_PATH}");

    let system_icon_path = Path::new(SYSTEM_ICON_PATH);
    let generated_dir = Path::new("assets/generated");
    fs::create_dir_all(generated_dir)?;

    let bundle_icon_path = generated_dir.join("app-icon.png");
    let bundle_icon_2x_path = generated_dir.join("app-icon@2x.png");
    let windows_icon_path = generated_dir.join("app-icon.ico");
    let x11_icon_path = PathBuf::from(env::var("OUT_DIR")?).join("app_icon.png");

    write_png(system_icon_path, &bundle_icon_path, BUNDLE_ICON_SIZE)?;
    write_png(system_icon_path, &bundle_icon_2x_path, BUNDLE_ICON_2X_SIZE)?;
    write_png(system_icon_path, &x11_icon_path, X11_ICON_SIZE)?;
    write_ico(system_icon_path, &windows_icon_path, WINDOWS_ICON_SIZES)?;

    #[cfg(windows)]
    embed_windows_icon(&windows_icon_path)?;

    Ok(())
}

fn render_icon(svg_path: &Path, icon_size: u32) -> Result<Pixmap, Box<dyn Error>> {
    let options = usvg::Options {
        resources_dir: svg_path.parent().map(Path::to_path_buf),
        ..usvg::Options::default()
    };
    let svg_data = fs::read(svg_path)?;
    let tree = usvg::Tree::from_data(&svg_data, &options)?;
    let source_size = tree.size().to_int_size();
    let scale_x = icon_size as f32 / source_size.width() as f32;
    let scale_y = icon_size as f32 / source_size.height() as f32;
    let scale = scale_x.min(scale_y);
    let target_width = source_size.width() as f32 * scale;
    let target_height = source_size.height() as f32 * scale;
    let translate_x = (icon_size as f32 - target_width) * 0.5;
    let translate_y = (icon_size as f32 - target_height) * 0.5;

    let transform = Transform::from_scale(scale, scale).post_translate(translate_x, translate_y);

    let mut pixmap = Pixmap::new(icon_size, icon_size)
        .ok_or_else(|| format!("failed to allocate {icon_size}x{icon_size} pixmap"))?;
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    Ok(pixmap)
}

fn write_png(svg_path: &Path, output_path: &Path, icon_size: u32) -> Result<(), Box<dyn Error>> {
    let pixmap = render_icon(svg_path, icon_size)?;
    pixmap.save_png(output_path)?;
    Ok(())
}

fn write_ico(
    svg_path: &Path,
    output_path: &Path,
    icon_sizes: &[u32],
) -> Result<(), Box<dyn Error>> {
    let mut frames = Vec::with_capacity(icon_sizes.len());
    for &icon_size in icon_sizes {
        let pixmap = render_icon(svg_path, icon_size)?;
        let encoded_image = pixmap.encode_png()?;
        let frame = IcoFrame::with_encoded(
            encoded_image,
            icon_size,
            icon_size,
            ExtendedColorType::Rgba8,
        )?;
        frames.push(frame);
    }

    let output = fs::File::create(output_path)?;
    IcoEncoder::new(BufWriter::new(output)).encode_images(&frames)?;
    Ok(())
}

#[cfg(windows)]
fn embed_windows_icon(icon_path: &Path) -> Result<(), Box<dyn Error>> {
    let mut resource = winresource::WindowsResource::new();
    resource.set_icon(icon_path.to_str().ok_or("icon path is not valid UTF-8")?);
    resource.compile()?;
    Ok(())
}

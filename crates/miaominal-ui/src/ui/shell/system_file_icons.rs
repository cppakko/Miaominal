use super::*;
use gpui::{Image, ImageFormat, img};
use std::collections::hash_map::Entry;
use std::path::Path;
use std::sync::{Arc, LazyLock, Mutex};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(in crate::ui::shell) struct SystemFileIconKey {
    kind: SystemFileIconKind,
    extension: Option<String>,
    is_directory: bool,
    size_px: u16,
    scale_bucket: u16,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum SystemFileIconKind {
    File,
    Directory,
    Symlink,
    Other,
}

impl SystemFileIconKey {
    pub(in crate::ui::shell) fn for_row(
        row: &SftpBrowserTableRow,
        size_px: u16,
        scale_factor: f32,
    ) -> Self {
        let extension = (!row.is_directory)
            .then(|| extension_for_name(row.name.as_ref()))
            .flatten();

        Self {
            kind: SystemFileIconKind::from(row.kind),
            extension,
            is_directory: row.is_directory,
            size_px,
            scale_bucket: scale_bucket(scale_factor),
        }
    }

    fn physical_size_px(&self) -> u32 {
        let scale = (self.scale_bucket as f32 / 100.0).max(1.0);
        ((self.size_px as f32 * scale).round() as u32).clamp(16, 256)
    }
}

impl From<miaominal_sftp::SftpEntryKind> for SystemFileIconKind {
    fn from(kind: miaominal_sftp::SftpEntryKind) -> Self {
        match kind {
            miaominal_sftp::SftpEntryKind::File => Self::File,
            miaominal_sftp::SftpEntryKind::Directory => Self::Directory,
            miaominal_sftp::SftpEntryKind::Symlink => Self::Symlink,
            miaominal_sftp::SftpEntryKind::Other => Self::Other,
        }
    }
}

#[derive(Clone)]
pub(in crate::ui::shell) struct SystemFileIcon {
    image: Arc<Image>,
}

struct SystemFileIconBytes {
    format: ImageFormat,
    bytes: Vec<u8>,
}

#[derive(Clone)]
enum CacheEntry {
    Loaded(SystemFileIcon),
    Missing,
    Loading,
}

static SYSTEM_FILE_ICON_CACHE: LazyLock<Mutex<HashMap<SystemFileIconKey, CacheEntry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub(in crate::ui::shell) fn render_system_file_icon(
    row: &SftpBrowserTableRow,
    fallback_icon: AppIcon,
    fallback_tint: u32,
    icon_size: Pixels,
    window: &mut Window,
    cx: &mut Context<TableState<SftpBrowserTableDelegate>>,
) -> AnyElement {
    let key = SystemFileIconKey::for_row(
        row,
        f32::from(icon_size).round() as u16,
        window.scale_factor(),
    );

    if let Some(icon) = cached_system_file_icon(&key) {
        return img(icon.image)
            .size(icon_size)
            .flex_none()
            .into_any_element();
    }

    queue_system_file_icon_load(key, cx);
    Icon::new(fallback_icon)
        .size(icon_size)
        .text_color(rgb(fallback_tint))
        .into_any_element()
}

fn cached_system_file_icon(key: &SystemFileIconKey) -> Option<SystemFileIcon> {
    let cache = SYSTEM_FILE_ICON_CACHE.lock().ok()?;
    match cache.get(key) {
        Some(CacheEntry::Loaded(icon)) => Some(icon.clone()),
        Some(CacheEntry::Missing | CacheEntry::Loading) | None => None,
    }
}

fn queue_system_file_icon_load(
    key: SystemFileIconKey,
    cx: &mut Context<TableState<SftpBrowserTableDelegate>>,
) {
    let should_load = {
        let Ok(mut cache) = SYSTEM_FILE_ICON_CACHE.lock() else {
            return;
        };
        match cache.entry(key.clone()) {
            Entry::Vacant(entry) => {
                entry.insert(CacheEntry::Loading);
                true
            }
            Entry::Occupied(_) => false,
        }
    };

    if !should_load {
        return;
    }

    cx.spawn(async move |table, cx| {
        let load_key = key.clone();
        let icon = cx
            .background_executor()
            .spawn(async move { load_system_file_icon(&load_key) })
            .await;

        if let Ok(mut cache) = SYSTEM_FILE_ICON_CACHE.lock() {
            cache.insert(
                key,
                icon.clone()
                    .map(CacheEntry::Loaded)
                    .unwrap_or(CacheEntry::Missing),
            );
        }

        if icon.is_some() {
            let _ = table.update(cx, |table, cx| {
                table.refresh(cx);
            });
        }
    })
    .detach();
}

fn load_system_file_icon(key: &SystemFileIconKey) -> Option<SystemFileIcon> {
    platform::load_system_file_icon_bytes(key).map(|icon| SystemFileIcon {
        image: Arc::new(Image::from_bytes(icon.format, icon.bytes)),
    })
}

fn extension_for_name(name: &str) -> Option<String> {
    let path = Path::new(name);
    let extension = path.extension()?.to_str()?.trim();
    if extension.is_empty() {
        None
    } else {
        Some(extension.to_ascii_lowercase())
    }
}

fn scale_bucket(scale_factor: f32) -> u16 {
    (scale_factor.max(1.0) * 100.0).round() as u16
}

#[cfg(windows)]
mod platform {
    use super::*;
    use image::{ImageEncoder as _, RgbaImage};
    use std::ffi::c_void;
    use std::mem::size_of;
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
        DeleteDC, DeleteObject, HGDIOBJ, SelectObject,
    };
    use windows::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL, FILE_FLAGS_AND_ATTRIBUTES,
    };
    use windows::Win32::UI::Shell::{
        SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGFI_OPENICON, SHGFI_SMALLICON,
        SHGFI_USEFILEATTRIBUTES, SHGetFileInfoW,
    };
    use windows::Win32::UI::WindowsAndMessaging::{DI_NORMAL, DestroyIcon, DrawIconEx, HICON};
    use windows::core::PCWSTR;

    pub(super) fn load_system_file_icon_bytes(
        key: &SystemFileIconKey,
    ) -> Option<SystemFileIconBytes> {
        let hicon = query_shell_icon(key, true).or_else(|| query_shell_icon(key, false))?;
        let output_size = key.physical_size_px();
        let source_size = source_icon_size(output_size);
        let png = unsafe { render_hicon_to_png(hicon, source_size, output_size) };
        unsafe {
            let _ = DestroyIcon(hicon);
        }
        png.map(|bytes| SystemFileIconBytes {
            format: ImageFormat::Png,
            bytes,
        })
    }

    fn query_shell_icon(key: &SystemFileIconKey, large: bool) -> Option<HICON> {
        let mut path = if key.is_directory {
            String::from("folder")
        } else if let Some(extension) = &key.extension {
            format!(".{extension}")
        } else {
            String::from("file")
        };
        path.push('\0');

        let attrs = if key.is_directory {
            FILE_ATTRIBUTE_DIRECTORY
        } else {
            FILE_ATTRIBUTE_NORMAL
        };
        let mut info = SHFILEINFOW::default();
        let mut flags = SHGFI_ICON | SHGFI_USEFILEATTRIBUTES;
        flags |= if large {
            SHGFI_LARGEICON
        } else {
            SHGFI_SMALLICON
        };
        if key.is_directory {
            flags |= SHGFI_OPENICON;
        }

        let result = unsafe {
            SHGetFileInfoW(
                PCWSTR(path.encode_utf16().collect::<Vec<_>>().as_ptr()),
                attrs,
                Some(&mut info),
                size_of::<SHFILEINFOW>() as u32,
                flags,
            )
        };

        (result != 0 && !info.hIcon.is_invalid()).then_some(info.hIcon)
    }

    fn source_icon_size(output_size: u32) -> u32 {
        output_size.max(32)
    }

    unsafe fn render_hicon_to_png(
        hicon: HICON,
        source_size: u32,
        output_size: u32,
    ) -> Option<Vec<u8>> {
        let hdc = unsafe { CreateCompatibleDC(None) };
        if hdc.is_invalid() {
            return None;
        }

        let mut bits: *mut c_void = std::ptr::null_mut();
        let bitmap_info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: source_size as i32,
                biHeight: -(source_size as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..BITMAPINFOHEADER::default()
            },
            ..BITMAPINFO::default()
        };

        let bitmap = match unsafe {
            CreateDIBSection(Some(hdc), &bitmap_info, DIB_RGB_COLORS, &mut bits, None, 0)
        } {
            Ok(bitmap) if !bitmap.is_invalid() && !bits.is_null() => bitmap,
            _ => {
                unsafe {
                    let _ = DeleteDC(hdc);
                }
                return None;
            }
        };

        let old_bitmap = unsafe { SelectObject(hdc, HGDIOBJ::from(bitmap)) };
        let draw_result = unsafe {
            DrawIconEx(
                hdc,
                0,
                0,
                hicon,
                source_size as i32,
                source_size as i32,
                0,
                None,
                DI_NORMAL,
            )
        };

        if !old_bitmap.is_invalid() {
            unsafe {
                let _ = SelectObject(hdc, old_bitmap);
            }
        }
        unsafe {
            let _ = DeleteDC(hdc);
        }

        let png = if draw_result.is_ok() {
            let pixel_count = (source_size * source_size) as usize;
            let bgra = unsafe { std::slice::from_raw_parts(bits as *const u8, pixel_count * 4) };
            let mut rgba = Vec::with_capacity(pixel_count * 4);
            for pixel in bgra.chunks_exact(4) {
                rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
            }
            let normalized = normalize_icon_alpha_bounds(&rgba, source_size, output_size);
            encode_rgba_png(&normalized, output_size, output_size)
        } else {
            None
        };

        unsafe {
            let _ = DeleteObject(HGDIOBJ::from(bitmap));
        }

        png
    }

    fn normalize_icon_alpha_bounds(rgba: &[u8], source_size: u32, output_size: u32) -> Vec<u8> {
        const ALPHA_THRESHOLD: u8 = 8;

        let output_len = (output_size as usize)
            .saturating_mul(output_size as usize)
            .saturating_mul(4);

        if rgba.len()
            != (source_size as usize)
                .saturating_mul(source_size as usize)
                .saturating_mul(4)
        {
            return vec![0; output_len];
        }

        let mut min_x = source_size;
        let mut min_y = source_size;
        let mut max_x = 0;
        let mut max_y = 0;

        for y in 0..source_size {
            for x in 0..source_size {
                let alpha_index = (((y * source_size + x) * 4) + 3) as usize;
                if rgba[alpha_index] > ALPHA_THRESHOLD {
                    min_x = min_x.min(x);
                    min_y = min_y.min(y);
                    max_x = max_x.max(x);
                    max_y = max_y.max(y);
                }
            }
        }

        if min_x > max_x || min_y > max_y {
            return vec![0; output_len];
        }

        let crop_width = max_x - min_x + 1;
        let crop_height = max_y - min_y + 1;
        let visual_margin = (output_size / 18).max(1).min(4);
        let target_limit = output_size.saturating_sub(visual_margin).max(1);

        if source_size == output_size && crop_width.max(crop_height) >= target_limit {
            return rgba.to_vec();
        }

        let scale = (target_limit as f32 / crop_width as f32)
            .min(target_limit as f32 / crop_height as f32);
        let dest_width = ((crop_width as f32 * scale).round() as u32).clamp(1, target_limit);
        let dest_height = ((crop_height as f32 * scale).round() as u32).clamp(1, target_limit);
        let dest_x = (output_size - dest_width) / 2;
        let dest_y = (output_size - dest_height) / 2;
        let Some(image) = RgbaImage::from_raw(source_size, source_size, rgba.to_vec()) else {
            return vec![0; output_len];
        };
        let cropped = image::imageops::crop_imm(&image, min_x, min_y, crop_width, crop_height)
            .to_image();
        let filter = if dest_width > crop_width || dest_height > crop_height {
            image::imageops::FilterType::Nearest
        } else {
            image::imageops::FilterType::CatmullRom
        };
        let resized = image::imageops::resize(&cropped, dest_width, dest_height, filter);
        let mut normalized = vec![0; output_len];
        let resized_pixels = resized.as_raw();

        for y in 0..dest_height {
            let dest_index = (((dest_y + y) * output_size + dest_x) * 4) as usize;
            let source_index = (y * dest_width * 4) as usize;
            let row_len = (dest_width * 4) as usize;
            normalized[dest_index..dest_index + row_len]
                .copy_from_slice(&resized_pixels[source_index..source_index + row_len]);
        }

        normalized
    }

    fn encode_rgba_png(rgba: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
        let mut bytes = Vec::new();
        let encoder = image::codecs::png::PngEncoder::new(&mut bytes);
        encoder
            .write_image(rgba, width, height, image::ColorType::Rgba8.into())
            .ok()?;
        Some(bytes)
    }

    #[allow(dead_code)]
    fn _assert_file_attributes(_: FILE_FLAGS_AND_ATTRIBUTES) {}
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use objc2::AnyThread as _;
    use objc2::rc::autoreleasepool;
    use objc2::runtime::AnyObject;
    use objc2_app_kit::{
        NSBitmapImageFileType, NSBitmapImageRep, NSBitmapImageRepPropertyKey, NSWorkspace,
    };
    use objc2_foundation::{NSDictionary, NSString, NSSize};

    pub(super) fn load_system_file_icon_bytes(
        key: &SystemFileIconKey,
    ) -> Option<SystemFileIconBytes> {
        autoreleasepool(|_| {
            let workspace = NSWorkspace::sharedWorkspace();
            let image = if key.is_directory {
                workspace.iconForFile(&NSString::from_str("/tmp"))
            } else {
                let file_type = key.extension.as_deref().unwrap_or("");
                #[allow(deprecated)]
                workspace.iconForFileType(&NSString::from_str(file_type))
            };

            let size = key.physical_size_px() as f64;
            image.setSize(NSSize::new(size, size));
            let tiff = image.TIFFRepresentation()?;
            let rep = NSBitmapImageRep::initWithData(NSBitmapImageRep::alloc(), &tiff)?;
            let properties =
                NSDictionary::<NSBitmapImageRepPropertyKey, AnyObject>::init(NSDictionary::alloc());
            let png = unsafe {
                rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &properties)
            }?;

            Some(SystemFileIconBytes {
                format: ImageFormat::Png,
                bytes: png.to_vec(),
            })
        })
    }
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
mod platform {
    use super::*;
    use std::env;
    use std::fs;
    use std::path::{Path, PathBuf};

    pub(super) fn load_system_file_icon_bytes(
        key: &SystemFileIconKey,
    ) -> Option<SystemFileIconBytes> {
        let icon_names = icon_names_for_key(key);
        let theme_names = icon_theme_names();
        let roots = icon_roots();
        let size_dirs = preferred_size_dirs(key.physical_size_px());

        find_icon_file(&icon_names, &theme_names, &roots, &size_dirs).and_then(read_icon_file)
    }

    fn icon_names_for_key(key: &SystemFileIconKey) -> Vec<String> {
        let mut names = Vec::new();
        if key.is_directory {
            push_unique(&mut names, "folder");
            push_unique(&mut names, "inode-directory");
            return names;
        }

        if let Some(extension) = &key.extension {
            if let Some(mime) = mime_guess::from_ext(extension).first_raw() {
                push_unique(&mut names, &mime.replace('/', "-"));
                for generic in generic_mime_icon_names(mime, extension) {
                    push_unique(&mut names, generic);
                }
            }
            for name in extension_icon_names(extension) {
                push_unique(&mut names, name);
            }
        }

        push_unique(&mut names, "text-x-generic");
        push_unique(&mut names, "unknown");
        names
    }

    fn generic_mime_icon_names(mime: &str, extension: &str) -> &'static [&'static str] {
        if matches!(
            extension,
            "7z" | "bz2" | "gz" | "rar" | "tar" | "tgz" | "xz" | "zip" | "zst"
        ) {
            return &["package-x-generic"];
        }

        match mime.split_once('/').map(|(top, _)| top) {
            Some("text") => &["text-x-generic"],
            Some("image") => &["image-x-generic"],
            Some("audio") => &["audio-x-generic"],
            Some("video") => &["video-x-generic"],
            Some("application") if mime == "application/pdf" => &["application-pdf"],
            Some("application")
                if matches!(
                    mime,
                    "application/x-executable"
                        | "application/x-msdownload"
                        | "application/x-sh"
                        | "application/x-shellscript"
                ) =>
            {
                &["application-x-executable"]
            }
            Some("application")
                if matches!(
                    extension,
                    "doc" | "docx" | "odp" | "ods" | "odt" | "ppt" | "pptx" | "rtf" | "xls"
                        | "xlsx"
                ) =>
            {
                &["x-office-document"]
            }
            Some("application")
                if matches!(extension, "js" | "json" | "php" | "py" | "rb" | "rs") =>
            {
                &["text-x-script", "text-x-generic"]
            }
            Some("application") => &["application-x-generic"],
            _ => &["unknown"],
        }
    }

    fn extension_icon_names(extension: &str) -> &'static [&'static str] {
        match extension {
            "desktop" => &["application-x-desktop"],
            "exe" | "bin" | "appimage" => &["application-x-executable"],
            "md" | "markdown" => &["text-markdown", "text-x-generic"],
            "rs" | "sh" | "bash" | "zsh" | "fish" | "py" | "rb" | "php" | "js" | "ts" => {
                &["text-x-script", "text-x-generic"]
            }
            _ => &[],
        }
    }

    fn icon_theme_names() -> Vec<String> {
        let mut names = Vec::new();

        if let Ok(theme) = env::var("GTK_THEME")
            && let Some(theme) = theme.split(':').next()
        {
            push_unique(&mut names, theme);
        }

        if let Ok(desktop) = env::var("XDG_CURRENT_DESKTOP") {
            for desktop in desktop.split(':') {
                match desktop.to_ascii_lowercase().as_str() {
                    "gnome" => push_unique(&mut names, "Adwaita"),
                    "kde" | "plasma" => push_unique(&mut names, "breeze"),
                    "ubuntu" => push_unique(&mut names, "Yaru"),
                    _ => {}
                }
            }
        }

        for name in ["hicolor", "Adwaita", "Yaru", "Papirus", "breeze", "gnome"] {
            push_unique(&mut names, name);
        }

        names
    }

    fn icon_roots() -> Vec<PathBuf> {
        let mut roots = Vec::new();

        if let Some(home) = env::var_os("HOME") {
            push_unique_path(&mut roots, PathBuf::from(home).join(".local/share/icons"));
        }

        let data_dirs = env::var_os("XDG_DATA_DIRS")
            .map(|dirs| env::split_paths(&dirs).collect::<Vec<_>>())
            .unwrap_or_else(|| {
                vec![
                    PathBuf::from("/usr/local/share"),
                    PathBuf::from("/usr/share"),
                ]
            });

        for data_dir in data_dirs {
            push_unique_path(&mut roots, data_dir.join("icons"));
        }

        push_unique_path(&mut roots, PathBuf::from("/usr/share/icons"));
        push_unique_path(&mut roots, PathBuf::from("/usr/local/share/icons"));
        push_unique_path(&mut roots, PathBuf::from("/usr/share/pixmaps"));
        roots
    }

    fn preferred_size_dirs(physical_size: u32) -> Vec<String> {
        let mut sizes = vec![16_u32, 22, 24, 32, 48, 64, 96, 128, 256, 512];
        sizes.push(physical_size.clamp(16, 512));
        sizes.sort_unstable_by_key(|size| (size.abs_diff(physical_size), *size));
        sizes.dedup();

        let mut dirs = sizes
            .into_iter()
            .map(|size| format!("{size}x{size}"))
            .collect::<Vec<_>>();
        dirs.push("scalable".to_string());
        dirs.push("symbolic".to_string());
        dirs
    }

    fn find_icon_file(
        icon_names: &[String],
        theme_names: &[String],
        roots: &[PathBuf],
        size_dirs: &[String],
    ) -> Option<PathBuf> {
        let categories = ["mimetypes", "places", "apps", "devices"];
        let extensions = ["png", "svg"];

        for root in roots {
            if root.ends_with("pixmaps") {
                if let Some(path) = find_in_directory(root, icon_names, &extensions) {
                    return Some(path);
                }
                continue;
            }

            for theme in theme_names {
                let theme_root = root.join(theme);
                for size_dir in size_dirs {
                    for category in categories {
                        let dir = theme_root.join(size_dir).join(category);
                        if let Some(path) = find_in_directory(&dir, icon_names, &extensions) {
                            return Some(path);
                        }
                    }
                }
            }
        }

        None
    }

    fn find_in_directory(dir: &Path, icon_names: &[String], extensions: &[&str]) -> Option<PathBuf> {
        for name in icon_names {
            for extension in extensions {
                let path = dir.join(format!("{name}.{extension}"));
                if path.is_file() {
                    return Some(path);
                }
            }
        }
        None
    }

    fn read_icon_file(path: PathBuf) -> Option<SystemFileIconBytes> {
        let format = match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "png" => ImageFormat::Png,
            "svg" => ImageFormat::Svg,
            _ => return None,
        };
        let bytes = fs::read(path).ok()?;
        Some(SystemFileIconBytes { format, bytes })
    }

    fn push_unique(values: &mut Vec<String>, value: &str) {
        if !value.is_empty() && !values.iter().any(|existing| existing == value) {
            values.push(value.to_string());
        }
    }

    fn push_unique_path(values: &mut Vec<PathBuf>, value: PathBuf) {
        if !values.iter().any(|existing| existing == &value) {
            values.push(value);
        }
    }

}

#[cfg(not(any(
    windows,
    target_os = "macos",
    target_os = "linux",
    target_os = "freebsd"
)))]
mod platform {
    use super::*;

    pub(super) fn load_system_file_icon_bytes(
        _key: &SystemFileIconKey,
    ) -> Option<SystemFileIconBytes> {
        None
    }
}

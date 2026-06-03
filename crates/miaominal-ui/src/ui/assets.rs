use gpui::{App, AssetSource, IntoElement, RenderOnce, Result, SharedString, Window};
use gpui_component::{Icon, IconNamed};
use rust_embed::RustEmbed;
use std::borrow::Cow;

#[derive(RustEmbed)]
#[folder = "../../assets"]
#[include = "icons/**/*.svg"]
#[include = "miaominal_icon.svg"]
struct CustomAssets;

pub struct AppAssets;

#[derive(Clone, Copy, Debug, Eq, PartialEq, IntoElement)]
pub enum AppIcon {
    Computer,
    ChevronDown,
    ChevronUp,
    CornerLeftUp,
    Download,
    FingerPrint,
    Forward,
    FolderSymlink,
    Key,
    LaptopMinimal,
    Miaominal,
    Sparkles,
    FolderOpen,
    Notebook,
    Settings,
    Close,
    Maximize,
    Restore,
    Minimize,
    Grid,
    List,
    Edit,
    Plus,
    File,
    Folder,
    Upload,
    Pause,
    Play,
    Trash,
    Rotate,
    Eye,
    EyeOff,
    Check,
    Next,
    Vault,
}

impl IconNamed for AppIcon {
    #[allow(deprecated)]
    fn path(self) -> SharedString {
        match self {
            Self::Vault => "icons/vault.svg",
            Self::FolderOpen => "icons/folder-open.svg",
            Self::Computer => "icons/computer.svg",
            Self::ChevronDown => "icons/chevron-down.svg",
            Self::ChevronUp => "icons/chevron-up.svg",
            Self::CornerLeftUp => "icons/corner-left-up.svg",
            Self::Download => "icons/download.svg",
            Self::Sparkles => "icons/sparkles.svg",
            Self::FingerPrint => "icons/fingerprint-pattern.svg",
            Self::Forward => "icons/forward.svg",
            Self::FolderSymlink => "icons/folder-symlink.svg",
            Self::Key => "icons/key-round.svg",
            Self::LaptopMinimal => "icons/laptop-minimal.svg",
            Self::Miaominal => "miaominal_icon.svg",
            Self::Notebook => "icons/notebook-pen.svg",
            Self::Settings => "icons/settings.svg",
            Self::Close => "icons/x.svg",
            Self::Maximize => "icons/maximize.svg",
            Self::Restore => "icons/minimize-2.svg",
            Self::Minimize => "icons/minus.svg",
            Self::Grid => "icons/grid-2x2.svg",
            Self::List => "icons/list.svg",
            Self::Edit => "icons/pencil.svg",
            Self::Plus => "icons/plus.svg",
            Self::File => "icons/file.svg",
            Self::Folder => "icons/folder.svg",
            Self::Upload => "icons/upload.svg",
            Self::Pause => "icons/pause.svg",
            Self::Play => "icons/play.svg",
            Self::Trash => "icons/trash-2.svg",
            Self::Rotate => "icons/rotate-ccw.svg",
            Self::Eye => "icons/eye.svg",
            Self::EyeOff => "icons/eye-off.svg",
            Self::Check => "icons/check.svg",
            Self::Next => "icons/chevron-right.svg",
        }
        .into()
    }
}

impl RenderOnce for AppIcon {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        Icon::from(self)
    }
}

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path.is_empty() {
            return Ok(None);
        }

        if let Some(file) = CustomAssets::get(path) {
            return Ok(Some(file.data));
        }

        let bundled = gpui_component_assets::Assets;
        match bundled.load(path) {
            Ok(data) => Ok(data),
            Err(_) => Ok(None),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut items: Vec<SharedString> = CustomAssets::iter()
            .filter_map(|entry| entry.starts_with(path).then(|| entry.into()))
            .collect();

        let bundled = gpui_component_assets::Assets;
        if let Ok(mut extra) = bundled.list(path) {
            items.append(&mut extra);
        }

        items.sort();
        items.dedup();
        Ok(items)
    }
}

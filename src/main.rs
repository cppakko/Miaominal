#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod app;

#[cfg(target_os = "macos")]
use gpui::point;
use gpui::{App, AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};
use gpui_component::Root;
use miaominal_ui::AppAssets;
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
use std::io::Cursor;
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
use std::sync::{Arc, LazyLock};
use tokio::runtime::Handle as TokioHandle;

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
const DESKTOP_APP_ID: &str = env!("CARGO_PKG_NAME");

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
static APP_ICON: LazyLock<Option<Arc<image::RgbaImage>>> = LazyLock::new(|| {
    const BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/app_icon.png"));

    match image::ImageReader::new(Cursor::new(BYTES)).with_guessed_format() {
        Ok(reader) => match reader.decode() {
            Ok(image) => Some(Arc::new(image.into())),
            Err(error) => {
                eprintln!("failed to decode app icon: {error}");
                None
            }
        },
        Err(error) => {
            eprintln!("failed to read app icon format: {error}");
            None
        }
    }
});

fn main_window_titlebar() -> Option<TitlebarOptions> {
    #[cfg(target_os = "macos")]
    {
        Some(TitlebarOptions {
            title: None,
            appears_transparent: true,
            traffic_light_position: Some(point(px(12.0), px(18.0))),
        })
    }

    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

fn init_logging() {
    let default_filter = if cfg!(debug_assertions) {
        "info"
    } else {
        "off"
    };

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(default_filter))
        .init();
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn ensure_graphical_session() -> Result<(), String> {
    let has_wayland =
        std::env::var_os("WAYLAND_DISPLAY").is_some_and(|display| !display.is_empty());
    let has_x11 = std::env::var_os("DISPLAY").is_some_and(|display| !display.is_empty());

    if has_wayland || has_x11 {
        return Ok(());
    }

    let session_type = std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "unknown".into());
    Err(format!(
        "Miaominal requires a graphical Linux desktop session. Both WAYLAND_DISPLAY and DISPLAY are unset (XDG_SESSION_TYPE={session_type}). Launch it from a desktop session or export DISPLAY/WAYLAND_DISPLAY before running cargo run."
    ))
}

#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
fn ensure_graphical_session() -> Result<(), String> {
    Ok(())
}

fn open_main_window(cx: &mut App, runtime: TokioHandle) {
    let bounds = Bounds::centered(None, size(px(1240.0), px(800.0)), cx);

    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(720.0), px(480.0))),
            titlebar: main_window_titlebar(),
            #[cfg(target_os = "macos")]
            is_movable: false,
            #[cfg(any(target_os = "linux", target_os = "freebsd"))]
            app_id: Some(DESKTOP_APP_ID.to_string()),
            #[cfg(any(target_os = "linux", target_os = "freebsd"))]
            icon: APP_ICON.as_ref().cloned(),
            ..Default::default()
        },
        |window, cx| {
            let view = cx.new(|cx| miaominal_ui::AppView::new(runtime.clone(), window, cx));
            cx.new(|cx| Root::new(view, window, cx))
        },
    )
    .expect("failed to open main window");
}

fn main() {
    init_logging();

    if let Err(error) = miaominal_paths::cleanup_stale_atomic_write_files() {
        log::warn!("failed to clean stale atomic-write files: {error:?}");
    }

    if let Err(message) = ensure_graphical_session() {
        eprintln!("{message}");
        std::process::exit(1);
    }

    let runtime = app::runtime::start_tokio();

    let application = gpui_platform::application().with_assets(AppAssets);
    application.on_reopen({
        let runtime = runtime.clone();
        move |cx: &mut App| {
            if cx.windows().is_empty() {
                open_main_window(cx, runtime.clone());
            }
            cx.activate(true);
        }
    });

    application.run(move |cx: &mut App| {
        gpui_component::init(cx);
        miaominal_ui::init_markdown(cx);
        miaominal_ui::i18n::init();
        app::install_app_menus(cx);

        open_main_window(cx, runtime.clone());

        cx.activate(true);
    });
}

#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod app;

use futures::StreamExt;
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
use gpui_kit::WindowDecorations;
use gpui_kit::component::Root;
#[cfg(target_os = "macos")]
use gpui_kit::point;
use gpui_kit::{
    AnyWindowHandle, App, AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px,
    size,
};
use miaominal_ui::AppAssets;
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
use std::io::Cursor;
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
use std::sync::{Arc, LazyLock};
use std::{cell::RefCell, rc::Rc};
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

fn app_window_options(cx: &App) -> WindowOptions {
    let bounds = Bounds::centered(None, size(px(1240.0), px(800.0)), cx);
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(720.0), px(480.0))),
        titlebar: main_window_titlebar(),
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        window_decorations: Some(WindowDecorations::Client),
        #[cfg(target_os = "macos")]
        is_movable: false,
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        app_id: Some(DESKTOP_APP_ID.to_string()),
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        icon: APP_ICON.as_ref().cloned(),
        ..Default::default()
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

fn open_main_window(cx: &mut App, runtime: TokioHandle) -> AnyWindowHandle {
    cx.open_window(app_window_options(cx), |window, cx| {
        miaominal_ui::configure_main_window_close(window, cx);
        let view = cx.new(|cx| miaominal_ui::AppView::new(runtime.clone(), window, cx));
        cx.new(|cx| Root::new(view, window, cx))
    })
    .expect("failed to open main window")
    .into()
}

fn open_detached_window(
    cx: &mut App,
    runtime: TokioHandle,
) -> anyhow::Result<miaominal_ui::DetachedWindowTarget> {
    let view_slot = Rc::new(RefCell::new(None));
    let closure_view_slot = view_slot.clone();
    let handle = cx.open_window(app_window_options(cx), move |window, cx| {
        let view = cx.new(|cx| miaominal_ui::AppView::new_detached(runtime, window, cx));
        miaominal_ui::configure_detached_window_close(&view, window, cx);
        closure_view_slot.borrow_mut().replace(view.clone());
        cx.new(|cx| Root::new(view, window, cx))
    })?;
    let view = view_slot
        .borrow_mut()
        .take()
        .ok_or_else(|| anyhow::anyhow!("detached window view was not initialized"))?;
    Ok(miaominal_ui::DetachedWindowTarget::new(handle.into(), view))
}

fn activate_main_window(cx: &mut App, runtime: TokioHandle) {
    if miaominal_ui::restore_main_window(cx) {
        return;
    }

    if cx.windows().is_empty() {
        let main_window = open_main_window(cx, runtime);
        miaominal_ui::initialize_system_tray(main_window, cx);
    }

    let window = cx
        .active_window()
        .or_else(|| {
            cx.window_stack()
                .and_then(|windows| windows.into_iter().next())
        })
        .or_else(|| cx.windows().into_iter().next());
    if let Some(window) = window
        && let Err(error) = window.update(cx, |_, window, _| window.activate_window())
    {
        log::debug!("failed to activate existing Miaominal window: {error:?}");
    }
    cx.activate(true);
}

fn show_startup_error(title: String, message: String) {
    eprintln!("{title}: {message}");
    if ensure_graphical_session().is_ok() {
        let _ = rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Error)
            .set_title(&title)
            .set_description(&message)
            .set_buttons(rfd::MessageButtons::Ok)
            .show();
    }
}

fn run_hidden_ssh_bridge_helper() -> Option<i32> {
    let arguments = match miaominal_ssh::parse_ssh_bridge_helper_args(std::env::args_os()) {
        Ok(Some(arguments)) => arguments,
        Ok(None) => return None,
        Err(error) => {
            eprintln!("miaominal ssh-bridge-helper: {error:#}");
            return Some(2);
        }
    };

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("miaominal ssh-bridge-helper: failed to start runtime: {error}");
            return Some(1);
        }
    };
    match runtime.block_on(miaominal_ssh::run_ssh_bridge_helper(arguments)) {
        Ok(()) => Some(0),
        Err(error) => {
            eprintln!("miaominal ssh-bridge-helper: {error:#}");
            Some(1)
        }
    }
}

fn main() {
    if let Some(exit_code) = run_hidden_ssh_bridge_helper() {
        if exit_code != 0 {
            std::process::exit(exit_code);
        }
        return;
    }

    init_logging();
    miaominal_ui::i18n::init();

    let runtime_context = match miaominal_paths::initialize_runtime() {
        Ok(context) => context,
        Err(error) => {
            eprintln!("failed to initialize user data directory: {error:#}");
            std::process::exit(1);
        }
    };
    match runtime_context.config_initialization() {
        miaominal_paths::ConfigDirInitialization::Current { .. } => {}
        miaominal_paths::ConfigDirInitialization::Migrated { from, to } => {
            log::info!(
                "migrated legacy config directory from {} to {}",
                from.display(),
                to.display()
            );
        }
        miaominal_paths::ConfigDirInitialization::LegacyFallback {
            path,
            intended,
            error,
        } => {
            log::warn!(
                "using legacy config directory {} because migration to {} failed: {}",
                path.display(),
                intended.display(),
                error
            );
        }
    }
    if runtime_context.mode() == miaominal_paths::RuntimeMode::Portable {
        log::info!(
            "portable mode enabled with data directory {}",
            runtime_context.active_data_dir().display()
        );
    }
    if let Some(warning) = runtime_context.warning() {
        log::warn!("{warning}");
    }

    let instance_guard =
        match app::instance::AppInstanceGuard::acquire(runtime_context.active_data_dir()) {
            Ok(app::instance::AppInstanceDisposition::Primary(guard)) => guard,
            Ok(app::instance::AppInstanceDisposition::Secondary(client)) => {
                match client.activate_existing_blocking() {
                    Ok(()) => return,
                    Err(error) => {
                        let error = format!("{error:#}");
                        let (title, message) =
                            miaominal_ui::i18n::single_instance_activation_error(&error);
                        show_startup_error(title, message);
                        std::process::exit(1);
                    }
                }
            }
            Err(error) => {
                let error = format!("{error:#}");
                let (title, message) = miaominal_ui::i18n::single_instance_startup_error(&error);
                show_startup_error(title, message);
                std::process::exit(1);
            }
        };
    log::debug!(
        "acquired app instance ownership {} for {}",
        instance_guard.instance_id(),
        runtime_context.active_data_dir().display()
    );

    if let Err(error) = miaominal_paths::cleanup_stale_atomic_write_files() {
        log::warn!("failed to clean stale atomic-write files: {error:?}");
    }

    if let Err(message) = ensure_graphical_session() {
        eprintln!("{message}");
        std::process::exit(1);
    }

    let runtime = app::runtime::start_tokio();
    let (activation_sender, mut activation_receiver) = futures::channel::mpsc::unbounded();
    let instance_server = match runtime.block_on(instance_guard.start_server(activation_sender)) {
        Ok(server) => server,
        Err(error) => {
            let error = format!("{error:#}");
            let (title, message) = miaominal_ui::i18n::single_instance_startup_error(&error);
            show_startup_error(title, message);
            std::process::exit(1);
        }
    };

    let application = gpui_kit::application().with_assets(AppAssets);
    application.on_reopen({
        let runtime = runtime.clone();
        move |cx: &mut App| {
            activate_main_window(cx, runtime.clone());
        }
    });

    let application_runtime = runtime.clone();
    application.run(move |cx: &mut App| {
        gpui_kit::init(cx);
        miaominal_ui::initialize_application_state(application_runtime.clone(), cx);
        miaominal_ui::init_markdown(cx);
        app::install_app_menus(cx);

        let detached_window_runtime = application_runtime.clone();
        miaominal_ui::register_detached_window_opener(cx, move |cx| {
            open_detached_window(cx, detached_window_runtime.clone())
        });

        let activation_runtime = application_runtime.clone();
        cx.spawn(async move |cx| {
            while let Some(app::instance::AppInstanceCommand::Activate) =
                activation_receiver.next().await
            {
                let runtime = activation_runtime.clone();
                cx.update(move |cx| activate_main_window(cx, runtime));
            }
        })
        .detach();

        let main_window = open_main_window(cx, application_runtime.clone());
        miaominal_ui::initialize_system_tray(main_window, cx);

        cx.activate(true);
    });

    runtime.block_on(instance_server.shutdown());
    drop(instance_guard);
}

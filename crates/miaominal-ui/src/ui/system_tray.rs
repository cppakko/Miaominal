use gpui::{AnyWindowHandle, App, Entity, Global, Window};
use miaominal_settings::WindowCloseBehavior;

use super::AppView;
use super::i18n;

const SHOW_MENU_ID: &str = "miaominal-tray-show";
const QUIT_MENU_ID: &str = "miaominal-tray-quit";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrayCommand {
    Show,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CloseAction {
    Exit,
    HideToTray,
    Minimize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrimaryExitRoute {
    NativeWindowClose,
    QuitApplication,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DesktopPlatform {
    Windows,
    Linux,
    MacOs,
    Other,
}

#[derive(Clone)]
struct TraySnapshot {
    visible: bool,
    show_label: String,
    quit_label: String,
}

impl TraySnapshot {
    fn current() -> Self {
        Self {
            visible: miaominal_settings::current_settings().window_close_behavior
                == WindowCloseBehavior::MinimizeToTray,
            show_label: i18n::string("tray.show"),
            quit_label: i18n::string("tray.quit"),
        }
    }
}

struct SystemTrayState {
    main_window: AnyWindowHandle,
    platform: platform::PlatformTray,
}

impl Global for SystemTrayState {}

pub fn initialize_system_tray(main_window: AnyWindowHandle, cx: &mut App) {
    if cx.has_global::<SystemTrayState>() {
        cx.global_mut::<SystemTrayState>().main_window = main_window;
        sync_system_tray(cx);
        return;
    }

    let (command_tx, mut command_rx) = tokio::sync::mpsc::unbounded_channel();
    let snapshot = TraySnapshot::current();
    let platform = platform::PlatformTray::new(snapshot, command_tx);
    cx.set_global(SystemTrayState {
        main_window,
        platform,
    });

    cx.spawn(async move |cx| {
        while let Some(command) = command_rx.recv().await {
            // Tray menu callbacks can run inside the native menu message dispatch. Yield through
            // the timer before touching GPUI so focus/show notifications cannot re-enter App's
            // RefCell while the native callback is still unwinding.
            cx.background_executor()
                .timer(std::time::Duration::from_millis(10))
                .await;
            cx.update(|cx| handle_tray_command(command, cx));
        }
    })
    .detach();
}

pub fn sync_system_tray(cx: &mut App) {
    let snapshot = TraySnapshot::current();
    if let Some(state) = cx.try_global::<SystemTrayState>() {
        state.platform.sync(snapshot);
    }
}

pub fn configure_main_window_close(window: &Window, cx: &App) {
    window.on_window_should_close(cx, main_window_should_close);
}

pub fn configure_detached_window_close(view: &Entity<AppView>, window: &Window, cx: &App) {
    let view = view.downgrade();
    window.on_window_should_close(cx, move |window, cx| {
        if let Some(view) = view.upgrade() {
            view.update(cx, |view, cx| {
                view.prepare_detached_window_close(window, cx);
            });
        }
        true
    });
}

pub fn request_main_window_close(window: &mut Window, cx: &mut App) {
    if main_window_should_close(window, cx) {
        window.remove_window();
    }
}

fn main_window_should_close(window: &mut Window, cx: &mut App) -> bool {
    let behavior = miaominal_settings::current_settings().window_close_behavior;
    let tray_available = cx
        .try_global::<SystemTrayState>()
        .is_some_and(|state| state.platform.available());
    match close_action(behavior, current_platform(), tray_available) {
        CloseAction::Exit => match primary_exit_route(cx.windows().len()) {
            // Preserve GPUI's original single-window close path so its native window teardown and
            // application shutdown behavior remain unchanged when no detached window exists.
            PrimaryExitRoute::NativeWindowClose => true,
            PrimaryExitRoute::QuitApplication => {
                // GPUI's App::quit clears all windows without invoking each window's
                // on_window_should_close callback, so retire detached tab resources first.
                quit_application(cx);
                false
            }
        },
        CloseAction::HideToTray => !platform::hide_window(window),
        CloseAction::Minimize => {
            window.minimize_window();
            false
        }
    }
}

fn handle_tray_command(command: TrayCommand, cx: &mut App) {
    match command {
        TrayCommand::Show => {
            restore_main_window(cx);
        }
        TrayCommand::Quit => quit_application(cx),
    }
}

fn quit_application(cx: &mut App) {
    crate::ui::windowing::prepare_detached_windows_for_application_quit(cx);
    cx.quit();
}

const fn primary_exit_route(open_window_count: usize) -> PrimaryExitRoute {
    if open_window_count <= 1 {
        PrimaryExitRoute::NativeWindowClose
    } else {
        PrimaryExitRoute::QuitApplication
    }
}

pub fn restore_main_window(cx: &mut App) -> bool {
    let Some(main_window) = cx
        .try_global::<SystemTrayState>()
        .map(|state| state.main_window)
    else {
        return false;
    };
    cx.activate(true);
    match main_window.update(cx, |_, window, _| {
        platform::show_window(window);
        window.activate_window();
    }) {
        Ok(()) => true,
        Err(error) => {
            log::debug!("failed to restore main window from tray: {error:?}");
            false
        }
    }
}

fn close_action(
    behavior: WindowCloseBehavior,
    platform: DesktopPlatform,
    tray_available: bool,
) -> CloseAction {
    if behavior == WindowCloseBehavior::ExitApplication {
        return CloseAction::Exit;
    }
    match platform {
        DesktopPlatform::Windows if tray_available => CloseAction::HideToTray,
        DesktopPlatform::Windows => CloseAction::Exit,
        DesktopPlatform::Linux => CloseAction::Minimize,
        DesktopPlatform::MacOs | DesktopPlatform::Other => CloseAction::Exit,
    }
}

const fn current_platform() -> DesktopPlatform {
    if cfg!(target_os = "windows") {
        DesktopPlatform::Windows
    } else if cfg!(target_os = "linux") {
        DesktopPlatform::Linux
    } else if cfg!(target_os = "macos") {
        DesktopPlatform::MacOs
    } else {
        DesktopPlatform::Other
    }
}

#[cfg(windows)]
mod platform {
    use super::{QUIT_MENU_ID, SHOW_MENU_ID, TrayCommand, TraySnapshot};
    use gpui::Window;
    use raw_window_handle::RawWindowHandle;
    use tokio::sync::mpsc::UnboundedSender;
    use tray_icon::{
        Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
        menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    };
    use windows::Win32::{
        Foundation::HWND,
        UI::WindowsAndMessaging::{IsIconic, SW_HIDE, SW_RESTORE, SW_SHOW, ShowWindowAsync},
    };

    pub(super) struct PlatformTray {
        tray_icon: Option<TrayIcon>,
        show_item: Option<MenuItem>,
        quit_item: Option<MenuItem>,
    }

    impl PlatformTray {
        pub(super) fn new(
            snapshot: TraySnapshot,
            command_tx: UnboundedSender<TrayCommand>,
        ) -> Self {
            let show_item = MenuItem::with_id(SHOW_MENU_ID, &snapshot.show_label, true, None);
            let quit_item = MenuItem::with_id(QUIT_MENU_ID, &snapshot.quit_label, true, None);
            let separator = PredefinedMenuItem::separator();
            let menu = Menu::new();
            if let Err(error) = menu.append_items(&[&show_item, &separator, &quit_item]) {
                log::warn!("failed to build system tray menu: {error}");
                return Self::unavailable();
            }

            let menu_command_tx = command_tx.clone();
            MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                let command = match event.id.as_ref() {
                    SHOW_MENU_ID => Some(TrayCommand::Show),
                    QUIT_MENU_ID => Some(TrayCommand::Quit),
                    _ => None,
                };
                if let Some(command) = command {
                    let _ = menu_command_tx.send(command);
                }
            }));
            TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
                let should_show = matches!(
                    event,
                    TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } | TrayIconEvent::DoubleClick {
                        button: MouseButton::Left,
                        ..
                    }
                );
                if should_show {
                    let _ = command_tx.send(TrayCommand::Show);
                }
            }));

            let tray_icon = match load_icon().and_then(|icon| {
                TrayIconBuilder::new()
                    .with_tooltip("Miaominal")
                    .with_menu(Box::new(menu))
                    .with_menu_on_left_click(false)
                    .with_icon(icon)
                    .build()
                    .map_err(|error| error.to_string())
            }) {
                Ok(tray_icon) => tray_icon,
                Err(error) => {
                    log::warn!("failed to initialize system tray: {error}");
                    return Self::unavailable();
                }
            };
            if let Err(error) = tray_icon.set_visible(snapshot.visible) {
                log::warn!("failed to set system tray visibility: {error}");
            }
            Self {
                tray_icon: Some(tray_icon),
                show_item: Some(show_item),
                quit_item: Some(quit_item),
            }
        }

        fn unavailable() -> Self {
            Self {
                tray_icon: None,
                show_item: None,
                quit_item: None,
            }
        }

        pub(super) fn available(&self) -> bool {
            self.tray_icon.is_some()
        }

        pub(super) fn sync(&self, snapshot: TraySnapshot) {
            if let Some(item) = &self.show_item {
                item.set_text(snapshot.show_label);
            }
            if let Some(item) = &self.quit_item {
                item.set_text(snapshot.quit_label);
            }
            if let Some(tray_icon) = &self.tray_icon
                && let Err(error) = tray_icon.set_visible(snapshot.visible)
            {
                log::warn!("failed to update system tray visibility: {error}");
            }
        }
    }

    fn load_icon() -> Result<Icon, String> {
        let image = image::load_from_memory(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/generated/app-icon.png"
        )))
        .map_err(|error| format!("failed to decode tray icon: {error}"))?
        .into_rgba8();
        let (width, height) = image.dimensions();
        Icon::from_rgba(image.into_raw(), width, height)
            .map_err(|error| format!("failed to create tray icon: {error}"))
    }

    fn window_hwnd(window: &Window) -> Option<HWND> {
        let handle = raw_window_handle::HasWindowHandle::window_handle(window)
            .ok()?
            .as_raw();
        let RawWindowHandle::Win32(handle) = handle else {
            return None;
        };
        Some(HWND(handle.hwnd.get() as *mut core::ffi::c_void))
    }

    pub(super) fn hide_window(window: &Window) -> bool {
        let Some(hwnd) = window_hwnd(window) else {
            return false;
        };
        let _ = unsafe { ShowWindowAsync(hwnd, SW_HIDE) };
        true
    }

    pub(super) fn show_window(window: &Window) {
        let Some(hwnd) = window_hwnd(window) else {
            return;
        };
        unsafe {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindowAsync(hwnd, SW_RESTORE);
            } else {
                let _ = ShowWindowAsync(hwnd, SW_SHOW);
            }
        }
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::{QUIT_MENU_ID, SHOW_MENU_ID, TrayCommand, TraySnapshot};
    use gpui::Window;
    use gtk::glib::ControlFlow;
    use std::{
        panic::{AssertUnwindSafe, catch_unwind},
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        thread,
        time::Duration,
    };
    use tokio::sync::mpsc::UnboundedSender;
    use tray_icon::{
        Icon, TrayIconBuilder,
        menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    };

    enum ThreadCommand {
        Sync(TraySnapshot),
        Shutdown,
    }

    pub(super) struct PlatformTray {
        command_tx: mpsc::Sender<ThreadCommand>,
        available: Arc<AtomicBool>,
        thread: Option<thread::JoinHandle<()>>,
    }

    impl PlatformTray {
        pub(super) fn new(
            snapshot: TraySnapshot,
            app_command_tx: UnboundedSender<TrayCommand>,
        ) -> Self {
            let (command_tx, command_rx) = mpsc::channel();
            let available = Arc::new(AtomicBool::new(false));
            let thread_available = available.clone();
            let thread = thread::Builder::new()
                .name("miaominal-system-tray".into())
                .spawn(move || {
                    let result = catch_unwind(AssertUnwindSafe(|| {
                        run_tray_thread(snapshot, command_rx, app_command_tx, thread_available)
                    }));
                    if result.is_err() {
                        log::warn!("Linux system tray thread terminated unexpectedly");
                    }
                })
                .map_err(|error| {
                    log::warn!("failed to start Linux system tray thread: {error}");
                    error
                })
                .ok();
            Self {
                command_tx,
                available,
                thread,
            }
        }

        pub(super) fn available(&self) -> bool {
            self.available.load(Ordering::Acquire)
        }

        pub(super) fn sync(&self, snapshot: TraySnapshot) {
            let _ = self.command_tx.send(ThreadCommand::Sync(snapshot));
        }
    }

    impl Drop for PlatformTray {
        fn drop(&mut self) {
            let _ = self.command_tx.send(ThreadCommand::Shutdown);
            if let Some(thread) = self.thread.take()
                && thread.join().is_err()
            {
                log::warn!("Linux system tray thread panicked during shutdown");
            }
        }
    }

    fn run_tray_thread(
        snapshot: TraySnapshot,
        command_rx: mpsc::Receiver<ThreadCommand>,
        app_command_tx: UnboundedSender<TrayCommand>,
        available: Arc<AtomicBool>,
    ) {
        if let Err(error) = gtk::init() {
            log::warn!("failed to initialize GTK system tray support: {error}");
            return;
        }

        let show_item = MenuItem::with_id(SHOW_MENU_ID, &snapshot.show_label, true, None);
        let quit_item = MenuItem::with_id(QUIT_MENU_ID, &snapshot.quit_label, true, None);
        let separator = PredefinedMenuItem::separator();
        let menu = Menu::new();
        if let Err(error) = menu.append_items(&[&show_item, &separator, &quit_item]) {
            log::warn!("failed to build Linux system tray menu: {error}");
            return;
        }

        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let command = match event.id.as_ref() {
                SHOW_MENU_ID => Some(TrayCommand::Show),
                QUIT_MENU_ID => Some(TrayCommand::Quit),
                _ => None,
            };
            if let Some(command) = command {
                let _ = app_command_tx.send(command);
            }
        }));

        let tray_icon = match load_icon().and_then(|icon| {
            TrayIconBuilder::new()
                .with_tooltip("Miaominal")
                .with_menu(Box::new(menu))
                .with_icon(icon)
                .build()
                .map_err(|error| error.to_string())
        }) {
            Ok(tray_icon) => tray_icon,
            Err(error) => {
                log::warn!("failed to initialize Linux system tray: {error}");
                return;
            }
        };
        if let Err(error) = tray_icon.set_visible(snapshot.visible) {
            log::warn!("failed to set Linux system tray visibility: {error}");
        }
        available.store(true, Ordering::Release);

        let timer_available = Arc::clone(&available);
        gtk::glib::timeout_add_local(Duration::from_millis(50), move || {
            for command in command_rx.try_iter() {
                match command {
                    ThreadCommand::Sync(snapshot) => {
                        show_item.set_text(snapshot.show_label);
                        quit_item.set_text(snapshot.quit_label);
                        if let Err(error) = tray_icon.set_visible(snapshot.visible) {
                            log::warn!("failed to update Linux system tray visibility: {error}");
                        }
                    }
                    ThreadCommand::Shutdown => {
                        timer_available.store(false, Ordering::Release);
                        gtk::main_quit();
                        return ControlFlow::Break;
                    }
                }
            }
            ControlFlow::Continue
        });
        gtk::main();
        available.store(false, Ordering::Release);
    }

    fn load_icon() -> Result<Icon, String> {
        let image = image::load_from_memory(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/generated/app-icon.png"
        )))
        .map_err(|error| format!("failed to decode tray icon: {error}"))?
        .into_rgba8();
        let (width, height) = image.dimensions();
        Icon::from_rgba(image.into_raw(), width, height)
            .map_err(|error| format!("failed to create tray icon: {error}"))
    }

    pub(super) fn hide_window(_window: &Window) -> bool {
        false
    }

    pub(super) fn show_window(_window: &Window) {}
}

#[cfg(not(any(windows, target_os = "linux")))]
mod platform {
    use super::{TrayCommand, TraySnapshot};
    use gpui::Window;
    use tokio::sync::mpsc::UnboundedSender;

    pub(super) struct PlatformTray;

    impl PlatformTray {
        pub(super) fn new(
            _snapshot: TraySnapshot,
            _command_tx: UnboundedSender<TrayCommand>,
        ) -> Self {
            Self
        }

        pub(super) fn available(&self) -> bool {
            false
        }

        pub(super) fn sync(&self, _snapshot: TraySnapshot) {}
    }

    pub(super) fn hide_window(_window: &Window) -> bool {
        false
    }

    pub(super) fn show_window(_window: &Window) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_exit_preserves_native_close_without_detached_windows() {
        assert_eq!(primary_exit_route(1), PrimaryExitRoute::NativeWindowClose);
        assert_eq!(primary_exit_route(2), PrimaryExitRoute::QuitApplication);
    }

    #[test]
    fn close_action_preserves_direct_exit_on_every_platform() {
        for platform in [
            DesktopPlatform::Windows,
            DesktopPlatform::Linux,
            DesktopPlatform::MacOs,
            DesktopPlatform::Other,
        ] {
            assert_eq!(
                close_action(WindowCloseBehavior::ExitApplication, platform, true),
                CloseAction::Exit
            );
        }
    }

    #[test]
    fn windows_requires_an_available_tray_before_hiding() {
        assert_eq!(
            close_action(
                WindowCloseBehavior::MinimizeToTray,
                DesktopPlatform::Windows,
                true
            ),
            CloseAction::HideToTray
        );
        assert_eq!(
            close_action(
                WindowCloseBehavior::MinimizeToTray,
                DesktopPlatform::Windows,
                false
            ),
            CloseAction::Exit
        );
    }

    #[test]
    fn linux_uses_recoverable_minimization() {
        assert_eq!(
            close_action(
                WindowCloseBehavior::MinimizeToTray,
                DesktopPlatform::Linux,
                false
            ),
            CloseAction::Minimize
        );
    }

    #[test]
    fn unsupported_platforms_ignore_minimize_to_tray() {
        assert_eq!(
            close_action(
                WindowCloseBehavior::MinimizeToTray,
                DesktopPlatform::MacOs,
                true
            ),
            CloseAction::Exit
        );
        assert_eq!(
            close_action(
                WindowCloseBehavior::MinimizeToTray,
                DesktopPlatform::Other,
                true
            ),
            CloseAction::Exit
        );
    }
}

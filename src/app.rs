pub(crate) mod instance;
pub(crate) mod runtime;

#[cfg(target_os = "macos")]
use gpui_kit::{App, KeyBinding, Menu, MenuItem, NoAction, SystemMenuType, actions};

#[cfg(target_os = "macos")]
actions!(
    main_menu,
    [
        ShowAboutApp,
        QuitApp,
        HideApp,
        HideOthers,
        ShowAll,
        OpenSettings,
        MinimizeWindow,
        ToggleFullScreen,
    ]
);

#[cfg(target_os = "macos")]
pub(crate) fn install_app_menus(cx: &mut App) {
    cx.on_action(|_: &ShowAboutApp, _cx| {
        log::info!("About Miaominal menu clicked");
    });
    cx.on_action(|_: &QuitApp, cx| {
        miaominal_ui::quit_application(cx);
    });
    cx.on_action(|_: &HideApp, cx| {
        cx.hide();
    });
    cx.on_action(|_: &HideOthers, cx| {
        cx.hide_other_apps();
    });
    cx.on_action(|_: &ShowAll, cx| {
        cx.unhide_other_apps();
    });
    cx.on_action(|_: &OpenSettings, cx| {
        miaominal_ui::open_settings_from_menu(cx);
    });
    cx.on_action(|_: &MinimizeWindow, cx| {
        if let Some(window) = cx.active_window() {
            let _ = window.update(cx, |_view, window, _cx| {
                window.minimize_window();
            });
        }
    });
    cx.on_action(|_: &ToggleFullScreen, cx| {
        if let Some(window) = cx.active_window() {
            let _ = window.update(cx, |_view, window, _cx| {
                window.toggle_fullscreen();
            });
        }
    });

    // Bind the macOS-standard application shortcuts before building the menus so the
    // native menu items pick up their key equivalents from the keymap.
    cx.bind_keys([
        KeyBinding::new("cmd-q", QuitApp, None),
        KeyBinding::new("cmd-h", HideApp, None),
        KeyBinding::new("cmd-alt-h", HideOthers, None),
        KeyBinding::new("cmd-,", OpenSettings, None),
        KeyBinding::new("cmd-m", MinimizeWindow, None),
        KeyBinding::new("ctrl-cmd-f", ToggleFullScreen, None),
    ]);

    cx.set_menus([
        Menu::new("Miaominal").items([
            MenuItem::action("About Miaominal", ShowAboutApp),
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Preferences", OpenSettings),
            MenuItem::separator(),
            MenuItem::action("Hide Miaominal", HideApp),
            MenuItem::action("Hide Others", HideOthers),
            MenuItem::action("Show All", ShowAll),
            MenuItem::separator(),
            MenuItem::action("Quit Miaominal", QuitApp),
        ]),
        Menu::new("Window").items([
            MenuItem::action("Minimize", MinimizeWindow),
            // Zoom is intentionally disabled: gpui does not expose a window
            // "zoom" (toggle max size) action to bind here.
            // TODO: implement when gpui gains window zoom support.
            MenuItem::action("Zoom", NoAction).disabled(true),
            MenuItem::action("Enter Full Screen", ToggleFullScreen),
        ]),
    ]);
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn install_app_menus(_: &mut gpui_kit::App) {}

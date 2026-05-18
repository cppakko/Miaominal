pub(crate) mod paths;
pub(crate) mod runtime;

#[cfg(target_os = "macos")]
use gpui::{App, Menu, MenuItem, NoAction, SystemMenuType, actions};

#[cfg(target_os = "macos")]
actions!(main_menu, [ShowAboutApp, QuitApp]);

#[cfg(target_os = "macos")]
pub(crate) fn install_app_menus(cx: &mut App) {
    cx.on_action(|_: &ShowAboutApp, _cx| {
        log::info!("About Miaominal menu clicked");
    });
    cx.on_action(|_: &QuitApp, cx| {
        cx.quit();
    });

    cx.set_menus([
        Menu::new("Miaominal").items([
            MenuItem::action("About Miaominal", ShowAboutApp),
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Preferences", NoAction).disabled(true),
            MenuItem::separator(),
            MenuItem::action("Quit Miaominal", QuitApp),
        ]),
        Menu::new("Window").items([
            MenuItem::action("Minimize", NoAction).disabled(true),
            MenuItem::action("Zoom", NoAction).disabled(true),
            MenuItem::action("Enter Full Screen", NoAction).disabled(true),
        ]),
    ]);
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn install_app_menus(_: &mut gpui::App) {}

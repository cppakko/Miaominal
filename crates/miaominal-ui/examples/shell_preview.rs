//! Native GPUI visual fixture. Uses isolated data and never opens a network connection.
//! cargo run -p miaominal-ui --example shell_preview -- <output.png> [width] [height] [light|dark] [font-size] [seed] [scenario] [react-project]
use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{
    App, AppContext as _, Bounds, InputEvent as _, WindowBounds, WindowOptions, point, px, size,
};
use miaominal_core::profile::SessionProfile;
use std::{borrow::Cow, path::PathBuf, time::Duration};

fn assert_catalog_grid(window: &gpui_kit::Window, grid_id: &str, card_ids: &[String]) {
    let scale = f32::from(window.rem_size()) / 14.0;
    let grid = window
        .find(gpui_kit::SharedString::from(grid_id.to_owned()))
        .bounds();
    let width = f32::from(grid.size.width);
    assert!(
        width <= 1280.0 * scale + 1.0,
        "{grid_id}: content width must be capped: {width}"
    );
    let columns = (((width / scale + 12.0) / 276.0).floor().max(1.0)) as usize;
    // GPUI snaps authored gaps before distributing the grid tracks, then snaps
    // each track's edges. Use that gap rather than accumulating fractional rems.
    let device_scale = window.scale_factor();
    let pixel = 1.0 / device_scale;
    let gap = (12.0 * scale * device_scale - 0.5).ceil() / device_scale;
    let expected_card_width = (width - (columns - 1) as f32 * gap) / columns as f32;
    let first = window
        .find(gpui_kit::SharedString::from(card_ids[0].clone()))
        .bounds();
    let mut first_row_count = 0;
    for (index, id) in card_ids.iter().enumerate() {
        let bounds = window
            .find(gpui_kit::SharedString::from(id.clone()))
            .bounds();
        assert!(
            (f32::from(bounds.size.width) - expected_card_width).abs() <= pixel,
            "{id}: unequal card width {:?}, expected {expected_card_width} in {columns} columns",
            bounds.size
        );
        assert!(
            bounds.left() >= grid.left() - px(pixel) && bounds.right() <= grid.right() + px(pixel),
            "{id}: card overflows its grid"
        );
        let expected_x = grid.left() + px((index % columns) as f32 * (expected_card_width + gap));
        assert!(
            f32::from(bounds.left() - expected_x).abs() <= pixel,
            "{id}: inconsistent column or gap"
        );
        if index % columns > 0 {
            let previous = window
                .find(gpui_kit::SharedString::from(card_ids[index - 1].clone()))
                .bounds();
            assert!(
                (f32::from(bounds.left() - previous.right()) - gap).abs() <= pixel * 0.01,
                "{id}: adjacent cards must keep the snapped gap of {gap}"
            );
        }
        if f32::from(bounds.top() - first.top()).abs() <= 1.0 {
            first_row_count += 1;
        }
    }
    assert_eq!(
        first_row_count,
        columns.min(card_ids.len()),
        "{grid_id}: column count must respond to its available width"
    );
    println!(
        "{grid_id}: grid={width:.1}, columns={columns}, card={expected_card_width:.1}, scale={scale:.3}"
    );
    if grid_id == "snippet-packages-grid" {
        assert_catalog_grid(
            window,
            "snippets-grid",
            &(0..7)
                .map(|i| format!("snippet-card-card-{i}"))
                .collect::<Vec<_>>(),
        );
    }
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    // With no arguments, open an isolated visible fixture for native window
    // interaction checks. It still never reads the user's connection catalog.
    let interactive =
        args.len() == 1 || std::env::var_os("MIAOMINAL_PREVIEW_INTERACTIVE").is_some();
    let output = args
        .get(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_exe().unwrap().with_extension("png"));
    let output = if output.is_absolute() {
        output
    } else {
        std::env::current_dir().unwrap().join(output)
    };
    let width = args
        .get(2)
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(if interactive { 800.0 } else { 1242.0 });
    let height = args
        .get(3)
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(if interactive { 600.0 } else { 801.0 });
    let theme = args.get(4).map(String::as_str).unwrap_or("dark");
    let font = args
        .get(5)
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(14.0);
    let seed = args.get(6).map(String::as_str).unwrap_or("#8ECFAD");
    let scenario = args.get(7).cloned().unwrap_or_default();
    // Reference fonts belong to the isolated visual fixture, never user settings
    // or the shipped application. Read the exact asset used by the React app.
    let reference = args
        .get(8)
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("MIAOMINAL_REACT_REFERENCE").map(PathBuf::from));
    let reference_font = reference.map(|root| {
        std::fs::read(root.join("public/assets/RobotoFlex.ttf"))
            .expect("React reference must contain public/assets/RobotoFlex.ttf")
    });
    let reference_typography = reference_font.is_some();
    let fixture = output
        .parent()
        .unwrap()
        .join(format!("fixture-{}", std::process::id()));
    let data = fixture.join("data");
    let cache = fixture.join("cache");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(&cache).unwrap();
    miaominal_paths::initialize_embedded_runtime(&data, &cache).unwrap();
    let mut settings = format!(
        "language = \"zh-CN\"\ncompleted_onboarding_version = 1\nlocal_vault_enabled = false\ntheme_id = \"{theme}\"\ninterface_font_size = {font}\nseed_color = \"{seed}\"\n"
    );
    if scenario == "vault-unlock" {
        settings = settings.replace("local_vault_enabled = false", "local_vault_enabled = true");
        let passphrase = miaominal_secrets::ProtectedPassphrase::try_from_string(
            "isolated-preview-vault".to_string(),
        )
        .unwrap();
        miaominal_secrets::CredentialStore::with_backend(
            "miaominal-preview",
            miaominal_secrets::VaultCredentialBackend::new(passphrase).unwrap(),
        )
        .initialize()
        .unwrap();
        std::fs::write(
            data.join("sync_config.toml"),
            toml::to_string(&miaominal_sync::SyncConfig {
                provider: miaominal_sync::SyncProvider::WebDav,
                auto_sync_enabled: false,
                capability_probe_migrated: true,
                ..Default::default()
            })
            .unwrap(),
        )
        .unwrap();
    }
    if reference_font.is_some() {
        settings.push_str("font_family = \"Roboto Flex\"\nfont_fallbacks = [\"Microsoft YaHei UI\", \"Microsoft YaHei\"]\n");
    }
    std::fs::write(data.join("settings.toml"), settings).unwrap();
    let names = [
        "生产环境 · Web",
        "数据库主节点",
        "测试环境",
        "备份服务器",
        "日志与监控",
        "一台名称很长的主机，用于验证真实布局省略行为",
    ];
    let mut profiles = Vec::new();
    for (index, name) in names.iter().enumerate() {
        let mut profile = SessionProfile::blank(format!("preview-{index}"), index);
        profile.name = (*name).into();
        profile.host = format!("192.0.2.{}", index + 1);
        profile.username = "fixture".into();
        profile.group = if index < 2 {
            "生产环境"
        } else {
            "开发与测试"
        }
        .into();
        profile.tags = vec!["Linux".into(), "SSH".into(), "Preview".into()];
        profile.is_favorite = index < 2;
        profile.last_connected_at = (index < 2).then_some(100 - index as u64);
        profiles.push(profile);
    }
    if scenario == "vault-unlock" {
        profiles[0].auth_method = Some(miaominal_core::profile::AuthMethod::Password);
        profiles[0].has_stored_password = true;
        profiles[0]
            .port_forwarding_rules
            .push(miaominal_core::profile::PortForwardRule {
                id: "vault-rule".into(),
                label: "保险库解锁验证".into(),
                kind: miaominal_core::profile::PortForwardKind::Local,
                listen_host: "127.0.0.1".into(),
                listen_port: 15432,
                target_host: "127.0.0.1".into(),
                target_port: 5432,
                enabled: false,
            });
    }
    if scenario == "global-search" {
        profiles[0]
            .port_forwarding_rules
            .push(miaominal_core::profile::PortForwardRule {
                id: "search-rule".into(),
                label: "SSH database tunnel".into(),
                kind: miaominal_core::profile::PortForwardKind::Local,
                listen_host: "127.0.0.1".into(),
                listen_port: 15432,
                target_host: "127.0.0.1".into(),
                target_port: 5432,
                enabled: false,
            });
    }
    if matches!(scenario.as_str(), "adaptive-cards" | "view-modes") {
        for index in 0..7 {
            profiles[0]
                .port_forwarding_rules
                .push(miaominal_core::profile::PortForwardRule {
                    id: format!("card-{index}"),
                    label: format!("{index} · 数据库转发规则名称较长，用于检查自适应省略"),
                    kind: miaominal_core::profile::PortForwardKind::Local,
                    listen_host: "127.0.0.1".into(),
                    listen_port: 15432 + index,
                    target_host: "127.0.0.1".into(),
                    target_port: 5432,
                    enabled: false,
                });
        }
        let public_key =
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        let keys = (0..7)
            .map(|index| miaominal_core::keychain::ManagedKeyRecord {
                id: format!("card-{index}"),
                name: format!("{index} · 生产环境部署密钥，检查长名称在卡片内省略"),
                algorithm: "ssh-ed25519".into(),
                public_key: public_key.into(),
                source: miaominal_core::keychain::ManagedKeySource::Imported,
            })
            .collect::<Vec<_>>();
        miaominal_storage::keychain_store::ManagedKeyStore::with_path(
            data.join("managed_keys.toml"),
        )
        .save(&keys)
        .unwrap();
        let snippets = (0..7)
            .map(|index| miaominal_core::snippet::SnippetRecord {
                id: format!("card-{index}"),
                description: format!("{index} · 检查数据库服务状态与长脚本名称"),
                package: format!("分组 {index}"),
                language: "bash".into(),
                script: "systemctl status database-service --no-pager --full".into(),
            })
            .collect::<Vec<_>>();
        miaominal_storage::config_store::store::SnippetStore::with_path(data.join("snippets.toml"))
            .save(&snippets)
            .unwrap();
        std::fs::write(
            data.join("known_hosts"),
            (0..7)
                .map(|index| format!("card-{index}.example.com {public_key}\n"))
                .collect::<String>(),
        )
        .unwrap();
    }
    let mut local = SessionProfile::blank("preview-local", 7);
    local.kind = miaominal_core::profile::ProfileKind::Local;
    local.name = "PowerShell".into();
    local.local_shell = "pwsh.exe".into();
    profiles.push(local);
    if scenario.starts_with("reference") {
        // The same public demo data as the React reference makes geometry and
        // component comparisons independent of differing host names/counts.
        profiles.clear();
        for (index, (id, name, host, group, tag)) in [
            ("dev", "Osaka DevBox", "192.0.2.177", "Personal", "dev"),
            (
                "gateway",
                "prod-bastion-01",
                "198.51.100.10",
                "Infra",
                "prod",
            ),
            (
                "staging",
                "staging-api",
                "198.51.100.21",
                "Infra",
                "staging",
            ),
            ("echo", "Echo Worker", "192.0.2.32", "Network", ""),
            (
                "runner1",
                "linux-runner-01",
                "203.0.113.41",
                "CI Runners",
                "ci",
            ),
            (
                "runner2",
                "linux-runner-02",
                "203.0.113.42",
                "CI Runners",
                "ci",
            ),
            ("nas", "Home NAS", "192.0.2.50", "Homelab", "storage"),
            ("lab", "Lab Server", "192.0.2.51", "Homelab", "lab"),
            ("edge", "edge-node", "203.0.113.61", "", "edge"),
            ("backup", "backup-server", "203.0.113.62", "", "backup"),
        ]
        .into_iter()
        .enumerate()
        {
            let mut profile = SessionProfile::blank(id, index);
            profile.name = name.into();
            profile.host = host.into();
            profile.username = "root".into();
            profile.group = group.into();
            profile.tags = if tag.is_empty() {
                vec![]
            } else {
                vec![tag.into()]
            };
            profile.last_connected_at = match id {
                "dev" => Some(100),
                "gateway" => Some(99),
                "echo" => Some(98),
                _ => None,
            };
            profiles.push(profile);
        }
        let mut local = SessionProfile::blank("local", 10);
        local.kind = miaominal_core::profile::ProfileKind::Local;
        local.name = "PowerShell".into();
        local.local_shell = "pwsh.exe".into();
        profiles.push(local);
    }
    if scenario == "empty" {
        profiles.clear();
    } else if scenario == "local" {
        profiles.retain(SessionProfile::is_local);
    } else if scenario == "many" {
        for index in 8..68 {
            let mut profile = profiles[index % 6].clone();
            profile.id = format!("preview-{index}");
            profile.name = format!("主机 {index:02} · 长列表布局");
            profile.is_favorite = false;
            profile.last_connected_at = None;
            profiles.push(profile);
        }
    }
    std::fs::write(
        data.join("sessions.toml"),
        toml::to_string(&miaominal_storage::config_store::store::SessionsDocument {
            sessions: profiles,
        })
        .unwrap(),
    )
    .unwrap();
    let adaptive_trust_ids = if scenario == "adaptive-cards" {
        miaominal_storage::known_hosts_store::KnownHostsStore::with_path(data.join("known_hosts"))
            .list()
            .unwrap()
            .into_iter()
            .map(|entry| {
                format!(
                    "trusted-host-card-{}-{}-{}",
                    entry.host, entry.port, entry.fingerprint
                )
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let runtime_handle = runtime.handle().clone();
    gpui_kit::application()
        .with_assets(miaominal_ui::AppAssets)
        .run(move |cx: &mut App| {
            gpui_kit::init(cx);
            if let Some(bytes) = reference_font {
                cx.text_system()
                    .add_fonts(vec![Cow::Owned(bytes)])
                    .expect("load the React reference font into GPUI");
                assert!(
                    cx.text_system()
                        .all_font_names()
                        .iter()
                        .any(|name| name == "Roboto Flex"),
                    "Roboto Flex must be registered before rendering"
                );
            }
            miaominal_ui::initialize_application_state(runtime_handle.clone(), cx);
            let (handle, _) = gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(0.0), px(0.0)),
                        size(px(width), px(height)),
                    ))),
                    titlebar: None,
                    // Keep the requested review viewport even when the active
                    // display is narrower (e.g. a portrait remote desktop).
                    window_min_size: Some(size(px(width), px(height))),
                    show: interactive,
                    focus: interactive,
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    cx.new(|cx| miaominal_ui::AppView::new(runtime_handle.clone(), window, cx))
                },
            )
            .unwrap();
            // Creation can constrain the initial bounds to the active display.
            // Request the review viewport again through the real native window;
            // never override GPUI's scale without resizing its render target.
            handle
                .update(cx, |_, window, _| {
                    window.resize(size(px(width), px(height)))
                })
                .unwrap();
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(1500))
                    .await;
                cx.update(|cx| {
                    handle
                        .update(cx, |_, window, cx| {
                            window.render_frame(cx);
                            let viewport = window.viewport_size();
                            println!(
                                "Viewport {viewport:?}; native scale {}",
                                window.scale_factor()
                            );
                            assert!((f32::from(viewport.width) - width).abs() <= 1.0);
                            assert!((f32::from(viewport.height) - height).abs() <= 1.0);
                            match scenario.strip_prefix("reference-").unwrap_or(&scenario) {
                                "search" => {
                                    window.click("search-input-region", cx);
                                    window.input(if scenario.starts_with("reference") { "linux-runner" } else { "生产" }, cx);
                                }
                                "recent" => window.click("search-input-region", cx),
                                "group" => window.click(if scenario.starts_with("reference") { "host-filter:Infra" } else { "host-filter:生产环境" }, cx),
                                "editor" => window.click("hosts-create-fab", cx),
                                "forwarding" | "vault-unlock" => window.click("sidebar-forwarding", cx),
                                "keychain" => window.click("sidebar-keychain", cx),
                                "snippets" => window.click("sidebar-snippets", cx),
                                "known-hosts" => window.click("sidebar-known-hosts", cx),
                                "list" => window.click("hosts-view-mode-list", cx),
                                "menu" => window.click("library-host-preview-0:more", cx),
                                "many" => window.scroll(
                                    "hosts-collection-scroll",
                                    gpui_kit::ScrollDelta::Pixels(point(px(0.0), px(-10000.0))),
                                    cx,
                                ),
                                "tabs" | "tabs-scroll" => {
                                    for _ in 0..8 {
                                        window.click("top-add-host", cx);
                                    }
                                }
                                "tab-context-focus" | "chrome" | "chrome-idle" | "chrome-active" | "chrome-close" => {
                                    window.click("top-add-host", cx);
                                }
                                _ => {}
                            }
                        })
                        .unwrap();
                });
                // Navigation commands are deferred until the click update exits.
                // Start their transition before waiting, including in hidden windows.
                cx.update(|cx| {
                    handle.update(cx, |_, window, cx| window.render_frame(cx)).unwrap();
                });
                cx.background_executor()
                    .timer(Duration::from_millis(700))
                    .await;
                if scenario == "vault-unlock" {
                    cx.update(|cx| {
                        handle.update(cx, |_, window, cx| {
                            window.render_frame(cx);
                            window.click("forward-rule-switch-preview-0-vault-rule", cx);
                        }).unwrap();
                    });
                    for _ in 0..12 {
                        cx.background_executor().timer(Duration::from_millis(30)).await;
                        cx.update(|cx| { handle.update(cx, |_, window, cx| window.render_frame(cx)).unwrap(); });
                    }
                    cx.update(|cx| {
                        handle.update(cx, |_, window, cx| {
                            assert!(window.try_find("local-vault-passphrase-popup-submit").is_some(), "starting a locked forwarding rule must open the unlock dialog in its own window");
                            window.render_to_image().unwrap().save(output.with_file_name("vault-forwarding-start.png")).unwrap();
                            window.click("local-vault-passphrase-popup-cancel", cx);
                        }).unwrap();
                    });
                    for _ in 0..12 {
                        cx.background_executor().timer(Duration::from_millis(30)).await;
                        cx.update(|cx| { handle.update(cx, |_, window, cx| window.render_frame(cx)).unwrap(); });
                    }
                    cx.update(|cx| {
                        handle.update(cx, |_, window, cx| {
                            assert!(window.try_find("local-vault-passphrase-popup-submit").is_none());
                            window.render_to_image().unwrap().save(output.with_file_name("vault-forwarding-pointer.png")).unwrap();
                            window.click("forward-rule-card-preview-0-vault-rule", cx);
                        }).unwrap();
                    });
                    for _ in 0..12 {
                        cx.background_executor().timer(Duration::from_millis(30)).await;
                        cx.update(|cx| { handle.update(cx, |_, window, cx| window.render_frame(cx)).unwrap(); });
                    }
                    cx.update(|cx| {
                        handle.update(cx, |_, window, cx| window.click("forwarding-editor-save", cx)).unwrap();
                    });
                    for _ in 0..12 {
                        cx.background_executor().timer(Duration::from_millis(30)).await;
                        cx.update(|cx| { handle.update(cx, |_, window, cx| window.render_frame(cx)).unwrap(); });
                    }
                    cx.update(|cx| {
                        handle.update(cx, |_, window, _| {
                            assert!(window.try_find("local-vault-passphrase-popup-submit").is_some(), "saving a locked forwarding rule must open the unlock dialog and retain its deferred action");
                        }).unwrap();
                    });
                    println!("PASS: locked forwarding start and save open the vault prompt without opening a connection");
                }
                if scenario == "view-modes" {
                    cx.update(|cx| cx.set_reduce_motion(false));
                    let pages = [
                        ("sidebar-hosts", "hosts-view-mode", "library-host-preview-0:connect", None),
                        ("sidebar-forwarding", "forwarding-view-mode", "forward-rule-card-preview-0-card-0", Some("forward-rule-row-preview-0-card-0")),
                        ("sidebar-snippets", "snippets-view-mode", "snippet-card-card-0", Some("snippet-row-card-0")),
                    ];
                    let mut control_size = None;
                    for (page, id, grid_content, list_content) in pages {
                        cx.update(|cx| { handle.update(cx, |_, window, cx| window.click(page, cx)).unwrap(); });
                        for _ in 0..12 {
                            cx.background_executor().timer(Duration::from_millis(30)).await;
                            cx.update(|cx| { handle.update(cx, |_, window, cx| window.render_frame(cx)).unwrap(); });
                        }
                        if page == "sidebar-hosts" {
                            cx.update(|cx| {
                                handle.update(cx, |_, window, cx| {
                                    let top = window.find("hosts-collection-scroll").bounds().top() + px(8.0);
                                    let offset = top - window.find(id).bounds().top();
                                    window.scroll("hosts-collection-scroll", gpui_kit::ScrollDelta::Pixels(point(px(0.0), offset)), cx);
                                }).unwrap();
                            });
                            cx.update(|cx| { handle.update(cx, |_, window, cx| window.render_frame(cx)).unwrap(); });
                        }
                        let grid_width = cx.update(|cx| {
                            handle.update(cx, |_, window, _| {
                                let group = window.find(id);
                                assert_eq!(group.role(), Some(gpui_kit::Role::Group));
                                assert_eq!(group.label(), Some("视图模式"));
                                if let Some(expected) = control_size {
                                    assert_eq!(group.bounds().size, expected, "all view switchers must have identical geometry");
                                } else {
                                    control_size = Some(group.bounds().size);
                                }
                                for (suffix, selected, label) in [("grid", true, "网格视图"), ("list", false, "列表视图")] {
                                    let button = window.find(gpui_kit::SharedString::from(format!("{id}-{suffix}")));
                                    assert_eq!(button.role(), Some(gpui_kit::Role::Button));
                                    assert_eq!(button.checked(), Some(selected));
                                    assert_eq!(button.label(), Some(label));
                                }
                                window.render_to_image().unwrap().save(output.with_file_name(format!("{id}-grid.png"))).unwrap();
                                window.find(grid_content).bounds().size.width
                            }).unwrap()
                        });
                        // Sample real intermediate geometry, then reverse before the
                        // transition finishes. Use stable domain IDs in both modes.
                        let (collection_id, layout_id, second_id) = match id {
                            "hosts-view-mode" => ("library-host", "library-host-preview-0-layout", "library-host-preview-1-layout"),
                            "forwarding-view-mode" => ("forwarding-grid", "forward-rule-layout-preview-0-card-0", "forward-rule-layout-preview-0-card-1"),
                            _ => ("snippets-grid", "snippet-layout-card-0", "snippet-layout-card-1"),
                        };
                        let indicator_id = gpui_kit::SharedString::from(format!("{id}-indicator"));
                        let (start_x, end_x, start_width, end_width, second_start) = cx.update(|cx| {
                            handle.update(cx, |_, window, cx| {
                                let start_x = window.find(indicator_id.clone()).bounds().left();
                                let end_x = window.find(gpui_kit::SharedString::from(format!("{id}-list"))).bounds().left();
                                let start_width = window.find(layout_id).bounds().size.width;
                                let end_width = window.find(collection_id).bounds().size.width;
                                let second_start = window.find(second_id).bounds().origin - window.find(collection_id).bounds().origin;
                                window.click(gpui_kit::SharedString::from(format!("{id}-list")), cx);
                                window.render_frame(cx);
                                (start_x, end_x, start_width, end_width, second_start)
                            }).unwrap()
                        });
                        cx.background_executor().timer(Duration::from_millis(25)).await;
                        let middle_x = cx.update(|cx| {
                            handle.update(cx, |_, window, cx| {
                                window.render_frame(cx);
                                let middle_x = window.find(indicator_id.clone()).bounds().left();
                                let middle_width = window.find(layout_id).bounds().size.width;
                                assert!(middle_x > start_x && middle_x < end_x, "{id}: selection pill must slide through intermediate positions");
                                if end_width > start_width + px(1.0) {
                                    assert!(middle_width > start_width && middle_width < end_width, "{id}: real cards must resize through intermediate widths");
                                    let second_middle = window.find(second_id).bounds().origin - window.find(collection_id).bounds().origin;
                                    assert!(second_middle.x > px(0.0) && second_middle.x < second_start.x, "{id}: the second card must move between columns");
                                    assert!(second_middle.y > second_start.y, "{id}: the second card must move toward its list row");
                                }
                                window.render_to_image().unwrap().save(output.with_file_name(format!("{id}-motion.png"))).unwrap();
                                window.click(gpui_kit::SharedString::from(format!("{id}-grid")), cx);
                                window.render_frame(cx);
                                let reversed_x = window.find(indicator_id.clone()).bounds().left();
                                assert!(reversed_x > start_x, "{id}: reversal must continue from the current position");
                                if end_width > start_width + px(1.0) {
                                    assert!(window.find(layout_id).bounds().size.width > start_width, "{id}: reversing cards must keep their current width");
                                }
                                middle_x
                            }).unwrap()
                        });
                        for _ in 0..12 {
                            cx.background_executor().timer(Duration::from_millis(30)).await;
                            cx.update(|cx| { handle.update(cx, |_, window, cx| window.render_frame(cx)).unwrap(); });
                        }
                        cx.update(|cx| {
                            handle.update(cx, |_, window, cx| {
                                assert!(window.find(indicator_id.clone()).bounds().left() < middle_x);
                                assert_eq!(window.find(indicator_id.clone()).bounds().left(), start_x);
                                cx.set_reduce_motion(true);
                                window.click(gpui_kit::SharedString::from(format!("{id}-list")), cx);
                                window.render_frame(cx);
                                assert_eq!(window.find(indicator_id.clone()).bounds().left(), end_x, "{id}: reduced motion must settle immediately");
                                assert_eq!(window.find(layout_id).bounds().size.width, end_width, "{id}: reduced motion must settle card layout immediately");
                                window.click(gpui_kit::SharedString::from(format!("{id}-grid")), cx);
                                window.render_frame(cx);
                                cx.set_reduce_motion(false);
                            }).unwrap();
                        });
                        // Pointer, Enter and Space must all update the real page content.
                        for (list, keyboard) in [(true, None), (false, Some("enter")), (true, Some("space"))] {
                            let button_id = gpui_kit::SharedString::from(format!("{id}-{}", if list { "list" } else { "grid" }));
                            if keyboard.is_some() {
                                // Hidden native windows have no initial keyboard focus.
                                cx.update(|cx| {
                                    handle.update(cx, |_, window, cx| {
                                        window.blur(cx);
                                        window.focus_next(cx);
                                        window.render_frame(cx);
                                    }).unwrap();
                                });
                                for _ in 0..40 {
                                    let focused = cx.update(|cx| {
                                        handle.update(cx, |_, window, _| window.find(button_id.clone()).focused() == Some(true)).unwrap()
                                    });
                                    if focused { break; }
                                    cx.update(|cx| { handle.update(cx, |_, window, cx| window.press("tab", cx)).unwrap(); });
                                    cx.background_executor().timer(Duration::from_millis(20)).await;
                                }
                            }
                            cx.update(|cx| {
                                handle.update(cx, |_, window, cx| {
                                    if let Some(activate_key) = keyboard {
                                        assert_eq!(window.find(button_id).focused(), Some(true), "{id}: both view buttons must be keyboard reachable");
                                        window.press(activate_key, cx);
                                    } else {
                                        window.click(button_id, cx);
                                    }
                                }).unwrap();
                            });
                            for _ in 0..12 {
                                cx.background_executor().timer(Duration::from_millis(30)).await;
                                cx.update(|cx| { handle.update(cx, |_, window, cx| window.render_frame(cx)).unwrap(); });
                            }
                            if page == "sidebar-hosts" {
                                cx.update(|cx| {
                                    handle.update(cx, |_, window, cx| {
                                        let top = window.find("hosts-collection-scroll").bounds().top() + px(8.0);
                                        let offset = top - window.find(id).bounds().top();
                                        window.scroll("hosts-collection-scroll", gpui_kit::ScrollDelta::Pixels(point(px(0.0), offset)), cx);
                                    }).unwrap();
                                });
                            }
                            cx.update(|cx| {
                                handle.update(cx, |_, window, _| {
                                    for (suffix, selected) in [("grid", !list), ("list", list)] {
                                        assert_eq!(window.find(gpui_kit::SharedString::from(format!("{id}-{suffix}"))).checked(), Some(selected));
                                    }
                                    if let Some(row) = list_content {
                                        assert_eq!(window.try_find(grid_content).is_some(), !list);
                                        assert_eq!(window.try_find(row).is_some(), list);
                                    } else if list {
                                        assert!(window.find(grid_content).bounds().size.width > grid_width, "hosts must change from grid tracks to full-width rows");
                                    }
                                }).unwrap();
                            });
                        }
                        cx.update(|cx| {
                            handle.update(cx, |_, window, cx| {
                                window.click(gpui_kit::SharedString::from(format!("{id}-list")), cx);
                                window.render_to_image().unwrap().save(output.with_file_name(format!("{id}-list.png"))).unwrap();
                            }).unwrap();
                        });
                        if let Some(row) = list_content {
                            let close = if page == "sidebar-forwarding" { "forwarding-editor-cancel" } else { "snippets-editor-cancel" };
                            if page == "sidebar-snippets" {
                                cx.update(|cx| { handle.update(cx, |_, window, cx| {
                                    let top = window.find("snippets-collection-scroll").bounds().top() + px(8.0);
                                    let offset = top - window.find(row).bounds().top();
                                    window.scroll("snippets-collection-scroll", gpui_kit::ScrollDelta::Pixels(point(px(0.0), offset)), cx);
                                }).unwrap(); });
                            }
                            cx.update(|cx| { handle.update(cx, |_, window, cx| window.click(row, cx)).unwrap(); });
                            for _ in 0..12 {
                                cx.background_executor().timer(Duration::from_millis(30)).await;
                                cx.update(|cx| { handle.update(cx, |_, window, cx| window.render_frame(cx)).unwrap(); });
                            }
                            cx.update(|cx| { handle.update(cx, |_, window, cx| {
                                assert!(window.try_find(close).is_some(), "{id}: animated cards must retain their real editor action");
                                window.click(close, cx);
                            }).unwrap(); });
                            for _ in 0..12 {
                                cx.background_executor().timer(Duration::from_millis(30)).await;
                                cx.update(|cx| { handle.update(cx, |_, window, cx| window.render_frame(cx)).unwrap(); });
                            }
                        }
                    }
                    // Each page owns its mode; navigation must retain all three selections.
                    for (page, id, _, _) in pages {
                        cx.update(|cx| { handle.update(cx, |_, window, cx| window.click(page, cx)).unwrap(); });
                        for _ in 0..12 {
                            cx.background_executor().timer(Duration::from_millis(30)).await;
                            cx.update(|cx| { handle.update(cx, |_, window, cx| window.render_frame(cx)).unwrap(); });
                        }
                        cx.update(|cx| {
                            handle.update(cx, |_, window, _| {
                                assert_eq!(window.find(gpui_kit::SharedString::from(format!("{id}-list"))).checked(), Some(true));
                            }).unwrap();
                        });
                    }
                    println!("PASS: all three view switchers animate the indicator and real card geometry, reverse continuously, honor reduced motion, support pointer/Enter/Space and retain their own mode");
                }
                if scenario == "adaptive-cards" {
                    for (page, grid, cards, editor_trigger, editor_close) in [
                        ("sidebar-keychain", "keychain-managed-grid", (0..7).map(|i| format!("managed-key-card-card-{i}")).collect::<Vec<_>>(), "keychain-create-fab".to_owned(), "keychain-editor-footer-cancel".to_owned()),
                        ("sidebar-forwarding", "forwarding-grid", (0..7).map(|i| format!("forward-rule-card-preview-0-card-{i}")).collect(), "forward-rule-card-preview-0-card-0".to_owned(), "forwarding-editor-cancel".to_owned()),
                        ("sidebar-snippets", "snippet-packages-grid", (0..7).map(|i| format!("snippet-package-card-分组 {i}")).collect(), "snippet-card-card-0".to_owned(), "snippets-editor-cancel".to_owned()),
                        ("sidebar-known-hosts", "trusted-hosts-grid", adaptive_trust_ids.clone(), adaptive_trust_ids[0].clone(), "known-host-sidebar-close".to_owned()),
                    ] {
                        cx.update(|cx| { handle.update(cx, |_, window, cx| window.click(page, cx)).unwrap(); });
                        for _ in 0..12 {
                            cx.background_executor().timer(Duration::from_millis(30)).await;
                            cx.update(|cx| { handle.update(cx, |_, window, cx| window.render_frame(cx)).unwrap(); });
                        }
                        for viewport_width in [width, 960.0, 1242.0, 1440.0, 1920.0] {
                            cx.update(|cx| { handle.update(cx, |_, window, _| window.resize(size(px(viewport_width), px(height)))).unwrap(); });
                            cx.background_executor().timer(Duration::from_millis(100)).await;
                            cx.update(|cx| {
                                handle.update(cx, |_, window, cx| {
                                    window.render_frame(cx);
                                    assert_catalog_grid(window, grid, &cards);
                                    window.render_to_image().unwrap().save(output.with_file_name(format!("{page}-{}.png", viewport_width as u32))).unwrap();
                                }).unwrap();
                            });
                        }
                        // At the reference viewport, exercise the actual drawer command.
                        cx.update(|cx| { handle.update(cx, |_, window, _| window.resize(size(px(1242.0), px(height)))).unwrap(); });
                        cx.background_executor().timer(Duration::from_millis(100)).await;
                        cx.update(|cx| { handle.update(cx, |_, window, cx| {
                            window.render_frame(cx);
                            if page == "sidebar-snippets" {
                                let top = window.find("snippets-collection-scroll").bounds().top() + px(8.0);
                                let offset = top - window.find(gpui_kit::SharedString::from(editor_trigger.clone())).bounds().top();
                                window.scroll("snippets-collection-scroll", gpui_kit::ScrollDelta::Pixels(point(px(0.0), offset)), cx);
                            }
                            window.click(gpui_kit::SharedString::from(editor_trigger.clone()), cx);
                        }).unwrap(); });
                        for _ in 0..12 {
                            cx.background_executor().timer(Duration::from_millis(30)).await;
                            cx.update(|cx| { handle.update(cx, |_, window, cx| window.render_frame(cx)).unwrap(); });
                        }
                        cx.update(|cx| { handle.update(cx, |_, window, _| {
                            assert_catalog_grid(window, grid, &cards);
                            window.render_to_image().unwrap().save(output.with_file_name(format!("{page}-drawer.png"))).unwrap();
                        }).unwrap(); });
                        // The bottom navigation is intentionally hidden while an editor
                        // drawer is open. Close it before navigating to the next page.
                        cx.update(|cx| { handle.update(cx, |_, window, cx| {
                            if page == "sidebar-known-hosts" {
                                window.press("escape", cx);
                            } else {
                                window.click(gpui_kit::SharedString::from(editor_close.clone()), cx);
                            }
                        }).unwrap(); });
                        for _ in 0..12 {
                            cx.background_executor().timer(Duration::from_millis(30)).await;
                            cx.update(|cx| { handle.update(cx, |_, window, cx| window.render_frame(cx)).unwrap(); });
                        }
                    }
                    println!("PASS: all catalog grids keep equal bounded tracks at 800/960/1242/1440/1920 and with editor drawers");
                }
                if scenario == "global-search" {
                    for page in ["sidebar-hosts", "sidebar-keychain", "sidebar-forwarding", "sidebar-snippets", "sidebar-known-hosts"] {
                        cx.update(|cx| {
                            handle.update(cx, |_, window, cx| {
                                window.click(page, cx);
                            }).unwrap();
                        });
                        for _ in 0..12 {
                            cx.background_executor().timer(Duration::from_millis(30)).await;
                            cx.update(|cx| { handle.update(cx, |_, window, cx| window.render_frame(cx)).unwrap(); });
                        }
                        cx.update(|cx| {
                            handle.update(cx, |_, window, _| {
                                assert!(window.try_find("search-input-region").is_some(), "{page} must retain global search");
                                window.render_to_image().unwrap().save(output.with_file_name(format!("catalog-{page}.png"))).unwrap();
                            }).unwrap();
                        });
                        cx.update(|cx| {
                            handle.update(cx, |_, window, cx| {
                                window.click("search-input-region", cx);
                                window.press("ctrl-a", cx);
                                window.input("SSH", cx);
                            }).unwrap();
                        });
                        cx.background_executor().timer(Duration::from_millis(80)).await;
                        cx.update(|cx| {
                            handle.update(cx, |_, window, cx| {
                                window.render_frame(cx);
                                for id in ["search-result:host:preview-0", "search-result:forward:preview-0:search-rule", "search-result:setting:settings.openssh_integration.mode.label"] {
                                    assert!(window.try_find(id).is_some(), "{page} must include {id}");
                                }
                                window.render_to_image().unwrap().save(output.with_file_name(format!("global-search-{page}.png"))).unwrap();
                                window.press("escape", cx);
                            }).unwrap();
                        });
                        if page == "sidebar-keychain" {
                            cx.update(|cx| {
                                handle.update(cx, |_, window, cx| {
                                    window.click("search-input-region", cx);
                                    window.click("search-result:forward:preview-0:search-rule", cx);
                                }).unwrap();
                            });
                            for _ in 0..12 {
                                cx.background_executor().timer(Duration::from_millis(30)).await;
                                cx.update(|cx| { handle.update(cx, |_, window, cx| window.render_frame(cx)).unwrap(); });
                            }
                            cx.update(|cx| {
                                handle.update(cx, |_, window, _| {
                                    assert!(!window.find_all_by_label("保存规则").is_empty(), "forward result must open its rule editor");
                                }).unwrap();
                            });
                        }
                    }
                    cx.update(|cx| {
                        handle.update(cx, |_, window, cx| {
                            window.click("search-input-region", cx);
                            window.press("ctrl-a", cx);
                            window.input("界面字号", cx);
                        }).unwrap();
                    });
                    cx.background_executor().timer(Duration::from_millis(80)).await;
                    cx.update(|cx| {
                        handle.update(cx, |_, window, cx| {
                            window.render_frame(cx);
                            window.click("search-result:setting:settings.appearance.interface_font_size.label", cx);
                        }).unwrap();
                    });
                    for _ in 0..12 {
                        cx.background_executor().timer(Duration::from_millis(30)).await;
                        cx.update(|cx| { handle.update(cx, |_, window, cx| window.render_frame(cx)).unwrap(); });
                    }
                    cx.update(|cx| {
                        handle.update(cx, |_, window, _| {
                            assert!(window.try_find("settings-interface-font-size-0-1-3-inc").is_some(), "setting result must select the typography group");
                            assert!(window.try_find("search-results").is_none(), "navigation must dismiss global results");
                        }).unwrap();
                    });
                    println!("PASS: hosts, forwarding and settings are searchable from every management page; settings result navigates to typography");
                }
                if scenario == "tab-context-focus" {
                    cx.update(|cx| {
                        handle.update(cx, |_, window, cx| {
                            window.render_frame(cx);
                            let tabs = window.find_all_by_role(gpui_kit::Role::Tab);
                            let source = tabs[0].bounds();
                            let position = point(source.left() + px(70.0), source.center().y);
                            for down in [true, false] {
                                if down {
                                    window.dispatch_event(gpui_kit::MouseDownEvent { position, button: gpui_kit::MouseButton::Right, ..Default::default() }.to_platform_input(), cx);
                                } else {
                                    window.dispatch_event(gpui_kit::MouseUpEvent { position, button: gpui_kit::MouseButton::Right, ..Default::default() }.to_platform_input(), cx);
                                }
                            }
                        }).unwrap();
                    });
                    cx.background_executor().timer(Duration::from_millis(150)).await;
                    cx.update(|cx| {
                        handle.update(cx, |_, window, cx| {
                            window.render_frame(cx);
                            assert!(!window.find_all_by_role(gpui_kit::Role::MenuItem).is_empty(), "right click must open the tab menu");
                            window.press("escape", cx);
                        }).unwrap();
                    });
                    cx.background_executor().timer(Duration::from_millis(200)).await;
                }
                if scenario == "navigation" {
                    // Include interrupted transitions, not only settled pages:
                    // both surfaces build their accessibility nodes during animation.
                    for delay in [350, 30] {
                        for target in ["sidebar-settings", "sidebar-hosts", "sidebar-keychain", "sidebar-hosts", "sidebar-forwarding", "sidebar-hosts", "sidebar-snippets", "sidebar-hosts", "sidebar-known-hosts", "sidebar-hosts", "sidebar-hosts"] {
                            println!("Navigate ({delay} ms): {target}");
                            cx.update(|cx| {
                                handle.update(cx, |_, window, cx| {
                                    window.click(target, cx);
                                    window.render_frame(cx);
                                }).unwrap();
                            });
                            for _ in 0..if delay == 350 { 12 } else { 1 } {
                                cx.background_executor().timer(Duration::from_millis(30)).await;
                                cx.update(|cx| {
                                    handle.update(cx, |_, window, cx| window.render_frame(cx)).unwrap();
                                });
                            }
                        }
                    }
                    // Hidden native windows need explicit frames to finish animation.
                    for _ in 0..12 {
                        cx.background_executor().timer(Duration::from_millis(30)).await;
                        cx.update(|cx| {
                            handle.update(cx, |_, window, cx| window.render_frame(cx)).unwrap();
                        });
                    }
                    cx.update(|cx| {
                        handle.update(cx, |_, window, cx| {
                            window.render_frame(cx);
                            assert!(window.try_find("hosts-view-mode-grid").is_some(), "navigation must return to the hosts page");
                            assert_eq!(window.elements().iter().filter(|element| {
                                element.path().last() == Some(&gpui_kit::ElementId::from("hosts-create-fab"))
                            }).count(), 1, "hosts must expose exactly one create button");
                            window.click("hosts-create-fab", cx);
                        }).unwrap();
                    });
                    cx.background_executor().timer(Duration::from_millis(350)).await;
                    cx.update(|cx| {
                        handle.update(cx, |_, window, cx| {
                            window.render_frame(cx);
                            assert!(window.try_find("host-editor-save").is_some(), "the surviving create command must open the host editor");
                        }).unwrap();
                    });
                    println!("PASS: settled and interrupted management navigation, repeated hosts click, unique create button and working host editor");
                }
                cx.update(|cx| {
                    handle
                        .update(cx, |_, window, cx| {
                            window.render_frame(cx);
                            let chrome_scene = scenario.strip_prefix("reference-").unwrap_or(&scenario);
                            if chrome_scene == "tabs-scroll" {
                                verify_tab_wheel_scrolling(window, cx);
                            }
                            if chrome_scene == "tab-context-focus" {
                                let labels: Vec<_> = window.elements().into_iter().filter(|element| {
                                    format!("{:?}", element.path().last()).contains("top-tab-label-")
                                }).collect();
                                println!("Tab label focus after menu dismissal: {:?}", labels.iter().map(|label| label.focused()).collect::<Vec<_>>());
                                assert_eq!(labels.len(), 2);
                                assert!(labels.iter().all(|label| label.focused() == Some(false)), "a pointer context menu must not leave focus on the nested tab label");
                                let tabs = window.find_all_by_role(gpui_kit::Role::Tab);
                                assert_eq!(tabs[0].selected(), Some(false));
                                assert_eq!(tabs[1].selected(), Some(true));
                            }
                            if matches!(chrome_scene, "chrome" | "chrome-active" | "chrome-close") {
                                let mut tabs: Vec<_> = window.find_all_by_role(gpui_kit::Role::Tab)
                                    .into_iter().collect();
                                tabs.sort_by(|a, b| f32::from(a.bounds().origin.x).total_cmp(&f32::from(b.bounds().origin.x)));
                                let tab = &tabs[usize::from(chrome_scene == "chrome-active")];
                                let mut position = tab.bounds().center();
                                if chrome_scene == "chrome-close" {
                                    position.x = tab.bounds().right() - px(22.0 * font / 14.0);
                                }
                                window.dispatch_event(gpui_kit::MouseMoveEvent {
                                    position,
                                    ..Default::default()
                                }.to_platform_input(), cx);
                                window.render_frame(cx);
                            }
                            #[cfg(target_os = "windows")]
                            if reference_typography {
                                verify_reference_fonts(window, &output);
                            }
                            assert!(
                                window.try_find("footer-session-info-trigger").is_none(),
                                "a preview interaction must never open a connection"
                            );
                            if scenario == "editor" {
                                assert!(
                                    window.try_find("host-editor-save").is_some(),
                                    "FAB must open the existing editor"
                                );
                            }
                            // Capture the settled surface after the transition wait.
                            window.render_frame(cx);
                            window
                                .render_to_image()
                                .expect("capture native GPUI frame")
                                .save(&output)
                                .expect("save preview screenshot");
                            let layout: Vec<_> = [
                                "search-input-region", "host-group-filters", "sidebar-hosts", "sidebar-settings",
                                "quick-host-dev:connect", "quick-host-local:connect", "library-host-dev:connect",
                                "hosts-view-mode", "hosts-create-fab", "window-minimize", "top-add-host", "status-footer-context", "topbar-window-drag",
                            ].into_iter().filter_map(|id| {
                                window.try_find(id).map(|element| {
                                    let bounds = element.bounds();
                                    serde_json::json!({"id": id, "x": f32::from(bounds.origin.x), "y": f32::from(bounds.origin.y), "w": f32::from(bounds.size.width), "h": f32::from(bounds.size.height)})
                                })
                            }).collect();
                            std::fs::write(output.with_extension("layout.json"), serde_json::to_vec_pretty(&layout).unwrap()).unwrap();
                            println!("Saved {}", output.display());
                        })
                        .unwrap();
                    if !interactive {
                        cx.quit();
                    }
                });
            })
            .detach();
        });
}

/// Exercise the production shell with real pointer dispatch and overflowing tabs.
/// A tab's native-caption occlusion must not swallow vertical or horizontal wheels.
fn verify_tab_wheel_scrolling(window: &mut gpui_kit::Window, cx: &mut App) {
    use gpui_kit::{ScrollDelta, ScrollWheelEvent};
    let add_bounds = window.find("top-add-host").bounds();
    let origin = window.bounds().origin;
    let first_x = |window: &gpui_kit::Window| {
        window.find_all_by_role(gpui_kit::Role::Tab)[0]
            .bounds()
            .origin
            .x
    };
    let wheel = |window: &mut gpui_kit::Window,
                 position: gpui_kit::Point<gpui_kit::Pixels>,
                 delta,
                 cx: &mut App| {
        window.dispatch_event(
            gpui_kit::MouseMoveEvent {
                position,
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
        window.dispatch_event(
            ScrollWheelEvent {
                position,
                delta,
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
    };
    assert!(
        first_x(window) < px(0.0),
        "the active last tab must initially be revealed"
    );
    for close_button in [false, true] {
        let tab = window
            .find_all_by_role(gpui_kit::Role::Tab)
            .into_iter()
            .find(|tab| tab.bounds().left() >= px(20.0) && tab.bounds().right() < add_bounds.left())
            .expect("fully visible tab");
        let bounds = tab.bounds();
        let position = point(
            if close_button {
                bounds.right() - px(22.0)
            } else {
                bounds.left() + px(60.0)
            },
            bounds.center().y,
        );
        let before = first_x(window);
        wheel(window, position, ScrollDelta::Lines(point(0.0, 3.0)), cx);
        assert_eq!(
            first_x(window) - before,
            px(120.0),
            "vertical wheel over tab/close must scroll exactly once"
        );
        wheel(
            window,
            position,
            ScrollDelta::Pixels(point(px(-120.0), px(0.0))),
            cx,
        );
        assert_eq!(
            first_x(window),
            before,
            "horizontal wheel must reverse the movement"
        );
    }
    let before = first_x(window);
    wheel(
        window,
        point(px(300.0), px(6.0)),
        ScrollDelta::Lines(point(0.0, 3.0)),
        cx,
    );
    assert_eq!(
        first_x(window) - before,
        px(120.0),
        "empty titlebar padding must scroll tabs"
    );
    wheel(
        window,
        add_bounds.center(),
        ScrollDelta::Pixels(point(px(0.0), px(10000.0))),
        cx,
    );
    let start = first_x(window);
    assert!(start >= px(0.0), "scrolling back must reveal the first tab");
    wheel(
        window,
        add_bounds.center(),
        ScrollDelta::Pixels(point(px(0.0), px(10000.0))),
        cx,
    );
    assert_eq!(first_x(window), start, "start boundary must clamp");
    wheel(
        window,
        add_bounds.center(),
        ScrollDelta::Pixels(point(px(0.0), px(-10000.0))),
        cx,
    );
    let end = first_x(window);
    wheel(
        window,
        add_bounds.center(),
        ScrollDelta::Pixels(point(px(0.0), px(-10000.0))),
        cx,
    );
    assert_eq!(first_x(window), end, "end boundary must clamp");
    assert_eq!(window.find_all_by_role(gpui_kit::Role::Tab).len(), 9);
    assert_eq!(window.find("top-add-host").bounds(), add_bounds);
    assert_eq!(
        window.bounds().origin,
        origin,
        "wheel must not move the OS window"
    );
    println!(
        "PASS: overflowing tabs scroll over labels, close buttons, new-tab and empty chrome; horizontal/vertical input, both boundaries, fixed controls"
    );
}

/// Validate the glyphs used by the native shaper, including the CJK fallback.
/// A family name in a settings file alone does not prove that it rendered.
#[cfg(target_os = "windows")]
fn verify_reference_fonts(window: &mut gpui_kit::Window, output: &std::path::Path) {
    use gpui_kit::{FontWeight, SharedString, TextRun};
    let mut measurements = Vec::new();
    for (text, size, weight, expected_family) in [
        ("PowerShell", 16.0, FontWeight::SEMIBOLD, "Roboto Flex"),
        ("快捷连接", 16.0, FontWeight::SEMIBOLD, "Microsoft YaHei UI"),
        ("本地终端", 12.0, FontWeight::NORMAL, "Microsoft YaHei UI"),
        ("搜索主机", 16.0, FontWeight::NORMAL, "Microsoft YaHei UI"),
    ] {
        let mut configured = miaominal_settings::interface_font();
        configured.weight = weight;
        let mut expected = gpui_kit::font(expected_family);
        expected.weight = weight;
        let text: SharedString = text.into();
        let font_size = miaominal_settings::scaled_font_size(size);
        let observed = window.text_system().shape_line(
            text.clone(),
            font_size,
            &[TextRun {
                len: text.len(),
                font: configured,
                ..Default::default()
            }],
            None,
        );
        let reference = window.text_system().shape_line(
            text.clone(),
            font_size,
            &[TextRun {
                len: text.len(),
                font: expected,
                ..Default::default()
            }],
            None,
        );
        let glyphs = |line: &gpui_kit::ShapedLine| {
            line.runs
                .iter()
                .flat_map(|run| run.glyphs.iter().map(|glyph| glyph.id.0))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            glyphs(&observed),
            glyphs(&reference),
            "{text} must render with {expected_family}"
        );
        measurements.push(serde_json::json!({
            "text": text.as_str(), "font_size": f32::from(font_size),
            "weight": weight.0, "verified_family": expected_family,
            "width": f32::from(observed.width),
        }));
    }
    std::fs::write(
        output.with_extension("fonts.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "viewport": {
                "width": f32::from(window.viewport_size().width),
                "height": f32::from(window.viewport_size().height),
            },
            "device_scale_factor": window.scale_factor(),
            "samples": measurements,
        }))
        .unwrap(),
    )
    .unwrap();
}

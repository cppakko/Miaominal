use super::*;
use gpui_kit::component::{
    Selectable as _,
    setting::{RenderOptions, SettingFieldElement},
};
use miaominal_settings::{RdpClientKind, RdpClientSettings};

fn choices() -> Vec<(RdpClientKind, String)> {
    let mut choices = vec![(RdpClientKind::Auto, i18n::string("rdp.auto"))];
    #[cfg(windows)]
    choices.push((RdpClientKind::Mstsc, "mstsc".into()));
    #[cfg(target_os = "macos")]
    choices.push((RdpClientKind::WindowsApp, "Windows App".into()));
    #[cfg(target_os = "linux")]
    choices.extend([
        (RdpClientKind::Remmina, "Remmina".into()),
        (RdpClientKind::SdlFreeRdp, "SDL3 FreeRDP".into()),
    ]);
    choices
}

/// The Linux detection order only matters (and is only shown) on Linux.
pub(super) fn settings_description() -> String {
    let description = i18n::string("rdp.local_only");
    if cfg!(target_os = "linux") {
        format!("{description} {}", i18n::string("rdp.linux_detection"))
    } else {
        description
    }
}

impl SettingsController {
    pub(super) fn save_rdp_path(&mut self, path: String, cx: &mut Context<Self>) {
        let kind = self.settings().rdp_client.kind();
        self.settings_store
            .update(|settings| settings.rdp_client = RdpClientSettings::new(kind, path.trim()));
        self.detect_rdp(cx);
        cx.notify();
    }
    fn set_rdp_kind(&mut self, kind: RdpClientKind, window: &mut Window, cx: &mut Context<Self>) {
        // Re-selecting the current client must not discard its configured path.
        if self.settings().rdp_client.kind() == kind {
            return;
        }
        self.settings_store
            .update(|settings| settings.rdp_client = RdpClientSettings::new(kind, ""));
        set_input_value(&self.forms.rdp_path_input, "", window, cx);
        self.detect_rdp(cx);
        cx.notify();
    }
    fn apply_picked_rdp_path(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        set_input_value(&self.forms.rdp_path_input, path.clone(), window, cx);
        self.save_rdp_path(path, cx);
    }
    pub(super) fn detect_rdp(&mut self, cx: &mut Context<Self>) {
        self.rdp_detection_revision = self.rdp_detection_revision.wrapping_add(1);
        let revision = self.rdp_detection_revision;
        self.rdp_detection = i18n::string("rdp.detecting");
        let settings = self.settings().rdp_client.clone();
        let work = self.runtime.spawn_blocking(move || {
            miaominal_services::resolve_rdp_client(&settings)
                .map(|client| client.path().display().to_string())
                .map_err(|e| e.to_string())
        });
        cx.spawn(async move |entity, cx| {
            let result = work.await;
            let _ = entity.update(cx, |owner, cx| {
                if owner.rdp_detection_revision != revision {
                    return;
                }
                owner.rdp_detection = match result {
                    Ok(Ok(path)) => i18n::string_args("rdp.detected", &[("path", &path)]),
                    Ok(Err(error)) => error,
                    Err(error) => error.to_string(),
                };
                cx.notify();
            });
        })
        .detach();
    }
}

#[derive(Clone)]
pub(super) struct RdpClientField {
    entity: Entity<SettingsController>,
}
impl RdpClientField {
    pub(super) fn new(entity: Entity<SettingsController>) -> Self {
        Self { entity }
    }
}
impl SettingFieldElement for RdpClientField {
    type Element = AnyElement;
    fn render_field(&self, options: &RenderOptions, _: &mut Window, cx: &mut App) -> AnyElement {
        let controller = self.entity.read(cx);
        let kind = controller.settings().rdp_client.kind();
        let input = controller.forms.rdp_path_input.clone();
        let detection = controller.rdp_detection.clone();
        let roles = miaominal_settings::current_theme().material.roles;
        let mut buttons = h_flex().gap_2().flex_wrap();
        for (choice, label) in choices() {
            let entity = self.entity.clone();
            buttons = buttons.child(
                Button::new(SharedString::from(format!("rdp-kind-{choice:?}")))
                    .label(label)
                    .ghost()
                    .selected(kind == choice)
                    .on_click(move |_, window, cx| {
                        entity.update(cx, |owner, cx| owner.set_rdp_kind(choice, window, cx))
                    }),
            );
        }
        let entity = self.entity.clone();
        let browse_entity = self.entity.clone();
        v_flex()
            .w_full()
            .gap_3()
            .child(buttons)
            .when(kind != RdpClientKind::Auto, |view| {
                view.child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .items_center()
                        .child(
                            div().flex_1().min_w_0().child(
                                surface_text_input(&input, TextInputSurface::Highest)
                                    .with_size(options.size()),
                            ),
                        )
                        .child(
                            Button::new("rdp-browse")
                                .label(i18n::string("rdp.browse"))
                                .ghost()
                                .on_click(move |_, window, cx| {
                                    let dialog = rfd::AsyncFileDialog::new()
                                        .set_parent(window)
                                        .set_title(i18n::string("rdp.browse_title"));
                                    let window_handle = window.window_handle();
                                    let entity = browse_entity.clone();
                                    cx.spawn(async move |cx| {
                                        let Some(file) = dialog.pick_file().await else {
                                            return;
                                        };
                                        let path = file.path().display().to_string();
                                        cx.update(move |cx| {
                                            if let Err(error) =
                                                window_handle.update(cx, move |_, window, cx| {
                                                    entity.update(cx, |owner, cx| {
                                                        owner.apply_picked_rdp_path(
                                                            path, window, cx,
                                                        )
                                                    });
                                                })
                                            {
                                                log::debug!(
                                                    "failed to apply the selected RDP client: {error:?}"
                                                );
                                            }
                                        });
                                    })
                                    .detach();
                                }),
                        ),
                )
            })
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .text_color(rgb(roles.on_surface_variant))
                            .child(detection),
                    )
                    .child(
                        Button::new("rdp-detect")
                            .label(i18n::string("rdp.detect"))
                            .ghost()
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |owner, cx| owner.detect_rdp(cx))
                            }),
                    ),
            )
            .into_any_element()
    }
}

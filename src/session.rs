//! Session controls live in one dropdown on every screen.
//! Only offline sources are supported until the live backend contract is agreed.
use crate::{ThreadsApp, layout};
use gpui_kit::{
    component::{
        Icon,
        button::Button,
        menu::{DropdownMenu, PopupMenuItem},
    },
    prelude::FluentBuilder,
    *,
};

impl ThreadsApp {
    pub(crate) fn session_menu(&self, cx: &Context<Self>) -> impl IntoElement {
        let app = cx.entity().downgrade();
        div()
            .debug_selector(|| "session-menu-trigger".into())
            .flex_shrink_0()
            .child(
                Button::new("session-menu")
                    .label("Offline")
                    .icon(Icon::new(assets::IconName::CircleDashed))
                    .dropdown_caret(true)
                    .accessibility_label("Offline session menu")
                    .tooltip("Session source and actions")
                    .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, cx| {
                        let Some(view) = app.upgrade() else {
                            return menu;
                        };
                        // Read when opened, not when the toolbar was rendered.
                        let state = view.read(cx);
                        let loading = state.loading.is_some();
                        let can_reload = state.source.is_some() && !loading;
                        let kind = if state.recording_elf.is_some() {
                            "Offline · Recording"
                        } else if state.source.is_some() {
                            "Offline · Dump"
                        } else {
                            "Offline · Bundled sample"
                        };
                        let source = state
                            .source
                            .as_ref()
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|| "Bundled real dump · sys_heap_v4.3, frame 2".into());
                        let elf = state
                            .recording_elf
                            .as_ref()
                            .map(|p| format!("ELF: {}", p.display()));
                        let open_dump = app.clone();
                        let open_recording = app.clone();
                        let reload = app.clone();
                        let cancel = app.clone();
                        menu.min_w(px(300.))
                            .max_w(px(360.))
                            .label(kind)
                            .item(
                                PopupMenuItem::element(move |_, _| {
                                    div()
                                        .debug_selector(|| "session-menu-source".into())
                                        .w(px(280.))
                                        .min_w_0()
                                        .flex()
                                        .flex_col()
                                        .gap_1()
                                        .text_xs()
                                        .child(layout::note("session-menu-path", source.clone()))
                                        .when_some(elf.clone(), |el, elf| {
                                            el.child(layout::note("session-menu-elf", elf))
                                        })
                                })
                                .disabled(true),
                            )
                            .separator()
                            .item(PopupMenuItem::new("Connect to target…").disabled(true))
                            .label("Live connection is not implemented yet.")
                            .separator()
                            .item(PopupMenuItem::new("Open dump…").disabled(loading).on_click(
                                move |event, window, cx| {
                                    let _ = open_dump.update(cx, |app, cx| {
                                        app.open(event, window, cx);
                                    });
                                },
                            ))
                            .item(
                                PopupMenuItem::new("Open recording…")
                                    .disabled(loading)
                                    .on_click(move |event, window, cx| {
                                        let _ = open_recording.update(cx, |app, cx| {
                                            app.open_recording(event, window, cx);
                                        });
                                    }),
                            )
                            .item(
                                PopupMenuItem::new("Reload current file")
                                    .disabled(!can_reload)
                                    .on_click(move |_, _, cx| {
                                        let _ = reload.update(cx, |app, cx| {
                                            if let Some(path) = app.source.clone() {
                                                app.start_load(path, app.recording_elf.clone(), cx);
                                            }
                                        });
                                    }),
                            )
                            .when(loading, |menu| {
                                menu.separator()
                                    .item(PopupMenuItem::new("Cancel import").on_click(
                                        move |_, _, cx| {
                                            let _ = cancel.update(cx, |app, _| {
                                                if let Some(cancel) = &app.loading {
                                                    cancel.store(
                                                        true,
                                                        std::sync::atomic::Ordering::Relaxed,
                                                    );
                                                }
                                            });
                                        },
                                    ))
                            })
                    }),
            )
    }
}

//! Rust browse controls request the local machine's system dialog, never a custom file list.
use super::*;
use crate::adapter_protocol::{FileChoice, PickerKind};

impl Draft {
    fn path_input(&self, kind: PickerKind) -> &Entity<InputState> {
        match kind {
            PickerKind::Directory => &self.build,
            PickerKind::Elf => &self.elf,
            PickerKind::Recording => &self.recording,
        }
    }

    pub(in crate::web_adapter) fn recording_path(&self, cx: &App) -> String {
        Self::value(&self.recording, cx)
    }
}

impl ThreadsApp {
    pub(super) fn path_field(
        &self,
        label: &'static str,
        kind: PickerKind,
        cx: &Context<Self>,
    ) -> AnyElement {
        let id = match kind {
            PickerKind::Directory => "browse-build",
            PickerKind::Elf => "browse-elf",
            PickerKind::Recording => "browse-recording",
        };
        div()
            .flex()
            .flex_col()
            .gap_1()
            .min_w_0()
            .child(div().text_xs().text_color(rgb(MUTED)).child(label))
            .child(
                Input::new(self.adapter.draft.path_input(kind))
                    .aria_label(label)
                    .suffix(
                        Button::new(id)
                            .small()
                            .ghost()
                            .icon(if kind == PickerKind::Directory {
                                IconName::Folder
                            } else {
                                IconName::File
                            })
                            .accessibility_label(format!("Browse {label} using the system dialog"))
                            .tooltip(kind.title())
                            .loading(self.adapter.draft.picker == Some(kind))
                            .disabled(
                                self.adapter.state.pending || self.adapter.draft.picker.is_some(),
                            )
                            .on_click(
                                cx.listener(move |app, _, _, cx| app.choose_native_path(kind, cx)),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn choose_native_path(&mut self, kind: PickerKind, cx: &mut Context<Self>) {
        if !self.adapter.draft.open
            || self.adapter.state.pending
            || self.adapter.draft.picker.is_some()
        {
            return;
        }
        let current = Draft::value(self.adapter.draft.path_input(kind), cx);
        let mode = self.adapter.draft.mode;
        self.adapter.draft.picker_revision = self.adapter.draft.picker_revision.wrapping_add(1);
        let revision = self.adapter.draft.picker_revision;
        self.adapter.draft.picker = Some(kind);
        self.adapter.draft.error = None;
        cx.notify();
        spawn_local(async move {
            let result = async {
                let response = Request::post("/api/files/choose")
                    .json(&json!({"kind": kind, "current": current}))
                    .map_err(|e| e.to_string())?
                    .send()
                    .await
                    .map_err(|e| e.to_string())?;
                if !response.ok() {
                    let error = response
                        .json::<serde_json::Value>()
                        .await
                        .map_err(|e| e.to_string())?;
                    return Err(error["error"]
                        .as_str()
                        .unwrap_or("Cannot open the system dialog.")
                        .to_owned());
                }
                response
                    .json::<FileChoice>()
                    .await
                    .map_err(|e| e.to_string())
            }
            .await;
            let _ = web::update_view(|app, window, cx| {
                if app.adapter.draft.picker_revision != revision || !app.adapter.draft.open {
                    return;
                }
                app.adapter.draft.picker = None;
                // A selection must not overwrite edits made while the OS dialog was open.
                if app.adapter.draft.mode == mode
                    && Draft::value(app.adapter.draft.path_input(kind), cx) == current
                {
                    match result {
                        Ok(FileChoice { path: Some(path) }) => {
                            app.adapter
                                .draft
                                .path_input(kind)
                                .update(cx, |input, cx| input.set_value(path, window, cx));
                            app.session_draft_changed(window, cx);
                        }
                        Ok(FileChoice { path: None }) => {}
                        Err(error) => app.adapter.draft.error = Some(error),
                    }
                    let focus = app.adapter.draft.path_input(kind).read(cx).focus_handle(cx);
                    focus.focus(window, cx);
                }
                cx.notify();
            });
        });
    }
}

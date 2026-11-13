//! Session configuration stays a draft until an HTTP command is accepted.
//! This child of web_adapter uses its transport, not a second session owner.
use super::*;
use crate::adapter_protocol::{
    BuildContext, Connection, Mode, SessionAction, SessionIndicator, Source, session_labels,
};
use gpui_kit::component::input::Input;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tab::{Tab, TabBar};

#[path = "web_native_picker.rs"]
mod picker;

pub(super) struct Draft {
    pub open: bool,
    pub mode: Mode,
    pub error: Option<String>,
    pub resolved: Option<(serde_json::Value, BuildContext)>,
    generation: u64,
    resolving: bool,
    resolution_revision: u64,
    scheduled_request: Option<serde_json::Value>,
    pub(super) advanced: bool,
    bound_build: String,
    build: Entity<InputState>,
    domain: Entity<InputState>,
    elf: Entity<InputState>,
    recording: Entity<InputState>,
    runner: Entity<InputState>,
    target: Entity<InputState>,
    clock: Entity<InputState>,
    pub(super) picker: Option<crate::adapter_protocol::PickerKind>,
    picker_revision: u64,
    mode_focus: FocusHandle,
}

impl Draft {
    pub fn new(window: &mut Window, cx: &mut Context<ThreadsApp>) -> Self {
        let mode_focus = cx.focus_handle();
        let mut input =
            |placeholder| cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        Self {
            open: false,
            mode: Mode::Replay,
            error: None,
            resolved: None,
            generation: 0,
            resolving: false,
            resolution_revision: 0,
            scheduled_request: None,
            advanced: false,
            bound_build: String::new(),
            build: input("Build directory under the server data root"),
            domain: input("Default firmware image"),
            elf: input("Matching ELF, or leave empty to use the build"),
            recording: input("Recording under the server data root (.ndjson.gz)"),
            runner: input("Runner from the build"),
            target: input("MCU or GDB host:port from the build"),
            clock: input("Default probe clock (Hz)"),
            picker: None,
            picker_revision: 0,
            mode_focus,
        }
    }

    pub fn inputs(&self) -> Vec<Entity<InputState>> {
        vec![
            self.build.clone(),
            self.domain.clone(),
            self.elf.clone(),
            self.recording.clone(),
            self.runner.clone(),
            self.target.clone(),
            self.clock.clone(),
        ]
    }

    pub fn dismiss(&mut self) {
        self.open = false;
        self.generation = self.generation.wrapping_add(1);
        self.resolving = false;
        self.resolution_revision = self.resolution_revision.wrapping_add(1);
        self.scheduled_request = None;
        self.resolved = None;
        self.error = None;
        self.picker = None;
        self.picker_revision = self.picker_revision.wrapping_add(1);
    }

    fn value(input: &Entity<InputState>, cx: &App) -> String {
        input.read(cx).value().trim().to_string()
    }

    pub(super) fn request(&self, cx: &App) -> serde_json::Value {
        let mut data = json!({"mode": self.mode, "build_dir": Self::value(&self.build, cx)});
        if self.mode == Mode::Live && !self.advanced {
            return data;
        }
        for (key, input) in [("domain", &self.domain), ("elf", &self.elf)] {
            let value = Self::value(input, cx);
            if !value.is_empty() {
                data[key] = json!(value);
            }
        }
        if self.mode == Mode::Live {
            for (key, input) in [("runner", &self.runner), ("target", &self.target)] {
                let value = Self::value(input, cx);
                if !value.is_empty() {
                    data[key] = json!(value);
                }
            }
            let clock = Self::value(&self.clock, cx);
            if !clock.is_empty() {
                // An invalid number is sent as text so the server rejects it,
                // rather than silently ignoring the user's override.
                data["frequency"] = clock.parse::<u64>().map_or(json!(clock), |v| json!(v));
            }
        }
        data
    }

    pub(super) fn context(&self, cx: &App) -> Option<&BuildContext> {
        self.resolved
            .as_ref()
            .and_then(|(request, context)| (*request == self.request(cx)).then_some(context))
    }

    fn can_open(&self, state: &ClientState, cx: &App) -> bool {
        self.picker.is_none()
            && state.can_open()
            && match self.mode {
                Mode::Live => !Self::value(&self.build, cx).is_empty(),
                Mode::Replay => {
                    !Self::value(&self.recording, cx).is_empty()
                        && (!Self::value(&self.elf, cx).is_empty()
                            || !Self::value(&self.build, cx).is_empty())
                }
            }
    }
}

fn field(label: &'static str, input: &Entity<InputState>) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .min_w_0()
        .child(div().text_xs().text_color(rgb(MUTED)).child(label))
        .child(Input::new(input).aria_label(label))
}

fn provenance_note(context: &BuildContext) -> impl IntoElement {
    let checkout = context
        .git
        .as_ref()
        .map(|git| {
            format!(
                "{} · {}{}",
                git.branch.as_deref().unwrap_or("detached HEAD"),
                git.commit.chars().take(8).collect::<String>(),
                if git.dirty { " · tracked changes" } else { "" }
            )
        })
        .unwrap_or_else(|| "Git unavailable".into());
    let tooltip = context.tooltip();
    div()
        .id("resolved-provenance")
        .debug_selector(|| "resolved-provenance".into())
        .text_xs()
        .text_color(rgb(MUTED))
        .min_w_0()
        .truncate()
        .child(format!(
            "{} · Checkout at resolution: {checkout}",
            context.project
        ))
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
}

impl ThreadsApp {
    pub(crate) fn adapter_session_controls(&self, cx: &Context<Self>) -> Option<AnyElement> {
        if !self.adapter.available {
            return None;
        }
        let state = &self.adapter.state;
        let indicator = self.adapter.indicator();
        let mut labels = session_labels(state.session.as_ref(), state.connection);
        if state.is_live()
            && !self.adapter.fresh_frame()
            && state
                .session
                .as_ref()
                .is_some_and(|s| s.status == crate::adapter_protocol::Status::Reading)
        {
            labels[2] = "— samples/s".into();
        }
        let mode = if state.is_live() { "LIVE" } else { "REPLAY" };
        let status = if state.connection != Connection::Connected {
            state.connection.label().to_owned()
        } else if state.pending {
            "Updating session…".into()
        } else if indicator == SessionIndicator::Waiting {
            format!("{mode} · Waiting for frames")
        } else if indicator == SessionIndicator::Warning {
            format!("{mode} · Read warning")
        } else {
            labels[0].clone()
        };
        let mut tooltip = labels.join("\n");
        tooltip.push_str("\nGreen: receiving frames. Amber: waiting for frames or a read warning. Red: acquisition failed. Gray: inactive or adapter disconnected.");
        if indicator == SessionIndicator::Waiting {
            tooltip.push_str("\nNo fresh frame received within the five-second freshness window.");
        }
        if let Some(session) = &state.session {
            tooltip.push_str(&format!(
                "\nSource: {}\nELF: {}",
                session.source_label(),
                session.elf
            ));
            if let Some(context) = &session.context {
                tooltip.push_str(&format!("\n{}", context.tooltip()));
            }
            if let Some(notice) = &session.last_notice {
                tooltip.push_str(&format!("\nLast acquisition notice: {notice}"));
            }
            if let Some(error) = &session.error {
                tooltip.push_str(&format!("\nError: {error}"));
            }
            if session.mode() == Mode::Live {
                if let Some(timing) = &session.timing {
                    tooltip.push_str(&format!(
                        "\n{}\n{}",
                        timing.polling_label(),
                        timing.heap_label()
                    ));
                }
                tooltip.push_str("\nObserved samples/s counts successful frames received by the adapter over up to 5 seconds, before browser delivery; it is not lossless board acquisition.");
            } else {
                tooltip.push_str("\nDecode progress is not the original recording's sample rate.");
            }
        }
        let icon = match indicator {
            SessionIndicator::Busy => Spinner::new()
                .icon(assets::IconName::LoaderCircle)
                .small()
                .color(rgb(BLUE).into())
                .into_any_element(),
            SessionIndicator::Healthy => div()
                .size_2p5()
                .rounded_full()
                .bg(rgb(GREEN))
                .into_any_element(),
            SessionIndicator::Waiting | SessionIndicator::Warning => {
                Icon::new(assets::IconName::CircleAlert)
                    .small()
                    .text_color(rgb(0xe0b76d))
                    .into_any_element()
            }
            SessionIndicator::Failed => Icon::new(assets::IconName::CircleAlert)
                .small()
                .text_color(rgb(0xf08b9f))
                .into_any_element(),
            SessionIndicator::Idle => Icon::new(assets::IconName::CircleDashed)
                .small()
                .text_color(rgb(MUTED))
                .into_any_element(),
        };
        let action = state.primary_action();
        let action_icon = match action {
            SessionAction::Connect => assets::IconName::Plug,
            SessionAction::Disconnect => assets::IconName::Unplug,
            SessionAction::Reconnect => assets::IconName::RotateCw,
            SessionAction::OpenReplay => assets::IconName::Folder,
            SessionAction::StopReplay => assets::IconName::Pause,
        };
        Some(
            div()
                .id("session-header-controls")
                .debug_selector(|| "session-header-controls".into())
                .w(px(640.))
                .max_w_full()
                .min_w_0()
                .ml_auto()
                .p_2()
                .rounded_md()
                .border_1()
                .border_color(rgb(BORDER))
                .bg(rgb(CARD))
                .flex()
                .flex_wrap()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .id("session-header-context")
                        .min_w_0()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .min_w_0()
                                .child(
                                    div()
                                        .id("session-status-indicator")
                                        .debug_selector(move || {
                                            format!("session-indicator-{}", indicator.as_str())
                                        })
                                        .flex_shrink_0()
                                        .aria_label(format!("Session status: {status}"))
                                        .child(icon),
                                )
                                .child(layout::note("session-header-status", status))
                                .when(
                                    state.session.is_some()
                                        && state.connection == Connection::Connected,
                                    |el| {
                                        el.child(layout::note(
                                            "session-header-progress",
                                            labels[2].clone(),
                                        ))
                                    },
                                ),
                        )
                        .child(
                            layout::note(
                                "session-header-source",
                                format!("{} · {}", labels[1], labels[3]),
                            )
                            .w_full(),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .flex_shrink_0()
                        .gap_2()
                        .child(
                            Button::new("session-configure")
                                .small()
                                .w(px(116.))
                                .label("Configure…")
                                .icon(Icon::new(assets::IconName::Settings))
                                .accessibility_label("Configure session")
                                .tooltip("Build, target, recording and connection settings")
                                .disabled(state.pending)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.open_session_dialog(window, cx)
                                })),
                        )
                        .child(
                            Button::new("session-primary-action")
                                .small()
                                .primary()
                                .w(px(128.))
                                .label(action.label())
                                .icon(Icon::new(action_icon))
                                .accessibility_label(format!("{} session", action.label()))
                                .tooltip(match action {
                                    SessionAction::Connect => {
                                        "Configure and connect to a live target"
                                    }
                                    SessionAction::OpenReplay => "Choose or reopen a recording",
                                    SessionAction::Disconnect => {
                                        "Stop acquisition and release the target"
                                    }
                                    SessionAction::StopReplay => "Stop decoding the recording",
                                    SessionAction::Reconnect => {
                                        "Reconnect with the saved live configuration"
                                    }
                                })
                                .disabled(!state.can_primary_action())
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    if !this.adapter.state.can_primary_action() {
                                        return;
                                    }
                                    match action {
                                        SessionAction::Connect | SessionAction::OpenReplay => {
                                            this.open_session_dialog(window, cx);
                                            this.adapter.draft.mode =
                                                if action == SessionAction::Connect {
                                                    Mode::Live
                                                } else {
                                                    Mode::Replay
                                                };
                                            cx.notify();
                                        }
                                        SessionAction::Disconnect | SessionAction::StopReplay => {
                                            this.stop_adapter(cx)
                                        }
                                        SessionAction::Reconnect => this.reconnect_adapter(cx),
                                    }
                                })),
                        ),
                )
                .into_any_element(),
        )
    }

    pub(super) fn open_session_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.adapter.draft.open {
            return;
        }
        let session = self.adapter.state.session.clone();
        let context = session.as_ref().and_then(|s| s.context.as_ref());
        let configuration = session.as_ref().and_then(|s| match s.source.as_ref() {
            Source::Live { configuration, .. } => Some(configuration),
            _ => None,
        });
        let draft = &mut self.adapter.draft;
        draft.dismiss();
        draft.open = true;
        draft.mode = session.as_ref().map(|s| s.mode()).unwrap_or(Mode::Replay);
        draft.error = None;
        draft.resolved = None;
        draft.advanced = configuration.is_some_and(|c| {
            c.domain.is_some()
                || c.elf.is_some()
                || c.runner.is_some()
                || c.target.is_some()
                || c.frequency.is_some()
        });
        draft.bound_build = context.map(|c| c.build_dir.clone()).unwrap_or_default();
        let values = [
            (
                &draft.build,
                context.map(|c| c.build_dir.clone()).unwrap_or_default(),
            ),
            (
                &draft.domain,
                if let Some(configuration) = configuration {
                    configuration.domain.clone().unwrap_or_default()
                } else {
                    context.map(|c| c.domain.clone()).unwrap_or_default()
                },
            ),
            (
                &draft.elf,
                if let Some(context) = context {
                    context.elf_override.clone().unwrap_or_default()
                } else {
                    session.as_ref().map(|s| s.elf.clone()).unwrap_or_default()
                },
            ),
            (
                &draft.recording,
                session
                    .as_ref()
                    .and_then(|s| s.recording().map(str::to_owned))
                    .unwrap_or_default(),
            ),
            (
                &draft.runner,
                configuration
                    .and_then(|c| c.runner.clone())
                    .unwrap_or_default(),
            ),
            (
                &draft.target,
                configuration
                    .and_then(|c| c.target.clone())
                    .unwrap_or_default(),
            ),
            (
                &draft.clock,
                configuration
                    .and_then(|c| c.frequency)
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
            ),
        ];
        for (input, value) in values {
            input.update(cx, |input, cx| input.set_value(value, window, cx));
        }
        let view = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, cx| {
            let Some(app) = view.upgrade() else {
                return dialog;
            };
            let (body, footer, pending) = app.update(cx, |app, cx| {
                (
                    app.session_form(cx),
                    app.session_actions(cx),
                    app.adapter.state.pending,
                )
            });
            let close = view.clone();
            let confirm = view.clone();
            dialog
                .title("Session configuration")
                .w(px(560.))
                .margin_top(px(32.))
                .overlay_closable(false)
                .keyboard(!pending)
                .close_button(!pending)
                .on_ok(move |_, window, cx| {
                    let _ = confirm.update(cx, |app, cx| {
                        if !app.adapter.draft.mode_focus.is_focused(window) {
                            app.open_session_draft(cx);
                        }
                    });
                    // Close only once the server has accepted the command.
                    false
                })
                .on_close(move |_, _, cx| {
                    let _ = close.update(cx, |app, cx| {
                        app.adapter.draft.dismiss();
                        cx.notify();
                    });
                })
                .child(body)
                .footer(footer)
        });
        self.schedule_session_build(cx);
        cx.notify();
    }

    fn session_form(&self, cx: &Context<Self>) -> AnyElement {
        let draft = &self.adapter.draft;
        let live = draft.mode == Mode::Live;
        let context = draft.context(cx);
        div().debug_selector(|| "session-configuration".into())
            .flex().flex_col().gap_3().min_w_0()
            .child(div().id("session-mode-keyboard")
                .track_focus(&draft.mode_focus).tab_stop(true)
                .aria_label("Session mode. Use Left and Right arrow keys to switch.")
                .on_key_down(cx.listener(|app, event: &KeyDownEvent, window, cx| {
                    if app.adapter.state.pending || app.adapter.draft.picker.is_some() { return; }
                    let mode = match event.keystroke.key.as_str() {
                        "left" | "home" => Mode::Live,
                        "right" | "end" => Mode::Replay,
                        _ => return,
                    };
                    app.adapter.draft.mode = mode;
                    app.adapter.draft.error = None;
                    app.session_draft_changed(window, cx);
                    cx.stop_propagation();
                    cx.notify();
                }))
                .child(TabBar::new("session-mode-tabs")
                .segmented()
                .selected_index(usize::from(!live))
                .child(Tab::new().label("Live").disabled(self.adapter.state.pending || draft.picker.is_some()))
                .child(Tab::new().label("Replay").disabled(self.adapter.state.pending || draft.picker.is_some()))
                .on_click(cx.listener(|app, index: &usize, window, cx| {
                    app.adapter.draft.mode = if *index == 0 { Mode::Live } else { Mode::Replay };
                    app.adapter.draft.error = None;
                    app.adapter.draft.mode_focus.focus(window, cx);
                    app.session_draft_changed(window, cx);
                    cx.notify();
                }))))
            .child(layout::note("session-path-help", if draft.picker.is_some() {
                "The system file dialog is open. Select a path or cancel it to continue."
            } else {
                "Local app · Browse opens the system dialog · Paths must stay inside the configured data root."
            }))
            .child(self.path_field(if live { "Build directory" } else { "Build directory (optional)" }, crate::adapter_protocol::PickerKind::Directory, cx))
            .when(live, |el| el.child(Button::new("advanced-session-overrides")
                .label("Advanced overrides").toggled(draft.advanced)
                .disabled(self.adapter.state.pending || draft.picker.is_some())
                .on_click(cx.listener(|app, _, _, cx| {
                    app.adapter.draft.advanced = !app.adapter.draft.advanced;
                    app.schedule_session_build(cx);
                    cx.notify();
                }))))
            .when(!live || draft.advanced, |el| el
                .child(field("Firmware image (optional)", &draft.domain))
                .child(self.path_field(if live { "ELF override (optional)" } else { "Matching ELF (required without a build)" }, crate::adapter_protocol::PickerKind::Elf, cx)))
            .when(!live, |el| el.child(self.path_field("Recording", crate::adapter_protocol::PickerKind::Recording, cx)))
            .when(live && draft.advanced, |el| el
                .child(field("Runner override (optional)", &draft.runner))
                .child(field("MCU / GDB server override (optional)", &draft.target))
                .child(field("Probe clock override in Hz (optional)", &draft.clock)))
            .child(Button::new("resolve-build").label(if draft.resolving { "Detecting…" } else if live { "Refresh detected settings" } else { "Resolve build context" })
                .disabled(draft.resolving || Draft::value(&draft.build, cx).is_empty() || self.adapter.state.pending)
                .on_click(cx.listener(|app, _, _, cx| app.resolve_session_build(cx))))
            .when_some(context, |el, context| {
                el.child(div().flex().flex_col().gap_1()
                    .child(layout::note("resolved-board", format!("Board: {}", context.board.as_deref().unwrap_or("Unknown"))))
                    .child(layout::note("resolved-image", format!("Firmware image: {} · Available: {}", context.domain, context.domains.join(", "))))
                    .child(layout::note("resolved-elf", format!("ELF: {}", context.elf)))
                    .when(live, |el| el.child(layout::note("resolved-runner", format!("Runner: {} · Target: {} · Clock: {}",
                        context.runner.as_deref().unwrap_or("Default"),
                        context.target.as_deref().unwrap_or("Default"),
                        context.frequency.map(|v| format!("{v} Hz")).unwrap_or_else(|| "Default".into())))))
                    .when(context.domains.len() > 1, |el| {
                        el.child(div().flex().flex_wrap().gap_1().children(
                            context.domains.iter().enumerate().map(|(i, domain)| {
                                let name = domain.clone();
                                Button::new(SharedString::from(format!("firmware-image-{i}")))
                                    .label(name.clone()).toggled(context.domain == name)
                                    .on_click(cx.listener(move |app, _, window, cx| {
                                        if app.adapter.draft.mode == Mode::Live {
                                            app.adapter.draft.advanced = true;
                                        }
                                        app.adapter.draft.domain.update(cx, |input, cx| {
                                            input.set_value(name.clone(), window, cx)
                                        });
                                        app.resolve_session_build(cx);
                                    }))
                            })
                        ))
                    })
                    .child(provenance_note(context))
                    .children(context.warnings.iter().enumerate().map(|(i, warning)| layout::note(format!("build-warning-{i}"), warning.clone()))))
            })
            .when_some(draft.error.clone(), |el, error| el.child(layout::note("session-draft-error", error)))
            .when_some(self.adapter.state.session.as_ref().and_then(|s| s.last_notice.clone()),
                |el, notice| el.child(layout::note("adapter-notice", notice)))
            .when_some(self.adapter.state.session.as_ref().and_then(|s| s.capabilities.as_ref()),
                |el, capabilities| el
                    .child(layout::note("live-groups", format!("Discovered groups: {}", capabilities.groups.join(", "))))
                    .child(layout::note("wait-queue-capability", if capabilities.is_waitq_walkable {
                        "Firmware wait-queue layout is walkable."
                    } else { "Firmware wait-queue layout cannot be walked; unavailable waiters are not empty queues." }))
                    .children(capabilities.missing.iter().enumerate().map(|(i, notice)|
                        layout::note(format!("capability-notice-{i}"), notice.clone()))))
            .child(layout::note("session-mode-help", if live {
                "Only the build directory is required. ELF, debug runner, target and probe clock are detected automatically. Advanced overrides are optional. Connect opens the probe; it does not flash firmware."
            } else {
                "Replay decodes up to 10 frames/s, not original recording time. Last 256 frames retained."
            }))
            .into_any_element()
    }

    fn session_actions(&self, cx: &Context<Self>) -> AnyElement {
        let draft = &self.adapter.draft;
        div()
            .flex()
            .flex_wrap()
            .gap_2()
            .justify_end()
            .child(
                Button::new("cancel-session")
                    .label("Cancel")
                    .disabled(self.adapter.state.pending)
                    .on_click(cx.listener(|app, _, window, cx| {
                        app.adapter.draft.dismiss();
                        window.close_dialog(cx);
                        cx.notify();
                    })),
            )
            .child(
                Button::new("adapter-open")
                    .label(if draft.mode == Mode::Live {
                        "Connect"
                    } else {
                        "Open replay"
                    })
                    .disabled(!draft.can_open(&self.adapter.state, cx))
                    .on_click(cx.listener(|app, _, _, cx| app.open_session_draft(cx))),
            )
            .into_any_element()
    }

    fn open_session_draft(&mut self, cx: &mut Context<Self>) {
        if !self.adapter.draft.can_open(&self.adapter.state, cx) {
            return;
        }
        let mut request = self.adapter.draft.request(cx);
        let data = request.as_object_mut().expect("draft request is an object");
        data.remove("mode");
        if Draft::value(&self.adapter.draft.build, cx).is_empty() {
            data.remove("build_dir");
        }
        let live = self.adapter.draft.mode == Mode::Live;
        if !live {
            data.insert(
                "recording".into(),
                json!(Draft::value(&self.adapter.draft.recording, cx)),
            );
        }
        self.adapter.draft.error = None;
        self.adapter_command(
            if live {
                "/api/session/live"
            } else {
                "/api/session/replay"
            },
            request,
            cx,
        );
    }

    pub(super) fn session_draft_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.adapter.draft.open {
            return;
        }
        let build = Draft::value(&self.adapter.draft.build, cx);
        if self.adapter.draft.mode == Mode::Live && build != self.adapter.draft.bound_build {
            self.adapter.draft.bound_build = build;
            self.adapter.draft.advanced = false;
            // Overrides belong to a particular build, not the next folder chosen.
            let draft = &self.adapter.draft;
            for input in [
                draft.domain.clone(),
                draft.elf.clone(),
                draft.runner.clone(),
                draft.target.clone(),
                draft.clock.clone(),
            ] {
                input.update(cx, |input, cx| input.set_value("", window, cx));
            }
        }
        self.schedule_session_build(cx);
        cx.notify();
    }

    fn schedule_session_build(&mut self, cx: &mut Context<Self>) {
        let request = self.adapter.draft.request(cx);
        let draft = &mut self.adapter.draft;
        if draft.scheduled_request.as_ref() == Some(&request) {
            return;
        }
        draft.scheduled_request = Some(request.clone());
        draft.resolution_revision = draft.resolution_revision.wrapping_add(1);
        draft.resolving = false;
        draft.resolved = None;
        draft.error = None;
        // Replay keeps its explicit preview button; live discovery is automatic.
        if draft.mode != Mode::Live || Draft::value(&draft.build, cx).is_empty() {
            return;
        }
        let generation = draft.generation;
        let revision = draft.resolution_revision;
        draft.resolving = true;
        spawn_local(async move {
            TimeoutFuture::new(250).await;
            let _ = web::update_view(|app, _, cx| {
                let draft = &app.adapter.draft;
                if draft.open
                    && draft.generation == generation
                    && draft.resolution_revision == revision
                    && draft.request(cx) == request
                {
                    app.start_build_resolution(request, revision, cx);
                }
            });
        });
    }

    fn resolve_session_build(&mut self, cx: &mut Context<Self>) {
        let request = self.adapter.draft.request(cx);
        self.adapter.draft.scheduled_request = Some(request.clone());
        self.adapter.draft.resolution_revision =
            self.adapter.draft.resolution_revision.wrapping_add(1);
        self.start_build_resolution(request, self.adapter.draft.resolution_revision, cx);
    }

    fn start_build_resolution(
        &mut self,
        request: serde_json::Value,
        revision: u64,
        cx: &mut Context<Self>,
    ) {
        let generation = self.adapter.draft.generation;
        self.adapter.draft.resolving = true;
        self.adapter.draft.error = None;
        cx.notify();
        spawn_local(async move {
            let result = async {
                let response = Request::post("/api/build/resolve")
                    .json(&request)
                    .map_err(|e| e.to_string())?
                    .send()
                    .await
                    .map_err(|e| e.to_string())?;
                if !response.ok() {
                    let data = response
                        .json::<serde_json::Value>()
                        .await
                        .map_err(|e| e.to_string())?;
                    return Err(data["error"]
                        .as_str()
                        .unwrap_or("Build resolution failed.")
                        .to_owned());
                }
                response
                    .json::<BuildContext>()
                    .await
                    .map_err(|e| e.to_string())
            }
            .await;
            let _ = web::update_view(|app, _, cx| {
                if !app.adapter.draft.open
                    || app.adapter.draft.generation != generation
                    || app.adapter.draft.resolution_revision != revision
                {
                    return;
                }
                app.adapter.draft.resolving = false;
                if request == app.adapter.draft.request(cx) {
                    match result {
                        Ok(context) => app.adapter.draft.resolved = Some((request, context)),
                        Err(error) => {
                            app.adapter.draft.resolved = None;
                            app.adapter.draft.error = Some(error);
                        }
                    }
                }
                cx.notify();
            });
        });
    }
}

/// Opens the same GPUI dialog as the header control, including its draft reset.
#[wasm_bindgen]
pub fn adapter_configure() -> Result<(), JsValue> {
    web::update_view(|app, window, cx| app.open_session_dialog(window, cx))
}

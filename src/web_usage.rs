//! Editable CPU/Load window; backend acknowledgement remains the applied value.
use super::*;
use crate::adapter_protocol::Session;
use gpui_kit::component::input::{NumberInput, NumberInputEvent, StepAction};

pub(super) struct UsageInput {
    pub state: Entity<InputState>,
    applied: Option<(String, u32)>,
    _subscriptions: Vec<Subscription>,
}

impl UsageInput {
    pub fn new(window: &mut Window, cx: &mut Context<ThreadsApp>) -> Self {
        let state = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value("16")
                .min(1.)
                .max(64.)
        });
        // Typing is a draft until Enter/blur; stepping applies immediately.
        // Opt out of the automatic step so it cannot submit an intermediate edit.
        state.update(cx, |input, cx| input.set_step(None, window, cx));
        let edit = cx.subscribe_in(&state, window, |app, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                app.commit_usage_input(window, cx);
            }
        });
        let step = cx.subscribe_in(&state, window, |app, _, event, window, cx| {
            let NumberInputEvent::Step(action) = event;
            if !app.can_edit_usage() {
                return;
            }
            let value = app
                .adapter
                .usage
                .state
                .read(cx)
                .value()
                .trim()
                .parse::<u32>()
                .ok()
                .filter(|v| (1..=64).contains(v))
                .or_else(|| app.adapter.state.session.as_ref()?.usage_window)
                .unwrap_or(16);
            let next = match action {
                StepAction::Increment => value.saturating_add(1).min(64),
                StepAction::Decrement => value.saturating_sub(1).max(1),
            };
            app.adapter.usage.state.update(cx, |input, cx| {
                input.set_value(next.to_string(), window, cx)
            });
            app.commit_usage_input(window, cx);
        });
        Self {
            state,
            applied: None,
            _subscriptions: vec![edit, step],
        }
    }

    pub fn sync(
        &mut self,
        session: Option<&Session>,
        window: &mut Window,
        cx: &mut Context<ThreadsApp>,
    ) {
        let applied = session.and_then(|s| s.usage_window.map(|polls| (s.id.clone(), polls)));
        if self.applied != applied {
            self.applied = applied;
            self.restore(window, cx);
        }
    }

    pub fn restore(&self, window: &mut Window, cx: &mut Context<ThreadsApp>) {
        let polls = self.applied.as_ref().map_or(16, |(_, polls)| *polls);
        self.state.update(cx, |input, cx| {
            input.set_value(polls.to_string(), window, cx)
        });
    }
}

impl ThreadsApp {
    fn can_edit_usage(&self) -> bool {
        self.adapter.state.session.as_ref().is_some_and(|session| {
            session.mode() == Mode::Live
                && session.status == Status::Reading
                && self.adapter.state.connection == Connection::Connected
                && !self.adapter.state.pending
                && session.live_view.as_ref().is_some_and(|view| {
                    view.owner.is_some()
                        && view.owner == self.adapter.client_id
                        && Some(view.usage_window) == session.usage_window
                })
        })
    }

    fn commit_usage_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_edit_usage() {
            self.adapter.usage.restore(window, cx);
            return;
        }
        let text = self.adapter.usage.state.read(cx).value().to_string();
        let Ok(polls) = text.trim().parse::<i64>() else {
            self.adapter.usage.restore(window, cx);
            return;
        };
        let polls = polls.clamp(1, 64) as u32;
        self.adapter.usage.state.update(cx, |input, cx| {
            input.set_value(polls.to_string(), window, cx)
        });
        if self
            .adapter
            .state
            .session
            .as_ref()
            .and_then(|s| s.usage_window)
            != Some(polls)
        {
            self.set_usage_window(polls, cx);
        }
    }

    pub(crate) fn adapter_usage_controls(&self, _cx: &Context<Self>) -> Option<AnyElement> {
        let session = self.adapter.state.session.as_ref()?;
        if session.mode() != Mode::Live {
            return None;
        }
        session.usage_window?;
        Some(div().id("cpu-averaging").debug_selector(|| "cpu-averaging".into())
            .flex().items_center().gap_2().flex_shrink_0()
            .aria_label("CPU and Load averaging samples, 1 to 64. Enter applies the edited value.")
            .tooltip(|window, cx| Tooltip::new(
                "CPU and Load over 1–64 polls. Type an integer and press Enter or leave the field to apply. Buttons and arrow keys step by one. Independent of chart zoom."
            ).build(window, cx))
            .child(div().text_xs().text_color(rgb(MUTED)).child("CPU / Load"))
            .child(div().debug_selector(|| "cpu-averaging-input".into()).w(px(104.))
                .child(NumberInput::new(&self.adapter.usage.state).small()
                    .disabled(!self.can_edit_usage())))
            .child(div().text_xs().text_color(rgb(MUTED)).child("samples"))
            .into_any_element())
    }
}

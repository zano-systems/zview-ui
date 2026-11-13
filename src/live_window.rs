//! One Live viewport preference for the screen; never an acquisition setting.
use crate::{ThreadsApp, live_history};
use gpui_kit::{
    component::{
        ActiveTheme, Sizable,
        button::{Button, ButtonVariants},
        slider::Slider,
    },
    *,
};

pub const MIN_SECONDS: f64 = 1.;
pub const MAX_SECONDS: f64 = live_history::RETENTION_SECONDS;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub seconds: f64,
    pub revision: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            seconds: live_history::DEFAULT_WINDOW_SECONDS,
            revision: 0,
        }
    }
}

impl Settings {
    pub fn set(&mut self, seconds: f64) -> bool {
        if !seconds.is_finite() {
            return false;
        }
        let seconds = seconds.round().clamp(MIN_SECONDS, MAX_SECONDS);
        if seconds == self.seconds {
            return false;
        }
        self.seconds = seconds;
        self.refresh();
        true
    }

    pub fn refresh(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }
}

impl ThreadsApp {
    pub(super) fn set_chart_window(
        &mut self,
        seconds: f64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.chart_window.set(seconds) {
            self.chart_window.refresh();
        }
        let value = self.chart_window.seconds as f32;
        self.window_slider
            .update(cx, |slider, cx| slider.set_value(value, window, cx));
        cx.notify();
    }

    pub(crate) fn chart_window_controls(&self, cx: &Context<Self>) -> impl IntoElement {
        div().id("live-time-window")
            .debug_selector(|| "live-time-window".into())
            .flex().items_center().gap_1().flex_shrink_0()
            .track_focus(&self.window_focus).tab_stop(true)
            .capture_any_mouse_down(cx.listener(|app, _, window, cx| {
                app.window_focus.focus(window, cx);
                cx.notify();
            }))
            .aria_label(format!("Chart time window: {:.0} seconds. Left and Right adjust by one second; Page Up and Page Down by ten; Home is one second; End is five minutes.",
                self.chart_window.seconds))
            .on_key_down(cx.listener(|app, event: &KeyDownEvent, window, cx| {
                let seconds = app.chart_window.seconds;
                let next = match event.keystroke.key.as_str() {
                    "left" | "down" => seconds - 1.,
                    "right" | "up" => seconds + 1.,
                    "pageup" | "page_up" => seconds + 10.,
                    "pagedown" | "page_down" => seconds - 10.,
                    "home" => MIN_SECONDS,
                    "end" => MAX_SECONDS,
                    _ => return,
                };
                app.set_chart_window(next, window, cx);
                cx.stop_propagation();
            }))
            .child(div().debug_selector(|| "live-time-window-label".into())
                .flex_shrink_0().mr_1().text_xs().line_height(px(16.)).child("Window"))
            .child(div().debug_selector(|| "live-time-window-track".into())
                .w(px(150.)).px_1()
                .child(Slider::new(&self.window_slider).w_full().bg(rgb(crate::PURPLE))
                    .text_color(cx.theme().foreground)))
            .child(div().debug_selector(|| "live-time-window-reset".into()).child(Button::new("live-time-window-reset")
                .small().ghost().w(px(32.)).h(px(24.)).px_0()
                .child(div().debug_selector(|| "live-time-window-duration".into())
                    .text_size(px(12.)).line_height(px(16.)).child(format!("{:.0}s", self.chart_window.seconds)))
                .accessibility_label(format!("Current chart window: {:.0} seconds. Reset all Live chart windows to 20 seconds.",
                    self.chart_window.seconds))
                .tooltip("Selected time window. Click to reset to 20 seconds. Range: 1 second to 5 minutes; changes the viewport, not sampling or stored history.")
                .on_click(cx.listener(|app, _, window, cx| {
                    app.set_chart_window(live_history::DEFAULT_WINDOW_SECONDS, window, cx);
                }))))
    }
}

#[cfg(test)]
mod tests {
    use super::Settings;
    use crate::live_history;

    #[test]
    fn window_defaults_and_limits_are_independent_of_retention() {
        let mut settings = Settings::default();
        assert_eq!(settings.seconds, 20.);
        assert!(settings.set(5.));
        assert_eq!(settings.seconds, 5.);
        assert_eq!(settings.revision, 1);
        assert!(!settings.set(5.));
        assert!(!settings.set(f64::NAN));
        settings.set(0.);
        assert_eq!(settings.seconds, 1.);
        settings.set(1000.);
        assert_eq!(settings.seconds, 300.);
        assert_eq!(live_history::RETENTION_SECONDS, 300.);
    }
}

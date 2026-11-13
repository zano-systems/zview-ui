//! Shared trailing-time viewport and wheel interaction for both plot renderers.
use crate::model::ChartSeries;
use gpui_kit::component::{
    IconName, Sizable,
    button::{Button, ButtonVariants},
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

#[derive(Default)]
pub(super) struct Zoom {
    factor: Option<f64>,
    live_seconds: Option<f64>,
    window: crate::live_window::Settings,
}

impl Zoom {
    pub fn sync_window(&mut self, window: crate::live_window::Settings) {
        if self.window != window {
            self.window = window;
            self.live_seconds = None;
        }
    }

    fn scroll(&mut self, delta: f32) {
        let multiplier = (f64::from(delta).clamp(-200., 200.) * 0.01).exp();
        self.factor = Some((self.factor.unwrap_or(1.) * multiplier).clamp(1., 128.));
    }

    pub fn domain(&self, series: &ChartSeries) -> (f64, f64) {
        if let Some(live) = &series.live {
            let span = self.live_seconds.unwrap_or(live.window.seconds);
            let span = if live.limited {
                span.min((series.x_domain.1 - series.x_domain.0).max(0.1))
            } else {
                span
            };
            let start = (series.x_domain.1 - span).max(series.x_domain.0);
            return (start, series.x_domain.1.max(start + 0.001));
        }
        let Some(factor) = self.factor else {
            return series.x_domain;
        };
        let Some(end) = series.view_end() else {
            return series.x_domain;
        };
        let start = (end - (series.x_domain.1 - series.x_domain.0) / factor).max(series.x_domain.0);
        if end > start {
            (start, end)
        } else {
            series.x_domain
        }
    }
    fn scroll_live(&mut self, delta: f32) {
        let multiplier = (f64::from(delta).clamp(-200., 200.) * 0.01).exp();
        self.live_seconds = Some(
            (self.live_seconds.unwrap_or(self.window.seconds) / multiplier)
                .clamp(0.1, crate::live_history::RETENTION_SECONDS),
        );
    }
}

pub(super) fn state(id: &'static str, window: &mut Window, cx: &mut App) -> Entity<Zoom> {
    window.use_keyed_state(SharedString::from(format!("{id}-time-zoom")), cx, |_, _| {
        Zoom::default()
    })
}

pub(super) fn wrap(
    id: &'static str,
    state: Entity<Zoom>,
    plot: impl IntoElement,
    live: bool,
    window: &mut Window,
    cx: &mut App,
) -> impl IntoElement {
    let view = window.current_view();
    let factor = state.read(cx).factor;
    let seconds = state
        .read(cx)
        .live_seconds
        .unwrap_or(state.read(cx).window.seconds);
    let base_seconds = state.read(cx).window.seconds;
    let reset = state.clone();
    let follow = state.clone();
    div()
        .id(SharedString::from(format!("{id}-zoom-surface")))
        .debug_selector(move || format!("{id}-zoom-surface"))
        .size_full()
        .flex()
        .flex_col()
        .min_h_0()
        .on_scroll_wheel(move |event, window, cx| {
            let delta = event.delta.pixel_delta(window.line_height()).y.as_f32();
            if delta != 0. {
                state.update(cx, |state, _| {
                    if live { state.scroll_live(delta); } else { state.scroll(delta); }
                });
                cx.stop_propagation();
                cx.notify(view);
            }
        })
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .flex_shrink_0()
                .gap_2()
                .child(
                    div()
                        .debug_selector(move || {
                            format!(
                                "{id}-{}",
                                if factor.is_some() {
                                    "zoomed"
                                } else {
                                    "unzoomed"
                                }
                            )
                        })
                        .text_xs()
                        .text_color(rgb(crate::MUTED))
                        .child(
                            if live {
                                format!("{seconds:.1}s · live")
                            } else {
                                factor.map(|factor| format!("{factor:.1}× · trailing"))
                                    .unwrap_or_else(|| "Scroll to zoom".into())
                            },
                        ),
                )
                .when(live, |el| el.child(
                    Button::new(SharedString::from(format!("{id}-follow-live")))
                        .small().ghost().label(format!("Last {base_seconds:.0}s"))
                        .tooltip(format!("Follow the selected {base_seconds:.0}-second window. Dense traces use pixel min/max envelopes; hover reads exact observations."))
                        .accessibility_label(format!("Follow live with the selected {base_seconds:.0}-second window"))
                        .on_click(move |_, _, cx| {
                            follow.update(cx, |state, _| state.live_seconds = None);
                            cx.notify(view);
                        }),
                ))
                .child(
                    div()
                        .debug_selector(move || format!("{id}-fit-history"))
                        .child(
                            Button::new(SharedString::from(format!("{id}-fit-history")))
                                .small()
                                .ghost()
                                .icon(IconName::Maximize)
                                .tooltip(if live { "Fit retained history (up to 5 minutes). Dense traces show pixel min/max envelopes." } else { "Fit history" })
                                .accessibility_label("Fit history")
                                .on_click(move |_, _, cx| {
                                    reset.update(cx, |state, _| {
                                        state.factor = None;
                                        state.live_seconds = Some(crate::live_history::RETENTION_SECONDS);
                                    });
                                    cx.notify(view);
                                }),
                        ),
                ),
        )
        .child(div().flex_1().min_h_0().child(plot))
}

#[cfg(test)]
mod tests {
    use super::Zoom;
    use crate::model::{ChartPoint, ChartSeries};
    #[test]
    fn domain_pins_current_time_and_bounds_zoom_without_mutating_source() {
        let series = ChartSeries {
            points: vec![ChartPoint { x: 8., value: None }],
            x_domain: (0., 10.),
            timed: true,
            caption: String::new(),
            playhead: None,
            live: None,
        };
        let mut zoom = Zoom {
            factor: Some(2.),
            ..Zoom::default()
        };
        assert_eq!(zoom.domain(&series), (3., 8.));
        assert_eq!(series.points[0].value, None);
        for _ in 0..20 {
            zoom.scroll(200.);
        }
        assert_eq!(zoom.factor, Some(128.));
        for _ in 0..20 {
            zoom.scroll(-200.);
        }
        assert_eq!(zoom.factor, Some(1.));
        assert_eq!(zoom.domain(&series), (0., 8.));
        zoom.factor = None;
        assert_eq!(zoom.domain(&series), (0., 10.));
    }

    #[test]
    fn trailing_view_moves_between_captures_without_adding_samples() {
        let mut series = ChartSeries {
            points: vec![ChartPoint {
                x: 8.,
                value: Some(3.),
            }],
            x_domain: (0., 10.),
            timed: true,
            caption: String::new(),
            playhead: Some(8.016),
            live: None,
        };
        let zoom = Zoom {
            factor: Some(2.),
            ..Zoom::default()
        };
        assert!((zoom.domain(&series).0 - 3.016).abs() < 1e-9);
        assert_eq!(zoom.domain(&series).1, 8.016);
        assert_eq!(series.interval_end(0), Some(8.016));
        assert_eq!(series.held_index(8.01), Some(0));
        assert_eq!(series.held_index(8.02), None);
        series.playhead = Some(8.032);
        assert!((zoom.domain(&series).0 - 3.032).abs() < 1e-9);
        assert_eq!(series.points.len(), 1);
        assert_eq!(series.points[0].x, 8.);
        assert_eq!(series.points[0].value, Some(3.));
        series.points[0].value = None;
        assert_eq!(series.held_index(8.02), None);
    }

    #[test]
    fn live_zoom_uses_seconds_without_changing_retention_or_observations() {
        let series = ChartSeries {
            points: vec![ChartPoint {
                x: 0.,
                value: Some(25.),
            }],
            x_domain: (100., 400.),
            timed: true,
            caption: String::new(),
            playhead: None,
            live: Some(crate::model::LiveSeries {
                ends: vec![400.],
                limited: false,
                window: crate::live_window::Settings::default(),
            }),
        };
        let mut zoom = Zoom::default();
        assert_eq!(zoom.domain(&series), (380., 400.));
        zoom.scroll_live(100.);
        let narrow = zoom.domain(&series);
        assert!(narrow.0 > 380.);
        assert_eq!(narrow.1, 400.);
        for _ in 0..10 {
            zoom.scroll_live(-200.);
        }
        assert_eq!(zoom.domain(&series), (100., 400.));
        zoom.live_seconds = None;
        assert_eq!(zoom.domain(&series), (380., 400.));
        assert_eq!(series.points.len(), 1);
        assert_eq!(series.x_domain, (100., 400.));
    }

    #[test]
    fn a_new_visit_grows_from_zero_before_the_time_window_starts_rolling() {
        let mut series = ChartSeries {
            points: vec![ChartPoint {
                x: 0.,
                value: Some(1.),
            }],
            x_domain: (0., 3.),
            timed: true,
            caption: String::new(),
            playhead: None,
            live: Some(crate::model::LiveSeries {
                ends: vec![3.],
                limited: false,
                window: crate::live_window::Settings::default(),
            }),
        };
        let zoom = Zoom::default();
        assert_eq!(zoom.domain(&series), (0., 3.));
        series.x_domain = (0., 61.);
        assert_eq!(zoom.domain(&series), (41., 61.));
    }

    #[test]
    fn changing_the_shared_window_resets_per_chart_zoom_without_changing_data() {
        let mut settings = crate::live_window::Settings::default();
        let mut series = ChartSeries {
            points: vec![ChartPoint {
                x: 0.,
                value: Some(1.),
            }],
            x_domain: (0., 100.),
            timed: true,
            caption: String::new(),
            playhead: None,
            live: Some(crate::model::LiveSeries {
                ends: vec![100.],
                limited: false,
                window: settings,
            }),
        };
        let mut zoom = Zoom::default();
        zoom.scroll_live(100.);
        assert!(zoom.domain(&series).0 > 80.);
        settings.set(5.);
        series.live.as_mut().unwrap().window = settings;
        zoom.sync_window(settings);
        assert_eq!(zoom.domain(&series), (95., 100.));
        assert_eq!(series.points.len(), 1);
        settings.set(120.);
        series.live.as_mut().unwrap().window = settings;
        zoom.sync_window(settings);
        assert_eq!(zoom.domain(&series), (0., 100.));
    }
}

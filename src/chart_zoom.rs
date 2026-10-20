//! Shared trailing-time viewport and wheel interaction for both plot renderers.
use crate::model::ChartSeries;
use gpui_kit::component::{
    IconName, Sizable,
    button::{Button, ButtonVariants},
};
use gpui_kit::*;

#[derive(Default)]
pub(super) struct Zoom {
    factor: Option<f64>,
}

impl Zoom {
    fn scroll(&mut self, delta: f32) {
        let multiplier = (f64::from(delta).clamp(-200., 200.) * 0.01).exp();
        self.factor = Some((self.factor.unwrap_or(1.) * multiplier).clamp(1., 128.));
    }

    pub fn domain(&self, series: &ChartSeries) -> (f64, f64) {
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
    window: &mut Window,
    cx: &mut App,
) -> impl IntoElement {
    let view = window.current_view();
    let factor = state.read(cx).factor;
    let reset = state.clone();
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
                state.update(cx, |state, _| state.scroll(delta));
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
                            factor
                                .map(|factor| format!("{factor:.1}× · trailing"))
                                .unwrap_or_else(|| "Scroll to zoom".into()),
                        ),
                )
                .child(
                    div()
                        .debug_selector(move || format!("{id}-fit-history"))
                        .child(
                            Button::new(SharedString::from(format!("{id}-fit-history")))
                                .small()
                                .ghost()
                                .icon(IconName::Maximize)
                                .tooltip("Fit history")
                                .accessibility_label("Fit history")
                                .on_click(move |_, _, cx| {
                                    reset.update(cx, |state, _| state.factor = None);
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
        };
        let mut zoom = Zoom { factor: Some(2.) };
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
        };
        let zoom = Zoom { factor: Some(2.) };
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
}

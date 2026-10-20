//! Sample-and-hold areas for independent nonnegative counts above/below zero.
use crate::model::ChartSeries;
use gpui_kit::component::{
    ActiveTheme,
    plot::{
        Plot, PlotElement, PlotLabel, TooltipState,
        label::Text,
        tooltip::{CrossLine, Dot, Tooltip},
    },
};
use gpui_kit::*;

const TOKENS: u32 = crate::PURPLE;
const WAITERS: u32 = 0xf18b94;

fn reading_color(negative: bool, value: f64) -> u32 {
    if negative {
        if value == 0. { crate::MUTED } else { WAITERS }
    } else {
        TOKENS
    }
}

#[derive(IntoElement)]
pub(super) struct StepAreaChart {
    tokens: ChartSeries,
    waiters: ChartSeries,
    limit: Option<f64>,
    maximum: f64,
    id: &'static str,
    upper_label: &'static str,
    upper_breaks: Vec<bool>,
    details: Vec<String>,
    ownership: Vec<Option<u64>>,
    limits: Vec<Option<u64>>,
}

#[derive(Clone, Copy)]
struct Geometry {
    left: f32,
    top: f32,
    width: f32,
    height: f32,
    maximum: f64,
}

impl Geometry {
    fn x(self, x: f64, domain: (f64, f64)) -> Pixels {
        px(self.left + ((x - domain.0) / (domain.1 - domain.0)) as f32 * self.width)
    }

    fn y(self, value: f64) -> Pixels {
        px(self.top + ((self.maximum - value) / (2. * self.maximum)) as f32 * self.height)
    }
}

impl StepAreaChart {
    pub fn new(tokens: ChartSeries, waiters: ChartSeries, limits: Vec<Option<u64>>) -> Self {
        let limit = limits
            .last()
            .copied()
            .flatten()
            .filter(|limit| *limit > 0 && *limit != 0xffff_ffff)
            .map(|n| n as f64);
        // Both halves use the same units. Only past/current source values set the
        // scale; a finite current limit remains visible even when count is zero.
        let maximum = tokens
            .points
            .iter()
            .chain(&waiters.points)
            .filter_map(|p| p.value)
            .fold(limit.unwrap_or(1.).max(1.), f64::max);
        Self {
            tokens,
            waiters,
            limit,
            maximum,
            id: "semaphore-step-area",
            upper_label: "Available tokens",
            upper_breaks: Vec::new(),
            details: Vec::new(),
            ownership: Vec::new(),
            limits,
        }
    }

    pub fn mutex(
        depth: ChartSeries,
        waiters: ChartSeries,
        owners: Vec<Option<u64>>,
        details: Vec<String>,
    ) -> Self {
        let mut chart = Self::new(depth, waiters, Vec::new());
        chart.id = "mutex-step-area";
        chart.upper_label = "Lock depth";
        chart.upper_breaks = owners
            .iter()
            .enumerate()
            .map(|(i, owner)| {
                i > 0
                    && matches!((owners[i - 1], *owner),
                (Some(a), Some(b)) if a != 0 && b != 0 && a != b)
            })
            .collect();
        chart.details = details;
        chart.ownership = owners;
        chart
    }

    fn sample_color(&self, index: usize, negative: bool, value: f64) -> u32 {
        if !negative && self.ownership.is_empty() {
            self.count_color(index, value)
        } else if negative && value == 0. {
            self.zero_line_color(index).unwrap_or(crate::MUTED)
        } else {
            reading_color(negative, value)
        }
    }

    fn count_color(&self, index: usize, value: f64) -> u32 {
        if value == 0. {
            return WAITERS;
        }
        match self.limits.get(index).copied().flatten() {
            Some(limit) if limit > 0 && limit != 0xffff_ffff && value == limit as f64 => {
                crate::GREEN
            }
            Some(limit) if limit > 0 && value < limit as f64 => 0xddb66b,
            _ => crate::MUTED,
        }
    }

    fn zero_line_color(&self, index: usize) -> Option<u32> {
        // Contention already has a below-zero area. Status on zero is only
        // meaningful when the queue is known empty.
        if self.ownership.is_empty() || self.waiters.points.get(index)?.value != Some(0.) {
            return None;
        }
        let depth = self.tokens.points.get(index)?.value?;
        match self.ownership.get(index).copied().flatten()? {
            0 if depth == 0. => Some(crate::GREEN),
            owner if owner != 0 && depth > 0. => Some(WAITERS),
            _ => None,
        }
    }

    fn upper_connects(&self, index: usize) -> bool {
        !self.upper_breaks.get(index).copied().unwrap_or(false)
    }

    fn geometry(&self, size: Size<Pixels>) -> Geometry {
        let left = (format!("-{:.0}", self.maximum).len() as f32 * 7. + 16.).max(48.);
        Geometry {
            left,
            top: 12.,
            width: (size.width.as_f32() - left - 12.).max(0.),
            height: (size.height.as_f32() - 48.).max(0.),
            maximum: self.maximum,
        }
    }

    // Hold the left reading until the next captured timestamp. If that next
    // reading is missing, stop there: the gap starts at the missing timestamp,
    // not one whole sample interval earlier.
    fn held_index(series: &ChartSeries, x: f64) -> Option<usize> {
        series.held_index(x)
    }

    fn paint_series(
        &self,
        series: &ChartSeries,
        negative: bool,
        g: Geometry,
        bounds: Bounds<Pixels>,
        window: &mut Window,
    ) {
        let sign = if negative { -1. } else { 1. };
        let zero = g.y(0.);
        for index in series.visible_range() {
            let a = &series.points[index];
            let Some(end_time) = series.interval_end(index) else {
                continue;
            };
            let Some(value) = a.value else { continue };
            let mut outline = PathBuilder::stroke(px(2.));
            let x = g.x(a.x, series.x_domain);
            let end = g.x(end_time, series.x_domain);
            let y = g.y(sign * value);
            window.paint_quad(fill(
                Bounds::new(
                    bounds.origin + point(x, y.min(zero)),
                    size(end - x, (y - zero).abs()),
                ),
                rgb(if negative { 0x533040 } else { 0x493762 }),
            ));
            outline.move_to(bounds.origin + point(x, y));
            outline.line_to(bounds.origin + point(end, y));
            // Do not overwrite a semaphore's zero-count outline with its
            // independent zero-waiter trace.
            if !(negative && value == 0. && self.ownership.is_empty())
                && let Ok(path) = outline.build()
            {
                window.paint_path(path, rgb(self.sample_color(index, negative, value)));
            }
            if let Some(next) = series.points.get(index + 1).and_then(|b| b.value)
                && next != value
                && (negative || self.upper_connects(index + 1))
            {
                // Semaphore count transitions through partially available
                // values are yellow; waiter transitions remain red.
                let mut transition = PathBuilder::stroke(px(2.));
                transition.move_to(bounds.origin + point(end, y));
                transition.line_to(bounds.origin + point(end, g.y(sign * next)));
                if let Ok(path) = transition.build() {
                    let color = if negative {
                        WAITERS
                    } else if self.ownership.is_empty() {
                        if self.count_color(index, value) == crate::MUTED
                            || self.count_color(index + 1, next) == crate::MUTED
                        {
                            crate::MUTED
                        } else {
                            0xddb66b
                        }
                    } else {
                        TOKENS
                    };
                    window.paint_path(path, rgb(color));
                }
            }
        }
    }
}

impl RenderOnce for StepAreaChart {
    fn render(mut self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = crate::chart_zoom::state(self.id, window, cx);
        let domain = state.read(cx).domain(&self.tokens);
        self.tokens.x_domain = domain;
        self.waiters.x_domain = domain;
        crate::chart_zoom::wrap(self.id, state, PlotElement::new(self), window, cx)
    }
}

impl Plot for StepAreaChart {
    fn id(&self) -> Option<ElementId> {
        Some(self.id.into())
    }

    fn paint(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        let g = self.geometry(bounds.size);
        if g.width <= 0. || g.height <= 0. {
            return;
        }
        let mut ticks = vec![-self.maximum, 0., self.maximum];
        if self.maximum <= 4. {
            ticks = (-(self.maximum as i64)..=self.maximum as i64)
                .map(|v| v as f64)
                .collect();
        } else {
            let mid = (self.maximum / 2.).floor();
            ticks.extend([-mid, mid]);
        }
        let mut labels = Vec::new();
        for value in ticks {
            let y = g.y(value);
            let mut path = PathBuilder::stroke(px(if value == 0. { 1.5 } else { 1. }));
            path.move_to(bounds.origin + point(px(g.left), y));
            path.line_to(bounds.origin + point(px(g.left + g.width), y));
            if let Ok(path) = path.build() {
                window.paint_path(path, cx.theme().border);
            }
            labels.push(
                Text::new(
                    format!("{value:.0}"),
                    point(px(g.left - 10.), y - px(6.)),
                    cx.theme().muted_foreground,
                )
                .align(TextAlign::Right),
            );
        }
        let mut baseline = PathBuilder::stroke(px(1.5));
        baseline.move_to(bounds.origin + point(px(g.left), g.y(0.)));
        baseline.line_to(bounds.origin + point(px(g.left + g.width), g.y(0.)));
        if let Ok(path) = baseline.build() {
            window.paint_path(path, cx.theme().muted_foreground);
        }
        window.with_content_mask(
            Some(ContentMask {
                bounds: Bounds::new(
                    bounds.origin + point(px(g.left), px(g.top)),
                    size(px(g.width), px(g.height)),
                ),
            }),
            |window| {
                self.paint_series(&self.tokens, false, g, bounds, window);
                self.paint_series(&self.waiters, true, g, bounds, window);
                // Show status on zero only when the wait queue is known empty.
                for index in self.tokens.visible_range() {
                    let sample = &self.tokens.points[index];
                    let Some(end_time) = self.tokens.interval_end(index) else {
                        continue;
                    };
                    let Some(color) = self.zero_line_color(index) else {
                        continue;
                    };
                    let mut state = PathBuilder::stroke(px(2.5));
                    state.move_to(
                        bounds.origin + point(g.x(sample.x, self.tokens.x_domain), g.y(0.)),
                    );
                    state.line_to(
                        bounds.origin + point(g.x(end_time, self.tokens.x_domain), g.y(0.)),
                    );
                    if let Ok(path) = state.build() {
                        window.paint_path(path, rgb(color));
                    }
                }
                // Mark an exact-time handoff without implying continuous recursion.
                for (index, changed) in self.upper_breaks.iter().enumerate() {
                    if !changed {
                        continue;
                    }
                    let Some(sample) = self.tokens.points.get(index) else {
                        continue;
                    };
                    let x = g.x(sample.x, self.tokens.x_domain);
                    let mut y = g.top;
                    while px(y) < g.y(0.) {
                        let mut dash = PathBuilder::stroke(px(1.));
                        dash.move_to(bounds.origin + point(x, px(y)));
                        dash.line_to(bounds.origin + point(x, px((y + 4.).min(g.y(0.).as_f32()))));
                        if let Ok(path) = dash.build() {
                            window.paint_path(path, cx.theme().muted_foreground);
                        }
                        y += 8.;
                    }
                }
                if let Some(limit) = self.limit {
                    let mut x = g.left;
                    while x < g.left + g.width {
                        let mut dash = PathBuilder::stroke(px(1.5));
                        dash.move_to(bounds.origin + point(px(x), g.y(limit)));
                        dash.line_to(
                            bounds.origin + point(px((x + 7.).min(g.left + g.width)), g.y(limit)),
                        );
                        if let Ok(path) = dash.build() {
                            window.paint_path(path, Hsla::from(rgb(TOKENS)).opacity(0.2));
                        }
                        x += 12.;
                    }
                }
            },
        );
        let (start, end) = self.tokens.x_domain;
        let mut xs = vec![start, (start + end) / 2., end];
        if !self.tokens.timed {
            xs = if self.tokens.points.len() == 1 && end == 2. {
                vec![start]
            } else {
                vec![start, ((start + end) / 2.).round(), end]
            };
            xs.dedup();
        }
        for x in xs {
            labels.push(
                Text::new(
                    if self.tokens.timed {
                        format!("{x:.1}s")
                    } else {
                        format!("{x:.0}")
                    },
                    point(g.x(x, self.tokens.x_domain), px(g.top + g.height + 10.)),
                    cx.theme().muted_foreground,
                )
                .align(if x == start {
                    TextAlign::Left
                } else if x == end {
                    TextAlign::Right
                } else {
                    TextAlign::Center
                }),
            );
        }
        PlotLabel::new(labels).paint(&bounds, window, cx);
    }

    fn tooltip_state(
        &self,
        position: Point<Pixels>,
        bounds: Bounds<Pixels>,
        _: &App,
    ) -> Option<TooltipState> {
        let g = self.geometry(bounds.size);
        if g.width <= 0.
            || g.height <= 0.
            || position.x < px(g.left)
            || position.x > px(g.left + g.width)
            || position.y < px(g.top)
            || position.y > px(g.top + g.height)
        {
            return None;
        }
        let x = self.tokens.x_domain.0
            + ((position.x.as_f32() - g.left) / g.width) as f64
                * (self.tokens.x_domain.1 - self.tokens.x_domain.0);
        // Hover follows the held source reading, not a nearer future sample.
        let index = Self::held_index(&self.tokens, x)
            .or_else(|| Self::held_index(&self.waiters, x))
            .or_else(|| {
                self.tokens.points.iter().enumerate().find_map(|(i, p)| {
                    let known = p.value.is_some() || self.waiters.points[i].value.is_some();
                    (known && (g.x(p.x, self.tokens.x_domain) - position.x).abs() <= px(3.))
                        .then_some(i)
                })
            })?;
        let p = &self.tokens.points[index];
        if position.x > g.x(self.tokens.view_end()?, self.tokens.x_domain) + px(0.01) {
            return None;
        }
        let dot = point(
            g.x(p.x, self.tokens.x_domain).max(px(g.left)),
            g.y(p.value.unwrap_or(0.)),
        );
        Some(TooltipState::new(index, dot, vec![]))
    }

    fn tooltip(
        &self,
        state: &TooltipState,
        cursor: Point<Pixels>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        let tokens = self.tokens.points.get(state.index)?;
        let waiters = self.waiters.points.get(state.index)?;
        let count = |v: Option<f64>| v.map(|v| format!("{v:.0}")).unwrap_or_else(|| "—".into());
        let g = self.geometry(bounds.size);
        let details = self.details.get(state.index).cloned();
        let tooltip_selector = if self.id == "mutex-step-area" {
            "mutex-area-tooltip"
        } else {
            "semaphore-area-tooltip"
        };
        let mut dots = Vec::new();
        for (value, sign) in [(tokens.value, 1.), (waiters.value, -1.)] {
            if let Some(value) = value {
                dots.push(
                    Dot::new(point(
                        g.x(tokens.x, self.tokens.x_domain).max(px(g.left)),
                        g.y(sign * value),
                    ))
                    .size(px(7.))
                    .fill(rgb(self.sample_color(state.index, sign < 0., value)).into())
                    .stroke(cx.theme().background),
                );
            }
        }
        Some(
            Tooltip::new(cursor, bounds.size)
                .w(px(if details.is_some() { 310. } else { 245. }))
                .dots(dots)
                .cross_line(
                    CrossLine::new(point(
                        g.x(tokens.x, self.tokens.x_domain).max(px(g.left)),
                        g.y(0.),
                    ))
                    .span(g.top, g.height),
                )
                .child(
                    div()
                        .debug_selector(move || tooltip_selector.into())
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(self.tokens.source_label(tokens.x))
                        .child(format!("{}: {}", self.upper_label, count(tokens.value)))
                        .child(format!("Waiting threads: {}", count(waiters.value)))
                        .children(details.map(|details| div().text_xs().child(details))),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::StepAreaChart as SemaphoreChart;
    use super::{TOKENS, WAITERS, reading_color};
    use crate::{kernel_model::SemaphoreKey, model};
    use gpui_kit::component::plot::Plot;
    use gpui_kit::{Bounds, TestAppContext, point, px, size};

    #[test]
    fn default_colors_do_not_assume_empty_queues_are_full() {
        assert_eq!(reading_color(true, 0.), crate::MUTED);
        assert_eq!(reading_color(true, 1.), WAITERS);
        assert_eq!(reading_color(true, 6.), WAITERS);
        assert_eq!(reading_color(false, 0.), TOKENS);
        assert_eq!(reading_color(false, 3.), TOKENS);
    }

    fn chart(current: usize, limit: Option<u64>) -> SemaphoreChart {
        let frames = model::parse(include_str!("../test-data/semaphores.json")).unwrap();
        let key = SemaphoreKey {
            address: 8192,
            name: "buffer_tokens".into(),
        };
        SemaphoreChart::new(
            model::chart_series(&frames, current, |f| {
                key.in_frame(f)?.count.map(|v| v as f64)
            }),
            model::chart_series(&frames, current, |f| {
                Some(key.in_frame(f)?.object.waiters.as_ref()?.len() as f64)
            }),
            vec![limit; current + 1],
        )
    }

    #[test]
    fn semaphore_outline_tracks_source_count_and_limit() {
        let mut c = chart(12, Some(4));
        assert_eq!(c.sample_color(0, false, 4.), crate::GREEN);
        assert_eq!(c.sample_color(1, false, 3.), 0xddb66b);
        assert_eq!(c.sample_color(2, false, 1.), 0xddb66b);
        assert_eq!(c.sample_color(3, false, 0.), WAITERS); // Red even with waiters.
        assert_eq!(c.sample_color(10, false, 2.), 0xddb66b); // Queue unknown, count known.
        assert_eq!(c.zero_line_color(0), None); // No extra semaphore status strip.
        c.limits[1] = Some(3);
        assert_eq!(c.count_color(1, 3.), crate::GREEN);
        c.limits[1] = None;
        assert_eq!(c.count_color(1, 3.), crate::MUTED);
        c.limits[1] = Some(0);
        assert_eq!(c.count_color(1, 3.), crate::MUTED);
        c.limits[1] = Some(0xffff_ffff);
        assert_eq!(c.count_color(1, 3.), 0xddb66b);
        c.limits[1] = Some(4);
        assert_eq!(c.count_color(1, 5.), crate::MUTED);
        assert_eq!(c.sample_color(3, true, 1.), WAITERS); // Negative area stays red.
    }

    #[test]
    fn independent_counts_signs_limits_and_past_only_scale() {
        let c = chart(8, Some(4));
        assert_eq!(c.maximum, 4.);
        assert_eq!(c.limit, Some(4.));
        assert_eq!(c.tokens.points.len(), 9);
        assert_eq!(c.tokens.points[2].x, 2.5);
        assert_eq!(c.tokens.x_domain, (0., 20.));
        let g = c.geometry(size(px(600.), px(300.)));
        assert!(g.y(3.) < g.y(0.));
        assert!(g.y(-3.) > g.y(0.));
        assert_eq!(c.waiters.points[4].value, Some(3.));
        let c = chart(12, None);
        assert_eq!(c.tokens.points[10].value, Some(2.));
        assert_eq!(c.waiters.points[10].value, None);
        assert_eq!(c.tokens.points[11].value, None);
        assert_eq!(c.waiters.points[11].value, Some(0.));
        assert_eq!(c.tokens.points[12].value, None);
        assert_eq!(c.waiters.points[12].value, None);
        for limit in [None, Some(0), Some(0xffff_ffff)] {
            let c = chart(0, limit);
            assert_eq!(c.limit, None);
            assert_eq!(c.maximum, 4.);
        }
        let frames = model::parse(r#"[{"threads":[]},{"threads":[]}]"#).unwrap();
        let make = |current| model::chart_series(&frames, current, |_| Some(0.));
        let mut waiting = make(1);
        waiting.points[1].value = Some(100.);
        assert_eq!(
            SemaphoreChart::new(make(0), make(0), vec![None]).maximum,
            1.
        );
        assert_eq!(
            SemaphoreChart::new(make(1), waiting, vec![None; 2]).maximum,
            100.
        );
    }

    #[test]
    fn mutex_depth_and_waiters_are_independent_with_owner_handoffs() {
        use crate::kernel_model::MutexKey;
        let frames = model::parse(include_str!("../test-data/mutexes.json")).unwrap();
        let key = MutexKey {
            address: 12288,
            name: "resource_lock".into(),
        };
        let current = 10;
        let depth = model::chart_series(&frames, current, |f| {
            key.in_frame(f)?.lock_count.map(|v| v as f64)
        });
        let waiting = model::chart_series(&frames, current, |f| {
            Some(key.in_frame(f)?.object.waiters.as_ref()?.len() as f64)
        });
        let owners = frames[..=current]
            .iter()
            .map(|f| key.in_frame(f).and_then(|m| m.owner_address))
            .collect();
        let details = frames[..=current]
            .iter()
            .map(|f| {
                key.in_frame(f)
                    .map(|m| m.owner_label())
                    .unwrap_or_else(|| "—".into())
            })
            .collect();
        let c = SemaphoreChart::mutex(depth, waiting, owners, details);
        assert_eq!(c.id, "mutex-step-area");
        assert_eq!(c.upper_label, "Lock depth");
        assert_eq!(c.limit, None);
        assert_eq!(c.maximum, 6.); // Waiter count, not depth, sets the common scale.
        assert_eq!(c.sample_color(1, true, 0.), WAITERS);
        assert_eq!(c.sample_color(4, true, 0.), crate::MUTED); // Unknown owner.
        assert_eq!(c.sample_color(5, true, 0.), crate::MUTED); // Missing object.
        assert_eq!(c.sample_color(6, true, 0.), crate::GREEN);
        assert_eq!(c.sample_color(7, true, 6.), WAITERS);
        assert_eq!(c.zero_line_color(0), Some(crate::GREEN));
        assert_eq!(c.zero_line_color(1), Some(WAITERS));
        assert_eq!(c.zero_line_color(2), None); // Waiters > 0.
        assert_eq!(c.zero_line_color(3), None); // Queue unavailable.
        assert_eq!(c.zero_line_color(4), None); // Unknown depth/owner.
        assert_eq!(c.zero_line_color(5), None); // Object absent.
        assert_eq!(c.zero_line_color(6), Some(crate::GREEN));
        assert_eq!(c.zero_line_color(7), None); // Depth 3, six waiters.
        assert_eq!(c.tokens.points[7].value, Some(3.));
        assert_eq!(c.waiters.points[7].value, Some(6.));
        assert_eq!(c.waiters.points[3].value, None); // Unknown queue, known depth.
        assert_eq!(c.tokens.points[3].value, Some(1.));
        assert_eq!(c.tokens.points[4].value, None); // Unknown depth, known empty queue.
        assert_eq!(c.waiters.points[4].value, Some(0.));
        assert_eq!(c.tokens.points[5].value, None); // Identity replacement.
        assert_eq!(c.waiters.points[5].value, None);
        assert!(c.upper_connects(8)); // Same owner, recursive depth 3 -> 2.
        assert!(!c.upper_connects(9)); // storage_worker -> sensor_reader.
        assert!(!c.upper_connects(10)); // sensor_reader -> telemetry_writer.
        assert!(c.details[9].contains("sensor_reader"));
        assert_eq!(c.tokens.points.last().unwrap().x, 16.);
        assert_eq!(c.tokens.x_domain, (0., 20.));
    }

    #[test]
    fn hold_left_sample_without_interpolation_or_crossing_gaps() {
        let c = chart(12, Some(4));
        assert_eq!(SemaphoreChart::held_index(&c.tokens, 2.4), Some(1));
        assert_eq!(SemaphoreChart::held_index(&c.tokens, 2.5), Some(2));
        assert_eq!(SemaphoreChart::held_index(&c.tokens, 13.5), Some(10));
        assert_eq!(SemaphoreChart::held_index(&c.waiters, 13.5), None);
        assert_eq!(SemaphoreChart::held_index(&c.waiters, 12.5), Some(9));
        assert_eq!(SemaphoreChart::held_index(&c.waiters, 13.), None);
        assert_eq!(SemaphoreChart::held_index(&c.tokens, 14.), None);
        assert_eq!(SemaphoreChart::held_index(&c.tokens, 16.), None);
        assert_eq!(SemaphoreChart::held_index(&c.tokens, f64::NAN), None);
    }

    #[gpui_kit::test]
    fn hover_uses_held_source_time_and_rejects_gaps_and_future(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let c = chart(12, Some(4));
            let bounds = Bounds::new(point(px(0.), px(0.)), size(px(600.), px(300.)));
            let g = c.geometry(bounds.size);
            let at = |time| point(g.x(time, c.tokens.x_domain), g.y(0.));
            let hit = c.tooltip_state(at(2.4), bounds, cx).unwrap();
            assert_eq!(hit.index, 1);
            assert_eq!(c.tokens.points[hit.index].value, Some(3.));
            assert_eq!(c.tokens.points[hit.index].x, 1.);
            // Waiter count is known at 14s, independently of missing tokens.
            assert_eq!(c.tooltip_state(at(14.5), bounds, cx).unwrap().index, 11);
            assert!(c.tooltip_state(at(15.), bounds, cx).is_none());
            assert!(c.tooltip_state(at(18.), bounds, cx).is_none());
        });
    }
}

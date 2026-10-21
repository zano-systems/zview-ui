use crate::model::{ChartPoint, ChartSeries};
use gpui_kit::component::{
    ActiveTheme, StyledExt,
    plot::{
        Plot, PlotElement, PlotLabel, TooltipState, axis_gutter,
        label::{TEXT_SIZE, Text, measure_text_width},
        tooltip::{CrossLine, Dot, Tooltip},
    },
};
use gpui_kit::*;

/// Kit owns layout, hover tracking, and the styled tooltip. A numeric x-axis
/// avoids LineChart's evenly spaced categories. All readings use sample-and-hold
/// steps: horizontal until the next capture, then vertical at its timestamp.
#[derive(IntoElement)]
pub(super) struct TimeSeriesChart {
    id: &'static str,
    series: ChartSeries,
    color: Hsla,
    count_max: Option<f64>,
    show_dots: bool,
    shade_unknown_counts: bool,
}

#[derive(Clone, Copy)]
struct Geometry {
    left: f32,
    top: f32,
    width: f32,
    height: f32,
    y_max: f64,
}

impl Geometry {
    fn new(size: Size<Pixels>) -> Self {
        Self {
            left: 6.,
            top: 6.,
            width: (size.width.as_f32() - 12.).max(0.),
            height: (size.height.as_f32() - axis_gutter(px(TEXT_SIZE)) - 12.).max(0.),
            y_max: 100.,
        }
    }

    fn x(self, x: f64, domain: (f64, f64)) -> Pixels {
        px(self.left + ((x - domain.0) / (domain.1 - domain.0)) as f32 * self.width)
    }

    fn point(self, sample: &ChartPoint, domain: (f64, f64)) -> Option<Point<Pixels>> {
        Some(point(
            self.x(sample.x, domain),
            px(self.top
                + (1. - (sample.value?.clamp(0., self.y_max) / self.y_max) as f32) * self.height),
        ))
    }
}

impl TimeSeriesChart {
    pub fn new(id: &'static str, series: ChartSeries, color: Hsla) -> Self {
        Self {
            id,
            series,
            color,
            count_max: None,
            show_dots: true,
            shade_unknown_counts: false,
        }
    }

    /// A count axis, not a percentage. The caller supplies the finite limit or
    /// the largest observed count through the selected frame for unlimited data.
    pub fn count(id: &'static str, series: ChartSeries, color: Hsla, maximum: f64) -> Self {
        Self {
            id,
            series,
            color,
            count_max: Some(maximum.max(1.)),
            show_dots: true,
            shade_unknown_counts: false,
        }
    }

    pub fn without_dots(mut self) -> Self {
        self.show_dots = false;
        self
    }

    /// Shade unavailable intervals without making up a numeric count or
    /// covering future/unobserved playback space.
    pub fn with_shaded_unknown_counts(mut self) -> Self {
        self.shade_unknown_counts = true;
        self
    }

    fn unknown_ranges(&self) -> Vec<(f64, f64)> {
        let mut ranges = Vec::<(f64, f64)>::new();
        for (index, sample) in self.series.points.iter().enumerate() {
            if sample.value.is_some() {
                continue;
            }
            let Some(end) = self.series.interval_end(index) else {
                continue;
            };
            let start = sample.x.max(self.series.x_domain.0);
            let end = end.min(self.series.x_domain.1);
            if end <= start {
                continue;
            }
            if let Some(last) = ranges.last_mut()
                && last.1 == start
            {
                last.1 = end;
            } else {
                ranges.push((start, end));
            }
        }
        ranges
    }

    fn held_index(&self, x: f64) -> Option<usize> {
        self.series.held_index(x)
    }

    fn hover_index(&self, x: f64) -> Option<usize> {
        if !self.shade_unknown_counts {
            return self.held_index(x);
        }
        if !x.is_finite() || x > self.series.view_end()? {
            return None;
        }
        self.series
            .points
            .partition_point(|p| p.x <= x)
            .checked_sub(1)
    }

    fn geometry(&self, size: Size<Pixels>) -> Geometry {
        Geometry {
            y_max: self.count_max.unwrap_or(100.),
            ..Geometry::new(size)
        }
    }

    fn unknown_bands(&self, geometry: Geometry) -> Vec<Bounds<Pixels>> {
        self.unknown_ranges()
            .into_iter()
            .map(|(start, end)| {
                let left = geometry.x(start, self.series.x_domain);
                let right = geometry.x(end, self.series.x_domain);
                Bounds::new(
                    point(left, px(geometry.top)),
                    size(right - left, px(geometry.height)),
                )
            })
            .collect()
    }

    fn hover_position(&self, index: usize, geometry: Geometry) -> Option<Point<Pixels>> {
        let sample = self.series.points.get(index)?;
        let mut position = geometry.point(sample, self.series.x_domain).or_else(|| {
            self.shade_unknown_counts
                .then(|| point(geometry.x(sample.x, self.series.x_domain), px(geometry.top)))
        })?;
        position.x = position.x.max(px(geometry.left));
        Some(position)
    }

    fn value_label(&self, value: f64) -> String {
        if self.count_max.is_some() {
            format!("{value:.0}")
        } else {
            format!("{value:.2}%")
        }
    }
}

impl RenderOnce for TimeSeriesChart {
    fn render(mut self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = crate::chart_zoom::state(self.id, window, cx);
        self.series.x_domain = state.read(cx).domain(&self.series);
        crate::chart_zoom::wrap(self.id, state, PlotElement::new(self), window, cx)
    }
}

impl Plot for TimeSeriesChart {
    fn paint(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        let geometry = self.geometry(bounds.size);
        if geometry.width <= 0. || geometry.height <= 0. {
            return;
        }
        if self.shade_unknown_counts {
            for mut band in self.unknown_bands(geometry) {
                band.origin += bounds.origin;
                window.paint_quad(fill(band, rgba(0xd1d5db26)));
            }
        }
        for percent in [0., 50., 100.] {
            let y = geometry.top + (1. - percent / 100.) * geometry.height;
            let mut grid = PathBuilder::stroke(px(1.));
            grid.move_to(bounds.origin + point(px(geometry.left), px(y)));
            grid.line_to(bounds.origin + point(px(geometry.left + geometry.width), px(y)));
            if let Ok(path) = grid.build() {
                window.paint_path(path, cx.theme().border);
            }
        }
        window.with_content_mask(
            Some(ContentMask {
                bounds: Bounds::new(
                    bounds.origin + point(px(geometry.left), px(geometry.top)),
                    size(px(geometry.width), px(geometry.height)),
                ),
            }),
            |window| {
                for index in self.series.visible_range() {
                    let sample = &self.series.points[index];
                    let Some(end_time) = self.series.interval_end(index) else {
                        continue;
                    };
                    let Some(a) = geometry.point(sample, self.series.x_domain) else {
                        continue;
                    };
                    let end = geometry.x(end_time, self.series.x_domain);
                    let mut path = PathBuilder::stroke(px(2.));
                    path.move_to(bounds.origin + a);
                    path.line_to(bounds.origin + point(end, a.y));
                    if let Some(b) = self
                        .series
                        .points
                        .get(index + 1)
                        .and_then(|p| geometry.point(p, self.series.x_domain))
                    {
                        path.line_to(bounds.origin + b);
                    }
                    if let Ok(path) = path.build() {
                        window.paint_path(path, self.color);
                    }
                }
                // Retain visible single snapshots/isolated readings without
                // inventing a duration for them.
                if self.show_dots {
                    for segment in self.series.segments() {
                        if let [sample] = segment {
                            let position = bounds.origin
                                + geometry.point(sample, self.series.x_domain).unwrap();
                            window.paint_quad(
                                fill(
                                    Bounds::new(
                                        position - point(px(3.), px(3.)),
                                        size(px(6.), px(6.)),
                                    ),
                                    self.color,
                                )
                                .corner_radii(px(3.)),
                            );
                        }
                    }
                }
            },
        );
        let (start, end) = self.series.x_domain;
        let ticks = if !self.series.timed && end == 2. && self.series.points.len() == 1 {
            vec![start]
        } else if self.series.timed {
            vec![start, (start + end) / 2., end]
        } else {
            let mut ticks = vec![start, ((start + end) / 2.).round(), end];
            ticks.dedup();
            ticks
        };
        let labels = ticks
            .into_iter()
            .map(|x| {
                let label = if self.series.timed {
                    format!("{x:.1}s")
                } else {
                    format!("{x:.0}")
                };
                let align = if x == start {
                    TextAlign::Left
                } else if x == end {
                    TextAlign::Right
                } else {
                    TextAlign::Center
                };
                Text::new(
                    label,
                    point(
                        geometry.x(x, self.series.x_domain),
                        px(geometry.top + geometry.height + 6.),
                    ),
                    cx.theme().muted_foreground,
                )
                .align(align)
            })
            .collect();
        PlotLabel::new(labels).paint(&bounds, window, cx);
    }

    fn id(&self) -> Option<ElementId> {
        Some(self.id.into())
    }

    fn tooltip_state(
        &self,
        position: Point<Pixels>,
        bounds: Bounds<Pixels>,
        _: &App,
    ) -> Option<TooltipState> {
        let geometry = self.geometry(bounds.size);
        if geometry.width <= 0.
            || geometry.height <= 0.
            || position.y < px(geometry.top)
            || position.y > px(geometry.top + geometry.height)
            || position.x < px(geometry.left)
            || position.x > px(geometry.left + geometry.width)
            || position.x > geometry.x(self.series.view_end()?, self.series.x_domain) + px(0.01)
        {
            return None;
        }
        let (start, end) = self.series.x_domain;
        let x =
            start + ((position.x.as_f32() - geometry.left) / geometry.width) as f64 * (end - start);
        let index = self.hover_index(x).or_else(|| {
            // A small pixel target keeps isolated measurements reachable.
            // It still returns an exact source sample, never a synthetic value.
            self.series
                .points
                .iter()
                .enumerate()
                .find_map(|(index, sample)| {
                    let dot = geometry.point(sample, self.series.x_domain)?;
                    ((dot.x - position.x).abs() <= px(3.)).then_some(index)
                })
        })?;
        let position = self.hover_position(index, geometry)?;
        let dots = if self.show_dots && self.series.points[index].value.is_some() {
            vec![position]
        } else {
            vec![]
        };
        Some(TooltipState::new(index, position, dots))
    }

    fn tooltip(
        &self,
        state: &TooltipState,
        cursor: Point<Pixels>,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        let sample = self.series.points.get(state.index)?;
        let value_label = match sample.value {
            Some(value) => self.value_label(value),
            None if self.shade_unknown_counts => "Unknown count".into(),
            None => return None,
        };
        let time = self.series.source_label(sample.x);
        let geometry = self.geometry(bounds.size);
        let dot = self.hover_position(state.index, geometry)?;
        let width = (measure_text_width(&time.clone().into(), px(12.), window) + 48.)
            .max(measure_text_width(&value_label.clone().into(), px(12.), window) + 16.)
            .max(90.);
        Some(
            Tooltip::new(cursor, bounds.size)
                .min_w(px(90.))
                .w(px(width))
                .gap(px(8.))
                .cross_line(CrossLine::new(dot).span(geometry.top, geometry.height))
                .dots(if self.show_dots && sample.value.is_some() {
                    vec![
                        Dot::new(dot)
                            .size(px(8.))
                            .halo(px(20.))
                            .stroke(cx.theme().background)
                            .fill(self.color),
                    ]
                } else {
                    vec![]
                })
                .child(
                    div()
                        .debug_selector(|| "percentage-tooltip-content".into())
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(div().font_semibold().child(value_label))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap_3()
                                .child(div().size_2().rounded_sm().bg(self.color))
                                .child(time),
                        ),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{Geometry, TimeSeriesChart};
    use crate::model::{ChartPoint, ChartSeries};
    use gpui_kit::component::plot::Plot;
    use gpui_kit::{Bounds, TestAppContext, point, px, rgb, size};

    fn series() -> ChartSeries {
        ChartSeries {
            points: vec![
                ChartPoint {
                    x: 0.,
                    value: Some(0.),
                },
                ChartPoint {
                    x: 1.75,
                    value: Some(40.25),
                },
                ChartPoint { x: 5., value: None },
                ChartPoint {
                    x: 8.,
                    value: Some(100.),
                },
                ChartPoint {
                    x: 9.,
                    value: Some(0.),
                },
            ],
            x_domain: (0., 15.),
            timed: true,
            caption: String::new(),
            playhead: None,
        }
    }

    #[test]
    fn unknown_count_bands_merge_gaps_clip_to_zoom_and_leave_future_blank() {
        let mut data = series();
        data.points = [
            (0., Some(0.)),
            (9., None),
            (10., None),
            (11., Some(0.)),
            (12., None),
            (15., Some(0.)),
            (16., None),
            (18., Some(0.)),
            (20., Some(2.)),
        ]
        .into_iter()
        .map(|(x, value)| ChartPoint { x, value })
        .collect();
        data.x_domain = (0., 24.);
        let mut chart = TimeSeriesChart::count("backlog", data, rgb(0xffffff).into(), 2.)
            .without_dots()
            .with_shaded_unknown_counts();
        assert!(!chart.show_dots);
        let geometry = chart.geometry(size(px(320.), px(150.)));
        let bands = chart.unknown_bands(geometry);
        assert_eq!(bands.len(), 3);
        for band in bands {
            assert_eq!(band.top(), px(geometry.top));
            assert_eq!(band.size.height, px(geometry.height));
            assert!(
                band.left() >= px(geometry.left)
                    && band.right() <= px(geometry.left + geometry.width)
            );
        }
        assert_eq!(
            chart.unknown_ranges(),
            vec![(9., 11.), (12., 15.), (16., 18.)]
        );
        assert_eq!(chart.hover_index(10.5), Some(2));
        assert_eq!(chart.series.points[2].value, None);
        assert_eq!(chart.hover_index(21.), None);
        chart.series.x_domain = (9.5, 17.);
        assert_eq!(
            chart.unknown_ranges(),
            vec![(9.5, 11.), (12., 15.), (16., 17.)]
        );
        chart.series.x_domain = (0., 24.);
        chart.series.points.truncate(5);
        assert_eq!(
            chart.unknown_ranges(),
            vec![(9., 11.)],
            "a terminal unknown has no invented duration"
        );
        chart.series.playhead = Some(13.25);
        assert_eq!(chart.unknown_ranges(), vec![(9., 11.), (12., 13.25)]);
        assert_eq!(chart.hover_index(13.26), None);
    }

    #[gpui_kit::test]
    fn backlog_hover_has_no_dots_and_unknown_count_is_not_a_numeric_zero(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let chart = TimeSeriesChart::count("backlog", series(), rgb(0xffffff).into(), 100.)
                .without_dots()
                .with_shaded_unknown_counts();
            let bounds = Bounds::new(point(px(0.), px(0.)), size(px(320.), px(150.)));
            let g = chart.geometry(bounds.size);
            let at = |x| point(g.x(x, chart.series.x_domain), px(g.top + 2.));
            let known = chart.tooltip_state(at(0.), bounds, cx).unwrap();
            assert!(known.dots.is_empty());
            assert_eq!(chart.series.points[known.index].value, Some(0.));
            let unknown = chart.tooltip_state(at(6.), bounds, cx).unwrap();
            assert!(unknown.dots.is_empty());
            assert_eq!(chart.series.points[unknown.index].value, None);
            assert_eq!(chart.unknown_bands(g).len(), 1);
            assert!(chart.tooltip_state(at(10.), bounds, cx).is_none());
        });
    }

    #[test]
    fn numeric_geometry_does_not_change_measurements() {
        let geometry = Geometry::new(size(px(320.), px(100.)));
        let series = series();
        let first = geometry.point(&series.points[0], series.x_domain).unwrap();
        let second = geometry.point(&series.points[1], series.x_domain).unwrap();
        assert!(((second.x - first.x).as_f32() / geometry.width - 1.75 / 15.).abs() < 0.00001);
        assert_eq!(first.y, px(geometry.top + geometry.height));
        assert_eq!(
            geometry
                .point(&series.points[3], series.x_domain)
                .unwrap()
                .y,
            px(geometry.top)
        );
        let outside = ChartPoint {
            x: 9.,
            value: Some(101.25),
        };
        assert_eq!(
            geometry.point(&outside, series.x_domain).unwrap().y,
            px(geometry.top)
        );
        assert_eq!(outside.value, Some(101.25)); // Visual clamp does not rewrite source values.
    }

    #[test]
    fn count_axis_and_tooltips_use_counts_not_percentages() {
        let chart = TimeSeriesChart::count("count", series(), rgb(0xffffff).into(), 3.);
        let geometry = chart.geometry(size(px(320.), px(100.)));
        let sample = ChartPoint {
            x: 1.,
            value: Some(2.),
        };
        let dot = geometry.point(&sample, chart.series.x_domain).unwrap();
        assert!(((dot.y.as_f32() - geometry.top) / geometry.height - 1. / 3.).abs() < 0.00001);
        assert_eq!(chart.value_label(2.), "2");
        assert_eq!(chart.value_label(4294967295.), "4294967295");
        let percentage = TimeSeriesChart::new("percent", series(), rgb(0xffffff).into());
        assert_eq!(percentage.value_label(2.), "2.00%");
    }

    #[test]
    fn step_hover_holds_source_until_missing_timestamp_and_never_reads_ahead() {
        let chart = TimeSeriesChart::count("steps", series(), rgb(0xffffff).into(), 100.);
        assert_eq!(chart.held_index(1.74), Some(0));
        assert_eq!(chart.held_index(1.75), Some(1));
        assert_eq!(chart.held_index(4.99), Some(1));
        assert_eq!(chart.held_index(5.), None);
        assert_eq!(chart.held_index(7.99), None);
        assert_eq!(chart.held_index(8.5), Some(3));
        assert_eq!(chart.held_index(9.), Some(4));
        assert_eq!(chart.held_index(9.1), None);
    }

    #[gpui_kit::test]
    fn tooltips_snap_to_source_points_and_skip_gaps_and_future(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let chart = TimeSeriesChart::new("test", series(), rgb(0xffffff).into());
            let bounds = Bounds::new(point(px(0.), px(0.)), size(px(320.), px(100.)));
            let geometry = Geometry::new(bounds.size);
            let at = |x| point(geometry.x(x, chart.series.x_domain), px(20.));
            let hit = chart.tooltip_state(at(1.), bounds, cx).unwrap();
            assert_eq!(hit.index, 0);
            assert_eq!(chart.series.points[hit.index].value, Some(0.));
            assert_eq!(
                chart.series.source_label(chart.series.points[hit.index].x),
                "0s"
            );
            assert_eq!(
                hit.dots[0],
                geometry
                    .point(&chart.series.points[0], chart.series.x_domain)
                    .unwrap()
            );
            assert_eq!(chart.tooltip_state(at(3.), bounds, cx).unwrap().index, 1);
            for x in [5., 6., 10., 15.] {
                assert!(
                    chart.tooltip_state(at(x), bounds, cx).is_none(),
                    "no reading at {x}"
                );
            }
            assert_eq!(chart.tooltip_state(at(8.5), bounds, cx).unwrap().index, 3);
        });
    }
}

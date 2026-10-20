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
        }
    }

    fn held_index(&self, x: f64) -> Option<usize> {
        self.series.held_index(x)
    }

    fn geometry(&self, size: Size<Pixels>) -> Geometry {
        Geometry {
            y_max: self.count_max.unwrap_or(100.),
            ..Geometry::new(size)
        }
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
                for segment in self.series.segments() {
                    if let [sample] = segment {
                        let position =
                            bounds.origin + geometry.point(sample, self.series.x_domain).unwrap();
                        window.paint_quad(
                            fill(
                                Bounds::new(position - point(px(3.), px(3.)), size(px(6.), px(6.))),
                                self.color,
                            )
                            .corner_radii(px(3.)),
                        );
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
        let index = self.held_index(x).or_else(|| {
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
        let mut dot = geometry.point(&self.series.points[index], self.series.x_domain)?;
        dot.x = dot.x.max(px(geometry.left));
        Some(TooltipState::new(index, dot, vec![dot]))
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
        let value = sample.value?;
        let time = self.series.source_label(sample.x);
        let mut dot = self
            .geometry(bounds.size)
            .point(sample, self.series.x_domain)?;
        dot.x = dot.x.max(px(self.geometry(bounds.size).left));
        let value_label = self.value_label(value);
        let width = (measure_text_width(&time.clone().into(), px(12.), window) + 48.)
            .max(measure_text_width(&value_label.clone().into(), px(12.), window) + 16.)
            .max(90.);
        Some(
            Tooltip::new(cursor, bounds.size)
                .min_w(px(90.))
                .w(px(width))
                .gap(px(8.))
                .cross_line(CrossLine::new(dot).span(6., self.geometry(bounds.size).height))
                .dots([Dot::new(dot)
                    .size(px(8.))
                    .halo(px(20.))
                    .stroke(cx.theme().background)
                    .fill(self.color)])
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

//! Sampled work-queue state, time-weighted busy estimates, and activity strip.
use crate::{
    BORDER, CARD, MUTED,
    kernel_model::{WorkQueue, WorkQueueKey},
    layout,
    model::{self, ChartSeries, Frame},
    zoom_strip::{self, Navigation, Viewport as _},
};
use gpui_kit::component::{
    StyledExt,
    plot::{Plot, PlotElement, TooltipState, tooltip::Tooltip},
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::{cell::Cell, collections::HashMap, rc::Rc};

pub(super) const ACTIVE: u32 = 0xddb66b;
pub(super) const BACKLOG: u32 = 0xf18b94;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Activity {
    Unknown,
    NotStarted,
    Idle,
    Ready,
    Processing,
    NotProcessing,
}
impl Activity {
    pub fn from_queue(queue: Option<&WorkQueue>) -> Self {
        let Some(queue) = queue else {
            return Self::Unknown;
        };
        let Some(flags) = queue.flags else {
            return Self::Unknown;
        };
        if flags & 2 != 0 {
            Self::Processing
        } else if flags & 1 == 0 {
            Self::NotStarted
        } else if queue.pending.as_ref().is_some_and(|p| !p.is_empty()) {
            Self::Ready
        } else if queue.pending.as_ref().is_some_and(|p| p.is_empty()) && !queue.pending_truncated {
            Self::Idle
        } else {
            Self::NotProcessing
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "Unknown",
            Self::NotStarted => "Not started",
            Self::Idle => "Idle",
            Self::Ready => "Ready",
            Self::Processing => "Processing",
            Self::NotProcessing => "Not processing",
        }
    }
    fn known_started(self) -> bool {
        !matches!(self, Self::Unknown | Self::NotStarted)
    }
}

/// Pressure is independent of worker activity. A positive lower bound proves a
/// backlog, but an incomplete empty list never proves there is no backlog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Pressure {
    Unknown,
    Clear,
    Active,
    Backlog,
}

impl Pressure {
    pub fn from_queue(queue: Option<&WorkQueue>) -> Self {
        let Some(queue) = queue else {
            return Self::Unknown;
        };
        let Some(pending) = &queue.pending else {
            return Self::Unknown;
        };
        if !pending.is_empty() {
            return Self::Backlog;
        }
        if queue.pending_truncated {
            return Self::Unknown;
        }
        match Activity::from_queue(Some(queue)) {
            Activity::Processing => Self::Active,
            Activity::Idle | Activity::NotStarted => Self::Clear,
            _ => Self::Unknown,
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Clear => "clear",
            Self::Active => "active",
            Self::Backlog => "backlog",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "Pressure unknown",
            Self::Clear => "No backlog",
            Self::Active => "Processing without backlog",
            Self::Backlog => "Work waiting",
        }
    }
    pub fn color(self) -> Option<u32> {
        match self {
            Self::Active => Some(ACTIVE),
            Self::Backlog => Some(BACKLOG),
            _ => None,
        }
    }
    pub fn palette(self) -> (u32, u32, u32) {
        match self {
            Self::Active => (0x3c3440, ACTIVE, 0x746345),
            Self::Backlog => (0x433040, BACKLOG, 0x80515c),
            _ => (0x302c42, MUTED, BORDER),
        }
    }
}

#[derive(Default, Debug)]
pub(super) struct Busy {
    pub window: f64,
    pub known: f64,
    pub processing: f64,
    pub cadence: Option<f64>,
}
impl Busy {
    pub fn percent(&self) -> Option<f64> {
        (self.known > 0.).then(|| (100. * self.processing / self.known).clamp(0., 100.))
    }
    pub fn description(&self) -> String {
        if self.window <= 0. {
            return "Timed intervals required · sampled estimate, not CPU usage".into();
        }
        let cadence = self
            .cadence
            .map(|t| format!(" · ~{:.0}ms sampling", t * 1000.))
            .unwrap_or_default();
        format!(
            "{:.2}/{:.2}s known started time · {:.0}% coverage{cadence}",
            self.known,
            self.window,
            100. * self.known / self.window
        )
    }
}
pub(super) fn observed_busy(frames: &[Frame], current: usize, key: &WorkQueueKey) -> Busy {
    let Some(end) = frames[current].elapsed_seconds else {
        return Busy::default();
    };
    let start = (end - 10.).max(frames[0].elapsed_seconds.unwrap_or(end));
    let mut busy = Busy {
        window: end - start,
        ..Default::default()
    };
    let mut intervals = Vec::new();
    for pair in frames[..=current].windows(2) {
        let (Some(a), Some(b)) = (pair[0].elapsed_seconds, pair[1].elapsed_seconds) else {
            continue;
        };
        let duration = (b.min(end) - a.max(start)).max(0.);
        if duration <= 0. {
            continue;
        }
        intervals.push(b - a);
        let state = Activity::from_queue(key.in_frame(&pair[0]));
        if state.known_started() {
            busy.known += duration;
        }
        if state == Activity::Processing {
            busy.processing += duration;
        }
    }
    intervals.sort_by(f64::total_cmp);
    busy.cadence = intervals.get(intervals.len() / 2).copied();
    busy
}

struct Reading {
    state: Activity,
    pressure: Pressure,
    pending: Option<usize>,
    truncated: bool,
}
pub(super) struct History {
    series: ChartSeries,
    readings: Vec<Reading>,
}
impl History {
    pub fn new(frames: &[Frame], current: usize, key: &WorkQueueKey) -> Self {
        let readings = frames[..=current]
            .iter()
            .map(|f| {
                let q = key.in_frame(f);
                Reading {
                    state: Activity::from_queue(q),
                    pressure: Pressure::from_queue(q),
                    pending: q.and_then(|q| q.pending.as_ref()).map(Vec::len),
                    truncated: q.is_some_and(|q| q.pending_truncated),
                }
            })
            .collect();
        // Use the shared timestamp/held-reading machinery, not a numeric state
        // scale. Every captured frame has a reading, including explicit Unknown.
        let series = model::chart_series(frames, current, |_| Some(1.));
        Self { series, readings }
    }
    fn description(&self, index: usize) -> String {
        let reading = &self.readings[index];
        let pending = reading
            .pending
            .map(|n| format!("{n}{}", if reading.truncated { "+" } else { "" }))
            .unwrap_or_else(|| "—".into());
        format!(
            "{} · {} · source {} · {pending} pending · sampled state, not exact transition time",
            reading.state.label(),
            reading.pressure.label(),
            self.series.source_label(self.series.points[index].x)
        )
    }
    fn hit(&self, x: f64) -> Option<usize> {
        self.series.held_index(x)
    }
    fn runs(&self) -> Vec<(f64, f64, Activity, Pressure)> {
        let mut runs = Vec::<(f64, f64, Activity, Pressure)>::new();
        for (i, pair) in self.series.points.windows(2).enumerate() {
            let state = self.readings[i].state;
            let pressure = self.readings[i].pressure;
            if let Some(last) = runs.last_mut()
                && last.2 == state
                && last.3 == pressure
            {
                last.1 = pair[1].x;
            } else {
                runs.push((pair[0].x, pair[1].x, state, pressure));
            }
        }
        runs
    }
}

#[derive(Clone, Copy, Default, PartialEq, Debug)]
struct TimeViewport {
    domain: (f64, f64),
    start: f64,
    span: f64,
    minimum: f64,
}
impl TimeViewport {
    fn sync(&mut self, domain: (f64, f64), timed: bool) {
        let fitted = self.span == self.domain.1 - self.domain.0;
        self.domain = domain;
        self.minimum = if timed {
            1e-6f64.max(domain.1.abs() * f64::EPSILON * 16.)
        } else {
            1.
        };
        self.span = if fitted {
            domain.1 - domain.0
        } else {
            self.span.max(self.minimum).min(domain.1 - domain.0)
        };
        self.start = self.start.clamp(domain.0, domain.1 - self.span);
    }
    fn end(self) -> f64 {
        self.start + self.span
    }
    fn coordinate(self, fraction: f64) -> f64 {
        self.start + self.span * fraction.clamp(0., 1.)
    }
    fn pixel(self, x: f64, width: Pixels) -> Pixels {
        width * ((x - self.start) / self.span) as f32
    }
}
impl zoom_strip::Viewport for TimeViewport {
    fn zoom(&mut self, factor: f64, anchor: f64) {
        if !factor.is_finite() || factor <= 0. || self.span <= 0. {
            return;
        }
        let span = (self.span / factor)
            .max(self.minimum)
            .min(self.domain.1 - self.domain.0);
        self.start = (self.start + (self.span - span) * anchor.clamp(0., 1.))
            .clamp(self.domain.0, self.domain.1 - span);
        self.span = span;
    }
    fn fit(&mut self) {
        self.start = self.domain.0;
        self.span = self.domain.1 - self.domain.0;
    }
    fn pan_fraction(&mut self, fraction: f64) {
        self.start =
            (self.start + fraction * self.span).clamp(self.domain.0, self.domain.1 - self.span);
    }
    fn edge(&mut self, end: bool) {
        self.start = if end {
            self.domain.1 - self.span
        } else {
            self.domain.0
        };
    }
    fn can_zoom_in(&self) -> bool {
        self.span > self.minimum
    }
    fn can_zoom_out(&self) -> bool {
        self.span < self.domain.1 - self.domain.0
    }
    fn overview_range(&self) -> (f64, f64) {
        let total = self.domain.1 - self.domain.0;
        if total <= 0. {
            (0., 1.)
        } else {
            (
                (self.start - self.domain.0) / total,
                (self.end() - self.domain.0) / total,
            )
        }
    }
    fn set_overview_start(&mut self, fraction: f64) {
        if fraction.is_finite() {
            self.start = (self.domain.0 + fraction.clamp(0., 1.) * (self.domain.1 - self.domain.0))
                .clamp(self.domain.0, self.domain.1 - self.span);
        }
    }
}

#[derive(Default)]
pub(super) struct States {
    queues: HashMap<(u64, String), Entity<StripState>>,
}
impl States {
    pub fn clear(&mut self) {
        self.queues.clear();
    }
}
#[derive(Default)]
struct StripState {
    navigation: Navigation<TimeViewport>,
    history: Option<Rc<History>>,
    selected: Option<usize>,
}
impl zoom_strip::State for StripState {
    type Viewport = TimeViewport;
    fn navigation(&self) -> &Navigation<TimeViewport> {
        &self.navigation
    }
    fn navigation_mut(&mut self) -> &mut Navigation<TimeViewport> {
        &mut self.navigation
    }
    fn select(&mut self, position: Point<Pixels>) {
        let x = self
            .navigation
            .viewport
            .coordinate(self.navigation.fraction(position));
        self.selected = self.history.as_ref().and_then(|h| h.hit(x));
    }
}
struct Strip {
    history: Rc<History>,
    viewport: TimeViewport,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    overview: bool,
}
impl IntoElement for Strip {
    type Element = PlotElement<Self>;
    fn into_element(self) -> Self::Element {
        PlotElement::new(self)
    }
}
impl Plot for Strip {
    fn id(&self) -> Option<ElementId> {
        Some(
            if self.overview {
                "workq-activity-overview-plot"
            } else {
                "workq-activity-plot"
            }
            .into(),
        )
    }
    fn prepaint(&mut self, bounds: Bounds<Pixels>, _: &mut Window, _: &mut App) -> Vec<AnyElement> {
        self.bounds.set(bounds);
        Vec::new()
    }
    fn paint(&mut self, bounds: Bounds<Pixels>, window: &mut Window, _: &mut App) {
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for (a, b, _, pressure) in self.history.runs() {
                let a = a.max(self.viewport.start);
                let b = b.min(self.viewport.end());
                if b <= a {
                    continue;
                }
                let left = self.viewport.pixel(a, bounds.size.width);
                let right = self.viewport.pixel(b, bounds.size.width);
                let rect = Bounds::new(
                    bounds.origin + point(left, px(0.)),
                    size(right - left, bounds.size.height),
                );
                if let Some(color) = pressure.color() {
                    window.paint_quad(fill(rect, rgb(color)));
                }
                if pressure == Pressure::Unknown {
                    let mut x = rect.left();
                    while x < rect.right() {
                        window.paint_quad(fill(
                            Bounds::new(
                                point(x, rect.top()),
                                size(px(1.).min(rect.right() - x), rect.size.height),
                            ),
                            rgb(0x625675),
                        ));
                        x += px(6.);
                    }
                }
            }
            // An isolated/terminal reading is a marker, never an invented interval.
            if let Some(last) = self.history.series.points.last()
                && last.x >= self.viewport.start
                && last.x <= self.viewport.end()
            {
                let x = bounds.left() + self.viewport.pixel(last.x, bounds.size.width);
                let color = self
                    .history
                    .readings
                    .last()
                    .unwrap()
                    .pressure
                    .color()
                    .unwrap_or(MUTED);
                window.paint_quad(fill(
                    Bounds::new(
                        point(x.min(bounds.right() - px(1.)), bounds.top()),
                        size(px(1.), bounds.size.height),
                    ),
                    rgb(color),
                ));
            }
        });
    }
    fn tooltip_state(
        &self,
        position: Point<Pixels>,
        bounds: Bounds<Pixels>,
        _: &App,
    ) -> Option<TooltipState> {
        if self.overview || bounds.size.width <= px(0.) {
            return None;
        }
        let x = self
            .viewport
            .coordinate((position.x / bounds.size.width) as f64);
        let index = self.history.hit(x).or_else(|| {
            self.history
                .series
                .points
                .iter()
                .enumerate()
                .find_map(|(i, p)| {
                    ((self.viewport.pixel(p.x, bounds.size.width) - position.x).abs() <= px(2.))
                        .then_some(i)
                })
        })?;
        Some(TooltipState::new(index, position, vec![]))
    }
    fn tooltip(
        &self,
        state: &TooltipState,
        cursor: Point<Pixels>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut App,
    ) -> Option<AnyElement> {
        self.history.readings.get(state.index)?;
        Some(
            Tooltip::new(cursor, bounds.size)
                .w(px(280.))
                .child(
                    div()
                        .debug_selector(|| "workq-activity-tooltip".into())
                        .child(self.history.description(state.index)),
                )
                .into_any_element(),
        )
    }
}

pub(super) fn view(
    states: &Entity<States>,
    key: &WorkQueueKey,
    history: History,
    window: &mut Window,
    cx: &mut App,
) -> impl IntoElement {
    let state = states.update(cx, |states, cx| {
        states
            .queues
            .entry((key.address, key.name.clone()))
            .or_insert_with(|| cx.new(|_| StripState::default()))
            .clone()
    });
    let history = Rc::new(history);
    state.update(cx, |s, _| {
        // A selected historical reading remains valid as playback advances,
        // but must disappear if the session is stepped before that reading.
        if s.selected.is_some_and(|i| i >= history.readings.len()) {
            s.selected = None;
        }
        s.navigation
            .viewport
            .sync(history.series.x_domain, history.series.timed);
        s.history = Some(history.clone());
    });
    let s = state.read(cx);
    let viewport = s.navigation.viewport;
    let bounds = s.navigation.bounds.clone();
    let selected = s.selected;
    let format_range = |x: f64| {
        if history.series.timed {
            format!("{x:.3}s")
        } else {
            format!("Frame {x:.0}")
        }
    };
    let range = format!(
        "{}–{}",
        format_range(viewport.start),
        format_range(viewport.end())
    );
    let controls = zoom_strip::controls(
        &state,
        zoom_strip::Controls {
            zoom_in: "workq-activity-zoom-in",
            zoom_out: "workq-activity-zoom-out",
            fit: "workq-activity-fit",
            name: "activity history",
            fit_label: "Fit activity history",
        },
        !history.series.points.is_empty(),
        window,
        cx,
    );
    div().debug_selector(||"workq-activity".into()).flex().flex_col().flex_shrink_0().min_w_0().gap_1()
        .p_3().rounded_lg().border_1().border_color(rgb(BORDER)).bg(rgb(CARD))
        .child(div().flex().items_center().justify_between().gap_2()
            .child(div().flex().items_center().gap_3()
                .child(div().debug_selector(|| "workq-activity-heading".into())
                    .text_sm().font_semibold().child("Activity history"))
                .when(viewport.can_zoom_out(), |el| {
                    let mut full = viewport;
                    full.fit();
                    el.child(zoom_strip::minimap(&state, "workq-activity-minimap", "Work queue activity",
                        Strip { history:history.clone(), viewport:full, bounds:Rc::default(), overview:true }, window, cx))
                }))
            .child(div().flex().items_center().gap_2()
                .child(div().debug_selector(move||format!("workq-activity-view-{:.3}-{:.3}",viewport.start,viewport.end()))
                    .child(layout::note("workq-activity-range",range))).child(controls)))
        .child(zoom_strip::surface(&state,"workq-activity-strip",Strip{history:history.clone(),viewport,bounds,overview:false},window))
        .child(div().min_w_0().debug_selector(move||selected.map_or_else(||"workq-activity-unselected".into(),|i|format!("workq-activity-selected-{i}")))
            .child(layout::note("workq-activity-inspection",selected.filter(|i|*i<history.readings.len()).map(|i|history.description(i)).unwrap_or_else(||
                "Red: work waiting · Yellow: processing without backlog · Unfilled: idle/inactive · Striped: pressure unknown · Drag strip or minimap to pan · Vertical scroll to zoom".into()))))
}

#[cfg(test)]
mod tests {
    use super::{ACTIVE, Activity, BACKLOG, History, Pressure, TimeViewport, observed_busy};
    use crate::{kernel_model::WorkQueueKey, model, zoom_strip::Viewport};

    fn key() -> WorkQueueKey {
        WorkQueueKey {
            address: 28672,
            name: "system_workq".into(),
        }
    }
    fn frames() -> Vec<model::Frame> {
        let base = model::parse(include_str!("../test-data/workqs.json"))
            .unwrap()
            .remove(0);
        [0., 1., 4., 6., 9., 12.]
            .into_iter()
            .zip([Some(3), Some(1), Some(3), None, Some(1), Some(3)])
            .map(|(t, flags)| {
                let mut f = base.clone();
                f.elapsed_seconds = Some(t);
                f.workqs[0].flags = flags;
                f.workqs[0].pending = Some(vec![]);
                f
            })
            .collect()
    }
    #[test]
    fn running_is_independent_of_backlog_and_uncertain_queues_are_not_idle() {
        let mut f = frames().remove(0);
        let q = &mut f.workqs[0];
        assert_eq!(Activity::from_queue(Some(q)), Activity::Processing);
        q.flags = Some(1);
        assert_eq!(Activity::from_queue(Some(q)), Activity::Idle);
        q.pending = Some(vec!["task".into()]);
        assert_eq!(Activity::from_queue(Some(q)), Activity::Ready);
        q.flags = Some(0);
        assert_eq!(Activity::from_queue(Some(q)), Activity::NotStarted);
        q.flags = Some(1);
        q.pending = None;
        assert_eq!(Activity::from_queue(Some(q)), Activity::NotProcessing);
        q.pending = Some(vec![]);
        q.pending_truncated = true;
        assert_eq!(Activity::from_queue(Some(q)), Activity::NotProcessing);
        q.flags = None;
        assert_eq!(Activity::from_queue(Some(q)), Activity::Unknown);
        assert_eq!(Activity::from_queue(None), Activity::Unknown);
        assert_eq!(Pressure::Active.color(), Some(ACTIVE));
        assert_eq!(Pressure::Backlog.color(), Some(BACKLOG));
        assert_eq!(Pressure::Clear.color(), None);
    }
    #[test]
    fn busy_uses_actual_interval_durations_and_clips_to_the_past_ten_seconds() {
        let frames = frames();
        let b = observed_busy(&frames, 5, &key());
        assert_eq!((b.window, b.known, b.processing), (10., 7., 2.));
        assert!((b.percent().unwrap() - 200. / 7.).abs() < 1e-10);
        assert_eq!(observed_busy(&frames, 3, &key()).percent(), Some(50.));
        assert!(observed_busy(&frames, 0, &key()).percent().is_none());
        let mut future = frames.clone();
        future[4].workqs.clear();
        future[5].workqs[0].flags = Some(0);
        assert_eq!(observed_busy(&future, 3, &key()).percent(), Some(50.));
        let mut stopped = frames.clone();
        for f in &mut stopped {
            f.workqs[0].flags = Some(0);
        }
        assert!(observed_busy(&stopped, 5, &key()).percent().is_none());
        for f in &mut stopped {
            f.elapsed_seconds = None;
        }
        assert!(observed_busy(&stopped, 5, &key()).percent().is_none());
    }
    #[test]
    fn activity_preserves_unknowns_identity_timestamps_and_terminal_samples() {
        let mut frames = frames();
        frames[1].workqs[0].pending = Some(vec!["task".into()]);
        frames[2].workqs[0].pending = Some(vec!["partial".into()]);
        frames[2].workqs[0].pending_truncated = true;
        let h = History::new(&frames, 5, &key());
        assert_eq!(
            h.runs()
                .into_iter()
                .map(|(a, b, state, _)| (a, b, state))
                .collect::<Vec<_>>(),
            vec![
                (0., 1., Activity::Processing),
                (1., 4., Activity::Ready),
                (4., 6., Activity::Processing),
                (6., 9., Activity::Unknown),
                (9., 12., Activity::Idle),
            ]
        );
        assert_eq!(h.hit(2.5), Some(1));
        assert_eq!(h.hit(7.), Some(3));
        assert_eq!(h.hit(12.), Some(5));
        assert_eq!(h.hit(12.001), None);
        assert!(h.description(2).contains("1+ pending"));
        assert!(h.description(2).contains("source 4s"));
        let earlier = History::new(&frames, 0, &key());
        assert!(earlier.runs().is_empty());
        assert_eq!(earlier.hit(0.1), None);
        frames[2].workqs[0].object.name = "replacement".into();
        assert_eq!(
            History::new(&frames, 2, &key()).readings[2].state,
            Activity::Unknown
        );
        let other = WorkQueueKey {
            address: 28673,
            name: key().name,
        };
        assert_eq!(
            History::new(&frames, 0, &other).readings[0].state,
            Activity::NotStarted
        );
    }
    #[test]
    fn time_viewport_keeps_fractional_source_times_and_clamps_zoom_and_pan() {
        let mut v = TimeViewport::default();
        v.sync((0., 12.), true);
        v.zoom(2., 0.5);
        assert_eq!((v.start, v.end()), (3., 9.));
        v.pan_fraction(1000.);
        assert_eq!((v.start, v.end()), (6., 12.));
        v.edge(false);
        assert_eq!(v.start, 0.);
        v.zoom(3., 0.);
        assert_eq!(v.span, 2.);
        v.pan_fraction(0.125);
        assert_eq!(v.start, 0.25);
        v.sync((0., 12.), true);
        assert_eq!(v.start, 0.25);
        v.fit();
        assert_eq!((v.start, v.end()), (0., 12.));
        for _ in 0..100 {
            v.zoom(2., 0.5);
        }
        assert_eq!(v.span, 1e-6);
        v.fit();
        let before = v;
        v.zoom(f64::NAN, 0.5);
        assert_eq!(v, before);
        v.sync((1., 6.), false);
        v.zoom(1000., 0.5);
        assert_eq!(v.span, 1.);
        v.set_overview_start(0.25);
        assert_eq!((v.start, v.end()), (2.25, 3.25));
        assert_eq!(v.overview_range(), (0.25, 0.45));
        v.set_overview_start(2.);
        assert_eq!((v.start, v.end()), (5., 6.));
        v.set_overview_start(-2.);
        assert_eq!((v.start, v.end()), (1., 2.));
        let before = v;
        v.set_overview_start(f64::NAN);
        assert_eq!(v, before);
    }

    #[test]
    fn real_capture_distinguishes_saturated_and_idle_queues_independently_of_backlog() {
        let frames =
            model::parse(include_str!("../test-data/zview-bench-60s/session.json")).unwrap();
        let queue = frames[0]
            .workqs
            .iter()
            .find(|q| q.object.name == "bench_workq")
            .unwrap();
        let key = WorkQueueKey {
            address: queue.object.address,
            name: queue.object.name.clone(),
        };
        let history = History::new(&frames, frames.len() - 1, &key);
        assert!(
            history
                .readings
                .iter()
                .all(|r| r.state == Activity::Processing)
        );
        assert!(
            history
                .readings
                .iter()
                .any(|r| r.state == Activity::Processing && r.pending == Some(0))
        );
        let busy = observed_busy(&frames, frames.len() - 1, &key);
        assert!((busy.window - 10.).abs() < 1e-9);
        assert!((busy.known - busy.window).abs() < 1e-9);
        assert_eq!(busy.percent(), Some(100.));
        assert!(busy.cadence.is_some_and(|t| t > 0.));
        let end = frames.last().unwrap().elapsed_seconds.unwrap();
        assert!(history.runs().iter().all(|r| r.0 < r.1 && r.1 <= end));
        let system = frames[0]
            .workqs
            .iter()
            .find(|q| q.object.name == "k_sys_work_q")
            .unwrap();
        let key = WorkQueueKey {
            address: system.object.address,
            name: system.object.name.clone(),
        };
        assert!(
            History::new(&frames, frames.len() - 1, &key)
                .readings
                .iter()
                .all(|r| r.state == Activity::Idle)
        );
        assert_eq!(
            observed_busy(&frames, frames.len() - 1, &key).percent(),
            Some(0.)
        );
    }

    #[test]
    fn activity_demo_covers_every_supported_state_without_fabricating_item_events() {
        let frames = model::parse(include_str!("../test-data/workq-activity.json")).unwrap();
        let key = WorkQueueKey {
            address: 28672,
            name: "activity_demo".into(),
        };
        let history = History::new(&frames, frames.len() - 1, &key);
        for state in [
            Activity::Processing,
            Activity::Idle,
            Activity::Ready,
            Activity::Unknown,
            Activity::NotStarted,
            Activity::NotProcessing,
        ] {
            assert!(
                history.readings.iter().any(|r| r.state == state),
                "{state:?}"
            );
        }
        let idle_intervals = history
            .runs()
            .into_iter()
            .filter(|(_, _, state, _)| *state == Activity::Idle)
            .map(|(start, end, _, _)| (start, end))
            .collect::<Vec<_>>();
        assert_eq!(
            idle_intervals,
            vec![(0., 3.), (6., 9.), (11., 12.), (18., 20.)]
        );
        let busy = observed_busy(&frames, frames.len() - 1, &key);
        assert_eq!((busy.window, busy.known, busy.processing), (10., 8., 3.));
        assert_eq!(busy.percent(), Some(37.5));
    }

    #[test]
    fn pressure_requires_positive_evidence_of_backlog_or_a_complete_empty_list() {
        let mut f = frames().remove(0);
        let q = &mut f.workqs[0];
        assert_eq!(Pressure::from_queue(Some(q)), Pressure::Active);
        q.pending = Some(vec!["task".into()]);
        assert_eq!(Pressure::from_queue(Some(q)), Pressure::Backlog);
        q.pending_truncated = true;
        assert_eq!(Pressure::from_queue(Some(q)), Pressure::Backlog);
        q.flags = None;
        assert_eq!(
            Pressure::from_queue(Some(q)),
            Pressure::Backlog,
            "known waiting work does not require known activity flags"
        );
        q.pending = Some(vec![]);
        assert_eq!(Pressure::from_queue(Some(q)), Pressure::Unknown);
        q.flags = Some(3);
        assert_eq!(
            Pressure::from_queue(Some(q)),
            Pressure::Unknown,
            "an incomplete empty list is not proof of no backlog"
        );
        q.pending_truncated = false;
        assert_eq!(Pressure::from_queue(Some(q)), Pressure::Active);
        q.pending = None;
        assert_eq!(Pressure::from_queue(Some(q)), Pressure::Unknown);
        q.pending = Some(vec![]);
        q.flags = Some(1);
        assert_eq!(Pressure::from_queue(Some(q)), Pressure::Clear);
        q.flags = Some(0);
        assert_eq!(Pressure::from_queue(Some(q)), Pressure::Clear);
        q.flags = None;
        assert_eq!(Pressure::from_queue(Some(q)), Pressure::Unknown);
        assert_eq!(Pressure::from_queue(None), Pressure::Unknown);
    }

    #[test]
    fn pressure_changes_split_processing_runs_without_changing_busy_estimates() {
        let mut data = frames();
        for f in &mut data {
            f.workqs[0].flags = Some(3);
        }
        data[1].workqs[0].pending = Some(vec!["task".into()]);
        data[2].workqs[0].pending_truncated = true;
        data[3].workqs[0].flags = None;
        data[3].workqs[0].pending = Some(vec!["a".into(), "b".into()]);
        data[4].workqs[0].flags = Some(1);
        let history = History::new(&data, 5, &key());
        assert_eq!(
            history.runs(),
            vec![
                (0., 1., Activity::Processing, Pressure::Active),
                (1., 4., Activity::Processing, Pressure::Backlog),
                (4., 6., Activity::Processing, Pressure::Unknown),
                (6., 9., Activity::Unknown, Pressure::Backlog),
                (9., 12., Activity::Idle, Pressure::Clear),
            ]
        );
        assert!(history.description(1).contains("Work waiting"));
        assert!(history.description(2).contains("Pressure unknown"));
        let busy = observed_busy(&data, 5, &key()).percent();
        for f in &mut data {
            f.workqs[0].pending = Some(vec!["many".into(); 100]);
        }
        assert_eq!(
            observed_busy(&data, 5, &key()).percent(),
            busy,
            "queue pressure is not processing-time utilization"
        );
        assert_eq!(
            history.readings[0].pressure,
            Pressure::Active,
            "later backlogs do not rescale historical colors"
        );
    }
}

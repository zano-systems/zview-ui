//! A byte-accurate, single-row heap map with an independent spatial viewport.
use crate::{
    BORDER, CARD, MUTED, PURPLE, layout,
    model::{HeapChunk, HeapKey},
    zoom_strip::{self, Navigation, label_origin},
};
use gpui_kit::component::{
    ActiveTheme, StyledExt,
    plot::{Plot, PlotElement, TooltipState, tooltip::Tooltip},
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::{cell::Cell, collections::HashMap, rc::Rc};

#[derive(Default)]
pub(super) struct MapStates {
    heaps: HashMap<(u64, String), Entity<MapState>>,
}

impl MapStates {
    pub fn clear(&mut self) {
        self.heaps.clear();
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ChunkMap {
    pub total: u64,
    pub free: u64,
    pub largest_free: u64,
    pub used_count: usize,
}

impl ChunkMap {
    pub fn new(chunks: &[HeapChunk]) -> Option<Self> {
        if chunks.is_empty() || chunks.iter().any(|c| c.size == 0) {
            return None;
        }
        let total = chunks.iter().try_fold(0u64, |n, c| n.checked_add(c.size))?;
        let free = chunks.iter().filter(|c| !c.used).map(|c| c.size).sum();
        let largest_free = chunks
            .iter()
            .filter(|c| !c.used)
            .map(|c| c.size)
            .max()
            .unwrap_or(0);
        Some(Self {
            total,
            free,
            largest_free,
            used_count: chunks.iter().filter(|c| c.used).count(),
        })
    }

    pub fn fragmentation(&self) -> f64 {
        if self.free == 0 {
            0.
        } else {
            100. * (1. - self.largest_free as f64 / self.free as f64)
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Viewport {
    total: u64,
    start: u64,
    span: u64,
}

impl Viewport {
    fn sync(&mut self, total: u64) {
        let fitted = self.span == self.total;
        self.total = total;
        self.span = if fitted {
            total
        } else {
            self.span.min(total).max(1).min(total)
        };
        self.start = self.start.min(total - self.span);
    }

    fn fit(&mut self) {
        self.start = 0;
        self.span = self.total;
    }

    fn end(self) -> u64 {
        self.start + self.span
    }

    fn zoom(&mut self, factor: f64, anchor: f64) {
        if self.total == 0 || !factor.is_finite() || factor <= 0. || factor == 1. {
            return;
        }
        let span = ((self.span as f64 / factor).round() as u64).clamp(1, self.total);
        let shift = ((self.span as f64 - span as f64) * anchor.clamp(0., 1.)).round() as i128;
        self.start =
            (i128::from(self.start) + shift).clamp(0, i128::from(self.total - span)) as u64;
        self.span = span;
    }

    fn pan(&mut self, bytes: i128) {
        self.start = i128::from(self.start)
            .saturating_add(bytes)
            .clamp(0, i128::from(self.total - self.span)) as u64;
    }

    fn byte_at(self, fraction: f64) -> Option<u64> {
        (self.span > 0).then(|| {
            self.start + ((fraction.clamp(0., 1.) * self.span as f64) as u64).min(self.span - 1)
        })
    }
}

impl zoom_strip::Viewport for Viewport {
    fn zoom(&mut self, factor: f64, anchor: f64) {
        self.zoom(factor, anchor);
    }
    fn fit(&mut self) {
        self.fit();
    }
    fn pan_fraction(&mut self, fraction: f64) {
        self.pan((fraction * self.span as f64).round() as i128);
    }
    fn step(&mut self, forward: bool) {
        let step = i128::from((self.span / 10).max(1));
        self.pan(if forward { step } else { -step });
    }
    fn edge(&mut self, end: bool) {
        self.start = if end { self.total - self.span } else { 0 };
    }
    fn can_zoom_in(&self) -> bool {
        self.span > 1
    }
    fn can_zoom_out(&self) -> bool {
        self.span < self.total
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Block {
    index: usize,
    start: u64,
    size: u64,
    used: bool,
}

impl Block {
    fn end(self) -> u64 {
        self.start + self.size
    }
    fn description(self) -> String {
        format!(
            "Chunk {} · {} · {}B · relative offset 0x{:x}–0x{:x} (end exclusive).",
            self.index + 1,
            if self.used { "Allocated" } else { "Free" },
            self.size,
            self.start,
            self.end()
        )
    }
}

struct MapData {
    stats: ChunkMap,
    blocks: Vec<Block>,
}

impl MapData {
    fn new(chunks: &[HeapChunk]) -> Option<Self> {
        let stats = ChunkMap::new(chunks)?;
        let mut start = 0;
        let blocks = chunks
            .iter()
            .enumerate()
            .map(|(index, chunk)| {
                let block = Block {
                    index,
                    start,
                    size: chunk.size,
                    used: chunk.used,
                };
                start += chunk.size;
                block
            })
            .collect();
        Some(Self { stats, blocks })
    }

    fn matches(&self, chunks: &[HeapChunk]) -> bool {
        self.blocks.len() == chunks.len()
            && self
                .blocks
                .iter()
                .zip(chunks)
                .all(|(a, b)| a.size == b.size && a.used == b.used)
    }

    fn hit(&self, viewport: Viewport, fraction: f64) -> Option<Block> {
        let byte = viewport.byte_at(fraction)?;
        self.blocks
            .get(self.blocks.partition_point(|b| b.end() <= byte))
            .copied()
    }

    fn visible(&self, viewport: Viewport) -> impl Iterator<Item = &Block> {
        let first = self.blocks.partition_point(|b| b.end() <= viewport.start);
        self.blocks[first..]
            .iter()
            .take_while(move |b| b.start < viewport.end())
    }

    fn interval(block: Block, viewport: Viewport, width: f64) -> (f64, f64) {
        let left = block.start.max(viewport.start) - viewport.start;
        let right = block.end().min(viewport.end()) - viewport.start;
        (
            left as f64 / viewport.span as f64 * width,
            right as f64 / viewport.span as f64 * width,
        )
    }

    /// Subpixel blocks are coverage-weighted into pixel bins. Zoom/hit testing
    /// still use the original blocks; no allocation is widened or invented.
    fn marks(&self, viewport: Viewport, width: f64) -> Vec<Mark> {
        if width <= 0. || viewport.span == 0 {
            return Vec::new();
        }
        let mut marks = Vec::new();
        let mut bins = vec![None::<Mark>; width.ceil() as usize + 1];
        for block in self.visible(viewport) {
            let (left, right) = Self::interval(*block, viewport, width);
            if right - left >= 1. {
                marks.push(Mark {
                    left,
                    right,
                    used: if block.used { right - left } else { 0. },
                    block: Some(*block),
                });
            } else {
                let mut x = left;
                while x < right {
                    let bin = x.floor() as usize;
                    let end = right.min(bin as f64 + 1.);
                    let mark = bins[bin].get_or_insert(Mark {
                        left: x,
                        right: end,
                        used: 0.,
                        block: None,
                    });
                    mark.right = end;
                    if block.used {
                        mark.used += end - x;
                    }
                    x = end;
                }
            }
        }
        marks.extend(bins.into_iter().flatten());
        marks
    }
}

#[derive(Clone, Copy, Debug)]
struct Mark {
    left: f64,
    right: f64,
    used: f64,
    block: Option<Block>,
}

#[derive(Default)]
struct MapState {
    data: Option<Rc<MapData>>,
    frame: usize,
    navigation: Navigation<Viewport>,
    selected: Option<Block>,
}

impl MapState {
    fn sync(&mut self, chunks: Option<&[HeapChunk]>, frame: usize) {
        let same = self
            .data
            .as_ref()
            .zip(chunks)
            .is_some_and(|(data, chunks)| data.matches(chunks));
        if !same || self.frame != frame {
            self.selected = None;
        }
        self.frame = frame;
        if !same {
            self.data = chunks.and_then(MapData::new).map(Rc::new);
        }
        if let Some(data) = &self.data {
            if self.navigation.viewport.total != data.stats.total {
                self.navigation.cancel_drag();
            }
            self.navigation.viewport.sync(data.stats.total);
        } else {
            self.navigation.cancel_drag();
        }
    }
}

impl zoom_strip::State for MapState {
    type Viewport = Viewport;
    fn navigation(&self) -> &Navigation<Viewport> {
        &self.navigation
    }
    fn navigation_mut(&mut self) -> &mut Navigation<Viewport> {
        &mut self.navigation
    }
    fn select(&mut self, position: Point<Pixels>) {
        self.selected = self.data.as_ref().and_then(|data| {
            data.hit(self.navigation.viewport, self.navigation.fraction(position))
        });
    }
}

pub(super) fn compact_size(bytes: u64) -> String {
    for (unit, suffix) in [(1u64 << 30, "GiB"), (1 << 20, "MiB"), (1 << 10, "KiB")] {
        if bytes >= unit && bytes.is_multiple_of(unit) {
            return format!("{}{suffix}", bytes / unit);
        }
    }
    format!("{bytes}B")
}

struct HeapStrip {
    data: Rc<MapData>,
    viewport: Viewport,
    selected: Option<Block>,
    bounds: Rc<Cell<Bounds<Pixels>>>,
}

impl IntoElement for HeapStrip {
    type Element = PlotElement<Self>;
    fn into_element(self) -> Self::Element {
        PlotElement::new(self)
    }
}

impl Plot for HeapStrip {
    fn id(&self) -> Option<ElementId> {
        Some("heap-map-strip-plot".into())
    }

    fn prepaint(&mut self, bounds: Bounds<Pixels>, _: &mut Window, _: &mut App) -> Vec<AnyElement> {
        self.bounds.set(bounds);
        Vec::new()
    }

    fn paint(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        let marks = self
            .data
            .marks(self.viewport, bounds.size.width.as_f32() as f64);
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for mark in marks {
                let width = px((mark.right - mark.left) as f32);
                let rect = Bounds::new(
                    bounds.origin + point(px(mark.left as f32), px(0.)),
                    size(width, bounds.size.height),
                );
                let fraction = (mark.used / (mark.right - mark.left)).clamp(0., 1.) as f32;
                let used = rgb(PURPLE);
                let free = rgb(CARD);
                let color = Rgba {
                    r: used.r * fraction + free.r * (1. - fraction),
                    g: used.g * fraction + free.g * (1. - fraction),
                    b: used.b * fraction + free.b * (1. - fraction),
                    a: 1.,
                };
                window.paint_quad(fill(rect, color));
                if let Some(block) = mark.block {
                    if width >= px(2.) {
                        window.paint_quad(fill(
                            Bounds::new(rect.origin, size(px(0.75), rect.size.height)),
                            rgb(0x625675),
                        ));
                    }
                    let text: SharedString = compact_size(block.size).into();
                    let line = window.text_system().shape_line(
                        text.clone(),
                        px(11.),
                        &[TextRun {
                            len: text.len(),
                            font: Font {
                                family: cx.theme().mono_font_family.clone(),
                                ..Default::default()
                            },
                            color: rgb(if block.used { 0x241e36 } else { MUTED }).into(),
                            background_color: None,
                            underline: None,
                            strikethrough: None,
                        }],
                        None,
                    );
                    if let Some(origin) = label_origin(rect, line.width) {
                        let _ = line.paint(origin, px(14.), TextAlign::Left, None, window, cx);
                    }
                }
            }
            if let Some(block) = self
                .selected
                .filter(|b| b.end() > self.viewport.start && b.start < self.viewport.end())
            {
                let (left, right) =
                    MapData::interval(block, self.viewport, bounds.size.width.as_f32() as f64);
                let x = bounds.left() + px(left as f32);
                let w = px((right - left) as f32);
                let color = rgb(0xe6dfef);
                for rect in [
                    Bounds::new(point(x, bounds.top()), size(w, px(1.5))),
                    Bounds::new(point(x, bounds.bottom() - px(1.5)), size(w, px(1.5))),
                    Bounds::new(
                        point(x, bounds.top()),
                        size(px(1.).min(w), bounds.size.height),
                    ),
                    Bounds::new(
                        point(x + w - px(1.).min(w), bounds.top()),
                        size(px(1.).min(w), bounds.size.height),
                    ),
                ] {
                    window.paint_quad(fill(rect, color));
                }
            }
        });
    }

    fn tooltip_state(
        &self,
        position: Point<Pixels>,
        bounds: Bounds<Pixels>,
        _: &App,
    ) -> Option<TooltipState> {
        if bounds.size.width <= px(0.) {
            return None;
        }
        let block = self
            .data
            .hit(self.viewport, (position.x / bounds.size.width) as f64)?;
        Some(TooltipState::new(block.index, position, vec![]))
    }

    fn tooltip(
        &self,
        state: &TooltipState,
        cursor: Point<Pixels>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut App,
    ) -> Option<AnyElement> {
        let block = self.data.blocks.get(state.index)?;
        Some(
            Tooltip::new(cursor, bounds.size)
                .w(px(280.))
                .child(
                    div()
                        .debug_selector(|| "heap-chunk-tooltip".into())
                        .child(block.description()),
                )
                .into_any_element(),
        )
    }
}

pub(super) fn view(
    states: &Entity<MapStates>,
    key: &HeapKey,
    chunks: Option<&[HeapChunk]>,
    frame: usize,
    window: &mut Window,
    cx: &mut App,
) -> impl IntoElement {
    let state = states.update(cx, |states, cx| {
        states
            .heaps
            .entry((key.address, key.name.clone()))
            .or_insert_with(|| cx.new(|_| MapState::default()))
            .clone()
    });
    state.update(cx, |s, _| s.sync(chunks, frame));
    let s = state.read(cx);
    let data = s.data.clone();
    let viewport = s.navigation.viewport;
    let selected = s.selected;
    let bounds = s.navigation.bounds.clone();
    let valid = data.is_some();
    let controls = zoom_strip::controls(
        &state,
        zoom_strip::Controls {
            zoom_out: "heap-map-zoom-out",
            zoom_in: "heap-map-zoom-in",
            fit: "heap-map-fit",
            name: "heap map",
            fit_label: "Fit entire heap",
        },
        valid,
        window,
        cx,
    );
    div().debug_selector(|| "heap-map".into()).flex().flex_col().flex_shrink_0().min_w_0().gap_1()
        .p_3().rounded_lg().border_1().border_color(rgb(BORDER)).bg(rgb(CARD))
        .child(div().flex().items_center().justify_between().gap_2()
            .child(div().text_sm().font_semibold().child("Fragmentation map"))
            .child(div().flex().items_center().gap_2()
                .child(div().debug_selector(move || format!("heap-map-view-{}-{}",viewport.start,viewport.end()))
                    .child(layout::note("heap-map-range", if valid { format!("{}–{} / {} · {:.1}×", compact_size(viewport.start), compact_size(viewport.end()), compact_size(viewport.total), viewport.total as f64 / viewport.span as f64) } else { "Map unavailable".into() })))
                .child(controls)))
        .when(!valid, |el| el.child(div().debug_selector(|| "heap-map-unavailable".into()).text_sm()
            .child(match chunks { None => "Chunk map not captured for this frame.", Some([]) => "No chunks captured for this frame.", Some(_) => "Chunk map unavailable: zero-size chunks or byte-size overflow." })))
        .when_some(data, |el,data| {
            el.child(zoom_strip::surface(&state,"heap-map-strip",
                HeapStrip { data: data.clone(), viewport, selected, bounds },window))
                .child(layout::note("heap-map-metrics",format!(
                    "Largest free: {} · Fragmentation: {:.1}% · Allocated chunks: {} / {} · Span includes chunk headers",
                    compact_size(data.stats.largest_free),data.stats.fragmentation(),data.stats.used_count,data.blocks.len()
                )))
        })
        .child(div().min_w_0().debug_selector(move || selected.map_or_else(|| "heap-map-unselected".into(), |b| format!("heap-map-selected-{}",b.index)))
            .child(layout::note("heap-map-inspection",selected.map(Block::description).unwrap_or_else(||
                "Purple: allocated · Unfilled: free · Hover/click to inspect · Drag or horizontal scroll to pan · Vertical scroll to zoom".into()))))
}

#[cfg(test)]
mod tests {
    use super::{MapData, MapState, Viewport, compact_size, label_origin};
    use crate::model::HeapChunk;
    use gpui_kit::{Bounds, point, px, size};

    fn chunk(size: u64, used: bool) -> HeapChunk {
        HeapChunk { size, used }
    }

    #[test]
    fn single_strip_geometry_preserves_byte_order_sizes_and_clipped_identity() {
        let data = MapData::new(&[chunk(3, true), chunk(8, false), chunk(53, true)]).unwrap();
        let mut viewport = Viewport::default();
        viewport.sync(64);
        let intervals = data
            .visible(viewport)
            .map(|b| MapData::interval(*b, viewport, 640.))
            .collect::<Vec<_>>();
        assert_eq!(intervals, vec![(0., 30.), (30., 110.), (110., 640.)]);
        assert_eq!(data.hit(viewport, 3. / 64.).unwrap().index, 1);
        assert_eq!(data.hit(viewport, 1.).unwrap().index, 2);
        viewport.start = 16;
        viewport.span = 16;
        assert_eq!(data.visible(viewport).count(), 1);
        let block = data.hit(viewport, 0.).unwrap();
        assert_eq!((block.start, block.size), (11, 53));
        assert_eq!(MapData::interval(block, viewport, 640.), (0., 640.));
        assert!(block.description().contains("53B"));
        assert!(block.description().contains("0xb–0x40"));
    }

    #[test]
    fn viewport_zoom_pan_fit_and_capacity_changes_stay_inside_map() {
        let mut v = Viewport::default();
        v.sync(1024);
        v.zoom(2., 0.5);
        assert_eq!((v.start, v.end()), (256, 768));
        v.pan(10000);
        assert_eq!((v.start, v.end()), (512, 1024));
        v.pan(-10000);
        assert_eq!((v.start, v.end()), (0, 512));
        v.zoom(2., 0.);
        assert_eq!((v.start, v.end()), (0, 256));
        v.sync(128);
        assert_eq!((v.start, v.end()), (0, 128));
        v.sync(2048);
        assert_eq!((v.start, v.end()), (0, 2048));
        v.zoom(2., 1.);
        assert_eq!((v.start, v.end()), (1024, 2048));
        v.fit();
        assert_eq!((v.start, v.end()), (0, 2048));
        v.sync(u64::MAX);
        for _ in 0..70 {
            v.zoom(2., 1.);
        }
        assert_eq!(v.span, 1);
        assert_eq!(v.end(), u64::MAX);
        assert_eq!(v.byte_at(1.), Some(u64::MAX - 1));
        crate::zoom_strip::Viewport::edge(&mut v, false);
        crate::zoom_strip::Viewport::step(&mut v, true);
        assert_eq!(
            v.start, 1,
            "integer-coordinate strips can still pan a byte at maximum zoom"
        );
        v.fit();
        assert_eq!((v.start, v.span), (0, u64::MAX));
        let before = v;
        v.zoom(f64::NAN, 0.5);
        assert_eq!(v, before);
    }

    #[test]
    fn dense_overviews_bound_paint_work_without_distorting_coverage_or_hit_testing() {
        let chunks = (0..100_000)
            .map(|i| chunk(1, i % 2 == 0))
            .collect::<Vec<_>>();
        let data = MapData::new(&chunks).unwrap();
        let mut v = Viewport::default();
        v.sync(100_000);
        let marks = data.marks(v, 800.);
        assert!(marks.len() <= 801);
        assert!((marks.iter().map(|m| m.right - m.left).sum::<f64>() - 800.).abs() < 1e-6);
        assert!((marks.iter().map(|m| m.used).sum::<f64>() - 400.).abs() < 1e-6);
        assert!(
            marks
                .iter()
                .all(|m| m.left >= 0. && m.right <= 800. && m.right > m.left)
        );
        assert_eq!(data.hit(v, 0.5).unwrap().index, 50_000);
        v.start = 99_999;
        v.span = 1;
        let marks = data.marks(v, 800.);
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].block.unwrap().index, 99_999);
        assert!(!marks[0].block.unwrap().used);
    }

    #[test]
    fn labels_only_fit_wholly_inside_the_visible_part_of_a_block() {
        for (x, width) in [(0., 30.), (700., 30.), (0., 1000.)] {
            let bounds = Bounds::new(point(px(x), px(10.)), size(px(width), px(42.)));
            let origin = label_origin(bounds, px(20.)).unwrap();
            assert!(origin.x >= bounds.left() + px(4.));
            assert!(origin.x + px(20.) <= bounds.right() - px(4.));
            assert!(origin.y >= bounds.top() && origin.y + px(14.) <= bounds.bottom());
        }
        assert!(
            label_origin(
                Bounds::new(point(px(0.), px(0.)), size(px(27.), px(42.))),
                px(20.)
            )
            .is_none()
        );
        assert_eq!(compact_size(16), "16B");
        assert_eq!(compact_size(1024), "1KiB");
        assert_eq!(compact_size(1536), "1536B");
    }

    #[test]
    fn selection_is_sample_local_and_missing_maps_do_not_leave_stale_blocks() {
        let chunks = [chunk(16, true), chunk(48, false)];
        let mut s = MapState::default();
        s.sync(Some(&chunks), 0);
        s.navigation.viewport.zoom(2., 0.5);
        s.selected = s.data.as_ref().unwrap().hit(s.navigation.viewport, 0.5);
        s.navigation.begin_drag(point(px(10.), px(10.)));
        let viewport = s.navigation.viewport;
        s.sync(Some(&chunks), 0);
        assert!(s.selected.is_some());
        s.sync(Some(&chunks), 1);
        assert!(s.selected.is_none());
        assert!(
            s.navigation.is_dragging(),
            "spatial panning can continue during playback"
        );
        assert_eq!(s.navigation.viewport, viewport);
        s.sync(None, 2);
        assert!(s.data.is_none());
        assert!(s.selected.is_none());
        assert!(!s.navigation.is_dragging());
        s.sync(Some(&chunks), 3);
        assert_eq!(s.navigation.viewport, viewport);
        s.selected = s.data.as_ref().unwrap().hit(s.navigation.viewport, 0.5);
        s.sync(Some(&[chunk(8, true)]), 3);
        assert!(s.selected.is_none());
        assert_eq!(s.navigation.viewport.total, 8);
    }
}

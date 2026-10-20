use crate::{
    BORDER, CARD, GREEN, MUTED, PURPLE, ThreadsApp,
    model::{self, ChartSeries, Frame, HeapChunk, HeapKey},
    time_series_chart::TimeSeriesChart,
};
use gpui_kit::component::{StyledExt, button::Button, progress::Progress, tooltip::Tooltip};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

/// Map sizes include chunk headers; they need not equal the allocator's
/// payload accounting. Never derive absolute addresses from the k_heap address.
struct ChunkMap<'a> {
    chunks: &'a [HeapChunk],
    total: u64,
    free: u64,
    largest_free: u64,
    used_count: usize,
}

impl<'a> ChunkMap<'a> {
    fn new(chunks: &'a [HeapChunk]) -> Option<Self> {
        if chunks.is_empty() || chunks.iter().any(|chunk| chunk.size == 0) {
            return None;
        }
        let total = chunks
            .iter()
            .try_fold(0u64, |total, chunk| total.checked_add(chunk.size))?;
        let free = chunks.iter().filter(|c| !c.used).map(|c| c.size).sum();
        let largest_free = chunks
            .iter()
            .filter(|c| !c.used)
            .map(|c| c.size)
            .max()
            .unwrap_or(0);
        Some(Self {
            chunks,
            total,
            free,
            largest_free,
            used_count: chunks.iter().filter(|c| c.used).count(),
        })
    }

    fn fragmentation(&self) -> f64 {
        if self.free == 0 {
            0.
        } else {
            100. * (1. - self.largest_free as f64 / self.free as f64)
        }
    }

    /// Eight equal byte spans, read left-to-right then top-to-bottom. A chunk
    /// crossing a row is split visually but retains its identity and tooltip.
    fn rows(&self) -> Vec<Vec<MapSegment>> {
        let rows = self.total.min(8);
        let mut result = vec![Vec::new(); rows as usize];
        let mut start = 0;
        for (index, chunk) in self.chunks.iter().enumerate() {
            let end = start + chunk.size;
            for row in 0..rows {
                let row_start =
                    (u128::from(self.total) * u128::from(row) / u128::from(rows)) as u64;
                let row_end =
                    (u128::from(self.total) * u128::from(row + 1) / u128::from(rows)) as u64;
                let a = start.max(row_start);
                let b = end.min(row_end);
                if a < b {
                    result[row as usize].push(MapSegment {
                        index,
                        bytes: b - a,
                        chunk_start: start,
                    });
                }
            }
            start = end;
        }
        result
    }
}

#[derive(Clone, Debug)]
struct MapSegment {
    index: usize,
    bytes: u64,
    chunk_start: u64,
}

fn usage_history(frames: &[Frame], current: usize, key: &HeapKey) -> (ChartSeries, f64) {
    let series = model::chart_series(frames, current, |frame| {
        Some(key.in_frame(frame)?.allocated_bytes as f64)
    });
    // Include historical capacities so a smaller current heap cannot clip
    // earlier valid readings; future frames never affect this scale.
    let maximum = frames[..=current]
        .iter()
        .filter_map(|frame| key.in_frame(frame))
        .map(|heap| heap.total() as f64)
        .fold(1., f64::max);
    (series, maximum)
}

fn field(title: &'static str, value: String) -> impl IntoElement {
    div()
        .min_w(px(110.))
        .flex_1()
        .flex()
        .flex_col()
        .gap_2()
        .child(div().text_xs().text_color(rgb(MUTED)).child(title))
        .child(div().text_sm().child(value))
}

fn bytes(value: Option<u64>) -> String {
    value
        .map(|v| format!("{v} B"))
        .unwrap_or_else(|| "—".into())
}

fn fragmentation_map(chunks: Option<&[HeapChunk]>) -> impl IntoElement {
    let map = chunks.and_then(ChunkMap::new);
    div()
        .debug_selector(|| "heap-map".into())
        .flex()
        .flex_col()
        .gap_3()
        .child(div().text_sm().font_semibold().child("Fragmentation map"))
        .child(
            div().text_xs().text_color(rgb(MUTED)).child(
                "Purple: allocated · Unfilled: free · Outlines show chunk segments. Address order, left to right, then down. Hover for size and relative offset.",
            ),
        )
        .when(map.is_none(), |el| {
            el.child(
                div()
                    .debug_selector(|| "heap-map-unavailable".into())
                    .text_sm()
                    .child(match chunks {
                    None => "Chunk map not captured for this frame.",
                    Some([]) => "No chunks captured for this frame.",
                    Some(_) => "Chunk map unavailable: zero-size chunks or byte-size overflow.",
                    }),
            )
        })
        .when_some(map, |el, map| {
            let rows = map.rows().into_iter().enumerate().map(|(row, segments)| {
                let row_bytes = segments.iter().map(|s| s.bytes).sum::<u64>();
                div()
                    .h(px(18.))
                    .w_full()
                    .flex()
                    .children(segments.into_iter().map(|segment| {
                        let chunk = &map.chunks[segment.index];
                        let label = chunk_label(&segment, chunk);
                        div()
                            .id(("heap-map-segment", row * (map.chunks.len() + 1) + segment.index))
                            .debug_selector(move || format!("heap-map-{row}-{}", segment.index))
                            .h_full()
                            .min_w_0()
                            .w(relative(segment.bytes as f32 / row_bytes as f32))
                            .flex_shrink_0()
                            .when(chunk.used, |el| el.bg(rgb(PURPLE)))
                            .border(px(0.5))
                            .border_color(rgb(0x625675))
                            .tooltip(move |window, cx| {
                                let label = label.clone();
                                Tooltip::element(move |_, _| {
                                    div()
                                        .debug_selector(|| "heap-chunk-tooltip".into())
                                        .w(px(280.))
                                        .child(label.clone())
                                })
                                .build(window, cx)
                            })
                    }))
            });
            el.child(
                div()
                    .debug_selector(|| "heap-map-metrics".into())
                    .text_xs()
                    .child(format!(
                        "Largest free: {} B · Fragmentation: {:.1}% · Allocated chunks: {} / {} · Map span: {} B",
                        map.largest_free, map.fragmentation(), map.used_count, map.chunks.len(), map.total,
                    )),
            )
            .child(div().flex().flex_col().gap_0().children(rows))
        })
        .child(
            div().text_xs().text_color(rgb(MUTED)).child(
                "Map sizes include allocator chunk overhead; they may differ from payload usage above. Offsets start at the first captured chunk, not at the heap object address.",
            ),
        )
}

fn chunk_label(segment: &MapSegment, chunk: &HeapChunk) -> String {
    format!(
        "Chunk {} · {} · {} B · relative offset 0x{:x}–0x{:x} (end exclusive). Absolute address not provided.",
        segment.index + 1,
        if chunk.used { "Allocated" } else { "Free" },
        chunk.size,
        segment.chunk_start,
        segment.chunk_start + chunk.size,
    )
}

impl ThreadsApp {
    pub(super) fn heap_detail(&self, key: &HeapKey, cx: &mut Context<Self>) -> impl IntoElement {
        let heap = key.in_frame(&self.frames[self.frame]);
        let waiters = heap.and_then(|h| h.waiters.as_deref());
        let fill = heap.and_then(|h| h.percent());
        let color = if heap.is_some_and(|h| h.total() > 0 && h.free_bytes == 0) {
            0xf18b94
        } else if waiters.is_some_and(|w| !w.is_empty()) {
            0xddb66b
        } else {
            GREEN
        };
        let (series, maximum) = usage_history(&self.frames, self.frame, key);
        let series = series.with_playhead(self.playback_time(cx));
        let caption = series.caption.clone();
        div().id("heap-detail").debug_selector(|| "heap-detail".into())
            .flex().flex_col().flex_grow(1.).flex_shrink_0().gap_4()
            .child(div().flex().items_center().justify_between().flex_wrap().gap_3()
                .child(div().text_lg().font_semibold().child("Heap details"))
                .child(div().debug_selector(|| "back-to-objects".into()).child(
                    Button::new("back-to-objects").label("Back to Kernel objects")
                        .on_click(cx.listener(|this, _, window, cx| this.show_kernel_objects(this.object_kind, window, cx))))))
            .when(heap.is_none(), |el| el.child(div().debug_selector(|| "heap-absent".into())
                .p_4().child("This heap is not present in the selected frame. Current values are not carried forward.")))
            .child(div().debug_selector(|| "heap-summary".into()).flex().flex_wrap().flex_shrink_0()
                .gap_4().p_4().rounded_lg().border_1().border_color(rgb(BORDER)).bg(rgb(CARD))
                .child(field("Name", key.name.clone()))
                .child(field("Address", format!("0x{:08x}", key.address)))
                .child(field("Allocated", bytes(heap.map(|h| h.allocated_bytes))))
                .child(field("Free", bytes(heap.map(|h| h.free_bytes))))
                .child(field("Payload capacity", bytes(heap.map(|h| h.total()))))
                .child(field("Peak allocated", bytes(heap.and_then(|h| h.max_allocated_bytes))))
                .child(field("Waiting threads", waiters.map(|w| w.len().to_string()).unwrap_or_else(|| "—".into())))
                .child(div().w_full().flex().flex_col().gap_2()
                    .child(div().text_xs().text_color(rgb(MUTED)).child(
                        fill.map(|v| format!("{v:.1}% allocated")).unwrap_or_else(|| "Usage ratio unavailable".into())))
                    .child(Progress::new("heap-capacity").value(fill.unwrap_or(0.) as f32).color(rgb(color)).h(px(6.)))))
            .child(div().flex().flex_grow(1.).flex_shrink_0().flex_wrap().gap_4()
                .child(div().debug_selector(|| "heap-history".into()).flex_1().min_w(px(320.)).min_h(px(280.))
                    .p_4().rounded_lg().border_1().border_color(rgb(BORDER)).bg(rgb(CARD)).flex().flex_col().gap_3()
                    .child(div().text_sm().font_semibold().child("Allocated bytes"))
                    .child(div().text_xs().text_color(rgb(MUTED)).child(format!("0–{maximum:.0} B · capacity through selected frame")))
                    .child(div().debug_selector(|| "heap-allocated-plot".into()).flex_1().min_h(px(180.)).child(
                        TimeSeriesChart::count("heap-allocated", series, rgb(PURPLE).into(), maximum)))
                    .child(div().text_xs().text_color(rgb(MUTED)).child(caption)))
                .child(div().w(px(280.)).min_w(px(240.)).p_4().rounded_lg().border_1()
                    .border_color(rgb(BORDER)).bg(rgb(CARD)).flex().flex_col().gap_3()
                    .child(div().text_sm().font_semibold().child("Waiting threads"))
                    .child(crate::wait_queue::threads("heap", waiters, crate::wait_queue::Layout::Vertical))))
            .child(div().p_4().rounded_lg().border_1().border_color(rgb(BORDER)).bg(rgb(CARD))
                .child(fragmentation_map(heap.and_then(|h| h.chunks.as_deref()))))
            .child(div().text_xs().text_color(rgb(MUTED)).child(
                "Sampled allocator state, not individual allocation/free events. Peak is target-reported, not reconstructed. Declaration location and absolute chunk addresses are not available in the dump."))
    }
}

#[cfg(test)]
mod tests {
    use super::{ChunkMap, HeapChunk, HeapKey, model, usage_history};

    fn chunk(size: u64, used: bool) -> HeapChunk {
        HeapChunk { size, used }
    }

    #[test]
    fn map_metrics_match_tui_and_geometry_preserves_every_byte() {
        let chunks = [
            chunk(32, true),
            chunk(8, false),
            chunk(16, false),
            chunk(8, true),
        ];
        let map = ChunkMap::new(&chunks).unwrap();
        assert_eq!(
            (map.total, map.free, map.largest_free, map.used_count),
            (64, 24, 16, 2)
        );
        assert!((map.fragmentation() - 100. / 3.).abs() < 1e-10);
        let rows = map.rows();
        assert_eq!(rows.len(), 8);
        for row in &rows {
            assert_eq!(row.iter().map(|s| s.bytes).sum::<u64>(), 8);
        }
        for (index, chunk) in chunks.iter().enumerate() {
            assert_eq!(
                rows.iter()
                    .flatten()
                    .filter(|s| s.index == index)
                    .map(|s| s.bytes)
                    .sum::<u64>(),
                chunk.size
            );
        }
        assert_eq!(rows[4][0].chunk_start, 32);
        let odd = [chunk(3, true), chunk(8, false)];
        let map = ChunkMap::new(&odd).unwrap();
        assert_eq!(
            map.rows().iter().flatten().map(|s| s.bytes).sum::<u64>(),
            11
        );
    }

    #[test]
    fn maps_distinguish_uncaptured_empty_invalid_exhausted_and_free() {
        assert!(ChunkMap::new(&[]).is_none());
        assert!(ChunkMap::new(&[chunk(0, true)]).is_none());
        assert!(ChunkMap::new(&[chunk(u64::MAX, true), chunk(1, false)]).is_none());
        let chunks = [chunk(10, true)];
        let map = ChunkMap::new(&chunks).unwrap();
        assert_eq!(
            (map.free, map.largest_free, map.fragmentation()),
            (0, 0, 0.)
        );
        let chunks = [chunk(1, false)];
        let map = ChunkMap::new(&chunks).unwrap();
        assert_eq!(map.rows().len(), 1);
        assert_eq!((map.used_count, map.fragmentation()), (0, 0.));
    }

    #[test]
    fn history_matches_identity_gaps_and_only_past_capacity() {
        let frames = model::parse(include_str!("../test-data/heaps.json")).unwrap();
        let key = HeapKey {
            address: 4096,
            name: "packet_heap".into(),
        };
        let (series, maximum) = usage_history(&frames, 4, &key);
        assert_eq!(maximum, 256.);
        assert_eq!(
            series.points.iter().map(|p| p.value).collect::<Vec<_>>(),
            vec![Some(64.), Some(256.), Some(128.), None, Some(0.)]
        );
        assert_eq!(series.points[1].x, 0.7);
        assert_eq!(
            key.in_frame(&frames[2]).unwrap().max_allocated_bytes,
            Some(256)
        );
        assert!(key.in_frame(&frames[3]).is_none());
        assert_eq!(usage_history(&frames, 0, &key).1, 256.);
        let old = model::parse(include_str!("../test-data/kernel-objects-minimal.json")).unwrap();
        assert!(old[0].heaps[0].chunks.is_none());
        assert!(old[0].heaps[0].max_allocated_bytes.is_none());
    }
}

use crate::{
    BORDER, CARD, GREEN, MUTED, PURPLE, ThreadsApp,
    heap_map::{self, ChunkMap},
    model::{self, ChartSeries, Frame, HeapKey},
    time_series_chart::TimeSeriesChart,
};
use gpui_kit::component::{StyledExt, button::Button, progress::Progress, tooltip::Tooltip};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

const FRAGMENTATION_RED: u32 = 0xf18b94;

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

fn fragmentation_history(frames: &[Frame], current: usize, key: &HeapKey) -> ChartSeries {
    model::chart_series(frames, current, |frame| {
        let chunks = key.in_frame(frame)?.chunks.as_deref()?;
        Some(ChunkMap::new(chunks)?.fragmentation())
    })
}

fn field(title: &'static str, value: String) -> impl IntoElement {
    div()
        .min_w(px(88.))
        .flex_1()
        .flex()
        .flex_col()
        .gap_1()
        .child(div().text_xs().text_color(rgb(MUTED)).child(title))
        .child(crate::layout::note(title, value).text_sm())
}

fn bytes(value: Option<u64>) -> String {
    value
        .map(|v| format!("{v} B"))
        .unwrap_or_else(|| "—".into())
}

impl ThreadsApp {
    pub(super) fn heap_detail(
        &self,
        key: &HeapKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
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
        let fragmentation = heap
            .and_then(|h| h.chunks.as_deref())
            .and_then(ChunkMap::new)
            .map(|map| map.fragmentation());
        let fragmentation_series = fragmentation_history(&self.frames, self.frame, key)
            .with_playhead(self.playback_time(cx));
        div().id("heap-detail").debug_selector(|| "heap-detail".into())
            .flex().flex_col().flex_1().min_h_0().gap_3()
            .child(div().flex().flex_shrink_0().items_center().justify_between().flex_wrap().gap_3()
                .child(div().text_lg().font_semibold().child("Heap details"))
                .child(div().debug_selector(|| "back-to-objects".into()).child(
                    Button::new("back-to-objects").label("Back to Kernel objects")
                        .on_click(cx.listener(|this, _, window, cx| this.show_kernel_objects(this.object_kind, window, cx))))))
            .when(heap.is_none(), |el| el.child(div().debug_selector(|| "heap-absent".into())
                .p_2().flex_shrink_0().child(crate::layout::note("heap-absent-note",
                    "This heap is not present in the selected frame. Current values are not carried forward."))))
            .child(div().debug_selector(|| "heap-summary".into()).flex().flex_wrap().flex_shrink_0()
                .gap_3().p_3().rounded_lg().border_1().border_color(rgb(BORDER)).bg(rgb(CARD))
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
            .child(heap_map::view(&self.heap_map_states, key, heap.and_then(|h| h.chunks.as_deref()), self.frame, window, cx))
            .child(div().flex().flex_1().min_h_0().gap_3()
                .child(div().debug_selector(|| "heap-history".into()).flex_1().min_w_0().min_h_0()
                    .p_3().rounded_lg().border_1().border_color(rgb(BORDER)).bg(rgb(CARD)).flex().flex_col().gap_1()
                    .child(div().flex().flex_shrink_0().items_center().gap_2()
                        .child(div().text_sm().font_semibold().flex_shrink_0().child("Allocated bytes"))
                        .child(crate::layout::note("heap-history-scale", format!("0–{maximum:.0} B · capacity through selected frame")).flex_1()))
                    .child(div().debug_selector(|| "heap-allocated-plot".into()).flex_1().min_h_0().child(
                        TimeSeriesChart::count("heap-allocated", series, rgb(PURPLE).into(), maximum)))
                    .child(crate::layout::note("heap-history-caption", caption)))
                .child(div().debug_selector(|| "heap-fragmentation-history".into()).flex_1().min_w_0().min_h_0()
                    .p_3().rounded_lg().border_1().border_color(rgb(BORDER)).bg(rgb(CARD)).flex().flex_col().gap_1()
                    .child(div().id("heap-fragmentation-heading").flex().flex_shrink_0().items_center().justify_between().gap_2()
                        .tooltip(|window, cx| Tooltip::new(
                            "Fragmentation = 100 × (1 − largest free chunk / total free map bytes). Zero when no map bytes are free. Missing or invalid maps are unavailable."
                        ).build(window, cx))
                        .child(div().text_sm().font_semibold().child("Fragmentation ratio"))
                        .child(crate::layout::note("heap-fragmentation-value", format!("{} · 0–100%", model::percent(fragmentation)))
                            .text_color(rgb(FRAGMENTATION_RED))))
                    .child(div().debug_selector(|| "heap-fragmentation-plot".into()).flex_1().min_h_0().child(
                        TimeSeriesChart::new("heap-fragmentation", fragmentation_series, rgb(FRAGMENTATION_RED).into()))))
                .child(div().debug_selector(|| "heap-wait-queue".into()).w(px(180.)).flex_shrink_0().min_h_0().p_3().rounded_lg().border_1()
                    .border_color(rgb(BORDER)).bg(rgb(CARD)).flex().flex_col().gap_2()
                    .child(div().text_sm().font_semibold().child("Waiting threads"))
                    .child(crate::wait_queue::threads("heap", waiters, crate::wait_queue::Layout::Vertical))))
            .child(crate::layout::note("heap-note",
                "Sampled allocator state, not individual allocation/free events. Peak is target-reported, not reconstructed. Declaration location and absolute chunk addresses are not available in the dump."))
    }
}

#[cfg(test)]
mod tests {
    use super::{ChunkMap, HeapKey, fragmentation_history, model, usage_history};
    use crate::model::HeapChunk;

    fn chunk(size: u64, used: bool) -> HeapChunk {
        HeapChunk { size, used }
    }

    #[test]
    fn map_metrics_match_tui() {
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
        assert_eq!((map.used_count, map.fragmentation()), (0, 0.));
    }

    #[test]
    fn fragmentation_history_uses_map_accounting_identity_and_past_readings() {
        let frames = model::parse(include_str!("../test-data/heaps.json")).unwrap();
        let key = HeapKey {
            address: 4096,
            name: "packet_heap".into(),
        };
        let series = fragmentation_history(&frames, 5, &key);
        assert!((series.points[0].value.unwrap() - 100. * (1. - 64. / 192.)).abs() < 1e-10);
        assert_eq!(series.points[1].value, Some(0.)); // No free map bytes.
        assert_eq!(series.points[2].value, Some(50.));
        assert_eq!(series.points[3].value, None); // Address reused by another heap.
        assert_eq!(series.points[4].value, Some(0.)); // One free chunk.
        assert_eq!(series.points[5].value, None); // Map was not captured.
        assert_eq!(series.points[1].x, 0.7);
        let earlier = fragmentation_history(&frames, 2, &key);
        assert_eq!(earlier.points.len(), 3);
        assert_eq!(earlier.points[2].value, Some(50.));
        let other = HeapKey {
            address: 8192,
            name: key.name.clone(),
        };
        assert!(
            fragmentation_history(&frames, 5, &other)
                .points
                .iter()
                .all(|p| p.value.is_none())
        );

        let mut invalid = frames.clone();
        invalid[0].heaps[0].chunks = Some(vec![]);
        invalid[1].heaps[0].chunks = Some(vec![chunk(0, false)]);
        invalid[2].heaps[0].chunks = Some(vec![chunk(u64::MAX, false), chunk(1, true)]);
        assert!(
            fragmentation_history(&invalid, 2, &key)
                .points
                .iter()
                .all(|p| p.value.is_none())
        );
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

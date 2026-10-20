use crate::{
    BORDER, CARD, GREEN, MUTED, ThreadsApp,
    kernel_model::MemorySlabKey,
    model::{self, ChartSeries, Frame},
    time_series_chart::TimeSeriesChart,
};
use gpui_kit::component::{StyledExt, button::Button, progress::Progress};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

fn usage_history(frames: &[Frame], current: usize, key: &MemorySlabKey) -> (ChartSeries, f64) {
    let series = model::chart_series(frames, current, |frame| {
        key.in_frame(frame)?.num_used.map(|used| used as f64)
    });
    let maximum = key
        .in_frame(&frames[current])
        .and_then(|slab| slab.num_blocks)
        .filter(|capacity| *capacity > 0)
        .map(|capacity| capacity as f64)
        .unwrap_or_else(|| {
            series
                .points
                .iter()
                .filter_map(|point| point.value)
                .fold(1., f64::max)
        });
    (series, maximum)
}

fn count(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "—".into())
}

fn bytes(value: Option<u64>) -> String {
    value
        .map(|value| format!("{value} B"))
        .unwrap_or_else(|| "—".into())
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

impl ThreadsApp {
    pub(super) fn slab_detail(
        &self,
        key: &MemorySlabKey,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let slab = key.in_frame(&self.frames[self.frame]);
        let waiters = slab.and_then(|slab| slab.object.waiters.as_deref());
        let exhausted = slab.is_some_and(|slab| {
            matches!((slab.num_used, slab.num_blocks), (Some(u), Some(n)) if n > 0 && u >= n)
        });
        let color = if exhausted {
            0xf18b94
        } else if waiters.is_some_and(|names| !names.is_empty()) {
            0xddb66b
        } else {
            GREEN
        };
        let (series, maximum) = usage_history(&self.frames, self.frame, key);
        let series = series.with_playhead(self.playback_time(cx));
        let caption = series.caption.clone();
        let scale = match slab.and_then(|slab| slab.num_blocks) {
            Some(0) => format!("Zero capacity · scale 0–{maximum:.0} observed"),
            Some(capacity) => format!("0–{capacity} blocks"),
            None => format!("Capacity unavailable · scale 0–{maximum:.0} observed"),
        };
        let fill = slab.and_then(|slab| model::percentage(slab.num_used?, slab.num_blocks?));
        let total_bytes = slab.and_then(|slab| slab.block_size?.checked_mul(slab.num_blocks?));
        let peak = match slab {
            Some(slab) => slab
                .max_used
                .map(|peak| format!("{peak} blocks"))
                .unwrap_or_else(|| "— · Not tracked / unavailable".into()),
            None => "—".into(),
        };
        div().id("slab-detail").debug_selector(|| "slab-detail".into())
            .flex().flex_col().flex_grow(1.).flex_shrink_0().gap_4()
            .child(div().flex().items_center().justify_between().flex_wrap().gap_3()
                .child(div().text_lg().font_semibold().child("Memory slab details"))
                .child(div().debug_selector(|| "back-to-objects".into()).child(
                    Button::new("back-to-objects").label("Back to Kernel objects")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.show_kernel_objects(this.object_kind, window, cx);
                        })))))
            .when(slab.is_none(), |el| el.child(
                div().debug_selector(|| "slab-absent".into()).p_4().rounded_lg()
                    .border_1().border_color(rgb(BORDER)).child(
                        "This memory slab is not present in the selected frame. Move to another frame or return to Kernel objects. Current values are not carried forward.")))
            .child(div().debug_selector(|| "slab-summary".into()).flex().flex_wrap().flex_shrink_0()
                .gap_4().p_4().rounded_lg().border_1().border_color(rgb(BORDER)).bg(rgb(CARD))
                .child(field("Name", key.name.clone()))
                .child(field("Address", format!("0x{:08x}", key.address)))
                .child(field("Blocks in use", count(slab.and_then(|s| s.num_used))))
                .child(field("Capacity", count(slab.and_then(|s| s.num_blocks))))
                .child(field("Block size", bytes(slab.and_then(|s| s.block_size))))
                .child(field("Total capacity", bytes(total_bytes)))
                .child(field("Peak usage", peak))
                .child(field("Waiting threads", waiters.map(|names| names.len().to_string()).unwrap_or_else(|| "—".into())))
                .child(div().w_full().flex().flex_col().gap_2()
                    .child(div().debug_selector(move || if fill.is_some() {
                        "slab-capacity-known".into()
                    } else {
                        "slab-capacity-unavailable".into()
                    }).text_xs().text_color(rgb(MUTED)).child(
                        fill.map(|value| format!("{value:.1}% of block capacity"))
                            .unwrap_or_else(|| "— · Usage ratio unavailable".into())))
                    .child(div().debug_selector(|| "slab-capacity-track".into()).w_full().h(px(6.)).flex_shrink_0().child(
                        Progress::new("slab-capacity")
                            .value(fill.unwrap_or(0.).clamp(0., 100.) as f32)
                            .color(rgb(if fill.is_some() { color } else { MUTED }))
                            .accessibility_label(fill.map(|value| format!("Blocks in use: {value:.1}% of capacity"))
                                .unwrap_or_else(|| "Block usage ratio unavailable".into()))
                            .h(px(6.))))))
            .child(div().flex().flex_grow(1.).flex_shrink_0().flex_wrap().gap_4()
                .child(div().debug_selector(|| "slab-history".into()).flex_1().min_w(px(320.)).min_h(px(280.))
                    .p_4().rounded_lg().border_1().border_color(rgb(BORDER)).bg(rgb(CARD)).flex().flex_col().gap_3()
                    .child(div().text_sm().font_semibold().child("Blocks in use"))
                    .child(div().text_xs().text_color(rgb(MUTED)).child(scale))
                    .child(div().debug_selector(|| "slab-usage-plot".into()).flex_1().min_h(px(180.))
                        .child(TimeSeriesChart::count("slab-usage", series, rgb(color).into(), maximum)))
                    .child(div().text_xs().text_color(rgb(MUTED)).child(caption)))
                .child(div().debug_selector(|| "slab-wait-queue".into()).w(px(280.)).min_w(px(240.))
                    .p_4().rounded_lg().border_1().border_color(rgb(BORDER)).bg(rgb(CARD)).flex().flex_col().gap_3()
                    .child(div().text_sm().font_semibold().child("Waiting threads"))
                    .child(crate::wait_queue::threads("slab", waiters, crate::wait_queue::Layout::Vertical))))
            .child(div().text_xs().text_color(rgb(MUTED)).child(
                "Sampled block counts, not individual allocation/free events. Peak is the target-reported high-water mark when tracked. Missing readings break the history line."))
    }
}

#[cfg(test)]
mod tests {
    use super::{MemorySlabKey, model, usage_history};

    #[test]
    fn demo_preserves_reported_peak_and_missing_measurements() {
        let frames = model::parse(include_str!("../test-data/memory-slabs.json")).unwrap();
        let key = MemorySlabKey {
            address: 24576,
            name: "packet_pool".into(),
        };
        let (series, maximum) = usage_history(&frames, 6, &key);
        assert_eq!(maximum, 8.);
        assert_eq!(
            series
                .points
                .iter()
                .map(|point| point.value)
                .collect::<Vec<_>>(),
            vec![Some(2.), Some(8.), Some(4.), None, Some(1.), None, Some(3.)]
        );
        let slab = key.in_frame(&frames[2]).unwrap();
        assert_eq!(slab.max_used, Some(8));
        assert_eq!(slab.num_used, Some(4));
        assert_eq!(slab.block_size.unwrap() * slab.num_blocks.unwrap(), 256);
        assert_eq!(
            key.in_frame(&frames[1])
                .unwrap()
                .object
                .waiters
                .as_ref()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(frames[0].mem_slabs[1].max_used, None);
        assert_eq!(frames.len(), 13);
        assert_eq!(frames[6].mem_slabs.len(), 5);
        assert_eq!(key.in_frame(&frames[11]).unwrap().num_used, Some(0));
        assert_eq!(
            key.in_frame(&frames[9])
                .unwrap()
                .object
                .waiters
                .as_ref()
                .unwrap()
                .len(),
            4
        );
        assert_eq!(frames[6].mem_slabs[2].num_used, Some(0));
        assert_eq!(frames[6].mem_slabs[3].num_used, None);
        assert_eq!(frames[6].mem_slabs[4].num_blocks, Some(0));
    }

    #[test]
    fn slab_history_preserves_identity_gaps_and_past_only_scale() {
        let mut frames =
            model::parse(include_str!("../test-data/kernel-objects-minimal.json")).unwrap();
        let key = MemorySlabKey {
            address: 24576,
            name: "slab".into(),
        };
        let (series, maximum) = usage_history(&frames, 0, &key);
        assert_eq!(series.points[0].value, Some(2.));
        assert_eq!(maximum, 8.);
        assert_eq!(usage_history(&frames, 1, &key).0.points[1].value, None);
        let mut duplicate = frames[0].mem_slabs[0].clone();
        duplicate.object.address += 1;
        duplicate.num_used = Some(99);
        frames[0].mem_slabs.insert(0, duplicate);
        assert_eq!(key.in_frame(&frames[0]).unwrap().num_used, Some(2));
        frames[1].mem_slabs = vec![frames[0].mem_slabs[1].clone()];
        frames[1].mem_slabs[0].num_used = Some(100);
        frames[0].mem_slabs[1].num_blocks = None;
        assert_eq!(usage_history(&frames, 0, &key).1, 2.);
        frames[0].mem_slabs[1].num_blocks = Some(0);
        assert_eq!(usage_history(&frames, 0, &key).1, 2.);
        frames[0].mem_slabs[1].num_used = None;
        assert_eq!(usage_history(&frames, 0, &key).0.points[0].value, None);
        assert_eq!(usage_history(&frames, 0, &key).1, 1.);
        frames[1].mem_slabs[0].object.name = "replacement".into();
        assert!(key.in_frame(&frames[1]).is_none());
    }
}

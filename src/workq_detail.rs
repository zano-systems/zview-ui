use crate::{
    BORDER, CARD, MUTED, PURPLE, ThreadsApp,
    kernel_model::WorkQueueKey,
    model::{self, ChartSeries, Frame},
    time_series_chart::TimeSeriesChart,
};
use gpui_kit::component::{Icon, StyledExt, button::Button, scroll::ScrollableElement, tag::Tag};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

fn backlog_history(frames: &[Frame], current: usize, key: &WorkQueueKey) -> (ChartSeries, f64) {
    let series = model::chart_series(frames, current, |frame| {
        let queue = key.in_frame(frame)?;
        // The chart's tooltips represent exact counts, not lower bounds.
        if queue.pending_truncated {
            return None;
        }
        Some(queue.pending.as_ref()?.len() as f64)
    });
    let maximum = series
        .points
        .iter()
        .filter_map(|point| point.value)
        .fold(1., f64::max);
    (series, maximum)
}

fn flags_label(flags: Option<u64>) -> String {
    let Some(flags) = flags else {
        return "—".into();
    };
    let words = [(1, "Started"), (2, "Busy"), (4, "Draining"), (8, "Plugged")]
        .into_iter()
        .filter_map(|(bit, word)| (flags & bit != 0).then_some(word))
        .collect::<Vec<_>>();
    format!(
        "{} · 0x{flags:x}",
        if words.is_empty() {
            "Not started".into()
        } else {
            words.join(" · ")
        }
    )
}

fn field(title: &'static str, value: String) -> impl IntoElement {
    div()
        .min_w(px(140.))
        .flex_1()
        .flex()
        .flex_col()
        .gap_2()
        .child(div().text_xs().text_color(rgb(MUTED)).child(title))
        .child(div().text_sm().child(value))
}

fn queue_panel(
    id: &'static str,
    title: &'static str,
    names: Option<&[String]>,
    truncated: bool,
) -> impl IntoElement {
    let is_work = id == "workq-pending";
    // Inline SVG keeps these icons available without expanding the app's asset bundle.
    let icon: &[u8] =
        br#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="3" y="7" width="18" height="14" rx="2"/><path d="M8 7V3h8v4M3 12h18M10 12v3h4v-3"/></svg>"#
    ;
    let (background, foreground, border) = (0x24364e, crate::BLUE, 0x3b5879);
    let entries = if !is_work {
        vec![
            crate::wait_queue::threads("workq-drain", names, crate::wait_queue::Layout::Vertical)
                .into_any_element(),
        ]
    } else {
        match names {
            Some(names) if !names.is_empty() => names
                .iter()
                .enumerate()
                .map(|(index, name)| {
                    div()
                        .debug_selector(move || format!("{id}-tag-{index}"))
                        .max_w_full()
                        .child(
                            Tag::custom(
                                rgb(background).into(),
                                rgb(foreground).into(),
                                rgb(border).into(),
                            )
                            .rounded_full()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .min_w_0()
                                    .child(Icon::default().data(icon).size(px(14.)))
                                    .child(div().min_w_0().child(name.clone())),
                            ),
                        )
                        .into_any_element()
                })
                .collect::<Vec<_>>(),
            Some([]) if !truncated => Vec::new(),
            _ => vec![
                div()
                    .debug_selector(|| "workq-pending-message".into())
                    .text_sm()
                    .text_color(rgb(MUTED))
                    .child(if names.is_none() {
                        "— · Unavailable"
                    } else {
                        "No items captured"
                    })
                    .into_any_element(),
            ],
        }
    };
    div()
        .debug_selector(move || id.into())
        .flex_1()
        .min_h(px(130.))
        .min_w_0()
        .p_4()
        .rounded_lg()
        .border_1()
        .border_color(rgb(BORDER))
        .bg(rgb(CARD))
        .flex()
        .flex_col()
        .gap_3()
        .child(div().text_sm().font_semibold().child(title))
        .child(
            div()
                .id(id)
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_start()
                        .gap_2()
                        .children(entries),
                ),
        )
        .when(truncated, |el| {
            el.child(
                div()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child("Partial list · additional pending items were not captured."),
            )
        })
}

impl ThreadsApp {
    pub(super) fn workq_detail(
        &self,
        key: &WorkQueueKey,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let queue = key.in_frame(&self.frames[self.frame]);
        let pending = queue.and_then(|queue| queue.pending.as_deref());
        let truncated = queue.is_some_and(|queue| queue.pending_truncated);
        let waiters = queue.and_then(|queue| queue.object.waiters.as_deref());
        let (series, maximum) = backlog_history(&self.frames, self.frame, key);
        let series = series.with_playhead(self.playback_time(cx));
        let caption = series.caption.clone();
        div().id("workq-detail").debug_selector(|| "workq-detail".into())
            .flex().flex_col().flex_grow(1.).flex_shrink_0().gap_4()
            .child(div().flex().items_center().justify_between().flex_wrap().gap_3()
                .child(div().text_lg().font_semibold().child("Work queue details"))
                .child(div().debug_selector(|| "back-to-objects".into()).child(
                    Button::new("back-to-objects").label("Back to Kernel objects")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.show_kernel_objects(this.object_kind, window, cx);
                        })))))
            .child(div().debug_selector(|| "workq-summary".into()).flex().flex_wrap()
                .gap_4().p_4().rounded_lg().border_1().border_color(rgb(BORDER)).bg(rgb(CARD))
                .child(field("Name", key.name.clone()))
                .child(field("Address", format!("0x{:08x}", key.address)))
                .child(field("Worker thread", queue.and_then(|q| q.thread_name.clone()).unwrap_or_else(|| "—".into())))
                .child(field("Flags", flags_label(queue.and_then(|q| q.flags))))
                .child(field("Pending items", pending.map(|p| format!("{}{}", p.len(), if truncated { "+" } else { "" })).unwrap_or_else(|| "—".into())))
                .child(field("Drain waiters", waiters.map(|w| w.len().to_string()).unwrap_or_else(|| "—".into()))))
            .child(div().flex().flex_grow(1.).flex_shrink_0().flex_wrap().gap_4()
                .child(div().debug_selector(|| "workq-history".into()).flex_1().min_w(px(320.)).min_h(px(280.))
                    .p_4().rounded_lg().border_1().border_color(rgb(BORDER)).bg(rgb(CARD)).flex().flex_col().gap_3()
                    .child(div().text_sm().font_semibold().child("Pending backlog"))
                    .child(div().text_xs().text_color(rgb(MUTED)).child(format!("0–{maximum:.0} pending items · observed scale, not capacity")))
                    .child(div().flex_1().min_h(px(180.)).child(
                        TimeSeriesChart::count("workq-backlog", series, rgb(PURPLE).into(), maximum)))
                    .child(div().text_xs().text_color(rgb(MUTED)).child(caption)))
                .child(div().debug_selector(|| "workq-side-panels".into()).w(px(280.)).min_w(px(240.)).min_h(px(280.)).flex().flex_col().gap_4()
                    .child(queue_panel("workq-pending", "Pending items", pending, truncated))
                    .child(queue_panel("workq-drain-waiters", "Drain waiters", waiters, false))))
            .child(div().text_xs().text_color(rgb(MUTED)).child(
                "Sample-and-hold backlog, not execution events. The running handler has left the pending list. Missing and truncated readings break the exact-count history. Declaration location is not available in the dump."))
    }
}

#[cfg(test)]
mod tests {
    use super::{WorkQueueKey, backlog_history, flags_label, model};

    #[test]
    fn real_benchmark_capture_retains_work_items_and_all_object_groups() {
        let frames =
            model::parse(include_str!("../test-data/zview-bench-60s/session.json")).unwrap();
        assert_eq!(frames.len(), 577);
        let queue = frames[0]
            .workqs
            .iter()
            .find(|q| q.object.name == "bench_workq")
            .unwrap();
        let key = WorkQueueKey {
            address: queue.object.address,
            name: queue.object.name.clone(),
        };
        assert_eq!(queue.thread_name.as_deref(), Some("bench_workq"));
        let (series, maximum) = backlog_history(&frames, frames.len() - 1, &key);
        assert_eq!(maximum, 2.);
        for count in [0., 1., 2.] {
            assert!(series.points.iter().any(|point| point.value == Some(count)));
        }
        assert!(frames.iter().any(|frame| {
            key.in_frame(frame)
                .unwrap()
                .pending
                .as_ref()
                .unwrap()
                .iter()
                .any(|item| item.contains("work_handler"))
        }));
        assert!(frames.iter().all(|f| !f.heaps.is_empty()
            && !f.semaphores.is_empty()
            && !f.mutexes.is_empty()
            && !f.msgqs.is_empty()
            && !f.mem_slabs.is_empty()));
    }

    #[test]
    fn history_preserves_identity_unknowns_truncation_and_past_scale() {
        let frames = model::parse(include_str!("../test-data/workqs.json")).unwrap();
        let key = WorkQueueKey {
            address: 28672,
            name: "system_workq".into(),
        };
        let (series, maximum) = backlog_history(&frames, 3, &key);
        assert_eq!(maximum, 2.);
        assert_eq!(
            series.points.iter().map(|p| p.value).collect::<Vec<_>>(),
            vec![Some(2.), Some(0.), None, None]
        );
        assert!(key.in_frame(&frames[4]).is_none());
        assert_eq!(
            key.in_frame(&frames[0]).unwrap().thread_name.as_deref(),
            Some("sysworkq")
        );
        assert_eq!(backlog_history(&frames, 4, &key).0.points[4].value, None);
        assert!(flags_label(Some(15)).contains("Plugged"));
        assert_eq!(flags_label(None), "—");
    }
}

use crate::{
    BORDER, CARD, MUTED, PURPLE, ThreadsApp,
    kernel_model::WorkQueueKey,
    model::{self, ChartSeries, Frame},
    time_series_chart::TimeSeriesChart,
    workq_activity::{self, ACTIVE, Activity, BACKLOG, Pressure},
};
use gpui_kit::component::{
    Icon, Sizable, StyledExt, button::Button, progress::Progress, scroll::ScrollableElement,
    tag::Tag,
};
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
        .gap_1()
        .child(div().text_xs().text_color(rgb(MUTED)).child(title))
        .child(crate::layout::note(title, value).text_sm())
}

fn weighted_panel(weight: f32, panel: impl IntoElement) -> impl IntoElement {
    // Weight unpadded containers so card borders/padding cannot skew the ratio.
    div()
        .flex()
        .flex_col()
        .flex_grow(weight)
        .flex_basis(px(0.))
        .min_w_0()
        .min_h_0()
        .child(panel)
}

fn queue_panel(
    id: &'static str,
    title: &'static str,
    names: Option<&[String]>,
    truncated: bool,
    processing: bool,
    pressure: Pressure,
) -> impl IntoElement {
    let is_work = id == "workq-pending";
    // Inline SVG keeps these icons available without expanding the app's asset bundle.
    let icon: &[u8] =
        br#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="3" y="7" width="18" height="14" rx="2"/><path d="M8 7V3h8v4M3 12h18M10 12v3h4v-3"/></svg>"#
    ;
    let (background, foreground, border) = Pressure::Backlog.palette();
    let mut entries = if !is_work {
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
                        .w_full()
                        .min_w_0()
                        .max_w_full()
                        .child(
                            Tag::custom(
                                rgb(background).into(),
                                rgb(foreground).into(),
                                rgb(border).into(),
                            )
                            .min_w_0()
                            .max_w_full()
                            .rounded_full()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .flex_shrink_0()
                                            .child(Icon::default().data(icon).size(px(14.))),
                                    )
                                    .child(
                                        crate::layout::note(
                                            format!("workq-item-label-{index}"),
                                            name.clone(),
                                        )
                                        .flex_1()
                                        .text_sm(),
                                    ),
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
    if is_work {
        let pending_label = names
            .map(|n| format!("Pending · {}{}", n.len(), if truncated { "+" } else { "" }))
            .unwrap_or_else(|| "Pending · —".into());
        entries.insert(
            0,
            div()
                .text_xs()
                .when(names.is_some_and(|n| !n.is_empty()), |el| {
                    el.text_color(rgb(BACKLOG))
                })
                .child(pending_label)
                .into_any_element(),
        );
        if processing {
            let (background, foreground, border) = pressure.palette();
            entries.insert(
                0,
                div()
                    .debug_selector(|| "workq-processing-item".into())
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
                                .child(Icon::default().data(icon).size(px(14.)))
                                .child(
                                    crate::layout::note("workq-processing-label", "Work item")
                                        .text_sm(),
                                ),
                        ),
                    )
                    .into_any_element(),
            );
            entries.insert(
                0,
                div()
                    .text_xs()
                    .text_color(rgb(foreground))
                    .child("Processing · 1")
                    .into_any_element(),
            );
        }
    }
    div()
        .debug_selector(move || id.into())
        .flex_1()
        .min_h_0()
        .min_w_0()
        .p_3()
        .rounded_lg()
        .border_1()
        .border_color(rgb(BORDER))
        .bg(rgb(CARD))
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(div().text_sm().font_semibold().child(title))
                .when(!is_work, |el| {
                    el.child(
                        div().text_xs().text_color(rgb(MUTED)).child(
                            names
                                .map(|n| n.len().to_string())
                                .unwrap_or_else(|| "—".into()),
                        ),
                    )
                }),
        )
        .child(if is_work {
            div()
                .id(id)
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .id(id)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_start()
                        .gap_2()
                        .children(entries),
                )
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .children(entries)
                .into_any_element()
        })
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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let queue = key.in_frame(&self.frames[self.frame]);
        let pending = queue.and_then(|queue| queue.pending.as_deref());
        let truncated = queue.is_some_and(|queue| queue.pending_truncated);
        let waiters = queue.and_then(|queue| queue.object.waiters.as_deref());
        let state = Activity::from_queue(queue);
        let pressure = Pressure::from_queue(queue);
        let busy = workq_activity::observed_busy(&self.frames, self.frame, key);
        let busy_percent = busy.percent();
        let (series, maximum) = backlog_history(&self.frames, self.frame, key);
        let series = series.with_playhead(self.playback_time(cx));
        div().id("workq-detail").debug_selector(|| "workq-detail".into())
            .flex().flex_col().flex_1().min_h_0().gap_2()
            .child(div().flex().flex_shrink_0().items_center().justify_between().flex_wrap().gap_3()
                .child(div().text_lg().font_semibold().child("Work queue details"))
                .child(div().debug_selector(|| "back-to-objects".into()).child(
                    Button::new("back-to-objects").label("Back to Kernel objects")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.show_kernel_objects(this.object_kind, window, cx);
                        })))))
            .child(div().debug_selector(|| "workq-summary".into()).flex().flex_wrap().flex_shrink_0()
                .gap_3().p_3().rounded_lg().border_1().border_color(rgb(BORDER)).bg(rgb(CARD))
                .child(field("Name", key.name.clone()))
                .child(field("Address", format!("0x{:08x}", key.address)))
                .child(field("Worker thread", queue.and_then(|q| q.thread_name.clone()).unwrap_or_else(|| "—".into())))
                .child(field("Flags", flags_label(queue.and_then(|q| q.flags))))
                .child(div().w_full().min_w_0().flex().gap_3().border_t_1().border_color(rgb(BORDER)).pt_2()
                    .child(div().debug_selector(||"workq-current-state".into()).flex_1().min_w_0().flex().flex_col().gap_1()
                        .child(div().text_xs().text_color(rgb(MUTED)).child("Current state"))
                        .child(div().debug_selector(move||format!("workq-pressure-{}",pressure.key()))
                            .when_some(pressure.color(),|el,c|el.text_color(rgb(c)))
                            .child(div().debug_selector(move||format!("workq-state-{}",state.label())).text_lg().font_semibold().child(state.label())))
                        .child(crate::layout::note("workq-state-note",format!("{} · not CPU usage",pressure.label()))))
                    .child(div().debug_selector(||"workq-observed-busy".into()).flex_1().min_w_0().flex().flex_col().gap_1()
                        .child(div().text_xs().text_color(rgb(MUTED)).child("Observed busy · last 10s"))
                        .child(div().flex().items_center().gap_2()
                            .child(div().debug_selector(||"workq-busy-value".into()).text_lg().font_semibold()
                                .when(busy_percent.is_some_and(|p|p>0.),|el|el.text_color(rgb(ACTIVE)))
                                .child(model::percent(busy_percent)))
                            .when_some(busy_percent,|el,p|el.child(div().flex_1().child(Progress::new("workq-busy-meter").small().value(p as f32).color(rgb(ACTIVE))))))
                        .child(crate::layout::note("workq-busy-coverage",busy.description())))
                    .child(div().flex_1().min_w_0().flex().flex_col().gap_1()
                        .child(div().text_xs().text_color(rgb(MUTED)).child("Pending"))
                        .child(div().text_lg().font_semibold()
                            .when(pending.is_some_and(|p|!p.is_empty()),|el|el.text_color(rgb(BACKLOG)))
                            .child(pending.map(|p|format!("{}{}",p.len(),if truncated {"+"}else{""})).unwrap_or_else(||"—".into())))
                        .child(crate::layout::note("workq-pending-note","Excludes the executing work item")))))
            .child(workq_activity::view(&self.workq_activity_states,key,workq_activity::History::new(&self.frames,self.frame,key),window,cx))
            .child(div().debug_selector(||"workq-lower-panels".into()).flex().flex_1().min_h_0().gap_3()
                .child(weighted_panel(3.,div().debug_selector(|| "workq-history".into()).flex_1().min_w_0().min_h_0()
                    .p_3().rounded_lg().border_1().border_color(rgb(BORDER)).bg(rgb(CARD)).flex().flex_col().gap_2()
                    .child(div().text_sm().font_semibold().child("Pending backlog"))
                    .child(crate::layout::note("workq-backlog-scale",format!("0–{maximum:.0} items · grey bands: Unknown count")))
                    .child(div().flex_1().min_h_0().child(
                        TimeSeriesChart::count("workq-backlog", series, rgb(PURPLE).into(), maximum)
                            .without_dots().with_shaded_unknown_counts()))))
                .child(weighted_panel(1.,queue_panel("workq-pending", "Items queue", pending, truncated,state==Activity::Processing,pressure)))
                .child(weighted_panel(1.,queue_panel("workq-drain-waiters", "Drain waiters", waiters, false,false,pressure))))
            .child(crate::layout::note("workq-note",
                "Sampled state, not execution events or CPU utilization. Short jobs between polls can be missed. Busy estimates exclude unknown/not-started intervals. Cancelled and delayed item states are not captured."))
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

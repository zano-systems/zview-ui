use crate::{
    BORDER, CARD, GREEN, MUTED, ThreadsApp,
    kernel_model::SemaphoreKey,
    model::{self, ChartSeries, Frame},
    step_area_chart::StepAreaChart,
};
use gpui_kit::component::{StyledExt, button::Button, progress::Progress};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

fn count_history(frames: &[Frame], current: usize, key: &SemaphoreKey) -> ChartSeries {
    model::chart_series(frames, current, |frame| {
        key.in_frame(frame)?.count.map(|count| count as f64)
    })
}

impl ThreadsApp {
    pub(super) fn semaphore_detail(
        &self,
        key: &SemaphoreKey,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let sem = key.in_frame(&self.frames[self.frame]);
        let waiters = sem.and_then(|sem| sem.object.waiters.as_deref());
        let color = if waiters.is_some_and(|names| !names.is_empty()) {
            0xddb66b
        } else {
            GREEN
        };
        let series =
            count_history(&self.frames, self.frame, key).with_playhead(self.playback_time(cx));
        let waiting_history = model::chart_series(&self.frames, self.frame, |frame| {
            Some(key.in_frame(frame)?.object.waiters.as_ref()?.len() as f64)
        })
        .with_playhead(self.playback_time(cx));
        let limits = self.frames[..=self.frame]
            .iter()
            .map(|frame| key.in_frame(frame).and_then(|sem| sem.limit))
            .collect();
        let caption = series.caption.clone();
        let scale = match sem.and_then(|sem| sem.limit) {
            Some(0xffff_ffff) => "Unlimited · observed-count scale · no limit line".into(),
            Some(0) => "Not initialized · observed-count scale · no limit line".into(),
            Some(limit) => format!("Token limit: {limit} · dashed line"),
            None => "Limit unavailable · observed-count scale · no limit line".into(),
        };
        div()
            .id("semaphore-detail")
            .debug_selector(|| "semaphore-detail".into())
            .flex().flex_col().flex_grow(1.).flex_shrink_0().gap_4()
            .child(div().flex().items_center().justify_between().flex_wrap().gap_3()
                .child(div().text_lg().font_semibold().child("Semaphore details"))
                .child(div().debug_selector(|| "back-to-objects".into()).child(
                    Button::new("back-to-objects").label("Back to Kernel objects")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.show_kernel_objects(this.object_kind, window, cx);
                        })),
                )))
            .child(div().debug_selector(|| "semaphore-summary".into()).flex().flex_wrap()
                .flex_shrink_0().gap_4().p_4().rounded_lg().border_1().border_color(rgb(BORDER)).bg(rgb(CARD))
                .child(field("Name", key.name.clone()))
                .child(field("Address", format!("0x{:08x}", key.address)))
                .child(field("Count", sem.and_then(|sem| sem.count).map(|count| count.to_string()).unwrap_or_else(|| "—".into())))
                .child(field("Limit", sem.map(|sem| sem.limit_label()).unwrap_or_else(|| "—".into())))
                .child(field("Waiting threads", waiters.map(|names| names.len().to_string()).unwrap_or_else(|| "—".into())))
                .when_some(sem.and_then(|sem| sem.fill()), |el, fill| {
                    el.child(div().w_full().child(
                        Progress::new("semaphore-count-capacity")
                            .value(fill.clamp(0., 100.) as f32).color(rgb(color))
                            .accessibility_label("Available semaphore count relative to limit").h(px(6.)),
                    ))
                }))
            .child(div().flex().flex_wrap().items_stretch().flex_grow(1.).flex_shrink_0().gap_4()
                .child(div().debug_selector(|| "semaphore-history".into())
                    .flex_1().min_w(px(400.)).min_h(px(280.)).p_4().rounded_lg()
                    .border_1().border_color(rgb(BORDER)).bg(rgb(CARD))
                    .flex().flex_col().gap_3()
                    .child(div().text_sm().font_semibold().child("Available tokens & wait queue"))
                    .child(div().text_xs().text_color(rgb(MUTED)).child(scale))
                    .child(div().flex().flex_wrap().gap_4().text_xs()
                        .child(div().text_color(rgb(MUTED)).child("Above zero · Available tokens"))
                        .child(div().text_color(rgb(0xf18b94)).child("Below zero · Waiting threads"))
                        .child(div().text_color(rgb(GREEN)).child("Green · Full"))
                        .child(div().text_color(rgb(0xddb66b)).child("Yellow · Tokens available"))
                        .child(div().text_color(rgb(0xf18b94)).child("Red · Empty")))
                    .child(div().debug_selector(|| "semaphore-count-plot".into()).flex_1().min_h(px(300.))
                        .child(StepAreaChart::new(series, waiting_history, limits)))
                    .child(div().text_xs().text_color(rgb(MUTED)).child(caption)))
                .child(div().debug_selector(|| "semaphore-wait-queue".into())
                    .w(px(280.)).min_w(px(240.)).flex_shrink_0().p_4().rounded_lg()
                    .border_1().border_color(rgb(BORDER)).bg(rgb(CARD))
                    .flex().flex_col().gap_3()
                    .child(div().text_sm().font_semibold().child("Wait queue"))
                    .child(crate::wait_queue::threads("semaphore", waiters, crate::wait_queue::Layout::Vertical))))
            .child(div().text_xs().text_color(rgb(MUTED)).child(
                "Sample-and-hold: each value is held until the next captured timestamp, not the exact operation time. Gaps start at missing readings. Negative positions encode waiter counts, not negative tokens. No persistent dots or future readings are drawn.",
            ))
    }
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

#[cfg(test)]
mod tests {
    use super::{SemaphoreKey, count_history, model};

    #[test]
    fn count_history_uses_identity_gaps_and_no_future_readings() {
        let mut frames =
            model::parse(include_str!("../test-data/kernel-objects-minimal.json")).unwrap();
        let key = SemaphoreKey {
            address: 8192,
            name: "gate".into(),
        };
        frames[0].semaphores[0].limit = Some(0xffff_ffff);
        frames[1].semaphores[0].count = Some(100);
        let series = count_history(&frames, 0, &key);
        assert_eq!(series.points.len(), 1);
        assert_eq!(series.points[0].value, Some(0.));
        frames[1].semaphores[0].limit = Some(0xffff_ffff);
        assert_eq!(count_history(&frames, 1, &key).points[1].value, Some(100.));
        frames[1].semaphores[0].object.name = "replacement".into();
        assert!(key.in_frame(&frames[1]).is_none());
        assert_eq!(count_history(&frames, 1, &key).points[1].value, None);
        frames[1].semaphores[0].object.name = "gate".into();
        frames[1].semaphores[0].count = None;
        assert_eq!(count_history(&frames, 1, &key).points[1].value, None);
        frames[1].semaphores[0].limit = Some(3);
        assert_eq!(count_history(&frames, 1, &key).points[1].value, None);
    }

    #[test]
    fn limit_states_and_duplicate_names_remain_distinct() {
        let mut frames =
            model::parse(include_str!("../test-data/kernel-objects-minimal.json")).unwrap();
        let original = frames[0].semaphores[0].clone();
        let mut other = original.clone();
        other.object.address += 1;
        other.count = Some(3);
        frames[0].semaphores.insert(0, other);
        let key = SemaphoreKey {
            address: original.object.address,
            name: original.object.name.clone(),
        };
        assert_eq!(key.in_frame(&frames[0]).unwrap().count, Some(0));
        let sem = &mut frames[0].semaphores[1];
        assert_eq!(sem.fill(), Some(0.));
        assert_eq!(sem.limit_label(), "3");
        sem.limit = Some(0xffff_ffff);
        assert_eq!(sem.limit_label(), "Unlimited");
        assert_eq!(sem.fill(), None);
        sem.limit = Some(0);
        assert_eq!(sem.limit_label(), "Not initialized");
        assert_eq!(sem.fill(), None);
        sem.limit = None;
        assert_eq!(sem.limit_label(), "—");
        assert_eq!(sem.fill(), None);
        sem.limit = Some(3);
        sem.count = None;
        assert_eq!(sem.fill(), None);
    }
}

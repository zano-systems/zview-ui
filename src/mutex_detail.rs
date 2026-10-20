use crate::{
    BORDER, CARD, GREEN, MUTED, ThreadsApp,
    kernel_model::{MutexKey, MutexState},
    model::{self, ChartSeries, Frame},
    step_area_chart::StepAreaChart,
};
use gpui_kit::component::{Icon, IconName, StyledExt, button::Button};
use gpui_kit::*;

fn state_label(state: MutexState) -> &'static str {
    match state {
        MutexState::Free => "Free",
        MutexState::Locked => "Busy · Locked",
        MutexState::Contended => "Contention",
        MutexState::Unknown => "Unknown",
    }
}

fn state_color(state: MutexState) -> u32 {
    match state {
        MutexState::Free => GREEN,
        MutexState::Locked => 0xddb66b,
        MutexState::Contended => 0xf18b94,
        MutexState::Unknown => MUTED,
    }
}

fn state_icon(state: MutexState) -> Icon {
    Icon::new(match state {
        MutexState::Free => IconName::CircleCheck,
        MutexState::Locked => IconName::Pause,
        MutexState::Contended => IconName::TriangleAlert,
        MutexState::Unknown => IconName::Info,
    })
    .size(px(16.))
    .text_color(rgb(state_color(state)))
}

// State colors describe individual readings, not interpolated lock events.
fn contention_history(frames: &[Frame], current: usize, key: &MutexKey) -> Vec<MutexState> {
    frames[..=current]
        .iter()
        .map(|frame| {
            key.in_frame(frame)
                .map_or(MutexState::Unknown, |m| m.state())
        })
        .collect()
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

fn depth_history(frames: &[Frame], current: usize, key: &MutexKey) -> ChartSeries {
    model::chart_series(frames, current, |frame| {
        key.in_frame(frame)?.lock_count.map(|n| n as f64)
    })
}

fn chart_card(chart: StepAreaChart, caption: String) -> impl IntoElement {
    div()
        .debug_selector(|| "mutex-depth-chart".into())
        .flex()
        .flex_col()
        .gap_3()
        .min_h(px(390.))
        .p_4()
        .rounded_lg()
        .border_1()
        .border_color(rgb(BORDER))
        .bg(rgb(CARD))
        .child(
            div()
                .text_sm()
                .font_semibold()
                .child("Lock depth & contention"),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap_4()
                .text_xs()
                .child(
                    div()
                        .text_color(rgb(crate::PURPLE))
                        .child("Above zero · Recursive lock depth"),
                )
                .child(
                    div()
                        .text_color(rgb(0xf18b94))
                        .child("Below zero · Waiting threads"),
                )
                .child(div().text_color(rgb(GREEN)).child("Zero line · Free"))
                .child(div().text_color(rgb(0xf18b94)).child("Zero line · Held"))
                .child(
                    div()
                        .text_color(rgb(MUTED))
                        .child("Neutral · Ownership unknown"),
                ),
        )
        .child(
            div()
                .text_xs()
                .text_color(rgb(MUTED))
                .child("Dashed vertical markers indicate an owner change · no count limit"),
        )
        .child(
            div()
                .debug_selector(|| "mutex-area-plot".into())
                .h(px(300.))
                .child(chart),
        )
        .child(div().text_xs().text_color(rgb(MUTED)).child(caption))
}

impl ThreadsApp {
    pub(super) fn mutex_detail(&self, key: &MutexKey, cx: &mut Context<Self>) -> impl IntoElement {
        let mutex = key.in_frame(&self.frames[self.frame]);
        let waiters = mutex.and_then(|m| m.object.waiters.as_deref());
        let state = mutex.map_or(MutexState::Unknown, |m| m.state());
        let status = if mutex.is_some_and(|m| m.owner_address.is_some_and(|a| a != 0))
            && waiters.is_none()
        {
            "Locked · Contention unknown"
        } else {
            state_label(state)
        };
        let history = contention_history(&self.frames, self.frame, key);
        let depth = depth_history(&self.frames, self.frame, key);
        let caption = depth.caption.clone();
        let waiting = model::chart_series(&self.frames, self.frame, |frame| {
            Some(key.in_frame(frame)?.object.waiters.as_ref()?.len() as f64)
        });
        let owners = self.frames[..=self.frame]
            .iter()
            .map(|frame| key.in_frame(frame).and_then(|m| m.owner_address))
            .collect();
        let details = history
            .into_iter()
            .enumerate()
            .map(|(i, state)| {
                let mutex = key.in_frame(&self.frames[i]);
                let owner = mutex.map(|m| m.owner_label()).unwrap_or_else(|| "—".into());
                format!("{} · Owner: {owner}", state_label(state))
            })
            .collect();
        let chart = StepAreaChart::mutex(
            depth.with_playhead(self.playback_time(cx)),
            waiting.with_playhead(self.playback_time(cx)),
            owners,
            details,
        );
        div().id("mutex-detail").debug_selector(|| "mutex-detail".into())
            .flex().flex_col().flex_grow(1.).flex_shrink_0().gap_4()
            .child(div().flex().items_center().justify_between().flex_wrap().gap_3()
                .child(div().text_lg().font_semibold().child("Mutex details"))
                .child(div().debug_selector(|| "back-to-objects".into()).child(
                    Button::new("back-to-objects").label("Back to Kernel objects")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.show_kernel_objects(this.object_kind, window, cx);
                        })))))
            .child(div().debug_selector(|| "mutex-summary".into()).flex().flex_wrap().flex_shrink_0()
                .gap_4().p_4().rounded_lg().border_1().border_color(rgb(BORDER)).bg(rgb(CARD))
                .child(field("Name", key.name.clone()))
                .child(field("Address", format!("0x{:08x}", key.address)))
                .child(div().debug_selector(|| "mutex-state".into()).min_w(px(160.)).flex_1().flex().flex_col().gap_2()
                    .child(div().text_xs().text_color(rgb(MUTED)).child("State"))
                    .child(div().flex().items_center().gap_2()
                        .child(state_icon(state)).child(div().text_sm().child(status))))
                .child(field("Lock depth", mutex.and_then(|m| m.lock_count)
                    .map(|n| n.to_string()).unwrap_or_else(|| "—".into())))
                .child(field("Owner", mutex.map(|m| m.owner_label()).unwrap_or_else(|| "—".into())))
                .child(field("Waiting threads", waiters.map(|w| w.len().to_string()).unwrap_or_else(|| "—".into()))))
            .child(div().flex().flex_wrap().items_stretch().gap_4()
            .child(div().debug_selector(|| "mutex-history".into()).flex_1().min_w(px(400.)).flex().flex_col().gap_3()
                .p_4().rounded_lg().border_1().border_color(rgb(BORDER)).bg(rgb(CARD))
                .child(div().text_sm().font_semibold().child("Mutex history"))
                .child(div().text_xs().text_color(rgb(MUTED)).child(
                    "Two independent sampled counts. Hover for source time, depth, owner and waiting-thread count."))
                .child(chart_card(chart, caption)))
            .child(div().debug_selector(|| "mutex-wait-queue".into()).w(px(280.)).min_w(px(240.)).flex_shrink_0().flex().flex_col().gap_3()
                .p_4().rounded_lg().border_1().border_color(rgb(BORDER)).bg(rgb(CARD))
                .child(div().text_sm().font_semibold().child("Wait queue"))
                .child(crate::wait_queue::threads("mutex", waiters, crate::wait_queue::Layout::Vertical))))
            .child(div().text_xs().text_color(rgb(MUTED)).child(
                "Sample-and-hold until the next captured timestamp, not the exact lock/unlock time. Negative positions encode waiter counts, not negative depth. Missing readings break each area independently. No future readings are drawn."))
    }
}

#[cfg(test)]
mod tests {
    use super::{MutexKey, MutexState, contention_history, depth_history};
    use crate::model;

    #[test]
    fn history_preserves_identity_unknowns_and_selected_frame() {
        let frames = model::parse(include_str!("../test-data/mutexes.json")).unwrap();
        let key = MutexKey {
            address: 12288,
            name: "resource_lock".into(),
        };
        assert_eq!(
            contention_history(&frames, 6, &key),
            vec![
                MutexState::Free,
                MutexState::Locked,
                MutexState::Contended,
                MutexState::Unknown,
                MutexState::Unknown,
                MutexState::Unknown,
                MutexState::Free,
            ]
        );
        assert_eq!(contention_history(&frames, 0, &key), vec![MutexState::Free]);
        assert_eq!(key.in_frame(&frames[2]).unwrap().lock_count, Some(2));
        assert_eq!(
            key.in_frame(&frames[1]).unwrap().owner_label(),
            "worker · 0x00008000"
        );
        assert_eq!(
            key.in_frame(&frames[2]).unwrap().owner_label(),
            "0x00008000"
        );
        assert_eq!(
            key.in_frame(&frames[0]).unwrap().owner_label(),
            "None · Free"
        );
        assert!(key.in_frame(&frames[5]).is_none());
        let current = model::frame_at(&frames, 10.);
        assert_eq!(frames.len(), 13);
        assert_eq!(frames[current].elapsed_seconds, Some(10.));
        let mutex = key.in_frame(&frames[current]).unwrap();
        assert_eq!(mutex.state(), MutexState::Contended);
        assert_eq!(mutex.owner_name.as_deref(), Some("storage_worker"));
        assert_eq!(mutex.lock_count, Some(3));
        assert_eq!(mutex.object.waiters.as_ref().unwrap().len(), 6);
        assert_eq!(frames[current].mutexes.len(), 5);
        assert_eq!(
            contention_history(&frames, current, &key).len(),
            current + 1
        );
        let depth = depth_history(&frames, current, &key);
        assert_eq!(
            depth.points.iter().map(|p| p.value).collect::<Vec<_>>(),
            vec![
                Some(0.),
                Some(1.),
                Some(2.),
                Some(1.),
                None,
                None,
                Some(0.),
                Some(3.)
            ]
        );
        assert_eq!(depth.points[1].x, 1.5);
        assert_eq!(depth.points.last().unwrap().x, 10.);
        assert_eq!(depth.x_domain, (0., 20.));
        assert_eq!(depth.segments().count(), 2);
        assert_eq!(depth_history(&frames, 0, &key).points.len(), 1);
    }
}

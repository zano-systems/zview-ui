use crate::{
    BORDER, CARD, GREEN, MUTED, ThreadsApp,
    kernel_model::{MessageQueue, MessageQueueKey},
    model::{self, ChartSeries, Frame},
    time_series_chart::TimeSeriesChart,
};
use gpui_kit::component::{StyledExt, button::Button, progress::Progress};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

fn depth_history(frames: &[Frame], current: usize, key: &MessageQueueKey) -> (ChartSeries, f64) {
    let series = model::chart_series(frames, current, |frame| {
        key.in_frame(frame)?.used_msgs.map(|depth| depth as f64)
    });
    let maximum = key
        .in_frame(&frames[current])
        .and_then(|queue| queue.max_msgs)
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

/// Zephyr shares one wait queue: senders block when full, receivers when empty.
/// Do not infer a direction for an incomplete or inconsistent snapshot.
fn waiter_context(queue: Option<&MessageQueue>) -> &'static str {
    match queue {
        Some(queue) if queue.used_msgs == Some(0) && queue.max_msgs.is_some_and(|n| n > 0) => {
            "Empty queue · queued threads wait to receive"
        }
        Some(queue) if matches!((queue.used_msgs, queue.max_msgs), (Some(u), Some(n)) if n > 0 && u >= n) => {
            "Full queue · queued threads wait to send"
        }
        _ => "Shared send/receive wait queue · direction unavailable",
    }
}

fn count(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
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
    pub(super) fn msgq_detail(
        &self,
        key: &MessageQueueKey,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let queue = key.in_frame(&self.frames[self.frame]);
        let waiters = queue.and_then(|queue| queue.object.waiters.as_deref());
        let full = queue.is_some_and(|queue| {
            matches!((queue.used_msgs, queue.max_msgs), (Some(u), Some(n)) if n > 0 && u >= n)
        });
        let color = if full {
            0xf18b94
        } else if waiters.is_some_and(|names| !names.is_empty()) {
            0xddb66b
        } else {
            GREEN
        };
        let (series, maximum) = depth_history(&self.frames, self.frame, key);
        let series = series.with_playhead(self.playback_time(cx));
        let caption = series.caption.clone();
        let scale = match queue.and_then(|queue| queue.max_msgs) {
            Some(0) => format!("Zero capacity · scale 0–{maximum:.0} observed"),
            Some(capacity) => format!("0–{capacity} messages"),
            None => format!("Capacity unavailable · scale 0–{maximum:.0} observed"),
        };
        let fill = queue.and_then(|queue| model::percentage(queue.used_msgs?, queue.max_msgs?));
        div().id("msgq-detail").debug_selector(|| "msgq-detail".into())
            .flex().flex_col().flex_grow(1.).flex_shrink_0().gap_4()
            .child(div().flex().items_center().justify_between().flex_wrap().gap_3()
                .child(div().text_lg().font_semibold().child("Message queue details"))
                .child(div().debug_selector(|| "back-to-objects".into()).child(
                    Button::new("back-to-objects").label("Back to Kernel objects")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.show_kernel_objects(this.object_kind, window, cx);
                        })))))
            .when(queue.is_none(), |el| el.child(
                div().debug_selector(|| "msgq-absent".into()).p_4().rounded_lg()
                    .border_1().border_color(rgb(BORDER)).child(
                        "This message queue is not present in the selected frame. Move to another frame or return to Kernel objects. Current values are not carried forward.")))
            .child(div().debug_selector(|| "msgq-summary".into()).flex().flex_wrap().flex_shrink_0()
                .gap_4().p_4().rounded_lg().border_1().border_color(rgb(BORDER)).bg(rgb(CARD))
                .child(field("Name", key.name.clone()))
                .child(field("Address", format!("0x{:08x}", key.address)))
                .child(field("Depth", count(queue.and_then(|q| q.used_msgs))))
                .child(field("Capacity", count(queue.and_then(|q| q.max_msgs))))
                .child(field("Message size", queue.and_then(|q| q.msg_size).map(|size| format!("{size} B")).unwrap_or_else(|| "—".into())))
                .child(field("Waiting threads", waiters.map(|names| names.len().to_string()).unwrap_or_else(|| "—".into())))
                .when_some(fill, |el, fill| el.child(div().w_full().child(
                    Progress::new("msgq-capacity").value(fill.clamp(0., 100.) as f32).color(rgb(color))
                        .accessibility_label("Queued messages relative to capacity").h(px(6.))))))
            .child(div().flex().flex_grow(1.).flex_shrink_0().flex_wrap().gap_4()
                .child(div().debug_selector(|| "msgq-history".into()).flex_1().min_w(px(320.)).min_h(px(280.))
                    .p_4().rounded_lg().border_1().border_color(rgb(BORDER)).bg(rgb(CARD)).flex().flex_col().gap_3()
                    .child(div().text_sm().font_semibold().child("Queue depth"))
                    .child(div().text_xs().text_color(rgb(MUTED)).child(scale))
                    .child(div().debug_selector(|| "msgq-depth-plot".into()).flex_1().min_h(px(180.))
                        .child(TimeSeriesChart::count("msgq-depth", series, rgb(color).into(), maximum)))
                    .child(div().text_xs().text_color(rgb(MUTED)).child(caption)))
                .child(div().debug_selector(|| "msgq-wait-queue".into()).w(px(280.)).min_w(px(240.))
                    .p_4().rounded_lg().border_1().border_color(rgb(BORDER)).bg(rgb(CARD)).flex().flex_col().gap_3()
                    .child(div().text_sm().font_semibold().child("Waiting threads"))
                    .child(div().text_xs().text_color(rgb(MUTED)).child(waiter_context(queue)))
                    .child(crate::wait_queue::threads("msgq", waiters, crate::wait_queue::Layout::Vertical))))
            .child(div().text_xs().text_color(rgb(MUTED)).child(
                "Recorded queue depth, not individual send/receive events. Operations between snapshots may leave no trace. Missing readings break the history line."))
    }
}

#[cfg(test)]
mod tests {
    use super::{MessageQueueKey, depth_history, model, waiter_context};

    #[test]
    fn queue_history_preserves_identity_missing_values_and_capacity_scale() {
        let mut frames =
            model::parse(include_str!("../test-data/kernel-objects-minimal.json")).unwrap();
        let key = MessageQueueKey {
            address: 16384,
            name: "messages".into(),
        };
        let (series, maximum) = depth_history(&frames, 0, &key);
        assert_eq!(series.points.len(), 1);
        assert_eq!(series.points[0].value, Some(4.));
        assert_eq!(maximum, 4.);
        assert_eq!(depth_history(&frames, 1, &key).0.points[1].value, None);
        let mut duplicate = frames[0].msgqs[0].clone();
        duplicate.object.address += 1;
        duplicate.used_msgs = Some(99);
        frames[0].msgqs.insert(0, duplicate);
        assert_eq!(key.in_frame(&frames[0]).unwrap().used_msgs, Some(4));
        frames[1].msgqs = vec![frames[0].msgqs[1].clone()];
        frames[1].msgqs[0].used_msgs = Some(100);
        frames[0].msgqs[1].max_msgs = None;
        assert_eq!(depth_history(&frames, 0, &key).1, 4.); // No future autoscaling.
        frames[1].msgqs[0].object.name = "replacement".into();
        assert!(key.in_frame(&frames[1]).is_none());
        frames[0].msgqs[1].used_msgs = None;
        assert_eq!(depth_history(&frames, 0, &key).0.points[0].value, None);
        assert_eq!(depth_history(&frames, 0, &key).1, 1.);
    }

    #[test]
    fn shared_wait_queue_direction_requires_known_boundaries() {
        let mut frames =
            model::parse(include_str!("../test-data/kernel-objects-minimal.json")).unwrap();
        let queue = &mut frames[0].msgqs[0];
        assert!(waiter_context(Some(queue)).contains("send"));
        queue.used_msgs = Some(0);
        assert!(waiter_context(Some(queue)).contains("receive"));
        queue.used_msgs = Some(2);
        assert!(waiter_context(Some(queue)).contains("unavailable"));
        queue.max_msgs = None;
        assert!(waiter_context(Some(queue)).contains("unavailable"));
        queue.max_msgs = Some(0);
        queue.used_msgs = Some(0);
        assert!(waiter_context(Some(queue)).contains("unavailable"));
        assert!(waiter_context(None).contains("unavailable"));
    }
}

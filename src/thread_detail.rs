use crate::{BLUE, BORDER, CARD, GREEN, MUTED, PURPLE, ThreadsApp, bytes, model, percent};
use gpui_kit::component::{Sizable, StyledExt, button::Button, progress::Progress};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use model::ThreadKey;

/// Match the TUI's information hierarchy while retaining the desktop shell.
impl ThreadsApp {
    pub(super) fn thread_detail(
        &self,
        key: &ThreadKey,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let thread = key.in_frame(&self.frames[self.frame]);
        let runtime = thread.and_then(|t| t.runtime.as_ref());
        let watermark = runtime.and_then(|r| r.stack_watermark_percent);
        let stack_color = if watermark.is_some_and(|v| v >= 70.) {
            0xddb66b
        } else {
            GREEN
        };
        div()
            .id("thread-detail")
            .debug_selector(|| "thread-detail".into())
            .flex().flex_col().flex_grow(1.).flex_shrink_0().gap_4()
            .child(
                div().flex().flex_shrink_0().items_center().justify_between()
                    .child(div().text_lg().font_semibold().child("Thread details"))
                    .child(div().debug_selector(|| "back-to-threads".into())
                        .child(Button::new("back-to-threads").label("Back to Threads")
                            .on_click(cx.listener(|this, _, window, cx| this.show_overview(window, cx))))),
            )
            .when(thread.is_none(), |el| el.child(
                div().flex_shrink_0().p_4().rounded_lg().border_1().border_color(rgb(BORDER)).child(
                    "This thread is not present in the selected frame. Move to another frame or return to Threads. Values are not carried forward from an earlier frame.",
                ),
            ))
            .child(
                div().debug_selector(|| "thread-summary".into()).flex().flex_wrap().flex_shrink_0()
                    .gap_4().p_4().rounded_lg().border_1().border_color(rgb(BORDER)).bg(rgb(CARD))
                    .child(field("Thread", key.name.clone()).min_w(px(140.)))
                    .child(field("CPU %", percent(runtime.and_then(|r| r.cpu_normalized))))
                    .child(field("Load %", percent(runtime.and_then(|r| r.cpu))))
                    .child(div().min_w(px(260.)).flex_grow(2.).flex().flex_col().gap_2()
                        .child(label("Stack usage % (watermark)"))
                        .child(div().flex().items_center().gap_3()
                            .child(div().flex_1().when_some(watermark, |el, value| el.child(
                                Progress::new("thread-detail-stack").small().value(value as f32)
                                    .color(rgb(stack_color)).accessibility_label("Thread stack watermark"),
                            )))
                            .child(div().text_sm().text_color(rgb(stack_color)).child(percent(watermark)))))
                    .child(field("Watermark bytes", format!("{} / {}",
                        bytes(runtime.and_then(|r| r.stack_watermark)), bytes(thread.and_then(|t| t.stack_size))))),
            )
            .child(
                div().debug_selector(|| "thread-metadata".into()).flex().flex_wrap().flex_shrink_0().gap_3()
                    .child(info_card("Priority", thread.and_then(|t| t.priority).map(|v| v.to_string()).unwrap_or_else(unknown))
                        .min_w(px(120.)).flex_1())
                    .child(info_card("Options", thread.and_then(|t| t.user_options).map(|v| format!("0x{v:02x}")).unwrap_or_else(unknown))
                        .min_w(px(120.)).flex_1())
                    .child(info_card("Entry", thread.map(|t| t.entry_description()).unwrap_or_else(unknown))
                        .debug_selector(|| "thread-entry-source".into()).min_w(px(320.)).flex_grow(3.).flex_basis(px(320.))),
            )
            .child(
                div().flex().flex_grow(1.).flex_shrink_0().gap_4().flex_wrap()
                    .child(Self::chart_card(
                        "thread-cpu", "CPU %",
                        runtime.and_then(|r| r.cpu_normalized),
                        "Thread cycles / total cycles".into(), PURPLE,
                        model::chart_series(&self.frames, self.frame, |frame| {
                            key.in_frame(frame)?.runtime.as_ref()?.cpu_normalized
                        }).with_playhead(self.playback_time(cx)), None,
                    ).h_auto())
                    .child(Self::chart_card(
                        "thread-load", "LOAD %",
                        runtime.and_then(|r| r.cpu),
                        "Thread cycles / non-idle cycles".into(), BLUE,
                        model::chart_series(&self.frames, self.frame, |frame| {
                            key.in_frame(frame)?.runtime.as_ref()?.cpu
                        }).with_playhead(self.playback_time(cx)), None,
                    ).h_auto()),
            )
    }
}

fn label(text: &'static str) -> impl IntoElement {
    div().text_xs().text_color(rgb(MUTED)).child(text)
}

fn field(title: &'static str, value: String) -> impl IntoElement + Styled {
    div()
        .min_w(px(90.))
        .flex_1()
        .flex()
        .flex_col()
        .gap_2()
        .child(label(title))
        .child(div().text_sm().child(value))
}

fn info_card(title: &'static str, value: String) -> impl IntoElement + Styled + InteractiveElement {
    div()
        .debug_selector(move || format!("thread-info-{title}"))
        .p_3()
        .rounded_lg()
        .border_1()
        .border_color(rgb(BORDER))
        .bg(rgb(CARD))
        .flex()
        .flex_col()
        .gap_2()
        .child(label(title))
        .child(div().text_sm().child(value))
}

fn unknown() -> String {
    "—".into()
}

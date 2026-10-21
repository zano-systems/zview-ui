use crate::{
    BG, BORDER, CARD, GREEN, MUTED, OpenMemorySlab, OpenMessageQueue, OpenMutex, OpenSemaphore,
    OpenWorkQueue, Screen, ThreadsApp,
    kernel_model::{self, Kind, Row, State, Summary},
};
use gpui_kit::component::InteractiveElementExt;
use gpui_kit::component::{
    StyledExt,
    button::{Button, ButtonVariants},
    input::Input,
    progress::Progress,
    scroll::ScrollableElement,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

impl ThreadsApp {
    pub(super) fn kernel_objects(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = kernel_model::rows(&self.frames[self.frame]);
        let kind = self.object_kind;
        let query = self.object_search.read(cx).value();
        let visible = rows
            .iter()
            .filter(|row| row.matches(kind, &query))
            .collect::<Vec<_>>();
        div()
            .id("kernel-objects")
            .debug_selector(|| "kernel-objects".into())
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap_3()
            .child(div().flex_shrink_0().text_2xl().font_semibold().child("Kernel objects"))
            .child(crate::layout::note("kernel-description",
                "Statically declared objects · state and wait queues at the selected frame",
            ))
            .child(
                div().debug_selector(|| "kernel-table-toolbar".into())
                    .w_full().min_w_0().flex().flex_shrink_0().items_center().justify_between().flex_wrap().gap_3().text_sm()
                    .child(format!("{} of {} objects", visible.len(), rows.len()))
                    .child(
                        div().flex().items_center().flex_wrap().gap_3()
                            .child(div().debug_selector(|| "object-search".into()).w(px(310.)).child(
                                Input::new(&self.object_search).aria_label("Search kernel objects"),
                            ))
                            .child(div().debug_selector(|| "clear-object-filters".into()).child(
                                Button::new("clear-object-filters").label("Clear filters").ghost()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.object_search.update(cx, |input, cx| input.set_value("", window, cx));
                                        this.object_kind = None;
                                        cx.notify();
                                    })),
                            )),
                    )
                    .child(div().text_color(rgb(MUTED)).child("Sorted by type, name, address")),
            )
            .child(
                div().id("kernel-table").debug_selector(|| "kernel-table".into()).w_full().min_w_0()
                    .relative().flex_1().min_h_0().overflow_x_scroll().lock_scroll_axis()
                    .track_scroll(&self.object_columns_scroll).horizontal_scrollbar(&self.object_columns_scroll)
                    .child(
                        div().w_full().h_full().min_w(px(1040.)).flex().flex_col().flex_shrink_0()
                            .rounded_lg().border_1().border_color(rgb(BORDER)).overflow_hidden()
                            .child(div().debug_selector(|| "kernel-table-header".into()).flex().flex_shrink_0().p_3().bg(rgb(0x2a263e)).text_xs().text_color(rgb(MUTED))
                                .child(div().w(px(120.)).child("TYPE"))
                                .child(div().w(px(190.)).child("NAME"))
                                .child(div().w(px(115.)).child("ADDRESS"))
                                .child(div().flex_1().min_w(px(330.)).child("STATE / USAGE"))
                                .child(div().w(px(260.)).child("WAITING THREADS")))
                            .child(summary_row(&Summary::from_rows(&visible), kind))
                            .child(div().id("object-rows").debug_selector(|| "object-rows".into())
                                .relative().flex().flex_col().flex_1().min_h_0().overflow_y_scroll().lock_scroll_axis()
                                .track_scroll(&self.object_rows_scroll).vertical_scrollbar(&self.object_rows_scroll)
                                .children(visible.iter().enumerate().map(|(index, row)| object_row(row, index, cx)))
                            .when(visible.is_empty(), |el| {
                                el.child(div().debug_selector(|| "kernel-empty".into()).p_6().text_color(rgb(MUTED))
                                    .child(if rows.is_empty() {
                                        "No kernel objects in this snapshot. This build or dump may not include them."
                                    } else {
                                        "No objects match these filters. Clear filters to see all objects."
                                    }))
                            })),
                    ),
            )
            .child(crate::layout::note("kernel-state-legend",
                "Unknown wait queues are labelled unavailable · amber waiting / locked · red full, exhausted, contended, draining or plugged",
            ))
            .child(crate::layout::note("kernel-interaction-legend",
                "Bars represent each type’s state or capacity, not comparable utilization. Click an object, or focus its row with Tab and press Enter to inspect.",
            ))
    }
}

fn summary_row(summary: &Summary, kind: Option<Kind>) -> impl IntoElement {
    div()
        .debug_selector(|| "kernel-summary".into())
        .flex()
        .flex_shrink_0()
        .items_center()
        .p_3()
        .text_sm()
        .bg(rgb(0x302a46))
        .border_b_1()
        .border_color(rgb(BORDER))
        .child(div().w(px(120.)).text_color(rgb(MUTED)).child("SUMMARY"))
        .child(
            div()
                .debug_selector(|| "kernel-summary-label".into())
                .w(px(190.))
                .font_semibold()
                .child(Summary::label(kind)),
        )
        .child(div().w(px(115.)).text_color(rgb(MUTED)).child("—"))
        .child(
            div()
                .flex_1()
                .min_w(px(330.))
                .pr_4()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .debug_selector(|| "kernel-summary-state".into())
                        .child(summary.state_label(kind)),
                )
                .child(
                    Progress::new("object-summary-progress")
                        .value(summary.percent() as f32)
                        .color(rgb(0xddb66b))
                        .accessibility_label(format!(
                            "{}: {} of {} objects with waiters, in use, or under pressure",
                            Summary::label(kind),
                            summary.pressured,
                            summary.total
                        ))
                        .h(px(5.)),
                ),
        )
        .child(
            div()
                .w(px(260.))
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .debug_selector(|| "kernel-summary-waiters".into())
                        .child(summary.waiters_label()),
                )
                .when(summary.unknown_wait_queues > 0, |el| {
                    el.child(
                        div()
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child("Some wait queues unavailable"),
                    )
                }),
        )
}

fn object_row(
    row: &Row<'_>,
    index: usize,
    cx: &mut Context<ThreadsApp>,
) -> impl IntoElement + use<> {
    let color = match row.state {
        State::Normal => GREEN,
        State::Busy => 0xddb66b,
        State::Blocked => 0xf18b94,
    };
    div()
        .id(("kernel-row", index))
        .when(row.kind == Kind::Heap, |el| {
            let key = crate::model::HeapKey {
                address: row.address,
                name: row.name.to_string(),
            };
            let click_key = key.clone();
            el.focusable()
                .tab_stop(true)
                .key_context("HeapRow")
                .cursor_pointer()
                .border_1()
                .border_color(gpui_kit::transparent_black())
                .focus_visible(|style| style.border_color(rgb(crate::PURPLE)))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.screen = Screen::Heap(click_key.clone());
                    this.detail_focus.focus(window, cx);
                    cx.notify();
                }))
                .on_action(cx.listener(move |this, _: &crate::OpenHeap, window, cx| {
                    this.screen = Screen::Heap(key.clone());
                    this.detail_focus.focus(window, cx);
                    cx.notify();
                }))
        })
        .when(row.kind == Kind::WorkQueue, |el| {
            let key = kernel_model::WorkQueueKey {
                address: row.address,
                name: row.name.to_string(),
            };
            let click_key = key.clone();
            el.focusable()
                .tab_stop(true)
                .key_context("WorkQueueRow")
                .cursor_pointer()
                .border_1()
                .border_color(gpui_kit::transparent_black())
                .focus_visible(|style| style.border_color(rgb(crate::PURPLE)))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.screen = Screen::WorkQueue(click_key.clone());
                    this.detail_focus.focus(window, cx);
                    cx.notify();
                }))
                .on_action(cx.listener(move |this, _: &OpenWorkQueue, window, cx| {
                    this.screen = Screen::WorkQueue(key.clone());
                    this.detail_focus.focus(window, cx);
                    cx.notify();
                }))
        })
        .when(row.kind == Kind::Mutex, |el| {
            let key = kernel_model::MutexKey {
                address: row.address,
                name: row.name.to_string(),
            };
            let click_key = key.clone();
            el.focusable()
                .tab_stop(true)
                .key_context("MutexRow")
                .cursor_pointer()
                .border_1()
                .border_color(gpui_kit::transparent_black())
                .focus_visible(|style| style.border_color(rgb(crate::PURPLE)))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.screen = Screen::Mutex(click_key.clone());
                    this.detail_focus.focus(window, cx);
                    cx.notify();
                }))
                .on_action(cx.listener(move |this, _: &OpenMutex, window, cx| {
                    this.screen = Screen::Mutex(key.clone());
                    this.detail_focus.focus(window, cx);
                    cx.notify();
                }))
        })
        .when(row.kind == Kind::MemorySlab, |el| {
            let key = kernel_model::MemorySlabKey {
                address: row.address,
                name: row.name.to_string(),
            };
            let click_key = key.clone();
            el.focusable()
                .tab_stop(true)
                .key_context("MemorySlabRow")
                .cursor_pointer()
                .border_1()
                .border_color(gpui_kit::transparent_black())
                .focus_visible(|style| style.border_color(rgb(crate::PURPLE)))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.screen = Screen::MemorySlab(click_key.clone());
                    this.detail_focus.focus(window, cx);
                    cx.notify();
                }))
                .on_action(cx.listener(move |this, _: &OpenMemorySlab, window, cx| {
                    this.screen = Screen::MemorySlab(key.clone());
                    this.detail_focus.focus(window, cx);
                    cx.notify();
                }))
        })
        .when(row.kind == Kind::MessageQueue, |el| {
            let key = kernel_model::MessageQueueKey {
                address: row.address,
                name: row.name.to_string(),
            };
            let click_key = key.clone();
            el.focusable()
                .tab_stop(true)
                .key_context("MessageQueueRow")
                .cursor_pointer()
                .border_1()
                .border_color(gpui_kit::transparent_black())
                .focus_visible(|style| style.border_color(rgb(crate::PURPLE)))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.screen = Screen::MessageQueue(click_key.clone());
                    this.detail_focus.focus(window, cx);
                    cx.notify();
                }))
                .on_action(cx.listener(move |this, _: &OpenMessageQueue, window, cx| {
                    this.screen = Screen::MessageQueue(key.clone());
                    this.detail_focus.focus(window, cx);
                    cx.notify();
                }))
        })
        .when(row.kind == Kind::Semaphore, |el| {
            let key = kernel_model::SemaphoreKey {
                address: row.address,
                name: row.name.to_string(),
            };
            let click_key = key.clone();
            el.focusable()
                .tab_stop(true)
                .key_context("SemaphoreRow")
                .cursor_pointer()
                .border_1()
                .border_color(gpui_kit::transparent_black())
                .focus_visible(|style| style.border_color(rgb(crate::PURPLE)))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.screen = Screen::Semaphore(click_key.clone());
                    this.detail_focus.focus(window, cx);
                    cx.notify();
                }))
                .on_action(cx.listener(move |this, _: &OpenSemaphore, window, cx| {
                    this.screen = Screen::Semaphore(key.clone());
                    this.detail_focus.focus(window, cx);
                    cx.notify();
                }))
        })
        .debug_selector({
            let selector = format!("kernel-row-{}-{}", row.kind.label(), row.address);
            move || selector.clone()
        })
        .flex()
        .flex_shrink_0()
        .items_center()
        .p_3()
        .text_sm()
        .bg(rgb(if index.is_multiple_of(2) { CARD } else { BG }))
        .child(
            div()
                .w(px(120.))
                .text_color(rgb(MUTED))
                .child(row.kind.label()),
        )
        .child(
            div()
                .w(px(190.))
                .overflow_hidden()
                .child(row.name.to_string()),
        )
        .child(
            div()
                .w(px(115.))
                .text_color(rgb(MUTED))
                .child(format!("0x{:08x}", row.address)),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(330.))
                .pr_4()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_color(rgb(color)).child(row.summary.clone()))
                .when_some(row.fill, |el, fill| {
                    el.child(
                        Progress::new(("object-progress", index))
                            .value(fill.clamp(0., 100.) as f32)
                            .color(rgb(color))
                            .accessibility_label(format!("{} state / usage", row.name))
                            .h(px(5.)),
                    )
                })
                .when_some(row.peak, |el, peak| {
                    el.child(
                        div()
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child(format!("Peak {peak:.1}%")),
                    )
                }),
        )
        .child(
            div()
                .w(px(260.))
                .text_color(rgb(MUTED))
                .child(crate::wait_queue::threads(
                    format!("object-{}-{}", row.kind.label(), row.address),
                    row.waiters,
                    crate::wait_queue::Layout::Wrap,
                )),
        )
}

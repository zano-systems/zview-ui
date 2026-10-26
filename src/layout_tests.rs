use super::{Screen, ThreadsApp, open_kernel_objects, setup};
use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext, point, px, size};

fn scroll(cx: &mut VisualTestContext, position: gpui::Point<gpui::Pixels>, y: f32) {
    cx.simulate_event(gpui::ScrollWheelEvent {
        position,
        delta: gpui::ScrollDelta::Pixels(point(px(0.), px(y))),
        modifiers: Modifiers::default(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

#[gpui_kit::test]
fn thread_rows_scroll_and_keyboard_reveals_selection_without_moving_the_page(
    cx: &mut TestAppContext,
) {
    let (app, mut cx) = setup(cx);
    cx.simulate_resize(size(px(1000.), px(680.)));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.frames = crate::model::parse(include_str!(
                "../test-data/zview-bench-60s/session-with-heap-maps.json"
            ))
            .unwrap();
            app.frame = 0;
            app.sync_heaps(window, cx);
            app.table_focus.focus(window, cx);
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    let charts = cx.debug_bounds("overview-charts").unwrap();
    let header = cx.debug_bounds("thread-table-header").unwrap();
    let footer = cx.debug_bounds("session-footer").unwrap();
    let rows = cx.debug_bounds("thread-rows").unwrap();
    assert!(rows.size.height >= px(80.), "{rows:?}");
    assert!(footer.bottom() <= px(680.));
    scroll(&mut cx, rows.origin + point(px(40.), px(30.)), -600.);
    app.read_with(&cx, |app, _| {
        assert!(app.thread_rows_scroll.offset().y < px(0.))
    });
    assert_eq!(cx.debug_bounds("overview-charts").unwrap(), charts);
    assert_eq!(cx.debug_bounds("thread-table-header").unwrap(), header);
    assert_eq!(cx.debug_bounds("session-footer").unwrap(), footer);
    cx.simulate_keystrokes(&vec!["down"; 30].join(" "));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let (selected, bounds, viewport) = app.read_with(&cx, |app, _| {
        let threads = app.frames[app.frame].visible_threads();
        let index = threads
            .iter()
            .position(|t| Some(t.address) == app.selected)
            .unwrap();
        let mut bounds = app.thread_rows_scroll.bounds_for_item(index).unwrap();
        bounds.origin += app.thread_rows_scroll.offset();
        (app.selected, bounds, app.thread_rows_scroll.bounds())
    });
    assert!(
        bounds.top() >= viewport.top() && bounds.bottom() <= viewport.bottom(),
        "{bounds:?} in {viewport:?}"
    );
    cx.simulate_keystrokes("enter escape");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    app.read_with(&cx, |app, _| {
        assert_eq!(app.screen, Screen::Overview);
        assert_eq!(app.selected, selected);
        assert_eq!(app.frame, 0);
    });
}

#[gpui_kit::test]
fn kernel_rows_scroll_below_pinned_headers_and_summary(cx: &mut TestAppContext) {
    let (app, mut cx) = setup(cx);
    open_kernel_objects(&app, &mut cx);
    cx.simulate_resize(size(px(1000.), px(680.)));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let heap = app.frames[0].heaps[0].clone();
            for index in 0..40 {
                let mut heap = heap.clone();
                heap.address += index + 1;
                app.frames[0].heaps.push(heap);
            }
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    let header = cx.debug_bounds("kernel-table-header").unwrap();
    let summary = cx.debug_bounds("kernel-summary").unwrap();
    let footer = cx.debug_bounds("session-footer").unwrap();
    let rows = cx.debug_bounds("object-rows").unwrap();
    assert!(rows.size.height >= px(80.));
    assert!(rows.bottom() <= footer.top());
    scroll(&mut cx, rows.origin + point(px(40.), px(30.)), -600.);
    app.read_with(&cx, |app, _| {
        assert!(app.object_rows_scroll.offset().y < px(0.))
    });
    assert_eq!(cx.debug_bounds("kernel-table-header").unwrap(), header);
    assert_eq!(cx.debug_bounds("kernel-summary").unwrap(), summary);
    assert_eq!(cx.debug_bounds("session-footer").unwrap(), footer);
}

#[gpui_kit::test]
fn summary_chart_cards_have_compact_aligned_headings_and_no_time_footer(cx: &mut TestAppContext) {
    fn check(
        cx: &mut VisualTestContext,
        cards: &[(
            &'static str,
            &'static str,
            &'static str,
            &'static str,
            &'static str,
        )],
    ) {
        let mut value_top = None;
        for &(card, title, value, plot, caption) in cards {
            let card = cx.debug_bounds(card).unwrap();
            let title = cx.debug_bounds(title).unwrap();
            let value = cx.debug_bounds(value).unwrap();
            let plot = cx.debug_bounds(plot).unwrap();
            let inset = title.top() - card.top();
            assert!(
                inset >= px(10.) && inset <= px(16.),
                "heading should sit near the card padding: {inset:?}"
            );
            if let Some(top) = value_top {
                assert_eq!(value.top(), top, "card values should align");
            }
            value_top = Some(value.top());
            assert!(
                cx.debug_bounds(caption).is_none(),
                "time is already shown on the axis and session controls"
            );
            assert!(
                (card.bottom() - plot.bottom() - px(13.)).abs() <= px(1.),
                "plot should use the removed footer space"
            );
        }
    }
    let (app, mut cx) = setup(cx);
    for (width, height) in [(1480., 940.), (1000., 680.)] {
        cx.simulate_resize(size(px(width), px(height)));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| app.show_overview(window, cx));
            window.draw(cx).clear(cx);
        });
        check(
            &mut cx,
            &[
                ("cpu", "cpu-title", "cpu-value", "cpu-plot", "cpu-caption"),
                (
                    "stack",
                    "stack-title",
                    "stack-value",
                    "stack-plot",
                    "stack-caption",
                ),
                (
                    "heap",
                    "heap-title",
                    "heap-value",
                    "heap-plot",
                    "heap-caption",
                ),
            ],
        );
        let selector = cx.debug_bounds("heap-selector").unwrap();
        let header = cx.debug_bounds("heap-header").unwrap();
        assert!(selector.left() >= header.left() && selector.right() <= header.right());
        assert!(selector.top() >= header.top() && selector.bottom() <= header.bottom());
        assert_eq!(header.size.height, px(28.));
        cx.simulate_keystrokes("down enter");
        cx.update(|window, cx| window.draw(cx).clear(cx));
        check(
            &mut cx,
            &[
                (
                    "thread-cpu",
                    "thread-cpu-title",
                    "thread-cpu-value",
                    "thread-cpu-plot",
                    "thread-cpu-caption",
                ),
                (
                    "thread-load",
                    "thread-load-title",
                    "thread-load-value",
                    "thread-load-plot",
                    "thread-load-caption",
                ),
            ],
        );
    }
}

type ScreenCase = (Screen, &'static [&'static str], &'static str);

#[gpui_kit::test]
fn overview_chart_row_and_table_have_equal_height_at_every_supported_size(cx: &mut TestAppContext) {
    let (app, mut cx) = setup(cx);
    let baseline = app.read_with(&cx, |app, _| {
        app.frames[app.frame].visible_threads()[0].clone()
    });
    for count in [0, 17, 80] {
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.frames[app.frame].threads = (0..count)
                    .map(|index| {
                        let mut thread = baseline.clone();
                        thread.address += index * 1024;
                        thread.name = format!("thread-{index}");
                        thread
                    })
                    .collect();
                cx.notify();
            });
            window.draw(cx).clear(cx);
        });
        for (width, height) in [(1000., 680.), (1480., 940.), (1920., 1080.)] {
            cx.simulate_resize(size(px(width), px(height)));
            for collapsed in [false, true] {
                cx.update(|window, cx| {
                    app.update(cx, |app, cx| {
                        app.collapsed = collapsed;
                        cx.notify();
                    });
                    window.draw(cx).clear(cx);
                });
                let table = cx.debug_bounds("thread-table-viewport").unwrap();
                for chart in ["cpu", "stack", "heap"] {
                    let bounds = cx.debug_bounds(chart).unwrap();
                    assert!(
                        (bounds.size.height - table.size.height).abs() <= px(1.),
                        "{chart} and table should split available height equally: {bounds:?}, {table:?}"
                    );
                }
                let charts = cx.debug_bounds("overview-charts").unwrap();
                let heading = cx.debug_bounds("thread-table-heading").unwrap();
                let legend = cx.debug_bounds("thread-activity-legend").unwrap();
                let footer = cx.debug_bounds("session-footer").unwrap();
                assert!(charts.bottom() <= heading.top() && heading.bottom() <= table.top());
                assert!(table.bottom() <= legend.top() && legend.bottom() <= footer.top());
                assert!(footer.bottom() <= px(height));
            }
        }
    }
}

fn complete_session(app: &Entity<ThreadsApp>, cx: &mut VisualTestContext) -> Vec<ScreenCase> {
    use crate::{kernel_model::*, model::*};
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.frames = parse(include_str!(
                "../test-data/zview-bench-60s/session-with-heap-maps.json"
            ))
            .unwrap();
            app.frame = 288;
            let frame = &mut app.frames[app.frame];
            let waiters = Some(
                (0..100)
                    .map(|i| format!("waiting-thread-{i}"))
                    .collect::<Vec<_>>(),
            );
            for heap in &mut frame.heaps {
                heap.waiters = waiters.clone();
                heap.chunks = Some(vec![
                    HeapChunk {
                        size: 1024,
                        used: true,
                    },
                    HeapChunk {
                        size: 32,
                        used: false,
                    },
                ]);
            }
            for object in frame
                .semaphores
                .iter_mut()
                .map(|o| &mut o.object)
                .chain(frame.mutexes.iter_mut().map(|o| &mut o.object))
                .chain(frame.msgqs.iter_mut().map(|o| &mut o.object))
                .chain(frame.mem_slabs.iter_mut().map(|o| &mut o.object))
                .chain(frame.workqs.iter_mut().map(|o| &mut o.object))
            {
                object.waiters = waiters.clone();
            }
            for queue in &mut frame.workqs {
                queue.pending = Some((0..100).map(|i| format!("pending-work-{i}")).collect());
                queue.pending_truncated = false;
            }
            app.source = Some(std::path::PathBuf::from(format!(
                "/a-long-source-path/{}/session.json",
                "recording-directory/".repeat(20)
            )));
            app.sync_heaps(window, cx);
            cx.notify();
        });
    });
    app.read_with(cx, |app, _| -> Vec<ScreenCase> {
        let f = &app.frames[app.frame];
        let sem = &f.semaphores[0].object;
        let mtx = &f.mutexes[0].object;
        let msg = &f.msgqs[0].object;
        let slab = &f.mem_slabs[0].object;
        let work = &f.workqs[0].object;
        let heap = &f.heaps[0];
        vec![
            (
                Screen::Overview,
                &["cpu", "stack", "heap", "thread-table"],
                "cpu-plot",
            ),
            (
                Screen::Thread(f.visible_threads()[0].key()),
                &[
                    "thread-summary",
                    "thread-metadata",
                    "thread-cpu",
                    "thread-load",
                ],
                "thread-cpu-plot",
            ),
            (
                Screen::KernelObjects,
                &["kernel-table-toolbar", "kernel-table"],
                "object-rows",
            ),
            (
                Screen::Semaphore(SemaphoreKey {
                    address: sem.address,
                    name: sem.name.clone(),
                }),
                &[
                    "semaphore-summary",
                    "semaphore-history",
                    "semaphore-wait-queue",
                ],
                "semaphore-count-plot",
            ),
            (
                Screen::Mutex(MutexKey {
                    address: mtx.address,
                    name: mtx.name.clone(),
                }),
                &["mutex-summary", "mutex-history", "mutex-wait-queue"],
                "mutex-area-plot",
            ),
            (
                Screen::MessageQueue(MessageQueueKey {
                    address: msg.address,
                    name: msg.name.clone(),
                }),
                &["msgq-summary", "msgq-history", "msgq-wait-queue"],
                "msgq-depth-plot",
            ),
            (
                Screen::MemorySlab(MemorySlabKey {
                    address: slab.address,
                    name: slab.name.clone(),
                }),
                &["slab-summary", "slab-history", "slab-wait-queue"],
                "slab-usage-plot",
            ),
            (
                Screen::WorkQueue(WorkQueueKey {
                    address: work.address,
                    name: work.name.clone(),
                }),
                &[
                    "workq-summary",
                    "workq-activity",
                    "workq-history",
                    "workq-pending",
                    "workq-drain-waiters",
                ],
                "workq-backlog-zoom-surface",
            ),
            (
                Screen::Heap(HeapKey {
                    address: heap.address,
                    name: heap.name.clone(),
                }),
                &[
                    "heap-summary",
                    "heap-history",
                    "heap-fragmentation-plot",
                    "heap-map",
                    "heap-wait-queue",
                ],
                "heap-allocated-plot",
            ),
        ]
    })
}

#[gpui_kit::test]
fn every_screen_keeps_its_sections_visible_with_large_lists(cx: &mut TestAppContext) {
    let (app, mut cx) = setup(cx);
    let cases = complete_session(&app, &mut cx);
    for empty in [false, true] {
        if empty {
            cx.update(|_, cx| {
                app.update(cx, |app, _| {
                    let frame = &mut app.frames[app.frame];
                    frame.threads.clear();
                    frame.heaps.clear();
                    frame.semaphores.clear();
                    frame.mutexes.clear();
                    frame.msgqs.clear();
                    frame.mem_slabs.clear();
                    frame.workqs.clear();
                })
            });
        }
        for (width, height) in [(1480., 940.), (1000., 680.), (1920., 1080.)] {
            cx.simulate_resize(size(px(width), px(height)));
            for (screen, sections, plot) in &cases {
                cx.update(|window, cx| {
                    app.update(cx, |app, cx| {
                        app.screen = screen.clone();
                        cx.notify();
                    });
                    window.draw(cx).clear(cx);
                });
                let toolbar = cx.debug_bounds("app-toolbar").unwrap();
                let footer = cx.debug_bounds("session-footer").unwrap();
                assert!(
                    toolbar.top() >= px(0.) && footer.bottom() <= px(height),
                    "{screen:?}: {footer:?}"
                );
                for section in *sections {
                    let bounds = cx.debug_bounds(section).expect(section);
                    assert!(
                        bounds.size.height > px(0.),
                        "{screen:?}: {section}: {bounds:?}"
                    );
                    assert!(
                        bounds.top() >= toolbar.bottom() && bounds.bottom() <= footer.top(),
                        "{screen:?}: {section}: {bounds:?}, footer {footer:?}"
                    );
                    assert!(
                        bounds.left() >= px(0.) && bounds.right() <= px(width),
                        "{screen:?}: {section}: {bounds:?}"
                    );
                }
                let bounds = cx.debug_bounds(plot).unwrap();
                assert!(
                    bounds.size.height >= px(80.),
                    "{screen:?}: {plot} is too short: {bounds:?}"
                );
                assert!(
                    bounds.bottom() <= footer.top(),
                    "{screen:?}: {plot}: {bounds:?}"
                );
                if matches!(screen, Screen::Heap(_)) {
                    let allocated = cx.debug_bounds("heap-allocated-plot").unwrap();
                    let fragmentation = cx.debug_bounds("heap-fragmentation-plot").unwrap();
                    assert!(fragmentation.left() >= allocated.right());
                    let map = cx.debug_bounds("heap-map").unwrap();
                    assert!(allocated.top() >= map.bottom());
                    assert!(fragmentation.size.height >= px(80.), "{fragmentation:?}");
                    assert!(fragmentation.bottom() <= footer.top());
                }
                if matches!(screen, Screen::Heap(_)) && !empty {
                    let map = cx.debug_bounds("heap-map").unwrap();
                    for child in ["heap-map-strip", "heap-map-metrics", "heap-map-inspection"] {
                        let bounds = cx.debug_bounds(child).unwrap();
                        assert!(
                            bounds.top() >= map.top() && bounds.bottom() <= map.bottom(),
                            "{child}: {bounds:?} in {map:?}"
                        );
                    }
                }
                app.read_with(&cx, |app, _| assert_eq!(app.frame, 288));
            }
        }
    }
}

#[gpui_kit::test]
fn waiters_and_pending_work_scroll_inside_their_panels(cx: &mut TestAppContext) {
    let (app, mut cx) = setup(cx);
    let cases = complete_session(&app, &mut cx);
    cx.simulate_resize(size(px(1000.), px(680.)));
    for (screen, panel, entry) in [
        (&cases[3].0, "semaphore-wait-queue", "semaphore-waiter-0"),
        (&cases[4].0, "mutex-wait-queue", "mutex-waiter-0"),
        (&cases[5].0, "msgq-wait-queue", "msgq-waiter-0"),
        (&cases[6].0, "slab-wait-queue", "slab-waiter-0"),
        (&cases[7].0, "workq-pending", "workq-pending-tag-0"),
        (&cases[7].0, "workq-drain-waiters", "workq-drain-waiter-0"),
        (&cases[8].0, "heap-wait-queue", "heap-waiter-0"),
    ] {
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.screen = screen.clone();
                cx.notify();
            });
            window.draw(cx).clear(cx);
        });
        let panel_bounds = cx.debug_bounds(panel).unwrap();
        let first = cx.debug_bounds(entry).unwrap();
        let footer = cx.debug_bounds("session-footer").unwrap();
        scroll(
            &mut cx,
            point(
                panel_bounds.left() + px(40.),
                panel_bounds.bottom() - px(20.),
            ),
            -500.,
        );
        assert!(
            cx.debug_bounds(entry).unwrap().top() < first.top(),
            "{panel} should scroll locally"
        );
        assert_eq!(cx.debug_bounds(panel).unwrap(), panel_bounds);
        assert_eq!(cx.debug_bounds("session-footer").unwrap(), footer);
    }
}

#[gpui_kit::test]
fn work_queue_panels_share_width_60_20_20_without_clipping_long_item_names(
    cx: &mut TestAppContext,
) {
    let (app, mut cx) = setup(cx);
    let cases = complete_session(&app, &mut cx);
    for (width, height) in [(1000., 680.), (1480., 940.), (1920., 1080.)] {
        cx.simulate_resize(size(px(width), px(height)));
        for collapsed in [false, true] {
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.screen = cases[7].0.clone();
                    app.collapsed = collapsed;
                    cx.notify();
                });
                window.draw(cx).clear(cx);
            });
            let row = cx.debug_bounds("workq-lower-panels").unwrap();
            let chart = cx.debug_bounds("workq-history").unwrap();
            let items = cx.debug_bounds("workq-pending").unwrap();
            let waiters = cx.debug_bounds("workq-drain-waiters").unwrap();
            let summary = cx.debug_bounds("workq-summary").unwrap();
            assert_eq!(row.left(), summary.left(), "keep the existing outer margin");
            assert_eq!(
                row.right(),
                summary.right(),
                "keep the existing outer margin"
            );
            assert!((items.left() - chart.right() - px(12.)).abs() <= px(1.));
            assert!((waiters.left() - items.right() - px(12.)).abs() <= px(1.));
            let usable = row.size.width - px(24.);
            assert!((chart.size.width - usable * 0.6).abs() <= px(1.));
            for panel in [items, waiters] {
                assert!(
                    (panel.size.width - usable * 0.2).abs() <= px(1.),
                    "{panel:?} in {row:?}"
                );
                assert!((panel.top() - chart.top()).abs() <= px(1.));
                assert!((panel.bottom() - chart.bottom()).abs() <= px(1.));
            }
            assert!(items.left() >= chart.right() && waiters.left() >= items.right());
            for (selector, panel) in [
                ("workq-item-label-0", items),
                ("workq-drain-waiter-label-0", waiters),
            ] {
                let label = cx.debug_bounds(selector).unwrap();
                assert!(
                    label.left() >= panel.left() + px(12.)
                        && label.right() <= panel.right() - px(12.),
                    "{selector}: {label:?} in {panel:?}"
                );
            }
        }
    }
}

use super::{Screen, ThreadsApp, apply_theme, bind_keys};
use gpui::{AppContext, Entity, Modifiers, TestAppContext, VisualTestContext, px, size};
use gpui_kit::component::{Root, WindowExt};
use gpui_kit::init;

#[path = "heap_map_tests.rs"]
mod heap_map_tests;
#[path = "layout_tests.rs"]
mod layout_tests;
#[path = "session_tests.rs"]
mod session_tests;
#[path = "workq_activity_tests.rs"]
mod workq_activity_tests;

#[test]
fn browser_help_only_describes_supported_file_actions() {
    let text = super::help_text(true);
    assert!(text.contains("already-decoded JSON"));
    assert!(text.contains("read locally without uploading"));
    assert!(text.contains("Select a changed file again"));
    assert!(text.contains("Heap maps show captured chunks"));
    for unsupported in [
        "Open recording",
        ".ndjson.gz",
        "ZVIEW_PYTHON",
        "Reload repeats",
    ] {
        assert!(!text.contains(unsupported), "{unsupported}");
    }
}

#[test]
fn native_help_keeps_recording_import_and_reload_instructions() {
    let text = super::help_text(false);
    assert!(text.contains("Open recording"));
    assert!(text.contains("ZVIEW_PYTHON"));
    assert!(text.contains("Reload repeats the import"));
    assert!(text.contains("Heap maps show captured chunks"));
}

/// This integration check deliberately requires the full, regenerated capture
/// rather than silently falling back to a synthetic or map-less document.
#[gpui_kit::test]
#[ignore = "First export local/bench-with-maps.json; see README.md"]
fn recorded_heap_maps_render_and_play_from_the_full_benchmark(cx: &mut TestAppContext) {
    use super::{kernel_model::Kind, model};
    use std::{collections::HashSet, time::Duration};

    // GPUI's test selector API requires static strings. These small generated
    // selectors live for the test process only, never in the application.
    fn selector(value: String) -> &'static str {
        Box::leak(value.into_boxed_str())
    }

    let path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("local/bench-with-maps.json");
    let frames =
        model::load(&path).expect("Export the benchmark with maps before running this test");
    assert_eq!(frames.len(), 577);
    let maps = frames.iter().map(|f| f.heaps[0].chunks.as_ref().unwrap());
    assert_eq!(maps.clone().map(|c| c.len()).min(), Some(2),);
    assert_eq!(maps.clone().map(|c| c.len()).max(), Some(130));
    let unique_maps: HashSet<_> = maps
        .map(|c| c.iter().map(|c| (c.size, c.used)).collect::<Vec<_>>())
        .collect();
    assert_eq!(unique_maps.len(), 421);
    assert_eq!(frames[288].heaps[0].allocated_bytes, 720);
    assert_eq!(frames[288].heaps[0].free_bytes, 852);

    let (app, mut cx) = setup(cx);
    cx.simulate_resize(size(px(1480.), px(1600.)));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.load(path, window, cx);
            assert!(app.error.is_none());
            assert_eq!(app.frame, 0);
            app.playback_tick(app.frames[288].elapsed_seconds.unwrap(), window, cx);
            assert_eq!(app.frame, 288);
            app.show_kernel_objects(Some(Kind::Heap), window, cx);
        });
        window.draw(cx).clear(cx);
    });
    let address = frames[0].heaps[0].address;
    let row = cx
        .debug_bounds(selector(format!("kernel-row-Heap-{address}")))
        .unwrap()
        .center();
    cx.simulate_click(row, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));

    // Start, waiting, simplest map, most chunks, peak allocation, midpoint,
    // peak fragmentation, and the last frame use the unchanged source data.
    for index in [0, 23, 66, 180, 255, 288, 437, 576] {
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.frame = index;
                app.sync_heaps(window, cx);
                cx.notify();
            });
            window.draw(cx).clear(cx);
        });
        assert!(cx.debug_bounds("heap-map-metrics").is_some());
        assert!(cx.debug_bounds("heap-fragmentation-plot").is_some());
        assert!(cx.debug_bounds("heap-map-unavailable").is_none());
        assert!(cx.debug_bounds("heap-absent").is_none());
        let source = &frames[index].heaps[0];
        let chunks = source.chunks.as_ref().unwrap();
        assert!(!chunks.is_empty());
        let strip = cx.debug_bounds("heap-map-strip").unwrap();
        let summary = cx.debug_bounds("heap-summary").unwrap();
        assert!(strip.size.width >= summary.size.width - px(28.));
        assert!(strip.size.height <= px(48.), "the map is a single strip");
        let known_waiters = source.waiters.as_ref().unwrap();
        assert_eq!(
            cx.debug_bounds("heap-waiter-0").is_some(),
            !known_waiters.is_empty()
        );
        app.read_with(&cx, |app, _| {
            let Screen::Heap(key) = &app.screen else {
                panic!("expected heap details")
            };
            assert_eq!(
                key.in_frame(&app.frames[index]).unwrap().allocated_bytes,
                source.allocated_bytes
            );
        });
    }

    // Move back to midpoint, then exercise a real chunk hover and history zoom.
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.frame = 288;
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    let chunk = cx.debug_bounds("heap-map-strip").unwrap().center();
    cx.simulate_event(gpui::MouseMoveEvent {
        position: chunk,
        ..Default::default()
    });
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let tooltip = cx
        .debug_bounds("heap-chunk-tooltip")
        .expect("hover inspects the original chunk");
    assert!(tooltip.size.width <= px(280.));
    let surface = cx
        .debug_bounds("heap-allocated-zoom-surface")
        .unwrap()
        .center();
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: surface,
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(80.))),
        modifiers: Modifiers::default(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("heap-allocated-zoomed").is_some());

    let play = cx.debug_bounds("play-pause").unwrap().center();
    cx.simulate_click(play, Modifiers::default());
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            assert!(app.playing);
            assert_eq!(
                app.frame,
                model::frame_at(&frames, frames[288].elapsed_seconds.unwrap() + 1.)
            );
            app.toggle_playback(window, cx);
        });
        window.draw(cx).clear(cx);
    });
    assert!(cx.debug_bounds("heap-map-metrics").is_some());
    cx.simulate_resize(size(px(1000.), px(680.)));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let map = cx.debug_bounds("heap-map").unwrap();
    let history = cx.debug_bounds("heap-history").unwrap();
    assert!(map.right() <= px(1000.));
    assert!(map.bottom() <= history.top());
    assert!(map.bottom() <= cx.debug_bounds("session-footer").unwrap().top());
    cx.simulate_keystrokes("escape");
    app.read_with(&cx, |app, _| assert_eq!(app.screen, Screen::KernelObjects));
}

#[gpui_kit::test]
fn heap_detail_navigation_preserves_session_and_handles_absence(cx: &mut TestAppContext) {
    use super::{kernel_model::Kind, model::HeapKey};
    let (app, mut cx) = setup(cx);
    cx.simulate_resize(size(px(1480.), px(1600.)));
    open_kernel_objects(&app, &mut cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.frames = super::model::parse(include_str!("../test-data/heaps.json")).unwrap();
            app.frame = 2;
            app.sync_heaps(window, cx);
            app.show_kernel_objects(Some(Kind::Heap), window, cx);
            app.object_search
                .update(cx, |input, cx| input.set_value("packet", window, cx));
        });
        window.draw(cx).clear(cx);
    });
    let selected_heap = app.read_with(&cx, |app, cx| app.heap.read(cx).selected_value().cloned());
    let key = HeapKey {
        address: 4096,
        name: "packet_heap".into(),
    };
    let row = cx.debug_bounds("kernel-row-Heap-4096").unwrap().center();
    cx.simulate_click(row, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    app.read_with(&cx, |app, _| {
        assert_eq!(app.screen, Screen::Heap(key.clone()))
    });
    assert!(cx.debug_bounds("heap-detail").is_some());
    assert!(cx.debug_bounds("heap-map-metrics").is_some());
    assert!(cx.debug_bounds("heap-allocated-plot").is_some());
    assert!(cx.debug_bounds("heap-waiter-0").is_some());
    assert!(cx.debug_bounds("heap-map-unavailable").is_none());
    cx.simulate_keystrokes("escape");
    app.read_with(&cx, |app, cx| {
        assert_eq!(app.screen, Screen::KernelObjects);
        assert_eq!(app.object_kind, Some(Kind::Heap));
        assert_eq!(app.object_search.read(cx).value().as_str(), "packet");
        assert_eq!(app.frame, 2);
        assert_eq!(app.heap.read(cx).selected_value(), selected_heap.as_ref());
    });
    // Type navigation focuses search; Tab passes Clear filters then the row.
    let kind = cx.debug_bounds("object-type-Heap").unwrap().center();
    cx.simulate_click(kind, Modifiers::default());
    cx.simulate_keystrokes("tab tab enter");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    app.read_with(&cx, |app, _| {
        assert_eq!(app.screen, Screen::Heap(key.clone()))
    });
    let next = cx.debug_bounds("next").unwrap().center();
    cx.simulate_click(next, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    app.read_with(&cx, |app, _| {
        assert_eq!(app.frame, 3, "Next at {next:?}, screen {:?}", app.screen)
    });
    assert!(cx.debug_bounds("heap-absent").is_some());
    assert!(cx.debug_bounds("heap-map-unavailable").is_some());
    assert_missing_notice(&app, &mut cx, true);
    let next = cx.debug_bounds("next").unwrap().center();
    cx.simulate_click(next, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert_missing_notice(&app, &mut cx, false);
    // Let the removed warning finish its exit animation before clicking the
    // Back button in the same top-right region.
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("heap-absent").is_none());
    assert!(cx.debug_bounds("heap-map-metrics").is_some());
    let breadcrumb = cx.debug_bounds("objects-breadcrumb").unwrap().center();
    cx.simulate_click(breadcrumb, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("kernel-objects").is_some());
    let row = cx.debug_bounds("kernel-row-Heap-4096").unwrap().center();
    cx.simulate_click(row, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let back = cx.debug_bounds("back-to-objects").unwrap().center();
    cx.simulate_click(back, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    app.read_with(&cx, |app, _| {
        assert_eq!(app.screen, Screen::KernelObjects, "Back at {back:?}");
        assert_eq!(app.frame, 4);
    });
}

#[gpui_kit::test]
fn heap_maps_render_legacy_empty_invalid_and_narrow_layouts(cx: &mut TestAppContext) {
    use super::model::{HeapChunk, HeapKey};
    let (app, mut cx) = setup(cx);
    open_kernel_objects(&app, &mut cx);
    let row = cx.debug_bounds("kernel-row-Heap-4096").unwrap().center();
    cx.simulate_click(row, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("heap-map-unavailable").is_some());
    for chunks in [
        Some(vec![]),
        Some(vec![HeapChunk {
            size: 0,
            used: true,
        }]),
        Some(vec![
            HeapChunk {
                size: 4096,
                used: false,
            },
            HeapChunk {
                size: 128,
                used: true,
            },
        ]),
    ] {
        let known = chunks.as_ref().is_some_and(|c| c.len() == 2);
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.frames[0].heaps[0].chunks = chunks.clone();
                app.screen = Screen::Heap(HeapKey {
                    address: 4096,
                    name: "heap".into(),
                });
                cx.notify();
            });
            window.draw(cx).clear(cx);
        });
        assert_eq!(cx.debug_bounds("heap-map-metrics").is_some(), known);
        assert_eq!(cx.debug_bounds("heap-map-unavailable").is_some(), !known);
    }
    cx.simulate_resize(size(px(1000.), px(680.)));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let summary = cx.debug_bounds("heap-summary").unwrap();
    let history = cx.debug_bounds("heap-history").unwrap();
    let map = cx.debug_bounds("heap-map").unwrap();
    assert!(history.top() >= summary.bottom());
    assert!(map.top() >= summary.bottom());
    assert!(map.bottom() <= history.top());
    assert!(map.right() <= px(1000.));
    assert!(cx.debug_bounds("heap-allocated-plot").unwrap().size.height >= px(80.));
    assert!(cx.debug_bounds("heap-wait-queue").unwrap().bottom() <= px(680.));
}

#[gpui_kit::test]
fn controls_and_popup_use_the_app_palette(cx: &mut TestAppContext) {
    use gpui_kit::component::ActiveTheme;
    let (_app, mut cx) = setup(cx);
    cx.update(|window, cx| {
        let theme = cx.theme();
        assert_eq!(theme.colors.button, gpui::rgb(super::CARD).into());
        assert_eq!(theme.colors.button_hover, gpui::rgb(0x353049).into());
        assert_eq!(theme.colors.button_active, gpui::rgb(0x3d365b).into());
        assert_eq!(theme.colors.input, gpui::rgb(super::BORDER).into());
        assert_eq!(theme.colors.ring, gpui::rgb(super::PURPLE).into());
        assert_eq!(
            theme.tokens.button.background,
            gpui::Background::from(gpui::rgb(super::CARD))
        );
        assert_eq!(
            theme.tokens.popover.background,
            gpui::Background::from(gpui::rgb(super::CARD))
        );
        assert_eq!(
            theme.tokens.accent.background,
            gpui::Background::from(gpui::rgb(0x3d365b))
        );
        window.draw(cx).clear(cx);
    });
    assert!(cx.debug_bounds("thread-activity-legend").is_some());
    let heap = cx.debug_bounds("heap-selector").unwrap().center();
    cx.simulate_click(heap, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.simulate_keystrokes("bench");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.simulate_keystrokes("escape");
}

#[test]
fn activity_descriptions_do_not_claim_scheduler_states() {
    let mut thread = super::model::parse(include_str!("../sample.json"))
        .unwrap()
        .remove(0)
        .threads
        .remove(0);
    thread.runtime = None;
    assert_eq!(
        thread.activity_description(),
        "CPU activity unavailable; scheduler state unknown"
    );
    let mut frame = super::model::parse(include_str!("../sample.json"))
        .unwrap()
        .remove(0);
    let runtime = frame
        .threads
        .iter_mut()
        .find_map(|thread| thread.runtime.take())
        .unwrap();
    thread.runtime = Some(runtime);
    thread.runtime.as_mut().unwrap().active = true;
    assert!(
        thread
            .activity_description()
            .starts_with("CPU activity observed")
    );
    thread.runtime.as_mut().unwrap().active = false;
    thread.runtime.as_mut().unwrap().cpu = Some(0.);
    assert!(
        thread
            .activity_description()
            .starts_with("No CPU activity observed")
    );
    thread.runtime.as_mut().unwrap().cpu = None;
    thread.runtime.as_mut().unwrap().cpu_normalized = None;
    assert!(
        thread
            .activity_description()
            .starts_with("CPU activity unavailable")
    );
}

#[gpui_kit::test]
fn shared_thread_tags_and_empty_states_render_in_every_wait_queue(cx: &mut TestAppContext) {
    use super::kernel_model::{
        MemorySlabKey, MessageQueueKey, MutexKey, SemaphoreKey, WorkQueueKey,
    };
    let (app, mut cx) = setup(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.load(
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("test-data/kernel-objects-minimal.json"),
                window,
                cx,
            );
            app.frame = 0;
        })
    });
    macro_rules! detail {
        ($screen:ident, $key:ident, $address:literal, $name:literal, $prefix:literal) => {
            (
                Screen::$screen($key {
                    address: $address,
                    name: $name.into(),
                }),
                concat!($prefix, "-waiter-0"),
                concat!($prefix, "-waiter-1"),
                concat!($prefix, "-wait-queue-empty"),
            )
        };
    }
    let cases = vec![
        (
            Screen::KernelObjects,
            "object-Semaphore-8192-waiter-0",
            "object-Semaphore-8192-waiter-1",
            "object-Semaphore-8192-wait-queue-empty",
        ),
        detail!(Semaphore, SemaphoreKey, 8192, "gate", "semaphore"),
        detail!(Mutex, MutexKey, 12288, "gate", "mutex"),
        detail!(MessageQueue, MessageQueueKey, 16384, "messages", "msgq"),
        detail!(MemorySlab, MemorySlabKey, 24576, "slab", "slab"),
        detail!(WorkQueue, WorkQueueKey, 28672, "work", "workq-drain"),
    ];
    for (screen, first, second, empty) in cases {
        for names in [
            Some(vec!["worker".to_string(), "logger".to_string()]),
            Some(vec![]),
            None,
        ] {
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    let f = &mut app.frames[0];
                    f.semaphores[0].object.waiters = names.clone();
                    f.mutexes[0].object.waiters = names.clone();
                    f.msgqs[0].object.waiters = names.clone();
                    f.mem_slabs[0].object.waiters = names.clone();
                    f.workqs[0].object.waiters = names.clone();
                    f.heaps[0].waiters = names.clone();
                    app.screen = screen.clone();
                    cx.notify();
                });
                window.draw(cx).clear(cx);
            });
            if names.as_ref().is_some_and(|n| !n.is_empty()) {
                let a = cx.debug_bounds(first).unwrap();
                let b = cx.debug_bounds(second).unwrap();
                assert!(cx.debug_bounds("wait-thread-icon").is_some());
                assert!(cx.debug_bounds(empty).is_none());
                if screen != Screen::KernelObjects {
                    assert_eq!(a.left(), b.left());
                    assert!(b.top() >= a.bottom());
                }
            } else {
                assert_eq!(cx.debug_bounds(empty).is_some(), names.is_none());
                assert!(cx.debug_bounds(first).is_none());
                assert!(cx.debug_bounds("wait-thread-icon").is_none());
            }
        }
    }
}

#[gpui_kit::test]
fn playback_status_and_buttons_do_not_shift_with_digits_or_state(cx: &mut TestAppContext) {
    let (app, mut cx) = setup(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.load(
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-data/workqs.json"),
                window,
                cx,
            );
            app.reset_playback(window, cx);
        });
        window.draw(cx).clear(cx);
    });
    let selectors = [
        "previous",
        "play-pause",
        "reset-playback",
        "playback-status",
        "next",
    ];
    let initial = selectors.map(|selector| cx.debug_bounds(selector).unwrap());
    for (time, play) in [(9.99, false), (10., false), (2., true)] {
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.pause(cx);
                app.frame = super::model::frame_at(&app.frames, time);
                app.playback_position = Some(time);
                if play {
                    app.toggle_playback(window, cx);
                }
                cx.notify();
            });
            window.draw(cx).clear(cx);
        });
        for (selector, expected) in selectors.into_iter().zip(initial) {
            let actual = cx.debug_bounds(selector).unwrap();
            assert_eq!(actual, expected, "{selector} shifted at {time}s");
        }
    }
    cx.update(|_, cx| app.update(cx, |app, cx| app.pause(cx)));
}

#[gpui_kit::test]
fn viewport_clock_moves_between_samples_and_pause_preserves_fractional_time(
    cx: &mut TestAppContext,
) {
    use std::time::Duration;
    let (app, mut cx) = setup(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.load(
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-data/workqs.json"),
                window,
                cx,
            );
            app.frame = 1; // Source reading at 2s, next reading at 5s.
            app.toggle_playback(window, cx);
        });
        window.draw(cx).clear(cx);
    });
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(16));
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    app.read_with(&cx, |app, cx| {
        assert_eq!(app.frame, 1);
        assert!((app.playback_time(cx).unwrap() - 2.016).abs() < 1e-6);
    });
    cx.update(|window, cx| app.update(cx, |app, cx| app.toggle_playback(window, cx)));
    cx.executor().advance_clock(Duration::from_secs(20));
    cx.run_until_parked();
    app.read_with(&cx, |app, cx| {
        assert_eq!(app.frame, 1);
        assert!((app.playback_time(cx).unwrap() - 2.016).abs() < 1e-6);
    });
    cx.update(|window, cx| app.update(cx, |app, cx| app.toggle_playback(window, cx)));
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(16));
    cx.run_until_parked();
    app.read_with(&cx, |app, cx| {
        assert_eq!(app.frame, 1);
        assert!((app.playback_time(cx).unwrap() - 2.032).abs() < 1e-6);
    });
    cx.update(|window, cx| app.update(cx, |app, cx| app.reset_playback(window, cx)));
}

#[gpui_kit::test]
fn every_chart_supports_independent_wheel_zoom_and_fit(cx: &mut TestAppContext) {
    macro_rules! charts {
        ($($id:literal),*) => { vec![$((
            $id, concat!($id, "-zoom-surface"), concat!($id, "-zoomed"),
            concat!($id, "-unzoomed"), concat!($id, "-fit-history"),
        )),*] };
    }
    use super::kernel_model::{
        MemorySlabKey, MessageQueueKey, MutexKey, SemaphoreKey, WorkQueueKey,
    };
    let (app, mut cx) = setup(cx);
    cx.simulate_resize(size(px(1600.), px(1100.)));
    let cases = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.load(
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("test-data/zview-bench-60s/session.json"),
                window,
                cx,
            );
            let f = &app.frames[app.frame];
            vec![
                (Screen::Overview, charts!["cpu", "stack", "heap"]),
                (
                    Screen::Thread(f.visible_threads()[0].key()),
                    charts!["thread-cpu", "thread-load"],
                ),
                (
                    Screen::Semaphore(SemaphoreKey {
                        address: f.semaphores[0].object.address,
                        name: f.semaphores[0].object.name.clone(),
                    }),
                    charts!["semaphore-step-area"],
                ),
                (
                    Screen::Mutex(MutexKey {
                        address: f.mutexes[0].object.address,
                        name: f.mutexes[0].object.name.clone(),
                    }),
                    charts!["mutex-step-area"],
                ),
                (
                    Screen::MessageQueue(MessageQueueKey {
                        address: f.msgqs[0].object.address,
                        name: f.msgqs[0].object.name.clone(),
                    }),
                    charts!["msgq-depth"],
                ),
                (
                    Screen::MemorySlab(MemorySlabKey {
                        address: f.mem_slabs[0].object.address,
                        name: f.mem_slabs[0].object.name.clone(),
                    }),
                    charts!["slab-usage"],
                ),
                (
                    Screen::WorkQueue(WorkQueueKey {
                        address: f.workqs[0].object.address,
                        name: f.workqs[0].object.name.clone(),
                    }),
                    charts!["workq-backlog"],
                ),
                (
                    Screen::Heap(super::model::HeapKey {
                        address: f.heaps[0].address,
                        name: f.heaps[0].name.clone(),
                    }),
                    charts!["heap-allocated", "heap-fragmentation"],
                ),
            ]
        })
    });
    for (screen, ids) in cases {
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.screen = screen;
                cx.notify();
            });
            window.draw(cx).clear(cx);
        });
        let frame = app.read_with(&cx, |app, _| app.frame);
        for (id, surface, zoomed, unzoomed, fit) in &ids {
            let surface = cx.debug_bounds(surface).unwrap();
            cx.simulate_event(gpui::ScrollWheelEvent {
                position: surface.center(),
                delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(80.))),
                modifiers: Modifiers::default(),
                touch_phase: gpui::TouchPhase::Moved,
            });
            cx.update(|window, cx| window.draw(cx).clear(cx));
            assert!(cx.debug_bounds(zoomed).is_some(), "{id}");
            for (other, _, _, other_unzoomed, _) in &ids {
                if other != id {
                    assert!(cx.debug_bounds(other_unzoomed).is_some());
                }
            }
            let fit = cx.debug_bounds(fit).unwrap().center();
            cx.simulate_click(fit, Modifiers::default());
            cx.update(|window, cx| window.draw(cx).clear(cx));
            assert!(cx.debug_bounds(unzoomed).is_some(), "{id}");
            app.read_with(&cx, |app, _| assert_eq!(app.frame, frame));
        }
    }
}

#[gpui_kit::test]
fn workq_scroll_zoom_is_local_and_keeps_session_time(cx: &mut TestAppContext) {
    let (app, mut cx) = setup(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.load(
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-data/workqs.json"),
                window,
                cx,
            );
            app.frame = 1;
            app.screen = Screen::WorkQueue(super::kernel_model::WorkQueueKey {
                address: 28672,
                name: "system_workq".into(),
            });
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    let surface = cx.debug_bounds("workq-backlog-zoom-surface").unwrap();
    for delta in [80., -40.] {
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: surface.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(delta))),
            modifiers: Modifiers::default(),
            touch_phase: gpui::TouchPhase::Moved,
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        app.read_with(&cx, |app, _| {
            assert_eq!(app.frame, 1);
        });
        assert!(cx.debug_bounds("workq-backlog-zoomed").is_some());
    }
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.step(true, window, cx);
            app.load(
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-data/workqs.json"),
                window,
                cx,
            );
        });
    });
}

#[gpui_kit::test]
fn real_capture_playback_matches_acquisition_timestamps(cx: &mut TestAppContext) {
    use std::time::Duration;
    let (app, mut cx) = setup(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.load(
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("test-data/zview-bench-60s/session.json"),
                window,
                cx,
            );
            app.reset_playback(window, cx);
            app.toggle_playback(window, cx);
        })
    });
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_secs(10));
    cx.run_until_parked();
    app.read_with(&cx, |app, _| {
        assert!(app.playing);
        assert_eq!(app.frame, super::model::frame_at(&app.frames, 10.));
        assert!(app.frames[app.frame].elapsed_seconds.unwrap() <= 10.);
    });
    cx.executor().advance_clock(Duration::from_secs(50));
    cx.run_until_parked();
    app.read_with(&cx, |app, _| {
        assert!(!app.playing);
        assert_eq!(app.frame, 576);
    });
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.toggle_playback(window, cx);
            assert!(app.playing);
            assert_eq!(app.frame, 0); // Play at the end restarts, without looping.
            app.reset_playback(window, cx);
        })
    });
    cx.executor().advance_clock(Duration::from_secs(60));
    cx.run_until_parked();
    app.read_with(&cx, |app, _| assert_eq!(app.frame, 0));
}

#[gpui_kit::test]
fn playback_handles_timestamps_beyond_timer_range(cx: &mut TestAppContext) {
    use std::time::Duration;
    let (app, mut cx) = setup(cx);
    for end in [1e20, 1e100] {
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                let document = serde_json::json!([
                    {"threads": [], "elapsed_seconds": 0.},
                    {"threads": [], "elapsed_seconds": end},
                ]);
                app.frames = super::model::parse(&document.to_string()).unwrap();
                app.reset_playback(window, cx);
                app.toggle_playback(window, cx);
                assert!(app.playing);
                let delay = app.playback_tick(0., window, cx).unwrap();
                assert!(delay > Duration::ZERO && delay <= Duration::from_secs(1));
            });
        });
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
        app.read_with(&cx, |app, cx| {
            assert!(app.playing);
            assert_eq!(app.frame, 0);
            assert_eq!(app.playback_time(cx), Some(1.));
        });
        cx.update(|window, cx| app.update(cx, |app, cx| app.toggle_playback(window, cx)));
        cx.executor().advance_clock(Duration::from_secs(5));
        cx.run_until_parked();
        app.read_with(&cx, |app, _| {
            assert!(!app.playing);
            assert_eq!(app.frame, 0);
        });
    }
}

#[gpui_kit::test]
fn playback_uses_source_timing_pauses_resets_and_stops_at_end(cx: &mut TestAppContext) {
    use std::time::Duration;
    let (app, mut cx) = setup(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.load(
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-data/workqs.json"),
                window,
                cx,
            );
            app.reset_playback(window, cx);
            app.screen = Screen::WorkQueue(super::kernel_model::WorkQueueKey {
                address: 28672,
                name: "system_workq".into(),
            });
        });
        window.draw(cx).clear(cx);
    });
    let play = cx.debug_bounds("play-pause").unwrap().center();
    cx.simulate_click(play, Modifiers::default());
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(1999));
    cx.run_until_parked();
    app.read_with(&cx, |app, _| {
        assert!(app.playing);
        assert_eq!(app.frame, 0);
    });
    cx.executor().advance_clock(Duration::from_millis(1));
    cx.run_until_parked();
    app.read_with(&cx, |app, _| assert_eq!(app.frame, 1));
    // Pause cancels the timer, even when it was already awaiting the next frame.
    cx.update(|window, cx| {
        app.update(cx, |app, cx| app.toggle_playback(window, cx));
        window.draw(cx).clear(cx);
    });
    cx.executor().advance_clock(Duration::from_secs(20));
    cx.run_until_parked();
    app.read_with(&cx, |app, _| {
        assert!(!app.playing);
        assert_eq!(app.frame, 1);
    });
    cx.update(|window, cx| app.update(cx, |app, cx| app.toggle_playback(window, cx)));
    cx.run_until_parked();
    // A delayed UI tick catches up by timestamp, not by advancing only one frame.
    cx.executor().advance_clock(Duration::from_secs(8));
    cx.run_until_parked();
    app.read_with(&cx, |app, _| {
        assert!(!app.playing);
        assert_eq!(app.frame, 4);
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let reset = cx.debug_bounds("reset-playback").unwrap().center();
    cx.simulate_click(reset, Modifiers::default());
    cx.run_until_parked();
    app.read_with(&cx, |app, _| {
        assert_eq!(app.frame, 0);
        assert!(!app.playing);
        assert!(matches!(app.screen, Screen::WorkQueue(_)));
    });
}

#[gpui_kit::test]
fn playback_cancels_on_stepping_and_load_but_not_navigation(cx: &mut TestAppContext) {
    use std::time::Duration;
    let (app, mut cx) = setup(cx);
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-data/workqs.json");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.load(path.clone(), window, cx);
            app.reset_playback(window, cx);
            app.toggle_playback(window, cx);
            app.show_kernel_objects(Some(super::kernel_model::Kind::WorkQueue), window, cx);
            assert!(app.playing);
        })
    });
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            assert_eq!(app.frame, 1);
            app.step(true, window, cx);
            assert!(!app.playing);
            assert_eq!(app.frame, 2);
        })
    });
    cx.executor().advance_clock(Duration::from_secs(20));
    cx.run_until_parked();
    app.read_with(&cx, |app, _| assert_eq!(app.frame, 2));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.toggle_playback(window, cx);
            app.load(path, window, cx);
            assert!(!app.playing);
            app.frames = super::model::parse(include_str!("../sample.json")).unwrap();
            app.frame = 0;
            app.toggle_playback(window, cx);
            assert!(!app.playing); // A single untimed snapshot has no acquisition cadence.
        })
    });
    cx.executor().advance_clock(Duration::from_secs(20));
    cx.run_until_parked();
    app.read_with(&cx, |app, _| assert_eq!(app.frame, 0));
}

fn assert_missing_notice(app: &Entity<ThreadsApp>, cx: &mut VisualTestContext, missing: bool) {
    app.read_with(cx, |app, _| {
        assert_eq!(app.missing_notice.is_some(), missing)
    });
    if missing {
        cx.update(|window, cx| assert_eq!(window.notifications(cx).len(), 1));
    }
    assert!(cx.debug_bounds("mutex-absent").is_none());
    assert!(cx.debug_bounds("semaphore-absent").is_none());
}

#[gpui_kit::test]
fn workq_detail_preserves_session_and_handles_partial_absent_and_narrow_frames(
    cx: &mut TestAppContext,
) {
    let (app, mut cx) = setup(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.load(
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-data/workqs.json"),
                window,
                cx,
            );
            app.frame = 0;
            app.show_kernel_objects(Some(super::kernel_model::Kind::WorkQueue), window, cx);
            app.object_search
                .update(cx, |input, cx| input.set_value("system", window, cx));
        });
        window.draw(cx).clear(cx);
    });
    let row = cx
        .debug_bounds("kernel-row-Work queue-28672")
        .unwrap()
        .center();
    cx.simulate_click(row, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("workq-detail").is_some());
    assert!(cx.debug_bounds("workq-pending").is_some());
    assert!(cx.debug_bounds("workq-drain-waiters").is_some());
    let history = cx.debug_bounds("workq-history").unwrap();
    let pending = cx.debug_bounds("workq-pending").unwrap();
    let drain = cx.debug_bounds("workq-drain-waiters").unwrap();
    assert!((pending.size.height - drain.size.height).abs() <= px(1.));
    assert!((pending.top() - history.top()).abs() <= px(1.));
    assert!((drain.bottom() - history.bottom()).abs() <= px(1.));
    assert!(drain.left() >= pending.right());
    assert!((history.size.width - 3. * pending.size.width).abs() <= px(2.));
    assert!(cx.debug_bounds("workq-pending-tag-0").is_some());
    let back = cx.debug_bounds("back-to-objects").unwrap().center();
    cx.simulate_click(back, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    app.read_with(&cx, |app, _| assert_eq!(app.screen, Screen::KernelObjects));
    cx.simulate_keystrokes("tab tab enter");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("workq-detail").is_some());
    cx.simulate_resize(size(px(1000.), px(680.)));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(
        cx.debug_bounds("workq-history").unwrap().top()
            >= cx.debug_bounds("workq-summary").unwrap().bottom()
    );
    for frame in 1..=4 {
        cx.update(|window, cx| {
            app.update(cx, |app, cx| app.step(true, window, cx));
            window.draw(cx).clear(cx);
        });
        if frame == 1 {
            assert!(cx.debug_bounds("workq-pending-tag-0").is_none());
            assert!(cx.debug_bounds("workq-pending-message").is_none());
        } else if frame == 3 {
            assert!(cx.debug_bounds("workq-pending-message").is_some());
        }
    }
    assert_missing_notice(&app, &mut cx, true);
    cx.simulate_keystrokes("escape");
    app.read_with(&cx, |app, cx| {
        assert_eq!(app.screen, Screen::KernelObjects);
        assert_eq!(app.frame, 4);
        assert_eq!(app.object_kind, Some(super::kernel_model::Kind::WorkQueue));
        assert_eq!(app.object_search.read(cx).value().as_str(), "system");
    });
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.frame = 0;
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    cx.simulate_keystrokes("tab tab enter");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("workq-detail").is_some());
    cx.simulate_keystrokes("escape");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    app.read_with(&cx, |app, _| assert_eq!(app.screen, Screen::KernelObjects));
}

#[gpui_kit::test]
fn missing_object_notifications_are_deduplicated_and_reset_on_recovery(cx: &mut TestAppContext) {
    let (app, mut cx) = setup(cx);
    for (fixture, frame, screen) in [
        (
            "mutexes.json",
            5,
            Screen::Mutex(super::kernel_model::MutexKey {
                address: 12288,
                name: "resource_lock".into(),
            }),
        ),
        (
            "semaphores.json",
            12,
            Screen::Semaphore(super::kernel_model::SemaphoreKey {
                address: 8192,
                name: "buffer_tokens".into(),
            }),
        ),
    ] {
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.load(
                    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                        .join("test-data")
                        .join(fixture),
                    window,
                    cx,
                );
                app.frame = frame;
                app.screen = screen;
                cx.notify();
            });
            window.draw(cx).clear(cx);
        });
        assert_missing_notice(&app, &mut cx, true);
        let first = cx.update(|window, cx| window.notifications(cx)[0].entity_id());
        for _ in 0..3 {
            cx.update(|window, cx| {
                app.update(cx, |_, cx| cx.notify());
                window.draw(cx).clear(cx);
            });
            cx.update(|window, cx| {
                let notes = window.notifications(cx);
                assert_eq!(notes.len(), 1);
                assert_eq!(notes[0].entity_id(), first);
            });
        }
        cx.update(|window, cx| {
            app.update(cx, |app, cx| app.step(false, window, cx));
            window.draw(cx).clear(cx);
        });
        assert_missing_notice(&app, &mut cx, false);
        cx.update(|window, cx| {
            app.update(cx, |app, cx| app.step(true, window, cx));
            window.draw(cx).clear(cx);
        });
        assert_missing_notice(&app, &mut cx, true);
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.show_kernel_objects(app.object_kind, window, cx)
            });
            window.draw(cx).clear(cx);
        });
        assert_missing_notice(&app, &mut cx, false);
    }
}

#[gpui_kit::test]
fn enriched_mutex_demo_renders_full_wait_queue_and_charts(cx: &mut TestAppContext) {
    let (app, mut cx) = setup(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.load(
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-data/mutexes.json"),
                window,
                cx,
            );
            assert_eq!(app.frame, 0);
            for _ in 0..7 {
                app.step(true, window, cx);
            }
            app.show_kernel_objects(Some(super::kernel_model::Kind::Mutex), window, cx);
        });
        window.draw(cx).clear(cx);
    });
    let row = cx.debug_bounds("kernel-row-Mutex-12288").unwrap().center();
    cx.simulate_click(row, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("mutex-detail").is_some());
    let first = cx.debug_bounds("mutex-waiter-0").unwrap();
    let second = cx.debug_bounds("mutex-waiter-1").unwrap();
    assert!(second.top() >= first.bottom());
    assert_eq!(second.left(), first.left());
    let queue = cx.debug_bounds("mutex-wait-queue").unwrap();
    let chart = cx.debug_bounds("mutex-history").unwrap();
    assert!(queue.left() >= chart.right());
    assert_eq!(queue.top(), chart.top());
    assert_eq!(queue.bottom(), chart.bottom());
    for selector in [
        "mutex-waiter-0",
        "mutex-waiter-1",
        "mutex-waiter-2",
        "mutex-waiter-3",
        "mutex-waiter-4",
        "mutex-waiter-5",
    ] {
        assert!(cx.debug_bounds(selector).is_some());
    }
    assert!(cx.debug_bounds("mutex-depth-chart").is_some());
    assert!(cx.debug_bounds("mutex-waiting-chart").is_none());
    let plot = cx.debug_bounds("mutex-area-plot").unwrap();
    cx.simulate_event(gpui::MouseMoveEvent {
        position: plot.origin + gpui::point(px(48.) + (plot.size.width - px(60.)) * 0.12, px(100.)),
        ..Default::default()
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("mutex-area-tooltip").is_some());
    assert!(cx.debug_bounds("semaphore-area-tooltip").is_none());
    cx.simulate_resize(size(px(1000.), px(680.)));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let summary = cx.debug_bounds("mutex-summary").unwrap();
    let history = cx.debug_bounds("mutex-history").unwrap();
    assert!(history.top() >= summary.bottom());
    let depth = cx.debug_bounds("mutex-depth-chart").unwrap();
    assert!(depth.size.height > px(0.));
    let plot = cx.debug_bounds("mutex-area-plot").unwrap();
    assert!(plot.size.height >= px(80.), "{plot:?}");
    assert!(history.bottom() <= cx.debug_bounds("session-footer").unwrap().top());
}

#[gpui_kit::test]
fn mutex_detail_navigation_history_and_partial_data(cx: &mut TestAppContext) {
    let (app, mut cx) = setup(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.load(
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-data/mutexes.json"),
                window,
                cx,
            );
            app.frame = 3; // Exercise partial-data navigation independently of demo start.
            app.show_kernel_objects(Some(super::kernel_model::Kind::Mutex), window, cx);
            app.object_search
                .update(cx, |input, cx| input.set_value("resource", window, cx));
        });
        window.draw(cx).clear(cx);
    });
    let row = cx.debug_bounds("kernel-row-Mutex-12288").unwrap().center();
    cx.simulate_click(row, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("mutex-detail").is_some());
    assert!(cx.debug_bounds("mutex-history").is_some());
    let original = app.read_with(&cx, |app, _| app.screen.clone());
    cx.simulate_resize(size(px(1000.), px(680.)));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let summary = cx.debug_bounds("mutex-summary").unwrap();
    let history = cx.debug_bounds("mutex-history").unwrap();
    let waiters = cx.debug_bounds("mutex-wait-queue").unwrap();
    assert!(history.top() >= summary.bottom());
    assert!(waiters.top() >= history.bottom() || waiters.left() >= history.right());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.step(true, window, cx);
            assert_eq!(app.frame, 4);
            assert_eq!(app.screen, original);
        });
        window.draw(cx).clear(cx);
    });
    assert!(cx.debug_bounds("mutex-wait-queue-empty").is_none()); // known empty queue
    cx.update(|window, cx| {
        app.update(cx, |app, cx| app.step(true, window, cx));
        window.draw(cx).clear(cx);
    });
    assert_missing_notice(&app, &mut cx, true);
    cx.simulate_keystrokes("escape");
    app.read_with(&cx, |app, cx| {
        assert_eq!(app.screen, Screen::KernelObjects);
        assert_eq!(app.frame, 5);
        assert_eq!(app.object_kind, Some(super::kernel_model::Kind::Mutex));
        assert_eq!(app.object_search.read(cx).value().as_str(), "resource");
    });
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.step(false, window, cx);
            app.step(false, window, cx);
        });
        window.draw(cx).clear(cx);
    });
    cx.simulate_keystrokes("tab tab enter");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("mutex-detail").is_some());
    let back = cx.debug_bounds("back-to-objects").unwrap().center();
    cx.simulate_click(back, Modifiers::default());
    cx.simulate_keystrokes("tab tab enter");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let breadcrumb = cx.debug_bounds("objects-breadcrumb").unwrap().center();
    cx.simulate_click(breadcrumb, Modifiers::default());
    app.read_with(&cx, |app, _| assert_eq!(app.screen, Screen::KernelObjects));
}

fn setup(cx: &mut TestAppContext) -> (Entity<ThreadsApp>, VisualTestContext) {
    cx.update(|cx| {
        init(cx);
        bind_keys(cx);
        apply_theme(cx);
    });
    let mut app = None;
    let window = cx.open_window(size(px(1480.), px(940.)), |window, cx| {
        let view = cx.new(|cx| ThreadsApp::from_source(None, window, cx));
        app = Some(view.clone());
        Root::new(view, window, cx)
    });
    let mut cx = VisualTestContext::from_window(*window, cx);
    let app = app.unwrap();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let mut frames = vec![app.frames[0].clone(); 4];
            for (i, f) in frames.iter_mut().enumerate() {
                f.elapsed_seconds = Some(i as f64 * 5.);
            }
            app.frames = frames;
            app.frame = 1;
            app.table_focus.focus(window, cx);
            cx.notify();
        });
    });
    (app, cx)
}

fn open_session_menu(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let trigger = cx.debug_bounds("session-menu-trigger").unwrap().center();
    cx.simulate_click(trigger, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("session-menu-source").is_some());
}

fn choose_open_recording(cx: &mut VisualTestContext) {
    open_session_menu(cx);
    // Kit first selects the heading, then skips non-interactive/disabled entries.
    cx.simulate_keystrokes("down down down enter");
}

#[gpui_kit::test]
fn opening_and_reloading_documents_start_paused_at_first_frame(cx: &mut TestAppContext) {
    let (_, mut cx) = setup(cx);
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for file in ["sample.json", "test-data/mutexes.json"] {
        cx.update(|window, cx| {
            let path = root.join(file);
            let view = cx.new(|cx| ThreadsApp::from_source(Some(path.clone()), window, cx));
            view.update(cx, |app, cx| {
                assert!(app.error.is_none());
                assert_eq!(app.frame, 0);
                assert!(!app.playing);
                app.step(true, window, cx);
                app.toggle_playback(window, cx);
                app.load(path, window, cx);
                assert_eq!(app.frame, 0);
                assert!(!app.playing);
                assert!(app.playback_position.is_none());
            });
        });
    }
}

#[gpui_kit::test]
fn recording_prompts_can_be_cancelled_without_changing_session(cx: &mut TestAppContext) {
    let (app, mut cx) = setup(cx);
    choose_open_recording(&mut cx);
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|options| {
        assert!(options.prompt.as_ref().unwrap().contains("recording"));
        None
    });
    cx.run_until_parked();
    assert!(!cx.did_prompt_for_paths());
    choose_open_recording(&mut cx);
    cx.simulate_path_prompt_response(|_| Some(vec!["capture.ndjson.gz".into()]));
    cx.run_until_parked();
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|options| {
        assert!(options.prompt.as_ref().unwrap().contains("ELF"));
        None
    });
    cx.run_until_parked();
    app.read_with(&cx, |app, _| {
        assert!(app.loading.is_none());
        assert!(app.error.is_none());
        assert!(app.source.is_none());
        assert_eq!(app.frame, 1);
    });
}

#[gpui_kit::test]
fn async_import_is_atomic_cancellable_and_recovers_after_failure(cx: &mut TestAppContext) {
    use std::path::PathBuf;
    let (app, mut cx) = setup(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.screen = Screen::KernelObjects;
            app.start_load(PathBuf::from("missing-phase10.json"), None, cx);
        });
    });
    cx.run_until_parked();
    app.read_with(&cx, |app, _| {
        assert!(app.error.is_some());
        assert!(app.loading.is_none());
        assert_eq!(app.frame, 1);
        assert_eq!(app.screen, Screen::KernelObjects);
    });
    let sample = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("sample.json");
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.start_load(sample.clone(), None, cx);
            app.loading
                .as_ref()
                .unwrap()
                .store(true, std::sync::atomic::Ordering::Relaxed);
        });
    });
    cx.run_until_parked();
    app.read_with(&cx, |app, _| {
        assert!(app.loading.is_none());
        assert!(app.source.is_none());
        assert_eq!(app.frame, 1);
        assert_eq!(app.screen, Screen::KernelObjects);
    });
    cx.update(|_, cx| app.update(cx, |app, cx| app.start_load(sample.clone(), None, cx)));
    cx.run_until_parked();
    app.read_with(&cx, |app, _| {
        assert!(app.error.is_none());
        assert_eq!(app.source.as_ref(), Some(&sample));
        assert_eq!(app.screen, Screen::Overview);
        assert!(!app.playing);
    });
}

#[gpui_kit::test]
fn loading_banner_cancel_button_and_small_layout_keep_session_available(cx: &mut TestAppContext) {
    let (app, mut cx) = setup(cx);
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    cx.simulate_resize(size(px(860.), px(700.)));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.loading = Some(cancel.clone());
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    assert!(cx.debug_bounds("session-loading").is_some());
    let button = cx.debug_bounds("cancel-load").unwrap();
    assert!(button.right() <= px(860.));
    cx.simulate_click(button.center(), Modifiers::default());
    assert!(cancel.load(std::sync::atomic::Ordering::Relaxed));
    app.read_with(&cx, |app, _| {
        assert_eq!(app.frame, 1);
        assert!(app.source.is_none());
    });
}

#[gpui_kit::test]
#[ignore = "Requires Python with ZView dependencies; set ZVIEW_PYTHON"]
fn direct_recording_opens_via_prompts_and_reloads_with_its_elf(cx: &mut TestAppContext) {
    let fixture =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-data/zview-bench-60s");
    let (app, mut cx) = setup(cx);
    choose_open_recording(&mut cx);
    cx.simulate_path_prompt_response(|_| Some(vec![fixture.join("capture.ndjson.gz")]));
    cx.run_until_parked();
    cx.simulate_path_prompt_response(|_| Some(vec![fixture.join("zephyr.elf")]));
    cx.run_until_parked();
    app.read_with(&cx, |app, _| {
        assert!(app.error.is_none(), "{:?}", app.error);
        assert_eq!(app.frames.len(), 578);
        assert_eq!(app.frame, 0);
        assert_eq!(app.recording_elf, Some(fixture.join("zephyr.elf")));
        assert!(!app.playing);
        assert!(app.frames[288].heaps[0].chunks.is_some());
    });
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.step(true, window, cx);
        });
    });
    open_session_menu(&mut cx);
    cx.simulate_keystrokes("down down down down enter");
    cx.run_until_parked();
    app.read_with(&cx, |app, _| {
        assert!(app.error.is_none(), "{:?}", app.error);
        assert_eq!(app.frames.len(), 578);
        assert_eq!(app.frame, 0);
    });
    // Failed decoding must not replace the recording or stop its playback.
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.show_kernel_objects(Some(super::kernel_model::Kind::Heap), window, cx);
            app.toggle_playback(window, cx);
            app.start_load(
                fixture.join("missing.ndjson.gz"),
                app.recording_elf.clone(),
                cx,
            );
        });
    });
    cx.run_until_parked();
    app.read_with(&cx, |app, _| {
        assert!(app.error.is_some());
        assert!(app.playing);
        assert_eq!(app.screen, Screen::KernelObjects);
        assert_eq!(app.object_kind, Some(super::kernel_model::Kind::Heap));
        assert_eq!(app.source, Some(fixture.join("capture.ndjson.gz")));
        assert_eq!(app.recording_elf, Some(fixture.join("zephyr.elf")));
    });
}

#[gpui_kit::test]
fn keyboard_opens_detail_and_escape_preserves_session(cx: &mut TestAppContext) {
    let (app, mut cx) = setup(cx);
    cx.simulate_keystrokes("down enter");
    app.read_with(&cx, |app, cx| {
        assert!(matches!(&app.screen, Screen::Thread(key) if key.name == "stress_id"));
        assert_eq!(app.frame, 1);
        assert_eq!(
            app.heap.read(cx).selected_value(),
            Some(&app.frames[1].heaps[0].label())
        );
    });
    cx.simulate_keystrokes("escape");
    app.read_with(&cx, |app, _| {
        assert_eq!(app.screen, Screen::Overview);
        assert_eq!(app.frame, 1);
        assert_eq!(app.selected, Some(536871096));
    });
    // The table regains focus, allowing another thread to be inspected.
    cx.simulate_keystrokes("down enter");
    app.read_with(&cx, |app, _| {
        assert!(matches!(&app.screen, Screen::Thread(key) if key.name == "main"));
        assert_eq!(app.frame, 1);
    });
}

#[gpui_kit::test]
fn stepping_and_missing_thread_keep_detail_identity(cx: &mut TestAppContext) {
    let (app, mut cx) = setup(cx);
    cx.simulate_keystrokes("down enter");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let screen = app.screen.clone();
            app.frames[2]
                .threads
                .retain(|t| Some(t.address) != app.selected);
            app.step(true, window, cx);
            assert_eq!(app.frame, 2);
            assert_eq!(app.screen, screen);
            if let Screen::Thread(key) = &app.screen {
                assert!(key.in_frame(&app.frames[app.frame]).is_none());
            } else {
                panic!("expected thread detail");
            }
        });
    });
    // Rendering the absent-thread view must not prevent navigating back.
    cx.simulate_keystrokes("escape");
    app.read_with(&cx, |app, _| {
        assert_eq!(app.screen, Screen::Overview);
        assert_eq!(app.frame, 2);
    });
}

#[gpui_kit::test]
fn single_click_and_return_controls_render_the_right_screen(cx: &mut TestAppContext) {
    let (app, mut cx) = setup(cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    let row = cx
        .debug_bounds("thread-row-536871096")
        .expect("first row is rendered")
        .center();
    assert!(cx.debug_bounds("threads-breadcrumb").is_none());
    cx.simulate_click(row, Modifiers::default());
    app.read_with(&cx, |app, _| {
        assert!(matches!(&app.screen, Screen::Thread(key) if key.address == 536871096));
        assert_eq!(app.frame, 1);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    assert!(cx.debug_bounds("thread-detail").is_some());
    assert!(cx.debug_bounds("thread-table").is_none());
    assert!(cx.debug_bounds("threads-breadcrumb").is_some());
    assert!(cx.debug_bounds("breadcrumb-current").is_some());
    assert!(cx.debug_bounds("object-type-breadcrumb").is_none());
    app.read_with(&cx, |app, _| {
        assert_eq!(app.screen.name(), Some("stress_id"));
    });
    assert!(cx.debug_bounds("thread-entry-source").is_some());
    for chart in ["thread-cpu", "thread-load"] {
        assert!(
            cx.debug_bounds(chart).is_some(),
            "{chart} should be rendered"
        );
    }
    assert!(cx.debug_bounds("thread-stack").is_none());
    assert!(cx.debug_bounds("thread-summary").is_some());
    let back = cx.debug_bounds("back-to-threads").unwrap().center();
    cx.simulate_click(back, Modifiers::default());
    app.read_with(&cx, |app, _| {
        assert_eq!(app.screen, Screen::Overview);
        assert_eq!(app.frame, 1);
    });
    cx.simulate_keystrokes("enter");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    let breadcrumb = cx.debug_bounds("threads-breadcrumb").unwrap().center();
    cx.simulate_click(breadcrumb, Modifiers::default());
    app.read_with(&cx, |app, _| {
        assert_eq!(app.screen, Screen::Overview);
        assert_eq!(app.frame, 1);
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("breadcrumb-current").is_none());
}

#[gpui_kit::test]
fn breadcrumbs_follow_every_kernel_route_and_type_filter(cx: &mut TestAppContext) {
    use super::kernel_model::Kind;

    let (app, mut cx) = setup(cx);
    open_kernel_objects(&app, &mut cx);
    let cases = [
        (Kind::Heap, "kernel-row-Heap-4096", "heap"),
        (Kind::MemorySlab, "kernel-row-Memory slab-24576", "slab"),
        (
            Kind::MessageQueue,
            "kernel-row-Message queue-16384",
            "messages",
        ),
        (Kind::Mutex, "kernel-row-Mutex-12288", "gate"),
        (Kind::Semaphore, "kernel-row-Semaphore-8192", "gate"),
        (Kind::WorkQueue, "kernel-row-Work queue-28672", "work"),
    ];
    for width in [1480., 1000.] {
        cx.simulate_resize(size(px(width), px(940.)));
        for (kind, row, name) in cases {
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.frame = 0;
                    app.sync_heaps(window, cx);
                    app.show_kernel_objects(None, window, cx);
                });
                window.draw(cx).clear(cx);
            });
            assert!(cx.debug_bounds("breadcrumb-current").is_none());
            assert!(cx.debug_bounds("object-type-breadcrumb").is_none());
            let row_position = cx.debug_bounds(row).unwrap().center();
            cx.simulate_click(row_position, Modifiers::default());
            cx.update(|window, cx| window.draw(cx).clear(cx));
            app.read_with(&cx, |app, _| {
                assert_eq!(app.screen.object_kind(), Some(kind));
                assert_eq!(app.screen.name(), Some(name));
                // The path must use the opened object's type, not the list filter.
                assert_eq!(app.object_kind, None);
            });
            for selector in [
                "objects-breadcrumb",
                "object-type-breadcrumb",
                "breadcrumb-current",
            ] {
                let bounds = cx.debug_bounds(selector).expect("full detail path");
                assert!(bounds.size.width > px(0.));
                assert!(bounds.left() >= px(0.) && bounds.right() <= px(width));
                assert!(bounds.top() >= px(0.) && bounds.bottom() <= px(940.));
            }
            assert!(cx.debug_bounds("threads-breadcrumb").is_none());

            // Keep the route even when a later frame no longer contains the object.
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.object_search
                        .update(cx, |input, cx| input.set_value(name, window, cx));
                    app.step(true, window, cx);
                });
                window.draw(cx).clear(cx);
            });
            assert!(cx.debug_bounds("breadcrumb-current").is_some());
            app.read_with(&cx, |app, _| {
                assert_eq!(app.frame, 1);
                assert_eq!(app.screen.name(), Some(name));
            });
            let heap = app.read_with(&cx, |app, cx| app.heap.read(cx).selected_value().cloned());
            let breadcrumb = cx.debug_bounds("object-type-breadcrumb").unwrap().center();
            cx.simulate_click(breadcrumb, Modifiers::default());
            cx.update(|window, cx| window.draw(cx).clear(cx));
            app.read_with(&cx, |app, cx| {
                assert_eq!(app.screen, Screen::KernelObjects);
                assert_eq!(app.object_kind, Some(kind));
                assert_eq!(app.frame, 1);
                assert_eq!(app.heap.read(cx).selected_value(), heap.as_ref());
                assert!(app.object_search.read(cx).value().is_empty());
            });
            assert!(cx.debug_bounds("object-type-breadcrumb").is_some());
            assert!(cx.debug_bounds("breadcrumb-current").is_none());

            // Kernel objects is also a parent link on every type-filtered list.
            let breadcrumb = cx.debug_bounds("objects-breadcrumb").unwrap().center();
            cx.simulate_click(breadcrumb, Modifiers::default());
            cx.update(|window, cx| window.draw(cx).clear(cx));
            app.read_with(&cx, |app, _| {
                assert_eq!(app.screen, Screen::KernelObjects);
                assert_eq!(app.object_kind, None);
                assert_eq!(app.frame, 1);
            });
            assert!(cx.debug_bounds("objects-breadcrumb").is_none());
            assert!(cx.debug_bounds("object-type-breadcrumb").is_none());

            // Opening from a filtered/searched list must not change the root
            // breadcrumb's destination into that previous list.
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.frame = 0;
                    app.sync_heaps(window, cx);
                    app.show_kernel_objects(Some(kind), window, cx);
                    app.object_search
                        .update(cx, |input, cx| input.set_value(name, window, cx));
                });
                window.draw(cx).clear(cx);
            });
            let row_position = cx.debug_bounds(row).unwrap().center();
            cx.simulate_click(row_position, Modifiers::default());
            cx.update(|window, cx| window.draw(cx).clear(cx));
            let breadcrumb = cx.debug_bounds("objects-breadcrumb").unwrap().center();
            cx.simulate_click(breadcrumb, Modifiers::default());
            cx.update(|window, cx| window.draw(cx).clear(cx));
            app.read_with(&cx, |app, cx| {
                assert_eq!(app.screen, Screen::KernelObjects);
                assert_eq!(app.object_kind, None);
                assert!(app.object_search.read(cx).value().is_empty());
                assert_eq!(app.frame, 0);
            });
            for (_, row, _) in cases {
                assert!(cx.debug_bounds(row).is_some(), "all types must be visible");
            }
        }
    }
}

#[gpui_kit::test]
fn invalid_load_keeps_detail_but_new_document_resets_it(cx: &mut TestAppContext) {
    let (app, mut cx) = setup(cx);
    cx.simulate_keystrokes("down enter");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            let previous = app.screen.clone();
            app.load(root.join("no-such-dump-phase2.json"), window, cx);
            assert!(app.error.is_some());
            assert_eq!(app.screen, previous);
            assert_eq!(app.frame, 1);
            app.load(root.join("sample.json"), window, cx);
            assert!(app.error.is_none());
            assert_eq!(app.screen, Screen::Overview);
            assert_eq!(app.frame, 0);
            assert_eq!(app.selected, None);
        });
    });
}

#[gpui_kit::test]
fn overview_charts_fill_spare_height_and_keep_the_table_clear(cx: &mut TestAppContext) {
    let (app, mut cx) = setup(cx);
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let original = cx.debug_bounds("cpu").unwrap().size.height;
    let plot = cx.debug_bounds("cpu-plot").unwrap().size.height;
    cx.simulate_resize(size(px(1480.), px(1200.)));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let cpu = cx.debug_bounds("cpu").unwrap();
    assert!(
        cpu.size.height >= original + px(100.),
        "charts and the table should share extra window height"
    );
    assert!(cx.debug_bounds("cpu-plot").unwrap().size.height >= plot + px(100.));
    for chart in ["stack", "heap"] {
        assert_eq!(cx.debug_bounds(chart).unwrap().size.height, cpu.size.height);
    }
    let table = cx.debug_bounds("thread-table").unwrap();
    let footer = cx.debug_bounds("session-footer").unwrap();
    assert!(table.top() >= cpu.bottom());
    assert!(footer.top() >= table.bottom());
    assert!(footer.bottom() <= px(1200.));

    cx.simulate_resize(size(px(1000.), px(680.)));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let mut charts = Vec::new();
    for chart in ["cpu", "stack", "heap"] {
        let bounds = cx.debug_bounds(chart).unwrap();
        assert!(bounds.size.height >= px(180.), "{chart}: {bounds:?}");
        charts.push(bounds);
    }
    let table = cx.debug_bounds("thread-table").unwrap();
    assert!(charts.iter().all(|chart| chart.bottom() <= table.top()));

    // Extra rows scroll inside the panel, without changing the screen layout.
    cx.simulate_resize(size(px(1480.), px(940.)));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let frame = &mut app.frames[app.frame];
            let thread = frame.threads[0].clone();
            for i in 0..30 {
                let mut thread = thread.clone();
                thread.address += 100 + i;
                thread.name = format!("worker-{i}");
                frame.threads.push(thread);
            }
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    let cpu = cx.debug_bounds("cpu").unwrap();
    assert_eq!(cpu.size.height, original);
    let table = cx.debug_bounds("thread-table").unwrap();
    assert!(table.top() >= cpu.bottom());
    assert!(table.bottom() <= cx.debug_bounds("session-footer").unwrap().top());
    app.read_with(&cx, |app, _| {
        assert!(app.thread_rows_scroll.max_offset().y > px(0.));
    });
}

#[gpui_kit::test]
fn detail_charts_fill_spare_height_without_overlapping_metadata(cx: &mut TestAppContext) {
    let (app, mut cx) = setup(cx);
    cx.simulate_keystrokes("down enter");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let original = cx.debug_bounds("thread-cpu").unwrap().size.height;
    let plot = cx.debug_bounds("thread-cpu-plot").unwrap().size.height;
    cx.simulate_resize(size(px(1480.), px(1200.)));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let cpu = cx.debug_bounds("thread-cpu").unwrap();
    assert!(cpu.size.height >= original + px(250.));
    assert!(cx.debug_bounds("thread-cpu-plot").unwrap().size.height >= plot + px(250.));
    assert_eq!(
        cx.debug_bounds("thread-load").unwrap().size.height,
        cpu.size.height
    );
    let metadata = cx.debug_bounds("thread-metadata").unwrap();
    let summary = cx.debug_bounds("thread-summary").unwrap();
    assert!(metadata.top() >= summary.bottom());
    let priority = cx.debug_bounds("thread-info-Priority").unwrap();
    let options = cx.debug_bounds("thread-info-Options").unwrap();
    let entry = cx.debug_bounds("thread-entry-source").unwrap();
    assert_eq!(priority.top(), options.top());
    assert_eq!(priority.top(), entry.top());
    assert!(entry.size.width > priority.size.width);
    let footer = cx.debug_bounds("session-footer").unwrap();
    assert!(cpu.top() >= metadata.bottom());
    assert!(footer.top() >= cpu.bottom());
    assert!(footer.bottom() <= px(1200.));

    cx.simulate_resize(size(px(1000.), px(680.)));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let metadata = cx.debug_bounds("thread-metadata").unwrap();
    for chart in ["thread-cpu", "thread-load"] {
        let bounds = cx.debug_bounds(chart).unwrap();
        assert!(bounds.size.height >= px(240.));
        assert!(bounds.top() >= metadata.bottom());
    }
    app.read_with(&cx, |app, _| {
        assert!(matches!(&app.screen, Screen::Thread(key) if key.name == "stress_id"));
        assert_eq!(app.frame, 1);
    });
}

#[gpui_kit::test]
fn percentage_tooltip_is_compact_when_hovering_a_chart(cx: &mut TestAppContext) {
    let (_, mut cx) = setup(cx);
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let plot = cx.debug_bounds("cpu-plot").unwrap();
    cx.simulate_event(gpui::MouseMoveEvent {
        position: plot.origin + gpui::point(px(20.), px(30.)),
        ..Default::default()
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let content = cx
        .debug_bounds("percentage-tooltip-content")
        .expect("tooltip appears on hover");
    // Kit adds 8px padding on each side: 74px of content gives a 90px box.
    assert!(content.size.width <= px(74.));
    assert!(content.size.height > px(20.));
}

fn open_kernel_objects(app: &Entity<ThreadsApp>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.frames =
                super::model::parse(include_str!("../test-data/kernel-objects-minimal.json"))
                    .unwrap();
            app.frame = 0;
            app.sync_heaps(window, cx);
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    let nav = cx
        .debug_bounds("kernel-navigation")
        .expect("sidebar entry")
        .center();
    cx.simulate_click(nav, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

#[gpui_kit::test]
fn kernel_sidebar_search_filter_and_shared_frame_navigation(cx: &mut TestAppContext) {
    let (app, mut cx) = setup(cx);
    open_kernel_objects(&app, &mut cx);
    app.read_with(&cx, |app, _| assert_eq!(app.screen, Screen::KernelObjects));
    assert!(cx.debug_bounds("kernel-objects").is_some());
    assert!(cx.debug_bounds("thread-table").is_none());
    let aggregate = cx.debug_bounds("kernel-summary").expect("aggregate row");
    assert!(aggregate.bottom() <= cx.debug_bounds("kernel-row-Heap-4096").unwrap().top());
    for selector in [
        "kernel-summary-label",
        "kernel-summary-state",
        "kernel-summary-waiters",
    ] {
        assert!(cx.debug_bounds(selector).is_some());
    }
    for selector in [
        "kernel-row-Heap-4096",
        "kernel-row-Semaphore-8192",
        "kernel-row-Mutex-12288",
        "kernel-row-Message queue-16384",
        "kernel-row-Memory slab-24576",
        "kernel-row-Work queue-28672",
    ] {
        assert!(cx.debug_bounds(selector).is_some());
    }
    // The sidebar focuses the search input; real keystrokes must filter the UI.
    cx.simulate_keystrokes("g a t e");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("kernel-row-Mutex-12288").is_some());
    assert!(cx.debug_bounds("kernel-row-Semaphore-8192").is_some());
    assert!(cx.debug_bounds("kernel-row-Heap-4096").is_none());
    assert!(cx.debug_bounds("kernel-summary").is_some());
    let mutex_menu = cx.debug_bounds("object-type-Mutex").unwrap().center();
    cx.simulate_click(mutex_menu, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    app.read_with(&cx, |app, _| {
        assert_eq!(app.object_kind, Some(super::kernel_model::Kind::Mutex));
    });
    assert!(cx.debug_bounds("kernel-row-Mutex-12288").is_some());
    assert!(cx.debug_bounds("kernel-row-Semaphore-8192").is_none());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| app.step(true, window, cx));
        window.draw(cx).clear(cx);
    });
    assert!(cx.debug_bounds("kernel-empty").is_some());
    assert!(cx.debug_bounds("kernel-row-Mutex-12288").is_none());
    assert!(cx.debug_bounds("kernel-summary").is_some());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            assert_eq!(app.frame, 1);
            assert_eq!(app.object_search.read(cx).value().as_str(), "gate");
            app.show_overview(window, cx);
            assert_eq!(app.frame, 1);
            app.screen = Screen::KernelObjects;
            assert_eq!(app.object_search.read(cx).value().as_str(), "gate");
            // Invalid load must preserve the current view, time, and filters.
            app.load(
                std::path::PathBuf::from("no-such-phase3-dump.json"),
                window,
                cx,
            );
            assert_eq!(app.screen, Screen::KernelObjects);
            assert_eq!(app.frame, 1);
            assert!(app.error.is_some());
        });
        window.draw(cx).clear(cx);
    });
    let clear = cx.debug_bounds("clear-object-filters").unwrap().center();
    cx.simulate_click(clear, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("kernel-empty").is_none());
    assert!(cx.debug_bounds("kernel-row-Semaphore-8192").is_some());
    app.read_with(&cx, |app, cx| {
        assert_eq!(app.frame, 1);
        assert!(app.object_search.read(cx).value().is_empty());
        assert_eq!(app.object_kind, None);
    });
}

#[gpui_kit::test]
fn kernel_table_scrolls_at_small_sizes_and_handles_an_empty_snapshot(cx: &mut TestAppContext) {
    let (app, mut cx) = setup(cx);
    open_kernel_objects(&app, &mut cx);
    cx.simulate_resize(size(px(1000.), px(680.)));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let table = cx.debug_bounds("kernel-table").unwrap();
    let row = cx.debug_bounds("kernel-row-Heap-4096").unwrap();
    // The 1040px table includes its 1px border on both sides.
    assert!(
        row.size.width >= px(1038.),
        "row width: {:?}",
        row.size.width
    );
    // Kit's scroll wrapper reports content bounds for kernel-table. The
    // surrounding browser is the constrained viewport.
    let viewport = cx.debug_bounds("kernel-objects").unwrap();
    assert!(viewport.size.width < row.size.width);
    assert!(cx.debug_bounds("object-type-tabs").is_none());
    let search = cx.debug_bounds("object-search").unwrap();
    let toolbar = cx.debug_bounds("kernel-table-toolbar").unwrap();
    assert!(search.top() >= toolbar.top());
    assert!(search.bottom() <= toolbar.bottom());
    assert!(search.left() >= viewport.left());
    assert!(search.right() <= viewport.right());
    assert!(toolbar.bottom() <= table.top());
    let aggregate = cx.debug_bounds("kernel-summary").unwrap();
    assert_eq!(aggregate.size.width, row.size.width);
    assert!(aggregate.bottom() <= row.top());
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: row.origin + gpui::point(px(40.), px(15.)),
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(-100.), px(0.))),
        modifiers: Modifiers::default(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("kernel-row-Heap-4096").unwrap().left() < row.left());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.frames[0].heaps.clear();
            app.frames[0].semaphores.clear();
            app.frames[0].mutexes.clear();
            app.frames[0].msgqs.clear();
            app.frames[0].mem_slabs.clear();
            app.frames[0].workqs.clear();
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    assert!(cx.debug_bounds("kernel-empty").is_some());
    assert!(cx.debug_bounds("kernel-row-Heap-4096").is_none());
    assert!(cx.debug_bounds("kernel-summary").is_some());
}

#[gpui_kit::test]
fn sidebar_type_choices_filter_without_changing_session(cx: &mut TestAppContext) {
    use super::kernel_model::Kind;
    let (app, mut cx) = setup(cx);
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(
        cx.debug_bounds("object-type-all").is_none(),
        "submenu starts closed"
    );
    open_kernel_objects(&app, &mut cx);
    assert!(cx.debug_bounds("object-type-tabs").is_none());
    let parent = cx.debug_bounds("kernel-navigation").unwrap();
    let all = cx.debug_bounds("object-type-all").unwrap();
    assert!(all.top() >= parent.bottom());
    let toolbar = cx.debug_bounds("kernel-table-toolbar").unwrap();
    let search = cx.debug_bounds("object-search").unwrap();
    let clear = cx.debug_bounds("clear-object-filters").unwrap();
    assert_eq!(search.top(), clear.top());
    assert!(search.right() <= clear.left());
    assert!(toolbar.bottom() <= cx.debug_bounds("kernel-table").unwrap().top());
    let heap = app.read_with(&cx, |app, cx| app.heap.read(cx).selected_value().cloned());
    for (selector, kind, row) in [
        ("object-type-Heap", Kind::Heap, "kernel-row-Heap-4096"),
        (
            "object-type-Memory slab",
            Kind::MemorySlab,
            "kernel-row-Memory slab-24576",
        ),
        (
            "object-type-Message queue",
            Kind::MessageQueue,
            "kernel-row-Message queue-16384",
        ),
        ("object-type-Mutex", Kind::Mutex, "kernel-row-Mutex-12288"),
        (
            "object-type-Semaphore",
            Kind::Semaphore,
            "kernel-row-Semaphore-8192",
        ),
        (
            "object-type-Work queue",
            Kind::WorkQueue,
            "kernel-row-Work queue-28672",
        ),
    ] {
        let entry = cx.debug_bounds(selector).expect("type submenu entry");
        assert!(entry.right() < cx.debug_bounds("kernel-objects").unwrap().left());
        cx.simulate_click(entry.center(), Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        app.read_with(&cx, |app, cx| {
            assert_eq!(app.object_kind, Some(kind));
            assert_eq!(app.frame, 0);
            assert_eq!(app.heap.read(cx).selected_value(), heap.as_ref());
        });
        assert!(cx.debug_bounds(row).is_some());
        assert!(cx.debug_bounds("kernel-summary").is_some());
        assert_eq!(
            cx.debug_bounds("kernel-row-Heap-4096").is_some(),
            kind == Kind::Heap
        );
        assert_eq!(
            cx.debug_bounds("kernel-row-Semaphore-8192").is_some(),
            kind == Kind::Semaphore
        );
    }
    let all = cx.debug_bounds("object-type-all").unwrap().center();
    cx.simulate_click(all, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    app.read_with(&cx, |app, _| assert_eq!(app.object_kind, None));
    assert!(cx.debug_bounds("kernel-row-Heap-4096").is_some());
    assert!(cx.debug_bounds("kernel-row-Semaphore-8192").is_some());
}

#[gpui_kit::test]
fn sidebar_type_selection_survives_collapse_navigation_and_empty_types(cx: &mut TestAppContext) {
    use super::kernel_model::Kind;
    let (app, mut cx) = setup(cx);
    open_kernel_objects(&app, &mut cx);
    let work = cx.debug_bounds("object-type-Work queue").unwrap().center();
    cx.simulate_click(work, Modifiers::default());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| app.show_overview(window, cx));
        window.draw(cx).clear(cx);
    });
    let parent = cx.debug_bounds("kernel-navigation").unwrap().center();
    cx.simulate_click(parent, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    app.read_with(&cx, |app, _| {
        assert_eq!(app.screen, Screen::KernelObjects);
    });
    assert!(cx.debug_bounds("object-type-Work queue").is_some());
    app.read_with(&cx, |app, _| {
        assert_eq!(app.object_kind, Some(Kind::WorkQueue));
        assert_eq!(app.frame, 0);
    });
    cx.update(|window, cx| {
        app.update(cx, |app, cx| app.step(true, window, cx));
        window.draw(cx).clear(cx);
    });
    assert!(cx.debug_bounds("kernel-empty").is_some());
    // Empty types stay available so a later frame can populate them.
    assert!(cx.debug_bounds("object-type-Work queue").is_some());
    let all = cx.debug_bounds("object-type-all").unwrap().center();
    cx.simulate_click(all, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("kernel-row-Semaphore-8192").is_some());
    app.read_with(&cx, |app, _| {
        assert_eq!(app.object_kind, None);
        assert_eq!(app.frame, 1);
    });
    let work = cx.debug_bounds("object-type-Work queue").unwrap().center();
    cx.simulate_click(work, Modifiers::default());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.collapsed = true;
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    assert!(cx.debug_bounds("object-type-Work queue").is_none());
    assert!(cx.debug_bounds("kernel-empty").is_some());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.collapsed = false;
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    assert!(cx.debug_bounds("object-type-Work queue").is_some());
    app.read_with(&cx, |app, _| {
        assert_eq!(app.object_kind, Some(Kind::WorkQueue));
        assert_eq!(app.frame, 1);
    });
}

#[gpui_kit::test]
fn numeric_charts_render_extremes_gaps_and_singletons_in_both_screens(cx: &mut TestAppContext) {
    let (app, mut cx) = setup(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.frames.push(app.frames[0].clone());
            for ((frame, time), value) in app
                .frames
                .iter_mut()
                .zip([0., 0.001, 2.75, 7., 15.])
                .zip([Some(0.), Some(100.), None, Some(0.), Some(100.)])
            {
                frame.elapsed_seconds = Some(time);
                for thread in &mut frame.threads {
                    let runtime = thread.runtime.as_mut().unwrap();
                    runtime.cpu = value;
                    runtime.cpu_normalized = value;
                }
            }
            app.frame = 3;
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    let plot = cx.debug_bounds("cpu-plot").unwrap();
    // The selected time is 7/15 seconds; the right side must stay empty.
    cx.simulate_event(gpui::MouseMoveEvent {
        position: plot.origin + gpui::point(plot.size.width * 0.9, px(20.)),
        ..Default::default()
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("percentage-tooltip-content").is_none());
    cx.simulate_keystrokes("down enter");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("thread-cpu-plot").is_some());
    assert!(cx.debug_bounds("thread-load-plot").is_some());
    cx.simulate_resize(size(px(1000.), px(680.)));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    for plot in ["thread-cpu-plot", "thread-load-plot"] {
        assert!(cx.debug_bounds(plot).unwrap().size.height > px(0.));
    }
}

fn open_slab(app: &Entity<ThreadsApp>, cx: &mut VisualTestContext) {
    open_kernel_objects(app, cx);
    cx.simulate_keystrokes("s l a b");
    let kind = cx.debug_bounds("object-type-Memory slab").unwrap().center();
    cx.simulate_click(kind, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let row = cx
        .debug_bounds("kernel-row-Memory slab-24576")
        .unwrap()
        .center();
    cx.simulate_click(row, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("slab-detail").is_some());
}

#[gpui_kit::test]
fn slab_detail_navigation_preserves_session_and_filters(cx: &mut TestAppContext) {
    let (app, mut cx) = setup(cx);
    open_slab(&app, &mut cx);
    let heap = app.read_with(&cx, |app, cx| app.heap.read(cx).selected_value().cloned());
    assert!(cx.debug_bounds("slab-usage-plot").is_some());
    assert!(cx.debug_bounds("slab-wait-queue-empty").is_none());
    let next = cx.debug_bounds("next").unwrap().center();
    cx.simulate_click(next, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("slab-absent").is_some());
    assert!(cx.debug_bounds("slab-capacity-track").unwrap().size.height >= px(6.));
    assert!(cx.debug_bounds("slab-capacity-unavailable").is_some());
    app.read_with(&cx, |app, _| {
        assert_eq!(app.frame, 1);
        assert!(matches!(&app.screen, Screen::MemorySlab(key) if key.address == 24576));
    });
    let previous = cx.debug_bounds("previous").unwrap().center();
    cx.simulate_click(previous, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("slab-absent").is_none());
    app.read_with(&cx, |app, cx| {
        assert_eq!(app.heap.read(cx).selected_value(), heap.as_ref())
    });
    for control in ["back-to-objects", "objects-breadcrumb"] {
        let back = cx.debug_bounds(control).unwrap().center();
        cx.simulate_click(back, Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        app.read_with(&cx, |app, cx| {
            assert_eq!(app.screen, Screen::KernelObjects);
            if control == "back-to-objects" {
                assert_eq!(app.object_kind, Some(super::kernel_model::Kind::MemorySlab));
                assert_eq!(app.object_search.read(cx).value().as_str(), "slab");
            } else {
                assert_eq!(app.object_kind, None);
                assert!(app.object_search.read(cx).value().is_empty());
            }
            assert_eq!(app.frame, 0);
        });
        let row = cx
            .debug_bounds("kernel-row-Memory slab-24576")
            .unwrap()
            .center();
        cx.simulate_click(row, Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }
    cx.simulate_keystrokes("escape");
    app.read_with(&cx, |app, _| assert_eq!(app.screen, Screen::KernelObjects));
}

#[gpui_kit::test]
fn slab_keyboard_partial_fields_and_narrow_layout(cx: &mut TestAppContext) {
    let (app, mut cx) = setup(cx);
    open_kernel_objects(&app, &mut cx);
    let kind = cx.debug_bounds("object-type-Memory slab").unwrap().center();
    cx.simulate_click(kind, Modifiers::default());
    cx.simulate_keystrokes("tab tab enter");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("slab-detail").is_some());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let slab = &mut app.frames[0].mem_slabs[0];
            slab.num_used = None;
            slab.num_blocks = None;
            slab.max_used = None;
            slab.block_size = None;
            slab.object.waiters = None;
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    cx.simulate_resize(size(px(1000.), px(680.)));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let history = cx.debug_bounds("slab-history").unwrap();
    let summary = cx.debug_bounds("slab-summary").unwrap();
    let waiters = cx.debug_bounds("slab-wait-queue").unwrap();
    assert!(history.top() >= summary.bottom());
    assert!(waiters.top() >= history.bottom() || waiters.left() >= history.right());
    assert!(cx.debug_bounds("slab-usage-plot").unwrap().size.height >= px(80.));
    assert!(cx.debug_bounds("slab-wait-queue-empty").is_some());
    assert!(cx.debug_bounds("slab-absent").is_none());
    cx.simulate_keystrokes("escape");
    app.read_with(&cx, |app, _| assert_eq!(app.screen, Screen::KernelObjects));
}

#[gpui_kit::test]
fn slab_capacity_track_remains_visible_for_zero_missing_and_zero_capacity(cx: &mut TestAppContext) {
    let (app, mut cx) = setup(cx);
    open_slab(&app, &mut cx);
    for (used, capacity, known) in [
        (Some(0), Some(8), true),
        (None, Some(8), false),
        (Some(2), None, false),
        (None, None, false),
        (Some(0), Some(0), false),
        (Some(8), Some(8), true),
    ] {
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                let slab = &mut app.frames[0].mem_slabs[0];
                slab.num_used = used;
                slab.num_blocks = capacity;
                cx.notify();
            });
            window.draw(cx).clear(cx);
        });
        let track = cx
            .debug_bounds("slab-capacity-track")
            .expect("capacity track");
        assert!(track.size.height >= px(6.));
        assert!(track.size.width > px(0.));
        assert_eq!(cx.debug_bounds("slab-capacity-known").is_some(), known);
        assert_eq!(
            cx.debug_bounds("slab-capacity-unavailable").is_some(),
            !known
        );
    }
}

fn open_semaphore(app: &Entity<ThreadsApp>, cx: &mut VisualTestContext) {
    open_kernel_objects(app, cx);
    cx.simulate_keystrokes("g a t e");
    let kind = cx.debug_bounds("object-type-Semaphore").unwrap().center();
    cx.simulate_click(kind, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let row = cx
        .debug_bounds("kernel-row-Semaphore-8192")
        .unwrap()
        .center();
    cx.simulate_click(row, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("semaphore-detail").is_some());
}

#[gpui_kit::test]
fn semaphore_area_demo_keeps_summary_wait_queue_and_responsive_layout(cx: &mut TestAppContext) {
    let (app, mut cx) = setup(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.load(
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("test-data/semaphores.json"),
                window,
                cx,
            );
            assert_eq!(app.frame, 0);
            for _ in 0..8 {
                app.step(true, window, cx);
            }
            app.show_kernel_objects(Some(super::kernel_model::Kind::Semaphore), window, cx);
        });
        window.draw(cx).clear(cx);
    });
    let row = cx
        .debug_bounds("kernel-row-Semaphore-8192")
        .unwrap()
        .center();
    cx.simulate_click(row, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("semaphore-summary").is_some());
    for selector in [
        "semaphore-waiter-0",
        "semaphore-waiter-1",
        "semaphore-waiter-2",
        "semaphore-waiter-3",
    ] {
        assert!(cx.debug_bounds(selector).is_some());
    }
    let a = cx.debug_bounds("semaphore-waiter-0").unwrap();
    let b = cx.debug_bounds("semaphore-waiter-1").unwrap();
    assert_eq!(a.left(), b.left());
    assert!(b.top() >= a.bottom());
    let history = cx.debug_bounds("semaphore-history").unwrap();
    let waiters = cx.debug_bounds("semaphore-wait-queue").unwrap();
    assert!(waiters.left() >= history.right());
    cx.simulate_resize(size(px(1000.), px(680.)));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let history = cx.debug_bounds("semaphore-history").unwrap();
    let summary = cx.debug_bounds("semaphore-summary").unwrap();
    let waiters = cx.debug_bounds("semaphore-wait-queue").unwrap();
    assert!(history.top() >= summary.bottom());
    assert!(waiters.top() >= history.bottom() || waiters.left() >= history.right());
    assert!(cx.debug_bounds("semaphore-count-plot").unwrap().size.height >= px(80.));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            for _ in 0..5 {
                app.step(true, window, cx);
            }
            assert_eq!(app.frame, 13);
            // Visit the absent frame while keeping the same inspected identity.
            app.step(false, window, cx);
        });
        window.draw(cx).clear(cx);
    });
    assert_missing_notice(&app, &mut cx, true);
    cx.simulate_keystrokes("escape");
    app.read_with(&cx, |app, _| {
        assert_eq!(app.screen, Screen::KernelObjects);
        assert_eq!(app.frame, 12);
        assert_eq!(app.object_kind, Some(super::kernel_model::Kind::Semaphore));
    });
}

#[gpui_kit::test]
fn semaphore_detail_navigation_preserves_filters_frame_and_heap(cx: &mut TestAppContext) {
    let (app, mut cx) = setup(cx);
    open_semaphore(&app, &mut cx);
    let heap = app.read_with(&cx, |app, cx| app.heap.read(cx).selected_value().cloned());
    assert!(cx.debug_bounds("semaphore-count-plot").is_some());
    assert!(cx.debug_bounds("semaphore-wait-queue").is_some());
    assert!(cx.debug_bounds("kernel-table").is_none());
    let next = cx.debug_bounds("next").unwrap().center();
    cx.simulate_click(next, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    app.read_with(&cx, |app, _| {
        assert_eq!(app.frame, 1);
        let Screen::Semaphore(key) = &app.screen else {
            panic!("expected semaphore")
        };
        let sem = key.in_frame(&app.frames[app.frame]).unwrap();
        assert_eq!(sem.count, Some(2));
        assert_eq!(sem.object.waiters, Some(vec![]));
    });
    assert!(cx.debug_bounds("semaphore-wait-queue-empty").is_none());
    let previous = cx.debug_bounds("previous").unwrap().center();
    cx.simulate_click(previous, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    app.read_with(&cx, |app, cx| {
        assert_eq!(app.frame, 0);
        assert_eq!(app.heap.read(cx).selected_value(), heap.as_ref());
    });
    let back = cx.debug_bounds("back-to-objects").unwrap().center();
    cx.simulate_click(back, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    app.read_with(&cx, |app, cx| {
        assert_eq!(app.screen, Screen::KernelObjects);
        assert_eq!(app.object_kind, Some(super::kernel_model::Kind::Semaphore));
        assert_eq!(app.object_search.read(cx).value().as_str(), "gate");
        assert_eq!(app.frame, 0);
    });
    let row = cx
        .debug_bounds("kernel-row-Semaphore-8192")
        .unwrap()
        .center();
    cx.simulate_click(row, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let breadcrumb = cx.debug_bounds("objects-breadcrumb").unwrap().center();
    cx.simulate_click(breadcrumb, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    app.read_with(&cx, |app, _| assert_eq!(app.screen, Screen::KernelObjects));
    let row = cx
        .debug_bounds("kernel-row-Semaphore-8192")
        .unwrap()
        .center();
    cx.simulate_click(row, Modifiers::default());
    cx.simulate_keystrokes("escape");
    app.read_with(&cx, |app, _| assert_eq!(app.screen, Screen::KernelObjects));
}

#[gpui_kit::test]
fn semaphore_missing_partial_and_small_layout_remain_navigable(cx: &mut TestAppContext) {
    let (app, mut cx) = setup(cx);
    open_semaphore(&app, &mut cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.frames[1].semaphores.clear();
            app.step(true, window, cx);
        });
        window.draw(cx).clear(cx);
    });
    assert_missing_notice(&app, &mut cx, true);
    assert!(cx.debug_bounds("semaphore-wait-queue-empty").is_some());
    cx.simulate_resize(size(px(1000.), px(680.)));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let history = cx.debug_bounds("semaphore-history").unwrap();
    let summary = cx.debug_bounds("semaphore-summary").unwrap();
    let waiters = cx.debug_bounds("semaphore-wait-queue").unwrap();
    assert!(history.top() >= summary.bottom());
    assert!(waiters.top() >= history.bottom() || waiters.left() >= history.right());
    assert!(cx.debug_bounds("semaphore-count-plot").unwrap().size.height >= px(180.));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let previous = app.screen.clone();
            app.load(std::path::PathBuf::from("no-phase4-dump.json"), window, cx);
            assert_eq!(app.screen, previous);
            assert_eq!(app.frame, 1);
            app.step(false, window, cx);
            let sem = &mut app.frames[0].semaphores[0];
            sem.count = None;
            sem.limit = None;
            sem.object.waiters = None;
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    assert_missing_notice(&app, &mut cx, false);
    assert!(cx.debug_bounds("semaphore-wait-queue-empty").is_some());
    cx.simulate_keystrokes("escape");
    app.read_with(&cx, |app, _| assert_eq!(app.screen, Screen::KernelObjects));
}

#[gpui_kit::test]
fn semaphore_row_opens_with_keyboard(cx: &mut TestAppContext) {
    let (app, mut cx) = setup(cx);
    open_kernel_objects(&app, &mut cx);
    let kind = cx.debug_bounds("object-type-Semaphore").unwrap().center();
    cx.simulate_click(kind, Modifiers::default());
    // Type selection focuses search; Tab passes Clear filters, then the row.
    cx.simulate_keystrokes("tab tab enter");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    app.read_with(&cx, |app, _| {
        assert!(matches!(app.screen, Screen::Semaphore(_)))
    });
    assert!(cx.debug_bounds("semaphore-detail").is_some());
    cx.simulate_keystrokes("escape");
    app.read_with(&cx, |app, _| assert_eq!(app.screen, Screen::KernelObjects));
}

#[gpui_kit::test]
fn message_queue_detail_click_keyboard_and_return_preserve_session(cx: &mut TestAppContext) {
    let (app, mut cx) = setup(cx);
    open_kernel_objects(&app, &mut cx);
    let kind = cx
        .debug_bounds("object-type-Message queue")
        .unwrap()
        .center();
    cx.simulate_click(kind, Modifiers::default());
    cx.simulate_keystrokes("m e s s");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let row = cx
        .debug_bounds("kernel-row-Message queue-16384")
        .unwrap()
        .center();
    cx.simulate_click(row, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("msgq-detail").is_some());
    assert!(cx.debug_bounds("msgq-depth-plot").is_some());
    assert!(cx.debug_bounds("msgq-wait-queue-empty").is_none());
    let next = cx.debug_bounds("next").unwrap().center();
    cx.simulate_click(next, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("msgq-absent").is_some());
    app.read_with(&cx, |app, _| {
        assert_eq!(app.frame, 1);
        assert!(matches!(&app.screen, Screen::MessageQueue(key) if key.address == 16384));
    });
    cx.simulate_keystrokes("escape");
    app.read_with(&cx, |app, cx| {
        assert_eq!(app.screen, Screen::KernelObjects);
        assert_eq!(
            app.object_kind,
            Some(super::kernel_model::Kind::MessageQueue)
        );
        assert_eq!(app.object_search.read(cx).value().as_str(), "mess");
        assert_eq!(app.frame, 1);
    });
    cx.update(|window, cx| {
        app.update(cx, |app, cx| app.step(false, window, cx));
        window.draw(cx).clear(cx);
    });
    cx.simulate_keystrokes("tab tab enter");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("msgq-detail").is_some());
    let back = cx.debug_bounds("back-to-objects").unwrap().center();
    cx.simulate_click(back, Modifiers::default());
    cx.simulate_keystrokes("tab tab enter");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("msgq-detail").is_some());
    let breadcrumb = cx.debug_bounds("objects-breadcrumb").unwrap().center();
    cx.simulate_click(breadcrumb, Modifiers::default());
    app.read_with(&cx, |app, _| assert_eq!(app.screen, Screen::KernelObjects));
}

#[gpui_kit::test]
fn enriched_demo_opens_with_three_queues_and_depth_history(cx: &mut TestAppContext) {
    let (app, mut cx) = setup(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.load(
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("test-data/kernel-objects.json"),
                window,
                cx,
            );
            assert!(app.error.is_none());
            assert_eq!(app.frames.len(), 7);
            assert_eq!(app.frame, 0);
            for _ in 0..3 {
                app.step(true, window, cx);
            }
            assert_eq!(app.frames[app.frame].elapsed_seconds, Some(6.));
            app.show_kernel_objects(Some(super::kernel_model::Kind::MessageQueue), window, cx);
        });
        window.draw(cx).clear(cx);
    });
    for selector in [
        "kernel-row-Message queue-16384",
        "kernel-row-Message queue-16640",
        "kernel-row-Message queue-16896",
    ] {
        assert!(cx.debug_bounds(selector).is_some());
    }
    let row = cx
        .debug_bounds("kernel-row-Message queue-16384")
        .unwrap()
        .center();
    cx.simulate_click(row, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("msgq-depth-plot").is_some());
    app.read_with(&cx, |app, _| {
        let Screen::MessageQueue(key) = &app.screen else {
            panic!("expected queue")
        };
        let series = super::model::chart_series(&app.frames, app.frame, |frame| {
            key.in_frame(frame)?.used_msgs.map(|value| value as f64)
        });
        assert_eq!(
            series.points.iter().map(|p| p.value).collect::<Vec<_>>(),
            vec![Some(0.), Some(2.), Some(5.), Some(8.)]
        );
        assert_eq!(
            key.in_frame(&app.frames[app.frame])
                .unwrap()
                .object
                .waiters
                .as_ref()
                .unwrap()
                .len(),
            2
        );
    });
}

#[gpui_kit::test]
fn message_queue_partial_data_and_small_layout_render_without_stale_values(
    cx: &mut TestAppContext,
) {
    let (app, mut cx) = setup(cx);
    open_kernel_objects(&app, &mut cx);
    let row = cx
        .debug_bounds("kernel-row-Message queue-16384")
        .unwrap()
        .center();
    cx.simulate_click(row, Modifiers::default());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let queue = &mut app.frames[0].msgqs[0];
            queue.used_msgs = None;
            queue.max_msgs = None;
            queue.msg_size = None;
            queue.object.waiters = None;
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    assert!(cx.debug_bounds("msgq-absent").is_none());
    assert!(cx.debug_bounds("msgq-wait-queue-empty").is_some());
    cx.simulate_resize(size(px(1000.), px(680.)));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let history = cx.debug_bounds("msgq-history").unwrap();
    let summary = cx.debug_bounds("msgq-summary").unwrap();
    let waiters = cx.debug_bounds("msgq-wait-queue").unwrap();
    assert!(history.top() >= summary.bottom());
    assert!(waiters.top() >= history.bottom() || waiters.left() >= history.right());
    assert!(cx.debug_bounds("msgq-depth-plot").unwrap().size.height >= px(180.));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let before = app.screen.clone();
            app.load(std::path::PathBuf::from("missing-phase5.json"), window, cx);
            assert_eq!(app.screen, before);
            app.load(
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("sample.json"),
                window,
                cx,
            );
            assert_eq!(app.screen, Screen::Overview);
        });
    });
}

use super::{Screen, setup};
use crate::model::{self, HeapChunk, HeapKey};
use gpui::{Modifiers, MouseButton, TestAppContext, VisualTestContext, point, px, size};

fn click(cx: &mut VisualTestContext, id: &'static str) {
    let p = cx.debug_bounds(id).expect(id).center();
    cx.simulate_click(p, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

#[gpui_kit::test]
fn single_line_map_icons_zoom_pan_select_fit_and_preserve_session(cx: &mut TestAppContext) {
    let (app, mut cx) = setup(cx);
    cx.simulate_resize(size(px(1000.), px(680.)));
    let key = HeapKey {
        address: 4096,
        name: "packet_heap".into(),
    };
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let mut frames = model::parse(include_str!("../test-data/heaps.json")).unwrap();
            let blocks = (0..64)
                .map(|i| HeapChunk {
                    size: 16,
                    used: i % 2 == 0,
                })
                .collect::<Vec<_>>();
            frames[0].heaps[0].chunks = Some(blocks.clone());
            frames[0].heaps[1].chunks = Some(blocks);
            frames[1] = frames[0].clone();
            frames[1].elapsed_seconds = Some(1.);
            app.frames = frames;
            app.frame = 0;
            app.screen = Screen::Heap(key.clone());
            app.sync_heaps(window, cx);
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    assert!(cx.debug_bounds("heap-map-view-0-1024").is_some());
    let summary = cx.debug_bounds("heap-summary").unwrap();
    let map = cx.debug_bounds("heap-map").unwrap();
    assert_eq!(map.size.width, summary.size.width);
    let footer = cx.debug_bounds("session-footer").unwrap();
    let allocated = cx.debug_bounds("heap-history").unwrap();
    let fragmentation = cx.debug_bounds("heap-fragmentation-history").unwrap();
    let waiters = cx.debug_bounds("heap-wait-queue").unwrap();
    assert!(allocated.top() >= map.bottom());
    assert!(fragmentation.left() >= allocated.right() && waiters.left() >= fragmentation.right());
    assert!(waiters.bottom() <= footer.top());

    click(&mut cx, "heap-map-zoom-in");
    assert!(cx.debug_bounds("heap-map-view-256-768").is_some());
    let strip = cx.debug_bounds("heap-map-strip").unwrap();
    cx.simulate_event(gpui::MouseMoveEvent {
        position: strip.center(),
        ..Default::default()
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("heap-chunk-tooltip").is_some());
    cx.simulate_click(strip.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("heap-map-selected-32").is_some());
    cx.simulate_keystrokes("right");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("heap-map-view-307-819").is_some());
    cx.simulate_keystrokes("home");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("heap-map-view-0-512").is_some());
    let position = strip.center();
    cx.simulate_event(gpui::MouseDownEvent {
        button: MouseButton::Left,
        position,
        ..Default::default()
    });
    cx.simulate_event(gpui::MouseMoveEvent {
        position: position - point(px(64.), px(0.)),
        pressed_button: Some(MouseButton::Left),
        ..Default::default()
    });
    cx.simulate_event(gpui::MouseUpEvent {
        button: MouseButton::Left,
        position: position - point(px(64.), px(0.)),
        ..Default::default()
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let start = (64. / (strip.size.width.as_f32() - 2.) * 512.).round() as u64;
    let marker = Box::leak(format!("heap-map-view-{start}-{}", start + 512).into_boxed_str());
    assert!(cx.debug_bounds(marker).is_some());
    click(&mut cx, "heap-map-zoom-out");
    assert!(cx.debug_bounds("heap-map-view-0-1024").is_some());
    click(&mut cx, "heap-map-zoom-in");
    click(&mut cx, "heap-map-fit");
    assert!(cx.debug_bounds("heap-map-view-0-1024").is_some());
    assert_eq!(cx.debug_bounds("session-footer").unwrap(), footer);
    app.read_with(&cx, |app, _| {
        assert_eq!(app.frame, 0);
        assert!(!app.playing);
    });

    click(&mut cx, "heap-map-zoom-in");
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: strip.center(),
        delta: gpui::ScrollDelta::Pixels(point(px(-40.), px(0.))),
        modifiers: Modifiers::default(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let start = 256 + (40. / (strip.size.width.as_f32() - 2.) * 512.).round() as u64;
    let marker = Box::leak(format!("heap-map-view-{start}-{}", start + 512).into_boxed_str());
    assert!(cx.debug_bounds(marker).is_some());
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: strip.center(),
        delta: gpui::ScrollDelta::Pixels(point(px(0.), px(100.))),
        modifiers: Modifiers::default(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(
        cx.debug_bounds(marker).is_none(),
        "vertical scrolling zooms the map"
    );
    click(&mut cx, "heap-map-fit");
    click(&mut cx, "heap-map-zoom-in");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.screen = Screen::Heap(HeapKey {
                address: 8192,
                name: key.name.clone(),
            });
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    assert!(
        cx.debug_bounds("heap-map-view-0-1024").is_some(),
        "different heaps have independent spatial zoom"
    );
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.screen = Screen::Heap(key);
            app.frame = 1;
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    assert!(cx.debug_bounds("heap-map-view-256-768").is_some());
    assert!(cx.debug_bounds("heap-map-unselected").is_some());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.frames[1].heaps[0].chunks = None;
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    assert!(cx.debug_bounds("heap-map-unavailable").is_some());
    assert!(cx.debug_bounds("heap-map-strip").is_none());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.load(
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-data/heaps.json"),
                window,
                cx,
            );
            app.screen = Screen::Heap(HeapKey {
                address: 4096,
                name: "packet_heap".into(),
            });
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    assert!(
        cx.debug_bounds("heap-map-view-0-256").is_some(),
        "a new document starts with a fitted map"
    );
}
